//! The simulation worker thread (Phase B).
//!
//! The sim + terrain live here, stepped at their own fixed-tick pace off the render thread, so the
//! renderer presents at display rate no matter how hard the sim is working (the win at fast-forward).
//! The worker owns the authoritative `Sim`/`VoxelTerrain`; the main thread hands it a ready world
//! ([`SimCommand::LoadWorld`] — worldgen/load stay on the main side), drives it with commands, and
//! reads a [`RenderSnapshot`] published into a double-buffer each time the sim advances.
//!
//! DETERMINISM: the worker steps `sim.step` in fixed sub-steps exactly as the old inline loop did
//! (its `WorldClock` folds wall-clock × time_scale into whole ticks). Snapshots are read-only, so
//! they can't perturb the sim. The headless harness + tests drive `sim.step` directly and are
//! untouched.

use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use animata_sim::clock::WorldClock;
use animata_sim::config::{COLS, ROWS, VOX};
use animata_sim::sim::{Sim, SimState};
use animata_sim::sim_config::SimConfig;
use animata_sim::terrain::{TerrainState, VoxelTerrain};
use macroquad::math::Vec2;

use crate::render_snapshot::RenderSnapshot;

/// A request from the main (render) thread to the sim worker. All sim/terrain mutation goes through
/// here — the worker is the single writer, so there are no data races and save is consistent.
/// (Some variants are only constructed behind `--features dev`, hence the allow.)
#[allow(dead_code)]
pub enum SimCommand {
    /// Install a freshly generated/loaded world (worldgen + file I/O stay on the main side). `tick`
    /// is 0 for a reseed, the saved tick for a load.
    LoadWorld { sim: Box<Sim>, terrain: Box<VoxelTerrain>, tick: u64 },
    /// Time-scale / pause (a `None` field leaves that knob unchanged).
    SetClock { scale: Option<f32>, paused: Option<bool> },
    /// Per-frame render hint: the morphology AABB (request high-zoom body layouts within it). Cheap.
    SetViewport { body_near: Option<(Vec2, f32)> },
    /// Inspector selection (creature id), or `None` to clear.
    Select(Option<u64>),
    /// Graze tool: crop a column's biomass (the debug clear-cut).
    Graze { x: usize, y: usize, amount: f32 },
    /// Live config reload (features/params).
    SetConfig(SimConfig),
    /// Capture a consistent save state; the main thread writes the file. Replies `(seed, tick, sim, terrain)`.
    Save { reply: Sender<(u64, u64, SimState, TerrainState)> },
    /// Live biomass at a column (lazy regrow to the current tick) — for the dev bridge.
    QueryBiomass { x: usize, y: usize, reply: Sender<f32> },
    /// Full sim status at the worker's current tick (for the dev bridge `animata/status`). `col` is
    /// the camera-centre column whose live biomass to include.
    QueryStatus { col: (usize, usize), reply: Sender<Box<StatusReport>> },
    /// DEV / RENDER-BENCH: inflate the population to `n` (clone the evolved multicellular bodies)
    /// and PAUSE the clock — a pure render-load test isolated from the sim step.
    DebugInflate { n: usize },
}

/// A full status read of the sim at the worker's current tick (dev bridge `animata/status`).
#[allow(dead_code)] // fields read only behind `--features dev`
pub struct StatusReport {
    pub population: usize,
    pub avg_energy: f32,
    pub avg_biomass: f32,
    pub multi: f32,
    pub complex: f32,
    pub frac_carnivore: f32,
    pub frac_autotroph: f32,
    pub avg_nutrient: f32,
    pub allopatry: f32,
    pub crypsis: f32,
    pub species: usize,
    pub niche_coverage: usize,
    pub strata: [f32; 4],
    pub births: u64,
    pub deaths: u64,
    pub kills: u64,
    pub profile: Vec<(&'static str, f32, f32)>,
    /// Live serial fraction of a tick (Amdahl); core-scaling ceiling = `1 / serial_frac`.
    pub serial_frac: f32,
    pub env_biomass: f32,
}

/// Handle to the sim worker, held by the main thread.
pub struct SimHandle {
    cmd: Sender<SimCommand>,
    snapshot: Arc<Mutex<Option<Arc<RenderSnapshot>>>>,
    _thread: JoinHandle<()>,
}

impl SimHandle {
    /// Send a command (ignores send errors — the worker only goes away on shutdown).
    pub fn send(&self, cmd: SimCommand) {
        let _ = self.cmd.send(cmd);
    }

    /// The latest published snapshot, or `None` until the first world is installed + stepped.
    pub fn latest(&self) -> Option<Arc<RenderSnapshot>> {
        self.snapshot.lock().unwrap().clone()
    }
}

/// Spawn the sim worker. It idles (no world) until a [`SimCommand::LoadWorld`] arrives.
pub fn spawn() -> SimHandle {
    let (tx, rx) = std::sync::mpsc::channel::<SimCommand>();
    let snapshot: Arc<Mutex<Option<Arc<RenderSnapshot>>>> = Arc::new(Mutex::new(None));
    let snap_w = snapshot.clone();
    let thread = std::thread::Builder::new()
        .name("animata-sim".into())
        .spawn(move || worker(rx, snap_w))
        .expect("spawn sim worker");
    SimHandle { cmd: tx, snapshot, _thread: thread }
}

/// The whole live world on the worker: clock + sim + terrain.
struct World {
    clock: WorldClock,
    sim: Sim,
    terrain: VoxelTerrain,
}

fn worker(rx: Receiver<SimCommand>, snapshot: Arc<Mutex<Option<Arc<RenderSnapshot>>>>) {
    let mut world: Option<World> = None;
    let mut selected: Option<u64> = None;
    let mut body_near: Option<(Vec2, f32)> = None;
    let mut view_dirty = false; // a viewport/selection change ⇒ republish even without a step
    let mut last = Instant::now();

    loop {
        // Drain every pending command. Returns false on disconnect (main dropped the handle) ⇒ exit.
        let mut alive = true;
        loop {
            match rx.try_recv() {
                Ok(cmd) => apply(cmd, &mut world, &mut selected, &mut body_near, &mut view_dirty),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    alive = false;
                    break;
                }
            }
        }
        if !alive {
            return;
        }

        let Some(w) = world.as_mut() else {
            // No world yet — wait for one without busy-spinning.
            last = Instant::now();
            std::thread::sleep(Duration::from_millis(8));
            continue;
        };

        // Advance the sim at its fixed-tick pace (wall-clock × time_scale, capped) — same cadence the
        // old inline loop used, just on this thread.
        let dt = last.elapsed().as_secs_f32();
        last = Instant::now();
        let substeps = w.clock.substeps(dt);
        for _ in 0..substeps {
            w.clock.advance(1);
            let tick = w.clock.tick();
            w.sim.step(&mut w.terrain, tick);
        }

        // Publish a fresh snapshot when the sim advanced or a render hint changed.
        if substeps > 0 || view_dirty {
            view_dirty = false;
            let snap = RenderSnapshot::build(&w.sim, &w.terrain, w.clock.tick(), selected, body_near);
            *snapshot.lock().unwrap() = Some(Arc::new(snap));
        }

        // Don't spin: if nothing stepped (paused or caught up to real time) sleep a render frame.
        if substeps == 0 {
            std::thread::sleep(Duration::from_millis(4));
        }
    }
}

fn apply(
    cmd: SimCommand,
    world: &mut Option<World>,
    selected: &mut Option<u64>,
    body_near: &mut Option<(Vec2, f32)>,
    view_dirty: &mut bool,
) {
    match cmd {
        SimCommand::LoadWorld { sim, terrain, tick } => {
            let mut clock = WorldClock::new();
            clock.set_tick(tick);
            *world = Some(World { clock, sim: *sim, terrain: *terrain });
            *view_dirty = true;
        }
        SimCommand::SetClock { scale, paused } => {
            if let Some(w) = world {
                if let Some(s) = scale {
                    w.clock.time_scale = s.max(0.0);
                }
                if let Some(p) = paused {
                    w.clock.paused = p;
                }
            }
        }
        SimCommand::SetViewport { body_near: bn } => {
            if *body_near != bn {
                *view_dirty = true;
            }
            *body_near = bn;
        }
        SimCommand::Select(s) => {
            if *selected != s {
                *view_dirty = true;
            }
            *selected = s;
        }
        SimCommand::Graze { x, y, amount } => {
            if let Some(w) = world {
                let tick = w.clock.tick();
                w.terrain.graze(x, y, amount, tick);
                *view_dirty = true;
            }
        }
        SimCommand::SetConfig(cfg) => {
            if let Some(w) = world {
                w.sim.set_config(cfg);
            }
        }
        SimCommand::Save { reply } => {
            if let Some(w) = world {
                let _ = reply.send((
                    w.terrain.seed,
                    w.clock.tick(),
                    w.sim.to_state(),
                    w.terrain.clone_state(),
                ));
            }
        }
        SimCommand::QueryBiomass { x, y, reply } => {
            let v = world.as_ref().map(|w| w.terrain.biomass_at(x, y, w.clock.tick())).unwrap_or(0.0);
            let _ = reply.send(v);
        }
        SimCommand::QueryStatus { col, reply } => {
            if let Some(w) = world {
                let tick = w.clock.tick();
                let s = &w.sim;
                let t = &w.terrain;
                let (multi, complex) = s.complexity_mix();
                let report = StatusReport {
                    population: s.population(),
                    avg_energy: s.avg_energy(),
                    avg_biomass: s.avg_biomass(),
                    multi,
                    complex,
                    frac_carnivore: s.frac_carnivore(),
                    frac_autotroph: s.frac_autotroph(),
                    avg_nutrient: s.avg_nutrient(t, tick),
                    allopatry: s.thermal_correlation(t),
                    crypsis: s.crypsis_correlation(t),
                    species: s.species_count(),
                    niche_coverage: s.niche_coverage(t),
                    strata: s.stratum_mix(t),
                    births: s.births,
                    deaths: s.deaths,
                    kills: s.kills,
                    profile: s.profile_report().into_iter().map(|(sp, m, mx)| (sp.label(), m, mx)).collect(),
                    serial_frac: s.profile_amdahl().2,
                    env_biomass: t.biomass_at(col.0.min(COLS - 1), col.1.min(ROWS - 1), tick),
                };
                let _ = reply.send(Box::new(report));
            }
        }
        SimCommand::DebugInflate { n } => {
            if let Some(w) = world {
                let (maxx, maxy) = (COLS as f32 * VOX, ROWS as f32 * VOX);
                w.sim.debug_inflate_to(n, maxx, maxy);
                w.clock.paused = true; // freeze the sim: from here it's a pure render-load test
                *view_dirty = true;
            }
        }
    }
}
