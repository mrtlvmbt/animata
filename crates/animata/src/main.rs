//! animata — voxel isometric world (environment viewer).
//!
//! Reset from the former a-life simulation (archived at git tag `sim-v1` / branch
//! `archive/sim-v1`). The simulation and all GUI are intentionally OFF: this is a
//! bare environment viewer that grows a Minecraft-like voxel world on macroquad's
//! 3D pipeline (real geometry + GPU depth buffer).
//!
//! Phase 2: the terrain is rendered as **batched chunk meshes** — one cached `Mesh`
//! per chunk, built once from exposed faces only (each column's top + the cliff side
//! faces toward lower neighbours), with shading baked into vertex colours per face
//! normal. The GPU depth buffer handles all occlusion. Replaces the phase-1 pillar
//! preview. (Macro-culling / streaming come with the ×16 map; ~54 chunks draw fine.)

#[cfg(feature = "dev")]
mod dev_bridge;

#[cfg(target_os = "macos")]
mod mac_icon;

mod render;
mod render_snapshot;
mod sim_driver;
mod ui;

use render_snapshot::CreatureDot;
use sim_driver::SimCommand;

// The simulation + world model live in the graphics-free `animata-sim` crate. The renderer only
// needs these modules by name; the rest (genome/grid/rng/tectonics/erosion/hydrology) are internal
// to the sim. `Vec2` comes from the same glam major macroquad re-exports, so types line up.
use animata_sim::{clock, config, sim, terrain};
use animata_sim::persist::Snapshot;
use animata_sim::sim_config::SimConfig;

use clock::WorldClock;
use config::*;
use sim::Sim;
use macroquad::prelude::*;
use macroquad::miniquad::{PassAction, RenderingBackend, UniformsSource};
use terrain::VoxelTerrain;

use render::camera::{aabb_in_view, new_scene_target, IsoCam};
use render::gpu::{chunk_pipeline, water_pipeline, ChunkUniforms, GpuChunk, WaterUniforms};
use render::streamer::{center_chunk, spawn_gen, GenJob, Streamer, SUPER};

fn window_conf() -> Conf {
    Conf {
        window_title: "animata — voxel world".to_owned(),
        window_width: WIN_W,
        window_height: WIN_H,
        high_dpi: true,
        ..Default::default()
    }
}

// ---- Camera / input tuning (no more magic numbers smeared across the loop) ----
/// Tightest zoom-in (smallest visible world height); the camera never zooms closer than this.
const MIN_ZOOM: f32 = 8.0;
/// Mouse-wheel zoom step — fraction of the current zoom per wheel notch.
const ZOOM_STEP: f32 = 0.1;
/// Keyboard pan speed factor (× zoom × dt) — pans faster when zoomed out.
const PAN_SPEED: f32 = 0.5;
/// Right-drag graze patch radius in columns (the debug clear-cut tool).
const GRAZE_PATCH_R: i32 = 24;
/// Time-scale keys (`[` slower / `]` faster): multiplicative step and the clamp range. The real
/// ceiling is `MAX_SUBSTEPS` (the per-frame sim-step cap) — past ~24–48× (30–60 fps) extra scale just
/// drops backlog instead of simulating faster — so this cap sits above that, not as the limiter.
const TIME_SCALE_STEP: f32 = 1.5;
const MIN_TIME_SCALE: f32 = 0.1;
/// Absolute hard ceiling for the time-scale slider — a safety bound. The EFFECTIVE max floats with
/// CPU headroom (see `max_time_scale_for`): on a light world it climbs toward this; on a heavy one it
/// drops to what the sim can actually sustain.
const MAX_TIME_SCALE: f32 = 512.0;
/// Floor for the floating cap, so there's always some fast-forward even on a heavy world.
const MIN_AUTO_MAX: f32 = 2.0;
/// Discrete values for the manual `CEIL` stepper (the user's hard cap on the slider ceiling). The
/// effective slider ceiling is `min(CEIL, floating CPU cap)`.
/// Last entry is `INFINITY` = "MAX" (no manual cap → the slider tops out at the floating CPU cap).
const MAX_STEPS: [f32; 7] = [16.0, 32.0, 64.0, 128.0, 256.0, 512.0, f32::INFINITY];
/// Default `CEIL` = `MAX` (no manual cap; the floating CPU cap is the only ceiling).
const DEFAULT_CEIL: f32 = f32::INFINITY;
/// "At the ceiling" tolerance for the lock-follow decision.
const AT_MAX_EPS: f32 = 1e-6;

/// Largest time-scale the CPU can actually sustain right now, from the live per-tick cost. At scale
/// `M` the sim must do `M / TICK_LEN` ticks/s; the CPU does `1000 / tick_ms` ticks/s; keep-up ⇒
/// `M ≤ 1000·TICK_LEN / tick_ms`. Clamped to `[MIN_AUTO_MAX, MAX_TIME_SCALE]`; falls back to the hard
/// ceiling before the profiler has data.
fn max_time_scale_for(tick_ms: f32) -> f32 {
    if tick_ms > 0.05 {
        (1000.0 * TICK_LEN / tick_ms).clamp(MIN_AUTO_MAX, MAX_TIME_SCALE)
    } else {
        MAX_TIME_SCALE
    }
}
/// Whole-map zoom-out margin: the coarse tier covers all of it, so no empty edges however far out.
const MAX_ZOOM_MARGIN: f32 = 1.2;
// ---- Creature LOD (zoom-aware: individuals up close, bacterial mats when zoomed out) ----
/// Body radius in METRES per √(biomass cells). A single cell (biomass 1) ≈ this radius — the
/// documented creature scale is ~0.12 m (mouse-sized; see `config.rs` density contract), so a lone
/// microbe is a sub-decimetre speck that shrinks with zoom instead of a fixed fat pixel.
const CREATURE_RADIUS_M: f32 = 0.06;
/// Legibility floor (px) for an individual dot when zoomed in, so a lone microbe stays clickable.
const CREATURE_MIN_PX: f32 = 1.0;
/// LOD switch: below this many pixels-per-metre (zoomed out) individuals fall sub-pixel, so we draw
/// per-column bacterial-mat coverage instead of dots.
const LOD_MAT_PX_PER_M: f32 = 4.0;
/// Colony density (creatures in a column) at which a mat tile reaches its peak opacity.
const MAT_FULL_COUNT: f32 = 6.0;
/// Opacity ceiling of a mat tile, so terrain stays faintly readable under a dense colony.
const MAT_MAX_ALPHA: f32 = 0.85;
/// Min on-screen size (px) of one body cell before we draw the MORPHOLOGY (cluster of typed cells)
/// instead of a single dot — i.e. only when zoomed in close (few creatures on screen, cost bounded).
const BODY_CELL_MIN_PX: f32 = 2.5;

/// Colour for a body cell by type (`genome::body_layout` ids): structural = the creature's evolved
/// greyscale coloration (camouflage still reads), function cells get a distinct hue so organs show.
fn cell_color(cell_type: u8, coloration: f32) -> Color {
    match cell_type {
        1 => Color::new(0.85, 0.25, 0.20, 1.0), // effector — muscle red
        2 => Color::new(0.90, 0.80, 0.20, 1.0), // storage — yellow
        3 => Color::new(0.20, 0.80, 0.90, 1.0), // sensor — cyan
        4 => Color::new(0.85, 0.20, 0.70, 1.0), // predator — magenta
        5 => Color::new(0.60, 0.70, 1.00, 1.0), // flight — sky blue
        6 => Color::new(0.55, 0.40, 0.25, 1.0), // burrow — brown
        7 => Color::new(0.30, 0.80, 0.30, 1.0), // photo — green
        _ => Color::new(coloration, coloration, coloration, 1.0), // structural — evolved coloration
    }
}
/// Default save file (cwd) for the `F5`/`F9` quick-save keys and the path-less dev-bridge save/load.
const SAVE_PATH: &str = "animata-save.bin";

/// Write a full-state snapshot of the running world to `path` (geometry is regenerated from the seed
/// on load, so only the creatures + terrain overlay + tick are stored). Human-readable error on fail.
fn save_world(
    path: &str,
    seed: u64,
    tick: u64,
    sim: sim::SimState,
    terrain: animata_sim::terrain::TerrainState,
) -> Result<(), String> {
    let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
    Snapshot::new(seed, tick, sim, terrain).write(std::io::BufWriter::new(f))
}

/// Read a snapshot from `path` (does not yet apply it — the caller regenerates terrain from the
/// snapshot's seed and restores the state).
fn load_snapshot(path: &str) -> Result<Snapshot, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    Snapshot::read(std::io::BufReader::new(f))
}

/// A snapshot load running on a background thread, so `F9` never blocks the render loop — the slow
/// part is regenerating terrain geometry from the saved seed, exactly like a reseed. The worker
/// reads + parses the file, regenerates terrain, applies the overlay, and ships the ready pieces
/// back; the main thread polls `rx` each frame and reads `progress` (permille) for the same bar the
/// generator uses. The current world stays live and interactive until the load is ready.
struct LoadJob {
    rx: std::sync::mpsc::Receiver<Result<LoadedWorld, String>>,
    progress: std::sync::Arc<std::sync::atomic::AtomicU32>,
}

/// The fully-prepared world a [`LoadJob`] ships back. Applied on the main thread (the only
/// GL-touching step — the streamer reset — stays there); `set_state` already ran on the worker, so a
/// size mismatch surfaces as the channel's `Err` before anything here is swapped in.
struct LoadedWorld {
    terrain: VoxelTerrain,
    sim: sim::SimState,
    tick: u64,
    seed: u64,
}

/// Kick off a background load of `path` (mirrors [`spawn_gen`](render::streamer::spawn_gen)). File
/// read + parse + terrain regen + overlay restore all run off the main thread.
fn spawn_load(path: String) -> LoadJob {
    use std::sync::atomic::Ordering;
    let progress = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let (tx, rx) = std::sync::mpsc::channel();
    let p = progress.clone();
    std::thread::spawn(move || {
        let res = (|| -> Result<LoadedWorld, String> {
            let t0 = std::time::Instant::now();
            let snap = load_snapshot(&path)?;
            let parse_ms = t0.elapsed().as_secs_f64() * 1000.0;
            let seed = snap.terrain_seed;
            let t1 = std::time::Instant::now();
            let mut terrain = VoxelTerrain::generate(seed, &|f| {
                p.store((f.clamp(0.0, 1.0) * 1000.0) as u32, Ordering::Relaxed);
            });
            let gen_ms = t1.elapsed().as_secs_f64() * 1000.0;
            let t2 = std::time::Instant::now();
            terrain.set_state(snap.terrain)?; // size-checked; Err aborts before the main swap
            let set_ms = t2.elapsed().as_secs_f64() * 1000.0;
            eprintln!("[load] parse {parse_ms:.0}ms · generate {gen_ms:.0}ms · set_state {set_ms:.1}ms (worker)");
            Ok(LoadedWorld { terrain, sim: snap.sim, tick: snap.tick, seed })
        })();
        let _ = tx.send(res); // receiver may be gone if the app exited mid-load — ignore
    });
    LoadJob { rx, progress }
}

/// Max zoom-out (visible world height): frame the whole map with margin — the coarse tier
/// covers all of it, so there are no empty edges however far out you go.
fn max_zoom() -> f32 {
    COLS.max(ROWS) as f32 * VOX * MAX_ZOOM_MARGIN
}

/// The ground-plane point (returned as `(x, z)`) under the mouse cursor: unproject the
/// cursor through the camera and intersect the ray with `y = 0`. Used for zoom-to-cursor.
fn ground_under_cursor(cam: &IsoCam) -> Vec2 {
    let (mx, my) = mouse_position();
    let (sw, sh) = (screen_width().max(1.0), screen_height().max(1.0));
    let nx = mx / sw * 2.0 - 1.0;
    let ny = 1.0 - my / sh * 2.0; // screen Y is top-down; NDC Y is bottom-up
    let inv = cam.camera().matrix().inverse();
    let near = inv.project_point3(vec3(nx, ny, -1.0));
    let far = inv.project_point3(vec3(nx, ny, 1.0));
    let d = far - near;
    let t = if d.y.abs() > 1e-6 { -near.y / d.y } else { 0.0 };
    let hit = near + d * t;
    vec2(hit.x, hit.z)
}

/// Project a world point to screen (logical px), or `None` if behind the camera / off-screen.
/// Single source for both the creature render pass and the inspector crosshair projection.
fn project_world(vp: Mat4, sw: f32, sh: f32, w: Vec3) -> Option<[f32; 2]> {
    let clip = vp * vec4(w.x, w.y, w.z, 1.0);
    if clip.w <= 0.0 {
        return None;
    }
    let (nx, ny) = (clip.x / clip.w, clip.y / clip.w);
    if !(-1.0..=1.0).contains(&nx) || !(-1.0..=1.0).contains(&ny) {
        return None;
    }
    Some([(nx * 0.5 + 0.5) * sw, (1.0 - (ny * 0.5 + 0.5)) * sh])
}

/// Top-of-column world point for a creature (its dot's render anchor), for projection.
pub(crate) fn creature_world(c: &sim::Creature, terrain: &VoxelTerrain) -> Vec3 {
    let (cx, cy) = sim::column_index(c.pos);
    let wy = terrain.height(cx as i32, cy as i32) as f32 * VOX + 0.5;
    vec3(c.pos.x, wy, c.pos.y)
}

/// Screen-space creature pick: nearest creature to the cursor within its own on-screen radius
/// (same radius the render pass draws), or `None` on a miss. Used to set the inspector selection.
/// Reads the render snapshot (not `Sim`) so the seam is the only source of creature positions.
fn pick_creature(cam: &IsoCam, creatures: &[CreatureDot], terrain: Option<&VoxelTerrain>) -> Option<u64> {
    let terrain = terrain?;
    let (sw, sh) = (screen_width(), screen_height());
    let px_per_m = sh / cam.zoom;
    let vp = cam.camera().matrix();
    let (mx, my) = mouse_position();
    let mut best: Option<(f32, u64)> = None;
    for c in creatures {
        let (cx, cy) = sim::column_index(c.pos);
        let wy = terrain.height(cx as i32, cy as i32) as f32 * VOX + 0.5;
        let Some(p) = project_world(vp, sw, sh, vec3(c.pos.x, wy, c.pos.y)) else {
            continue;
        };
        let r = (CREATURE_RADIUS_M * (c.biomass as f32).sqrt() * px_per_m).max(6.0);
        let d = ((p[0] - mx).powi(2) + (p[1] - my).powi(2)).sqrt();
        if d <= r && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, c.id));
        }
    }
    best.map(|(_, id)| id)
}

/// SplitMix64 — deterministic per-creature hashing for the inspector's mock fields (name, id,
/// generation, offspring), so each creature reads stably and alive (not a static placeholder).
fn hash64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Stable morphotype tag from a lineage founder, e.g. "AX-7" (mock — the sim has no morphotype id).
fn morphotype_id(founder: u64) -> String {
    let h = hash64(founder);
    let a = (b'A' + (h % 26) as u8) as char;
    let b = (b'A' + ((h >> 8) % 26) as u8) as char;
    let num = (h >> 16) % 90 + 1;
    format!("{a}{b}-{num}")
}

/// Stable pseudo-Latin species name from a lineage founder (mock — no species names in the sim).
fn species_name(founder: u64) -> String {
    const ON: [&str; 8] = ["Pel", "Cry", "Ther", "Lim", "Nyx", "Vor", "Aqu", "Strat"];
    const TW: [&str; 8] = ["arctos", "odon", "ophis", "ictis", "ander", "opter", "ursa", "ceps"];
    let h = hash64(founder ^ 0xABCD);
    format!("{}{}", ON[(h % 8) as usize], TW[((h >> 8) % 8) as usize])
}

/// Build the inspector snapshot for a creature. Real fields read from the phenotype/genome; fields
/// the sim doesn't model (name/generation/health/hydration/activity/offspring) are derived
/// deterministically so they stay stable per-creature and look alive (consensus: derive, not static).
pub(crate) fn creature_view(c: &sim::Creature, terrain: &VoxelTerrain) -> ui::CreatureView {
    use animata_sim::genome::MAX_CELLS;
    let p = &c.pheno;
    let n = p.n_cells.max(1) as f32;
    let frac = |x: u32| (x as f32 / n).clamp(0.0, 1.0);

    // Classify through the sim's single source of truth so the inspector and the population panel
    // always agree on a creature's diet.
    let kind = match animata_sim::genome::TrophicNiche::classify(p) {
        animata_sim::genome::TrophicNiche::Autotroph => ui::TrophicKind::Autotroph,
        animata_sim::genome::TrophicNiche::Carnivore => ui::TrophicKind::Carnivore,
        _ => ui::TrophicKind::Herbivore,
    };
    let diet = match kind {
        ui::TrophicKind::Autotroph => "Phototroph",
        ui::TrophicKind::Carnivore => "Carnivore",
        ui::TrophicKind::Herbivore => "Grazer",
    }
    .to_string();

    let (cx, cy) = sim::column_index(c.pos);
    let is_water = terrain.is_water(cx, cy);
    let strata = sim::stratum_of(p, is_water).name().to_string();
    let (fl, bu, ef) = (frac(p.flight), frac(p.burrow), frac(p.effector));
    let locomotion = if is_water && fl < STRATUM_THETA && bu < STRATUM_THETA {
        "Amphibious"
    } else if fl >= bu && fl >= ef && fl > 0.05 {
        "Drifter"
    } else if bu >= ef && bu > 0.05 {
        "Fossorial"
    } else if ef > 0.05 {
        "Cursorial"
    } else {
        "Sessile"
    }
    .to_string();

    let energy = c.energy_frac();
    let health = (0.45 + 0.55 * energy).clamp(0.0, 1.0); // placeholder: no health stat in the sim
    let hydration = terrain.moisture_at(cx, cy).clamp(0.0, 1.0); // semi-real: local moisture
    let age_days = c.age as f32 * TICK_LEN / DAY_LEN;
    let mass = c.biomass() as f32 * 0.1;
    let activity = match (kind, energy) {
        (_, e) if e < 0.25 => "Resting",
        (ui::TrophicKind::Carnivore, _) => "Hunting",
        (ui::TrophicKind::Autotroph, _) => "Basking",
        _ => "Foraging",
    }
    .to_string();
    let traits = [
        ("metabolism", (p.n_cells as f32 / MAX_CELLS as f32).clamp(0.0, 1.0)),
        ("speed", frac(p.effector)),
        ("sense", frac(p.sensor)),
        ("aggression", frac(p.predator)),
        ("fertility", frac(p.storage)),
        ("crypsis", c.coloration().clamp(0.0, 1.0)),
    ];

    ui::CreatureView {
        id: morphotype_id(c.founder),
        name: species_name(c.founder),
        kind,
        generation: ((hash64(c.founder) >> 16) % 400) as u32,
        age: format!("{age_days:.1} d"),
        diet,
        mass: format!("{mass:.1} kg"),
        locomotion,
        strata,
        energy,
        health,
        hydration,
        traits,
        activity,
        offspring: (hash64(c.id) % 13) as u32,
    }
}

/// Debug overlay selected by `G` (cycles in this order). `Topo` reshades the 3D scene on the
/// GPU; the climate / water-distance views overlay a per-column colourmap MINIMAP — the live
/// in-app consumer of the S1 environment getters.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DebugView {
    None,
    Topo,
    Temp,
    Moist,
    WaterDist,
    Slope,
    Biomass,
}

impl DebugView {
    fn next(self) -> Self {
        match self {
            DebugView::None => DebugView::Topo,
            DebugView::Topo => DebugView::Temp,
            DebugView::Temp => DebugView::Moist,
            DebugView::Moist => DebugView::WaterDist,
            DebugView::WaterDist => DebugView::Slope,
            DebugView::Slope => DebugView::Biomass,
            DebugView::Biomass => DebugView::None,
        }
    }
    /// The views drawn as a 2D field minimap (vs the 3D scene reshade / no overlay). Used by the
    /// egui minimap panel to recolour the preview and show the field legend.
    fn is_field_map(self) -> bool {
        matches!(
            self,
            DebugView::Temp | DebugView::Moist | DebugView::WaterDist | DebugView::Slope | DebugView::Biomass
        )
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    let mut cam = IsoCam::new();
    let mut seed: u64 = 1;
    // The world is generated on a background thread so the first frame (and every regen)
    // never blocks the render loop. `terrain` is `None` until the initial job finishes.
    let mut terrain: Option<VoxelTerrain> = None;
    let mut gen: Option<GenJob> = Some(spawn_gen(seed));
    // A background snapshot load (`F9`), polled like `gen`; the current world stays live until ready.
    let mut load: Option<LoadJob> = None;
    // Full-screen modal loader overlay: mirrors the active gen/load job (kind + its permille
    // progress). `done_at` is set when the job finishes, holding the "100% / last step done" frame
    // for 340 ms before the overlay is dismissed and the finish toast fires.
    struct Loading {
        kind: ui::loader::LoadKind,
        progress: std::sync::Arc<std::sync::atomic::AtomicU32>,
        done_at: Option<f64>,
    }
    let mut loading: Option<Loading> = gen.as_ref().map(|j| Loading {
        kind: ui::loader::LoadKind::Gen,
        progress: j.progress.clone(),
        done_at: None,
    });

    // The default sim config, loaded from assets/config/sim.ron (editable without a rebuild); falls
    // back to the built-in default if the file is missing or malformed. Live dev-bridge changes
    // apply on top until the next reseed.
    let mut sim_cfg = match macroquad::file::load_string("assets/config/sim.ron").await {
        Ok(s) => SimConfig::from_ron(&s).unwrap_or_else(|e| {
            eprintln!("[config] assets/config/sim.ron: {e}; using defaults");
            SimConfig::default()
        }),
        Err(_) => SimConfig::default(),
    };

    // Chunk meshes are STREAMED around the camera (see `Streamer`) rather than all built
    // up front — the world model is fully resident but the meshes are not, so a ×16 map
    // stays within memory. The streamer fills in each frame from `terrain`.
    // Shaders live in assets/shaders/ (editable without a rebuild); fall back to the copy baked in
    // with `include_str!` if the asset isn't reachable (e.g. running outside the repo).
    let chunk_vert = load_shader("assets/shaders/chunk.vert", include_str!("../../../assets/shaders/chunk.vert")).await;
    let chunk_frag = load_shader("assets/shaders/chunk.frag", include_str!("../../../assets/shaders/chunk.frag")).await;
    let water_vert = load_shader("assets/shaders/water.vert", include_str!("../../../assets/shaders/water.vert")).await;
    let water_frag = load_shader("assets/shaders/water.frag", include_str!("../../../assets/shaders/water.frag")).await;

    let pipeline;
    let water_pipe;
    let mut streamer = Streamer::new();
    {
        let InternalGlContext {
            quad_context: ctx, ..
        } = unsafe { get_internal_gl() };
        pipeline = chunk_pipeline(ctx, &chunk_vert, &chunk_frag);
        water_pipe = water_pipeline(ctx, &water_vert, &water_frag);
    }

    // The scene is rendered into this offscreen target every frame, then blitted to
    // the window. A screenshot reads the target's texture directly — i.e. the
    // finished pixels *before* the window present — so capture is decoupled from the
    // window back-buffer (GRAV-style framebuffer read) instead of `get_screen_data`,
    // which only sees the throttled front buffer of a foregrounded window.
    // NB: it MUST have its own depth attachment (`depth: true`) — the bare
    // `render_target()` has none, which silently disables depth testing in the pass
    // and lets far faces overdraw near ones.
    let mut scene_rt = new_scene_target(screen_width() as u32, screen_height() as u32);

    // Frame timing: a sliding-window mean over the last `FPS_WINDOW` frames (steadier than an EMA —
    // one slow frame can't visibly yank the readout). `fps`/`frame_ms` are recomputed from the ring.
    const FPS_WINDOW: usize = 60;
    let mut dt_ring: std::collections::VecDeque<f32> = std::collections::VecDeque::with_capacity(FPS_WINDOW);
    let mut fps = 0.0f32;
    let mut frame_ms = 0.0f32;
    // GUI toggle state (egui widgets + keyboard hotkeys flip the same fields); snapshotted into
    // Copy locals each frame so the render pass below reads plain `debug_view`/`mask`/… as before.
    let mut ui_state = ui::UiState {
        show_info: true,
        debug_view: DebugView::None,
        water_on: true,
        mask: false,
        outline: true,
        open_panel: None,
        selected: None,
        lock_max: true,
        manual_ceil: DEFAULT_CEIL,
    };
    // The ceiling the slider value was sitting against last frame — the lock-follow decision tests
    // "was the value at the ceiling" against THIS (pre-change) value, so the value rides a moving
    // ceiling in both directions (must be read before the ceiling is recomputed).
    let mut eff_max_prev = DEFAULT_CEIL;
    // Whether the value rode the ceiling last frame (lock on + at-max). Drives the lock-toggle armed
    // dot — computed here (where `time_scale == eff_max` exactly) rather than in the HUD, where a
    // cross-frame `time_scale`/`max` mismatch made the dot flicker against the jittering CPU cap.
    let mut armed_prev = false;
    // Population sparkline buffer: last 48 samples, pushed on a tick cadence (freezes when paused).
    let mut pop_hist: std::collections::VecDeque<f32> = std::collections::VecDeque::with_capacity(48);
    let mut pop_last_tick: u64 = u64::MAX;
    // Perf counters are produced DURING render (after the UI pass); the next frame's panels read
    // them with a one-frame lag — invisible on an fps/draw readout, and it keeps `wants_pointer`
    // fresh for input gating (the UI pass runs before world mouse input).
    let mut drawn = 0usize;
    let mut on_screen = 0usize;
    // Fonts/style are installed on the first egui pass (egui keeps them for the context lifetime).
    let mut fonts_set = false;
    // Persistent HUD GPU resources (the minimap egui texture), held across frames.
    let mut hud_cache = ui::HudCache::default();
    // Transient top-centre system notice (save/load feedback): (message, start time in `get_time()`
    // seconds). The HUD derives the slide-in + fade from the elapsed time; cleared after its life.
    let mut toast: Option<(String, f64)> = None;
    const TOAST_LIFE_MS: f32 = 2600.0;
    // Sim time base (S2). The main loop schedules fixed sub-steps from the real frame `dt`
    // (`clock.substeps`) and drives one `sim.step` per sub-step; `P` pauses. `advance` stays a
    // pure counter (HUD/day-frac). The creature sim (C0) is created once the world is ready.
    // The sim + terrain now live on a worker thread (see `sim_driver`); this is just the render-side
    // mirror. `clock` holds the DISPLAY/intent time (its `tick` is set from the snapshot each frame;
    // `time_scale`/`paused` are the user's intent, echoed to the worker via `SetClock`). `terrain`
    // below is a geo-only render-side copy (shared `Arc<TerrainGeo>`), repopulated on world swap.
    let mut clock = WorldClock::new();
    let sim_handle = sim_driver::spawn();
    let mut snapshot: Option<std::sync::Arc<render_snapshot::RenderSnapshot>>;
    // `G` cycles the debug view: off → Topo (GPU height/depth, water hidden) → Temp → Moist
    // → WaterDist → off. Topo reshades the 3D scene; the climate/water-dist modes overlay a
    // colourmap MINIMAP of the per-column field (the live consumer of the S1 env getters, so
    // they verify visually — poles cold / equator hot — and aren't dead code in any build).
    // `H` hides the translucent water surface; `J` toggles the WATER/LAND mask (land grey,
    // flagged water blue — dry holes inside blue flag a gen bug); `O` toggles the dark step-edge
    // outline. All three live in `ui_state` now (checkbox + hotkey share the field).
    // Left-drag pans the map: the ground point grabbed on press stays under the cursor.
    let mut grab: Option<Vec2> = None;

    // Dev bridge: localhost JSON-RPC for driving/inspecting the viewer (see
    // DEV_BRIDGE.md). Off unless built with `--features dev`.
    #[cfg(feature = "dev")]
    let bridge = dev_bridge::spawn(dev_bridge::port());
    #[cfg(feature = "dev")]
    let mut pending_shots: Vec<(String, bool, std::sync::mpsc::Sender<serde_json::Value>)> = Vec::new();

    // Last applied Dock-icon appearance (macOS); `None` until the first sync sets it.
    #[cfg(target_os = "macos")]
    let mut icon_dark: Option<bool> = None;

    loop {
        let dt = get_frame_time();
        #[cfg(target_os = "macos")]
        mac_icon::sync(&mut icon_dark);
        // Pick up a finished background world (non-blocking). On readiness, swap it in and
        // reset the streamer so meshes rebuild around the camera from the new terrain.
        if let Some(job) = &gen {
            if let Ok(t) = job.rx.try_recv() {
                // Seed the creature population from the new world (deterministic from its seed), keep a
                // geo-only render-side terrain, and hand the authoritative world to the sim worker.
                let sim_world = Sim::with_config(seed, &t, sim_cfg);
                terrain = Some(VoxelTerrain::render_side(seed, t.chunks_x, t.chunks_y, t.geo()));
                sim_handle.send(SimCommand::LoadWorld {
                    sim: Box::new(sim_world),
                    terrain: Box::new(t),
                    tick: 0,
                });
                clock.set_tick(0);
                gen = None;
                let InternalGlContext { quad_context: ctx, .. } = unsafe { get_internal_gl() };
                streamer.clear(ctx);
                // Hold the loader on "done" for 340 ms; it fires `World ready` and dismisses below.
                if let Some(ld) = &mut loading {
                    ld.done_at = Some(get_time());
                }
            }
        }
        // Pick up a finished background load the same way (restore overlay + creatures + tick).
        if let Some(job) = &load {
            if let Ok(res) = job.rx.try_recv() {
                match res {
                    Ok(w) => {
                        seed = w.seed;
                        sim_cfg = w.sim.cfg;
                        terrain = Some(VoxelTerrain::render_side(seed, w.terrain.chunks_x, w.terrain.chunks_y, w.terrain.geo()));
                        clock.set_tick(w.tick);
                        sim_handle.send(SimCommand::LoadWorld {
                            sim: Box::new(Sim::from_state(w.sim)),
                            terrain: Box::new(w.terrain),
                            tick: w.tick,
                        });
                        let InternalGlContext { quad_context: ctx, .. } = unsafe { get_internal_gl() };
                        streamer.clear(ctx);
                        eprintln!("[load] restored {SAVE_PATH} at tick {}", w.tick);
                        // Hold the loader on "done"; `Loaded` toast fires on dismissal below.
                        if let Some(ld) = &mut loading {
                            ld.done_at = Some(get_time());
                        }
                    }
                    Err(e) => {
                        eprintln!("[load] failed: {e}");
                        toast = Some((format!("Load failed: {e}"), get_time()));
                        loading = None; // abort the overlay; no success hold on failure
                    }
                }
                load = None;
            }
        }
        // Dismiss the loader 340 ms after the job finished (the "done" hold), then fire the toast.
        if let Some(ld) = &loading {
            if let Some(t0) = ld.done_at {
                if (get_time() - t0) * 1000.0 >= 340.0 {
                    let msg = match ld.kind {
                        ui::loader::LoadKind::Gen => "World ready",
                        ui::loader::LoadKind::Load => "Loaded",
                    };
                    toast = Some((msg.into(), get_time()));
                    loading = None;
                }
            }
        }
        // Smooth the readouts with a sliding-window mean so they don't jitter: push this frame's dt,
        // drop the oldest past the window, then derive both from the window average.
        if dt > 0.0 {
            if dt_ring.len() == FPS_WINDOW {
                dt_ring.pop_front();
            }
            dt_ring.push_back(dt);
        }
        if !dt_ring.is_empty() {
            let mean_dt = dt_ring.iter().sum::<f32>() / dt_ring.len() as f32;
            frame_ms = mean_dt * 1000.0;
            fps = 1.0 / mean_dt;
        }
        // The sim advances on its worker thread; pull the latest snapshot it published and mirror its
        // tick into the display clock (so `sim_time`/`day_frac` read right). The render pass, HUD and
        // picking all read this snapshot — never the live sim.
        snapshot = sim_handle.latest();
        if let Some(s) = &snapshot {
            clock.set_tick(s.tick);
        }
        // Floating time-scale cap from the live per-tick cost (serial+parallel ms): the slider tops
        // out at what the CPU can actually sustain, climbing on a light world and dropping on a heavy
        // one. `None` snapshot ⇒ hard ceiling.
        let sim_amdahl = snapshot.as_ref().map(|s| s.amdahl).unwrap_or((0.0, 0.0, 0.0));
        let max_ts = max_time_scale_for(sim_amdahl.0 + sim_amdahl.1);
        // EFFECTIVE slider ceiling = floating CPU cap clamped by the user's manual CEIL.
        let eff_max = max_ts.min(ui_state.manual_ceil);
        // Tell the worker which creatures need a high-zoom morphology layout next snapshot (the AABB is
        // a generous superset of the view; the render pass culls precisely).
        let px_per_m = screen_height() / cam.zoom;
        let cell_px = CREATURE_RADIUS_M * px_per_m * 2.0;
        let body_near = (px_per_m >= LOD_MAT_PX_PER_M && cell_px >= BODY_CELL_MIN_PX)
            .then(|| (vec2(cam.target.x, cam.target.z), cam.zoom * 2.0));
        sim_handle.send(SimCommand::SetViewport { body_near });
        // Inspector selection intent → worker (it builds the inspect bundle for this id next snapshot).
        sim_handle.send(SimCommand::Select(ui_state.selected));

        // ---- GUI pass (egui) — runs before world input so `wants_pointer` gates the mouse.
        // Perf counters (`drawn`/`on_screen`) come from LAST frame's render (produced after this
        // pass); `det`/`crs` read the current streamer state.
        let det = streamer.detail.len();
        let crs = streamer.coarse.len();
        // Population/evolution stats now come from the render snapshot (built above from the sim).
        let life = snapshot.as_ref().and_then(|s| s.life.clone());
        // Expire the transient save notice; otherwise hand the HUD the elapsed ms (it owns the fade).
        let toast_view = match &toast {
            Some((msg, start)) => {
                let dt = ((get_time() - *start) as f32) * 1000.0;
                if dt >= TOAST_LIFE_MS {
                    toast = None;
                    None
                } else {
                    Some((msg.clone(), dt))
                }
            }
            None => None,
        };
        // Sparkline sample on a tick cadence (~every 10 ticks); deduped by tick so it freezes on
        // pause and is independent of fps/time_scale.
        if let Some(l) = &life {
            let tnow = clock.tick();
            if pop_last_tick == u64::MAX || tnow >= pop_last_tick.wrapping_add(10) {
                if pop_hist.len() == 48 {
                    pop_hist.pop_front();
                }
                pop_hist.push_back(l.population as f32);
                pop_last_tick = tnow;
            }
        }
        // Visible-world quad on the map for the minimap viewport frame: unproject the 4 screen
        // corners onto the ground plane (exact at any yaw — the azimuth-45° view is a rotated quad,
        // not an axis-aligned box), expressed as map-space fractions.
        let minimap_view = {
            let inv = cam.camera().matrix().inverse();
            let (mw, mh) = (COLS as f32 * VOX, ROWS as f32 * VOX);
            [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                .iter()
                .map(|&(nx, ny)| {
                    let near = inv.project_point3(vec3(nx, ny, -1.0));
                    let far = inv.project_point3(vec3(nx, ny, 1.0));
                    let d = far - near;
                    let t = if d.y.abs() > 1e-6 { -near.y / d.y } else { 0.0 };
                    let hit = near + d * t;
                    [hit.x / mw, hit.z / mh]
                })
                .collect::<Vec<_>>()
        };
        // Inspector: the snapshot carries the view + WORLD anchors; project them to screen here with
        // the current camera (≤1-frame crosshair lag, imperceptible). Selection-death already handled
        // above (snapshot.inspect is None for a dead id).
        let (inspect, inspect_screen, conspecific_screen) = match snapshot.as_ref().and_then(|s| s.inspect.as_ref())
        {
            Some(iv) => {
                let vp = cam.camera().matrix();
                let (sw, sh) = (screen_width(), screen_height());
                let scr = iv.world.and_then(|w| project_world(vp, sw, sh, w));
                let cons: Vec<[f32; 2]> =
                    iv.conspecific_world.iter().filter_map(|&w| project_world(vp, sw, sh, w)).collect();
                (Some(iv.view.clone()), scr, cons)
            }
            None => (None, None, Vec::new()),
        };
        // Per-phase sim timing (mean ms) for the perf panel — from the snapshot.
        let sim_phases: Vec<(&'static str, f32)> =
            snapshot.as_ref().map(|s| s.sim_phases.clone()).unwrap_or_default();
        let hud_metrics = ui::SimMetrics {
            fps,
            frame_ms,
            drawn,
            detail: det,
            coarse: crs,
            on_screen,
            sim_phases,
            sim_amdahl,
            seed,
            cols: COLS,
            rows: ROWS,
            tick: snapshot.as_ref().map(|s| s.tick).unwrap_or_else(|| clock.tick()),
            sim_time: clock.sim_time() as f32,
            day_frac: clock.day_frac(),
            time_scale: clock.time_scale,
            max_time_scale: eff_max,
            manual_ceil: ui_state.manual_ceil,
            lock_max: ui_state.lock_max,
            armed: armed_prev,
            paused: clock.paused,
            life,
            pop_hist: pop_hist.iter().copied().collect(),
            minimap_view,
            toast: toast_view,
            inspect,
            inspect_screen,
            conspecific_screen,
        };
        // Loader overlay view: real permille progress (1.0 once done), step derived from the
        // fraction. `loader_active` makes the world input modal while it's up.
        let loader_view = loading.as_ref().map(|ld| {
            let p = if ld.done_at.is_some() {
                1.0
            } else {
                (ld.progress.load(std::sync::atomic::Ordering::Relaxed) as f32 / 1000.0).clamp(0.0, 1.0)
            };
            let n = match ld.kind {
                ui::loader::LoadKind::Gen => 5,
                ui::loader::LoadKind::Load => 4,
            };
            let idx = ((p * n as f32).floor() as usize).min(n - 1);
            (ld.kind, p, idx)
        });
        let loader_active = loading.is_some();
        let mut actions = ui::UiActions::default();
        egui_macroquad::ui(|ctx| {
            // Register IBM Plex + Phosphor and the global style ONCE (egui keeps them for the
            // context's lifetime). Fonts are vendored in assets/fonts/ (OFL), baked via include_bytes.
            if !fonts_set {
                ui::theme::install_fonts(
                    ctx,
                    include_bytes!("../../../assets/fonts/IBMPlexSans-Regular.ttf"),
                    include_bytes!("../../../assets/fonts/IBMPlexMono-Regular.ttf"),
                );
                ui::theme::install_style(ctx);
                fonts_set = true;
            }
            // high_dpi=true ⇒ macroquad reports physical px; match egui's scale so panels aren't
            // tiny on Retina (F2).
            ctx.set_pixels_per_point(macroquad::miniquad::window::dpi_scale());
            // While the modal loader is up it fully owns the screen — skip the HUD entirely (the
            // scrim isn't perfectly opaque in egui_macroquad, so drawing the HUD under it would
            // bleed through). Input is gated separately on `loader_active`.
            if let Some((kind, p, idx)) = loader_view {
                ui::loader::draw(ctx, kind, p, idx, seed);
            } else {
                actions = ui::draw_hud(
                    ctx,
                    &mut ui_state,
                    &hud_metrics,
                    &mut hud_cache,
                    terrain.as_ref(),
                    get_time() as f32,
                );
            }
        });
        let wants_ptr = actions.wants_pointer;

        // ---- Input ---- World hotkeys + mouse, gated while the modal loader is up (it's drawn
        // foreground and eats the pointer; we also block keys here so nothing reaches the world).
        // egui widgets flip the same `ui_state`.
        if !loader_active {
        if is_key_pressed(KeyCode::I) {
            ui_state.show_info = !ui_state.show_info;
        }
        if is_key_pressed(KeyCode::G) {
            ui_state.debug_view = ui_state.debug_view.next();
        }
        if is_key_pressed(KeyCode::H) {
            ui_state.water_on = !ui_state.water_on;
        }
        if is_key_pressed(KeyCode::Space) || actions.toggle_pause {
            clock.paused = !clock.paused;
        }
        // Time speed: `,` slows, `.` speeds (multiplicative), `/` resets to 1×; the panel slider/
        // buttons feed `actions.set_time_scale`.
        if is_key_pressed(KeyCode::Comma) {
            clock.time_scale = (clock.time_scale / TIME_SCALE_STEP).max(MIN_TIME_SCALE);
        }
        if is_key_pressed(KeyCode::Period) {
            clock.time_scale = (clock.time_scale * TIME_SCALE_STEP).min(eff_max);
        }
        if is_key_pressed(KeyCode::Slash) {
            clock.time_scale = 1.0;
        }
        if let Some(ts) = actions.set_time_scale {
            clock.time_scale = ts.clamp(MIN_TIME_SCALE, MAX_TIME_SCALE);
        }
        // Ceiling-lock controls: `L` toggles, `Shift+[`/`Shift+]` step the manual CEIL.
        if actions.toggle_lock || is_key_pressed(KeyCode::L) {
            ui_state.lock_max = !ui_state.lock_max;
        }
        let shift = is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift);
        let step = actions
            .step_ceil
            .or_else(|| match (shift && is_key_pressed(KeyCode::RightBracket), shift && is_key_pressed(KeyCode::LeftBracket)) {
                (true, _) => Some(1),
                (_, true) => Some(-1),
                _ => None,
            });
        let ceil_changed = step.is_some();
        if let Some(dir) = step {
            let i = MAX_STEPS.iter().position(|&x| x == ui_state.manual_ceil).unwrap_or(MAX_STEPS.len() - 1) as i32;
            let j = (i + dir).clamp(0, MAX_STEPS.len() as i32 - 1) as usize;
            ui_state.manual_ceil = MAX_STEPS[j];
        }
        // Lock-follow: decide AGAINST the ceiling the value sat at last frame (`eff_max_prev`) — BEFORE
        // recomputing it — so a value pinned at the ceiling rides it up or down. Then clamp to the new
        // ceiling (a frozen value above it gets pulled down; the thumb otherwise just re-slides).
        let eff_max = max_ts.min(ui_state.manual_ceil);
        let was_at_max = clock.time_scale >= eff_max_prev - AT_MAX_EPS;
        let follow = ui_state.lock_max && was_at_max;
        if follow {
            clock.time_scale = eff_max;
        }
        clock.time_scale = clock.time_scale.clamp(MIN_TIME_SCALE, eff_max);
        eff_max_prev = eff_max;
        armed_prev = follow;
        // Toast only on a manual CEIL change (not the continuous floating drift).
        if ceil_changed {
            let n = if ui_state.manual_ceil.is_finite() {
                format!("{}×", ui_state.manual_ceil.round() as i32)
            } else {
                "MAX".to_string()
            };
            let msg = if follow { format!("Locked to ceiling · {n}") } else { format!("Ceiling {n}") };
            toast = Some((msg, get_time()));
        }
        // Echo to the worker. `clock.time_scale` is already ≤ the effective ceiling (≤ CPU cap), so the
        // sim never tries to outrun the machine.
        sim_handle.send(SimCommand::SetClock {
            scale: Some(clock.time_scale),
            paused: Some(clock.paused),
        });
        // Quick-save (`F5`) / quick-load (`F9`) the whole world to/from `SAVE_PATH`. Both the load's
        // terrain regen and save's serialise stay off the hot path: save is fast; load runs on a
        // background thread (`spawn_load`) and swaps in when ready, like a reseed.
        if is_key_pressed(KeyCode::F5) || actions.save {
            // Ask the worker for a consistent state snapshot (it owns the world), then write the file.
            let (tx, rx) = std::sync::mpsc::channel();
            sim_handle.send(SimCommand::Save { reply: tx });
            match rx.recv() {
                Ok((sd, tk, ss, ts)) => match save_world(SAVE_PATH, sd, tk, ss, ts) {
                    Ok(()) => {
                        eprintln!("[save] wrote {SAVE_PATH} at tick {tk}");
                        toast = Some(("Saved".into(), get_time()));
                    }
                    Err(e) => {
                        eprintln!("[save] failed: {e}");
                        toast = Some((format!("Save failed: {e}"), get_time()));
                    }
                },
                Err(_) => eprintln!("[save] world not ready"),
            }
        }
        // Start a background load if one isn't already running. Cancel any in-flight regen — the
        // load wins. The current world stays interactive; the poll above swaps it in when ready.
        if (is_key_pressed(KeyCode::F9) || actions.load) && load.is_none() {
            gen = None;
            let job = spawn_load(SAVE_PATH.to_string());
            loading = Some(Loading {
                kind: ui::loader::LoadKind::Load,
                progress: job.progress.clone(),
                done_at: None,
            });
            load = Some(job);
        }
        if is_key_pressed(KeyCode::J) {
            ui_state.mask = !ui_state.mask;
        }
        if is_key_pressed(KeyCode::O) {
            ui_state.outline = !ui_state.outline;
        }
        if is_key_pressed(KeyCode::Escape) {
            ui_state.selected = None; // close the creature inspector
        }
        } // end loader-gated keyboard hotkeys
        // Snapshot toggle state into Copy locals (`debug_view`/`mask`/`outline`/`water_on`) so the
        // render pass below reads them as before. `show_info` is handled inside the GUI pass.
        let ui::UiState {
            show_info: _,
            debug_view,
            water_on,
            mask,
            outline,
            open_panel: _,
            selected: _,
            lock_max: _,
            manual_ceil: _,
        } = ui_state;

        // World mouse interactions are gated on `!wants_ptr` so a click on a panel doesn't reach
        // the world (F8). Keyboard pan/rotate stay live (egui claims no keys without a text focus).
        if !loader_active {
        let wheel = if wants_ptr { 0.0 } else { mouse_wheel().1 };
        if wheel != 0.0 {
            // Zoom toward the cursor: keep the ground point under the mouse fixed by
            // shifting the target by how much that point would otherwise move.
            let before = ground_under_cursor(&cam);
            cam.zoom = (cam.zoom * (1.0 - wheel.signum() * ZOOM_STEP)).clamp(MIN_ZOOM, max_zoom());
            let after = ground_under_cursor(&cam);
            cam.target.x += before.x - after.x;
            cam.target.z += before.y - after.y;
        }
        // Left-press: first try to PICK a creature (screen-space) → toggle the inspector selection
        // and DON'T start a pan; a miss falls through to the normal pan-grab. Don't START a pan when
        // the press lands on a panel (F8); an in-flight pan finishes normally. Picking is gated on
        // show_info (no selecting while the HUD is hidden).
        if !wants_ptr && is_mouse_button_pressed(MouseButton::Left) {
            let picked = if ui_state.show_info {
                let dots = snapshot.as_ref().map(|s| s.creatures.as_slice()).unwrap_or(&[]);
                pick_creature(&cam, dots, terrain.as_ref())
            } else {
                None
            };
            if let Some(pid) = picked {
                ui_state.selected = if ui_state.selected == Some(pid) { None } else { Some(pid) };
                grab = None;
            } else {
                grab = Some(ground_under_cursor(&cam));
            }
        }
        if !is_mouse_button_down(MouseButton::Left) {
            grab = None;
        } else if let Some(g) = grab {
            let cur = ground_under_cursor(&cam);
            cam.target.x += g.x - cur.x;
            cam.target.z += g.y - cur.y;
        }
        // Right-drag GRAZE (debug): clear-cut the vegetation in a patch under the cursor —
        // the default-build consumer of `graze`, and a manual way to verify regrowth (graze a
        // spot in the Biomass view, watch it grow back). Patch radius so it shows on the
        // down-sampled minimap.
        if !wants_ptr && is_mouse_button_down(MouseButton::Right) && terrain.is_some() {
            let g = ground_under_cursor(&cam);
            let (gx, gy) = ((g.x / VOX).floor() as i32, (g.y / VOX).floor() as i32);
            let r = GRAZE_PATCH_R;
            for yy in (gy - r).max(0)..(gy + r).min(ROWS as i32) {
                for xx in (gx - r).max(0)..(gx + r).min(COLS as i32) {
                    // The worker owns the terrain; graze runs there (clear-cut = take all).
                    sim_handle.send(SimCommand::Graze { x: xx as usize, y: yy as usize, amount: 1.0 });
                }
            }
        }
        // Pan in the ground plane (WASD / arrows), rotated by the current yaw.
        let mut pan = Vec2::ZERO;
        if is_key_down(KeyCode::A) || is_key_down(KeyCode::Left) {
            pan.x -= 1.0;
        }
        if is_key_down(KeyCode::D) || is_key_down(KeyCode::Right) {
            pan.x += 1.0;
        }
        if is_key_down(KeyCode::W) || is_key_down(KeyCode::Up) {
            pan.y -= 1.0;
        }
        if is_key_down(KeyCode::S) || is_key_down(KeyCode::Down) {
            pan.y += 1.0;
        }
        if pan != Vec2::ZERO {
            let speed = cam.zoom * dt * PAN_SPEED; // pan faster when zoomed out
            let (c, s) = (cam.yaw.cos(), cam.yaw.sin());
            cam.target.x += (pan.x * c - pan.y * s) * speed;
            cam.target.z += (pan.x * s + pan.y * c) * speed;
        }
        // Rotate the iso view in 90° steps.
        if is_key_pressed(KeyCode::Q) {
            cam.yaw -= std::f32::consts::FRAC_PI_2;
        }
        if is_key_pressed(KeyCode::E) {
            cam.yaw += std::f32::consts::FRAC_PI_2;
        }
        // Regenerate the world with a fresh seed — in the background. The current map stays
        // visible and interactive until the new one is ready (swapped in by the poll above).
        // A regen already in flight ignores further presses.
        if is_key_pressed(KeyCode::R) && gen.is_none() {
            seed = seed.wrapping_add(1);
            let job = spawn_gen(seed);
            loading = Some(Loading {
                kind: ui::loader::LoadKind::Gen,
                progress: job.progress.clone(),
                done_at: None,
            });
            gen = Some(job);
        }
        } // end loader-gated mouse / camera input

        // ---- Dev bridge: service queued commands on the main thread ----
        #[cfg(feature = "dev")]
        for req in dev_bridge::take(&bridge) {
            let dev_bridge::Req { cmd, reply } = req;
            match cmd {
                dev_bridge::Cmd::Status => {
                    let c = cam.camera();
                    let col_x = (cam.target.x / VOX).floor().clamp(0.0, (COLS - 1) as f32) as usize;
                    let col_y = (cam.target.z / VOX).floor().clamp(0.0, (ROWS - 1) as f32) as usize;
                    // Full sim status from the worker (it owns the world); `None` until a world exists.
                    let (tx, rx) = std::sync::mpsc::channel();
                    sim_handle.send(SimCommand::QueryStatus { col: (col_x, col_y), reply: tx });
                    let rep = rx.recv().ok();
                    // Geometry/climate env from the render-side geo; live biomass from the report.
                    let env = terrain.as_ref().map(|t| {
                        serde_json::json!({
                            "col": [col_x, col_y],
                            "temp": t.temperature_at(col_x, col_y),
                            "moist": t.moisture_at(col_x, col_y),
                            "slope": t.slope_at(col_x, col_y),
                            "water_dist": t.water_dist_at(col_x, col_y),
                            "biome": format!("{:?}", t.biome_at(col_x, col_y)),
                            "biomass": rep.as_ref().map(|r| r.env_biomass),
                        })
                    });
                    let sim_json = rep.as_ref().map(|r| {
                        let profile: serde_json::Map<String, serde_json::Value> = r
                            .profile
                            .iter()
                            .map(|(l, m, mx)| (l.to_string(), serde_json::json!({ "mean_ms": m, "max_ms": mx })))
                            .collect();
                        serde_json::json!({
                            "population": r.population,
                            "avg_energy": r.avg_energy,
                            "avg_biomass": r.avg_biomass,
                            "frac_multicellular": r.multi,
                            "frac_complex": r.complex,
                            "frac_carnivore": r.frac_carnivore,
                            "frac_autotroph": r.frac_autotroph,
                            "avg_nutrient": r.avg_nutrient,
                            "allopatry": r.allopatry,
                            "crypsis": r.crypsis,
                            "species": r.species,
                            "niche_coverage": r.niche_coverage,
                            "strata_und_surf_air_water": r.strata,
                            "births": r.births,
                            "deaths": r.deaths,
                            "kills": r.kills,
                            "profile": profile,
                            "serial_frac": r.serial_frac,
                            "core_ceiling": if r.serial_frac > 0.0 { 1.0 / r.serial_frac } else { 0.0 },
                        })
                    });
                    let _ = reply.send(serde_json::json!({
                        "fps": fps,
                        "frame_ms": frame_ms,
                        "seed": seed,
                        "depth": { "z_near": c.z_near, "z_far": c.z_far, "range": c.z_far - c.z_near },
                        "view": { "cx": cam.target.x, "cz": cam.target.z, "zoom": cam.zoom, "yaw": cam.yaw },
                        "map": { "cols": COLS, "rows": ROWS, "vox_m": VOX, "map_scale": MAP_SCALE,
                                 "detail_chunks": streamer.detail.len(), "coarse_tiles": streamer.coarse.len() },
                        "env": env,
                        "clock": { "tick": clock.tick(), "sim_time": clock.sim_time(),
                                   "day_frac": clock.day_frac(), "time_scale": clock.time_scale,
                                   "paused": clock.paused },
                        "sim": sim_json,
                        "config": config_json(&sim_cfg),
                    }));
                }
                dev_bridge::Cmd::SetClock { scale, paused } => {
                    if let Some(s) = scale {
                        clock.time_scale = s.max(0.0);
                    }
                    if let Some(p) = paused {
                        clock.paused = p;
                    }
                    sim_handle.send(SimCommand::SetClock {
                        scale: Some(clock.time_scale),
                        paused: Some(clock.paused),
                    });
                    let _ = reply.send(serde_json::json!({
                        "time_scale": clock.time_scale, "paused": clock.paused,
                    }));
                }
                dev_bridge::Cmd::Graze { x, y, amount } => {
                    let ok = x < COLS && y < ROWS;
                    if ok {
                        sim_handle.send(SimCommand::Graze { x, y, amount });
                    }
                    let _ = reply.send(serde_json::json!({ "ok": ok, "tick": clock.tick() }));
                }
                dev_bridge::Cmd::Biomass { x, y } => {
                    let biomass = if x < COLS && y < ROWS {
                        let (tx, rx) = std::sync::mpsc::channel();
                        sim_handle.send(SimCommand::QueryBiomass { x, y, reply: tx });
                        rx.recv().ok()
                    } else {
                        None
                    };
                    let _ = reply.send(serde_json::json!({ "biomass": biomass, "tick": clock.tick() }));
                }
                dev_bridge::Cmd::SetView { cx, cz, zoom, yaw } => {
                    if let Some(v) = cx {
                        cam.target.x = v;
                    }
                    if let Some(v) = cz {
                        cam.target.z = v;
                    }
                    if let Some(v) = zoom {
                        cam.zoom = v.clamp(MIN_ZOOM, max_zoom());
                    }
                    if let Some(v) = yaw {
                        cam.yaw = v;
                    }
                    let _ = reply.send(serde_json::json!({"ok": true}));
                }
                dev_bridge::Cmd::Reseed { seed: s } => {
                    // Synchronous on the dev path: scripted inspection expects the new world
                    // (e.g. an immediate screenshot) deterministically, so we block here.
                    seed = s.unwrap_or(seed.wrapping_add(1));
                    gen = None; // cancel any in-flight background regen — this wins
                    let t = VoxelTerrain::new(seed);
                    let sim_world = Sim::with_config(seed, &t, sim_cfg); // re-seed from the new world
                    terrain = Some(VoxelTerrain::render_side(seed, t.chunks_x, t.chunks_y, t.geo()));
                    clock.set_tick(0);
                    sim_handle.send(SimCommand::LoadWorld { sim: Box::new(sim_world), terrain: Box::new(t), tick: 0 });
                    let InternalGlContext { quad_context: ctx, .. } = unsafe { get_internal_gl() };
                    streamer.clear(ctx);
                    let _ = reply.send(serde_json::json!({"seed": seed}));
                }
                dev_bridge::Cmd::Render { water: w, topo: tp } => {
                    // Write to `ui_state` (the source of truth); the render snapshot picks it up next
                    // frame — same field the HUD checkbox / `H`/`G` hotkeys flip.
                    if let Some(w) = w {
                        ui_state.water_on = w;
                    }
                    if let Some(tp) = tp {
                        // `topo` stays a bool over the wire: true selects the Topo view, false
                        // clears to Off (the climate minimaps are driven by `G` interactively).
                        ui_state.debug_view = if tp { DebugView::Topo } else { DebugView::None };
                    }
                    let _ = reply.send(serde_json::json!({"water": ui_state.water_on, "topo": ui_state.debug_view == DebugView::Topo}));
                }
                dev_bridge::Cmd::SetPanel { panel, debug, show_info } => {
                    if let Some(p) = panel {
                        ui_state.open_panel = match p.as_str() {
                            "world" => Some(ui::Panel::World),
                            "view" => Some(ui::Panel::View),
                            "pop" => Some(ui::Panel::Pop),
                            "perf" => Some(ui::Panel::Perf),
                            _ => None,
                        };
                    }
                    if let Some(d) = debug {
                        ui_state.debug_view = match d.as_str() {
                            "topo" => DebugView::Topo,
                            "temp" => DebugView::Temp,
                            "moist" => DebugView::Moist,
                            "waterdist" => DebugView::WaterDist,
                            "slope" => DebugView::Slope,
                            "biomass" => DebugView::Biomass,
                            _ => DebugView::None,
                        };
                    }
                    if let Some(si) = show_info {
                        ui_state.show_info = si;
                    }
                    let _ = reply.send(serde_json::json!({"ok": true}));
                }
                dev_bridge::Cmd::Select { id, nearest } => {
                    if nearest {
                        if let (Some(snap), Some(t)) = (snapshot.as_ref(), terrain.as_ref()) {
                            let (sw, sh) = (screen_width(), screen_height());
                            let vp = cam.camera().matrix();
                            let (ccx, ccy) = (sw * 0.5, sh * 0.5);
                            let mut best: Option<(f32, u64)> = None;
                            for c in &snap.creatures {
                                let (cx, cy) = sim::column_index(c.pos);
                                let wy = t.height(cx as i32, cy as i32) as f32 * VOX + 0.5;
                                if let Some(p) = project_world(vp, sw, sh, vec3(c.pos.x, wy, c.pos.y)) {
                                    let d = ((p[0] - ccx).powi(2) + (p[1] - ccy).powi(2)).sqrt();
                                    if best.is_none_or(|(bd, _)| d < bd) {
                                        best = Some((d, c.id));
                                    }
                                }
                            }
                            ui_state.selected = best.map(|(_, id)| id);
                        }
                    } else {
                        ui_state.selected = id;
                    }
                    let _ = reply.send(serde_json::json!({"ok": true, "selected": ui_state.selected}));
                }
                dev_bridge::Cmd::Screenshot { path, window } => {
                    pending_shots.push((path, window, reply)); // serviced post-draw below
                }
                dev_bridge::Cmd::GetConfig => {
                    let _ = reply.send(config_json(&sim_cfg));
                }
                dev_bridge::Cmd::SetFeature { name, enabled } => {
                    // `sim_cfg` is the main-side config mirror; mutate it and push to the worker.
                    let resp = if sim_cfg.features.set(&name, enabled) {
                        sim_handle.send(SimCommand::SetConfig(sim_cfg));
                        serde_json::json!({ "ok": true, "feature": name, "enabled": enabled })
                    } else {
                        serde_json::json!({ "ok": false, "error": format!("unknown feature: {name}") })
                    };
                    let _ = reply.send(resp);
                }
                dev_bridge::Cmd::SetParam { name, value } => {
                    let resp = if sim_cfg.params.set(&name, value) {
                        sim_handle.send(SimCommand::SetConfig(sim_cfg));
                        serde_json::json!({ "ok": true, "param": name, "value": value })
                    } else {
                        serde_json::json!({ "ok": false, "error": format!("unknown param: {name}") })
                    };
                    let _ = reply.send(resp);
                }
                dev_bridge::Cmd::Metrics { id: _, last: _ } => {
                    // Live metric sampling moved to the sim worker is not yet wired; report empty.
                    let _ = reply.send(serde_json::json!({ "metrics": [], "note": "unavailable on the sim thread" }));
                }
                dev_bridge::Cmd::Save { path } => {
                    let p = path.unwrap_or_else(|| SAVE_PATH.to_string());
                    let (tx, rx) = std::sync::mpsc::channel();
                    sim_handle.send(SimCommand::Save { reply: tx });
                    let resp = match rx.recv() {
                        Ok((sd, tk, ss, ts)) => match save_world(&p, sd, tk, ss, ts) {
                            Ok(()) => {
                                toast = Some(("Saved".into(), get_time())); // mirror the HUD toast
                                serde_json::json!({ "ok": true, "saved": p, "tick": tk })
                            }
                            Err(e) => serde_json::json!({ "ok": false, "error": e }),
                        },
                        Err(_) => serde_json::json!({ "ok": false, "error": "world not ready" }),
                    };
                    let _ = reply.send(resp);
                }
                dev_bridge::Cmd::Load { path } => {
                    let p = path.unwrap_or_else(|| SAVE_PATH.to_string());
                    // Regenerate the geometry + restore the overlay, then hand the world to the worker.
                    let resp = match load_snapshot(&p) {
                        Ok(snap) => {
                            let mut t = VoxelTerrain::new(snap.terrain_seed);
                            match t.set_state(snap.terrain) {
                                Ok(()) => {
                                    let tick = snap.tick;
                                    seed = snap.terrain_seed;
                                    sim_cfg = snap.sim.cfg;
                                    terrain = Some(VoxelTerrain::render_side(seed, t.chunks_x, t.chunks_y, t.geo()));
                                    clock.set_tick(tick);
                                    sim_handle.send(SimCommand::LoadWorld {
                                        sim: Box::new(Sim::from_state(snap.sim)),
                                        terrain: Box::new(t),
                                        tick,
                                    });
                                    gen = None; // cancel any in-flight regen/load — the load wins
                                    load = None;
                                    let InternalGlContext { quad_context: ctx, .. } =
                                        unsafe { get_internal_gl() };
                                    streamer.clear(ctx);
                                    serde_json::json!({ "ok": true, "loaded": p, "tick": tick, "seed": seed })
                                }
                                Err(e) => serde_json::json!({ "ok": false, "error": e }),
                            }
                        }
                        Err(e) => serde_json::json!({ "ok": false, "error": e }),
                    };
                    let _ = reply.send(resp);
                }
            }
        }

        // ---- Render ----
        // Keep the offscreen target matched to the (possibly resized) window.
        if scene_rt.texture.width() != screen_width()
            || scene_rt.texture.height() != screen_height()
        {
            scene_rt = new_scene_target(screen_width() as u32, screen_height() as u32);
        }

        // Pass 1: render the visible chunks into the offscreen target via raw miniquad
        // — persistent buffers, one draw call per visible chunk, no per-frame upload.
        let vp = cam.camera().matrix();
        let center = center_chunk(&cam);
        drawn = 0; // reset the (loop-persistent) perf counter the GUI pass read this frame
        {
            let mut gl = unsafe { get_internal_gl() };
            gl.flush(); // flush any pending macroquad 2D before our own pass
            let ctx = gl.quad_context;
            // Stream: detail tier around the camera + coarse super-tiles over the rest.
            // No terrain yet (initial generation still running) ⇒ nothing to stream/draw;
            // the pass below just clears to sky and the progress bar shows over it.
            if let Some(terrain) = &terrain {
                streamer.update(ctx, terrain, center, cam.zoom);
            }
            ctx.begin_pass(
                Some(scene_rt.render_pass.raw_miniquad_id()),
                PassAction::Clear {
                    color: Some((0.53, 0.62, 0.78, 1.0)), // sky
                    depth: Some(1.0),
                    stencil: None,
                },
            );
            ctx.apply_pipeline(&pipeline);
            // dbg.x = topo height view, dbg.y = water/land mask, dbg.z = step-edge outline on.
            let dbg = vec4(
                if debug_view == DebugView::Topo { 1.0 } else { 0.0 },
                if mask { 1.0 } else { 0.0 },
                if outline { 1.0 } else { 0.0 },
                0.0,
            );
            ctx.apply_uniforms(UniformsSource::table(&ChunkUniforms { mvp: vp, dbg }));
            // Per super-tile draw EITHER its detail chunks (if ready) OR its coarse buffer
            // (otherwise) — never both. So the tiers never overlap (no z-fight) and a
            // not-yet-ready tile shows coarse instead of flashing empty (no flicker).
            // Frustum-culled by AABB.
            let ready = &streamer.ready;
            let draw = |chunks: &[GpuChunk], drawn: &mut usize, ctx: &mut dyn RenderingBackend| {
                for c in chunks {
                    if aabb_in_view(&vp, c.lo, c.hi) {
                        ctx.apply_bindings(&c.bindings);
                        ctx.draw(0, c.n_idx, 1);
                        *drawn += 1;
                    }
                }
            };
            for (key, lc) in &streamer.coarse {
                if !ready.contains(key) {
                    draw(&lc.opaque, &mut drawn, ctx);
                }
            }
            for (&(cx, cy), lc) in &streamer.detail {
                if ready.contains(&(cx.div_euclid(SUPER), cy.div_euclid(SUPER))) {
                    draw(&lc.opaque, &mut drawn, ctx);
                }
            }
            // Water: second, translucent, animated pass over the opaque scene. Skipped in
            // topo mode (bed laid bare) or when toggled off with `H`. Same draw rule as the
            // opaque tiers so the two never overlap; `depth_write:false` lets terrain in
            // front still occlude it without the water occluding itself.
            // Mask mode forces the water pass on (flat blue) even over the topo gate; normal
            // mode draws it unless topo or `H` hid it.
            if mask || (debug_view != DebugView::Topo && water_on) {
                ctx.apply_pipeline(&water_pipe);
                let params = vec4(get_time() as f32, if mask { 1.0 } else { 0.0 }, 0.0, 0.0);
                ctx.apply_uniforms(UniformsSource::table(&WaterUniforms { mvp: vp, params }));
                for (key, lc) in &streamer.coarse {
                    if !ready.contains(key) {
                        draw(&lc.water, &mut drawn, ctx);
                    }
                }
                for (&(cx, cy), lc) in &streamer.detail {
                    if ready.contains(&(cx.div_euclid(SUPER), cy.div_euclid(SUPER))) {
                        draw(&lc.water, &mut drawn, ctx);
                    }
                }
            }
            ctx.end_render_pass();
        }

        // Pass 2: blit the offscreen scene to the window (render targets are y-flipped).
        draw_texture_ex(
            &scene_rt.texture,
            0.0,
            0.0,
            WHITE,
            DrawTextureParams {
                dest_size: Some(vec2(screen_width(), screen_height())),
                flip_y: true,
                ..Default::default()
            },
        );

        // Creatures: zoom-aware LOD over the blitted scene. Close in, draw INDIVIDUALS sized in
        // world metres (a single-cell microbe is a sub-metre speck that shrinks as the camera pulls
        // back — not a fixed fat pixel). Past a zoom-out threshold an individual falls below a pixel,
        // so we switch to BACTERIAL MATS: per-column density tinted by the colony's mean coloration
        // (dense colony → solid mat, sparse → faint film). Off-screen points are culled by projection.
        on_screen = 0; // reset the (loop-persistent) perf counter the GUI pass read this frame
        if let (Some(snap), Some(terrain)) = (snapshot.as_ref(), terrain.as_ref()) {
            let (sw, sh) = (screen_width(), screen_height());
            let px_per_m = sh / cam.zoom; // ortho: visible world-height = zoom
            // Project a world point on a column top to screen px; None if behind/off-screen.
            let project = |wx: f32, wz: f32, wy: f32| -> Option<(f32, f32)> {
                project_world(vp, sw, sh, vec3(wx, wy, wz)).map(|p| (p[0], p[1]))
            };
            if px_per_m >= LOD_MAT_PX_PER_M {
                // INDIVIDUALS. Zoomed in far enough that a single cell spans ≥ a couple pixels, draw
                // the MORPHOLOGY — the developed body as a cluster of lattice cells, tinted by type
                // (structural = the evolved greyscale coloration, so camouflage still reads; function
                // cells get a type colour, so organs are visible). Otherwise the body is sub-cell, so
                // draw the cheaper world-scaled dot (also bounds cost: morphology only at high zoom,
                // where few creatures are on screen).
                let cell_px = CREATURE_RADIUS_M * px_per_m * 2.0;
                for c in &snap.creatures {
                    let (cx, cy) = sim::column_index(c.pos);
                    let wy = terrain.height(cx as i32, cy as i32) as f32 * VOX + 0.5;
                    let Some((px, py)) = project(c.pos.x, c.pos.y, wy) else {
                        continue;
                    };
                    let g = c.coloration;
                    match (cell_px >= BODY_CELL_MIN_PX).then_some(()).and(c.body.as_ref()) {
                        Some(body) => {
                            for &(dx, dy, ty) in body {
                                let bx = px + dx as f32 * cell_px;
                                let by = py + dy as f32 * cell_px;
                                let (h, s) = (cell_px * 0.5, cell_px);
                                draw_rectangle(bx - h - 0.5, by - h - 0.5, s + 1.0, s + 1.0, Color::new(0.0, 0.0, 0.0, 0.5));
                                draw_rectangle(bx - h, by - h, s, s, cell_color(ty, g));
                            }
                        }
                        None => {
                            // Body radius in metres (√biomass) → pixels; floor keeps a lone microbe clickable.
                            let r = (CREATURE_RADIUS_M * (c.biomass as f32).sqrt() * px_per_m).max(CREATURE_MIN_PX);
                            draw_circle(px, py, r + 0.8, Color::new(0.0, 0.0, 0.0, 0.6));
                            draw_circle(px, py, r, Color::new(g, g, g, 1.0));
                        }
                    }
                    on_screen += 1;
                }
            } else {
                // BACTERIAL MATS — aggregate creatures per column (count + summed coloration), then
                // draw one coverage tile per occupied column: alpha ramps with colony density.
                // Colour-tint: autotroph (green) creatures create visually distinct algae/plant-mats,
                // while heterotrophs render closer to greyscale. Since we don't have trophic type in
                // the snapshot at mat LOD, we detect autotrophs via their photo cells (green renderers)
                // in individual morphologies, or lean heavily on the green channel for mat colonies.
                let mut bucket: std::collections::HashMap<(usize, usize), (u32, f32)> =
                    std::collections::HashMap::new();
                for c in &snap.creatures {
                    let (cx, cy) = sim::column_index(c.pos);
                    let e = bucket.entry((cx, cy)).or_insert((0, 0.0));
                    e.0 += 1;
                    e.1 += c.coloration;
                }
                let tile = (VOX * px_per_m).max(1.0); // a column footprint in px
                for ((cx, cy), (count, colsum)) in bucket {
                    let wx = (cx as f32 + 0.5) * VOX;
                    let wz = (cy as f32 + 0.5) * VOX;
                    let wy = terrain.height(cx as i32, cy as i32) as f32 * VOX + 0.5;
                    let Some((px, py)) = project(wx, wz, wy) else {
                        continue;
                    };
                    let col_mean = colsum / count as f32;
                    let a = (count as f32 / MAT_FULL_COUNT).min(MAT_MAX_ALPHA);
                    // Mat tinting: shift toward green (algae/plant-mat look) by boosting the green
                    // channel relative to red/blue for colonies. Dense mats read as solid green;
                    // sparse mats fade. This makes autotroph mats visually distinct from mobile
                    // heterotroph flocks (which remain closer to grey dots at individual LOD).
                    let r = col_mean * 0.6;
                    let g = col_mean * 0.9 + 0.1 * a; // boost green, add density glow
                    let b = col_mean * 0.6;
                    draw_rectangle(px - tile * 0.5, py - tile * 0.5, tile, tile, Color::new(r, g, b, a));
                    on_screen += count as usize;
                }
            }
        }

        // (The HUD/stats text is now the egui panels rendered at the start of the frame and
        // composited by `egui_macroquad::draw()` at the end; see the GUI pass above.)

        // (The env-field minimap is now the egui minimap panel — top-right — built in the GUI pass
        // at the start of the frame; see `ui::minimap`.)

        // (The generation/load progress is now the full-screen egui loader overlay — `ui::loader`,
        // drawn in the GUI pass above — which replaces the old macroquad bottom bar.)

        // Composite the egui panels (built in the GUI pass at the frame's start) over the scene.
        // Its own render pass — drawn last so it sits on top, after creatures and the minimap.
        egui_macroquad::draw();

        // Dev bridge: service deferred screenshots now the frame is FULLY drawn (incl. the egui HUD
        // just composited above). `window` → the whole window back-buffer with the HUD
        // (`get_screen_data`); otherwise the offscreen 3D target only (no HUD, no foreground needed).
        #[cfg(feature = "dev")]
        for (path, window, reply) in pending_shots.drain(..) {
            let img = if window {
                get_screen_data()
            } else {
                capture_target(&scene_rt)
            };
            img.export_png(&path);
            let _ = reply.send(serde_json::json!({"saved": path, "window": window}));
        }

        next_frame().await;
    }
}

/// Read an offscreen render target's pixels into an `Image` ready for PNG export.
/// GPU render targets are stored bottom-up, so the rows are flipped back.
#[cfg(feature = "dev")]
fn capture_target(rt: &RenderTarget) -> Image {
    let mut img = rt.texture.get_texture_data();
    let (w, h) = (img.width as usize, img.height as usize);
    let row = w * 4;
    let bytes = &mut img.bytes;
    for y in 0..h / 2 {
        let (top, bot) = (y * row, (h - 1 - y) * row);
        for i in 0..row {
            bytes.swap(top + i, bot + i);
        }
    }
    img
}

/// Load a shader from `assets/` at runtime (editable without a rebuild), falling back to the copy
/// baked in with `include_str!` if the file isn't reachable.
async fn load_shader(path: &str, baked: &str) -> String {
    macroquad::file::load_string(path).await.unwrap_or_else(|_| baked.to_string())
}

/// The `SimConfig` (features + params) as JSON. Reads the main-side config mirror (`sim_cfg`), kept in
/// sync with the worker. Used by the dev bridge's `get_config` and embedded in `status`.
#[cfg(feature = "dev")]
fn config_json(cfg: &SimConfig) -> serde_json::Value {
    let features: serde_json::Map<String, serde_json::Value> =
        cfg.features.pairs().iter().map(|(k, v)| (k.to_string(), serde_json::json!(v))).collect();
    let params: serde_json::Map<String, serde_json::Value> =
        cfg.params.pairs().iter().map(|(k, v)| (k.to_string(), serde_json::json!(v))).collect();
    serde_json::json!({ "features": features, "params": params })
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
