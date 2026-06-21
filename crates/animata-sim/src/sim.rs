//! Life simulation — herbivore ecosystem on the voxel world (C0 loop + C1 developmental body).
//!
//! A creature senses the plant-biomass field (S3) around it, a fixed-topology brain with
//! evolvable weights decides throttle + turn, it grazes the column it stands on, pays a
//! Kleiber-scaled metabolic cost, buds a mutated child when well-fed, and dies of starvation
//! or senescence. **C1:** the body is no longer a fixed single cell — it is *grown* from the
//! genome's gene-regulatory network ([`crate::genome`]); biomass = the developed cell count
//! and the cell-type mix modulates the creature's stats (effector→speed, storage→energy cap).
//! The founder's empty GRN develops to one structural cell, recovering the C0 organism.
//!
//! Determinism invariants (see plan): randomness is a pure function of the world seed via
//! [`crate::rng`] (no `rand` crate); creatures live in a `Vec` (stable index); the tick is
//! multi-phase (snapshot/decide read the world unmutated → apply mutates → compact), so the
//! result is independent of iteration order; deaths flag-then-compact (never `swap_remove`
//! mid-apply); over-cap cull is deterministic-random, not tail-truncation.

use glam::{vec2, Vec2};
use rayon::prelude::*;

use crate::config::*;
use std::time::Instant;

use crate::genome::{Genome, GenomeV2, Phenotype, TrophicNiche};
use crate::grid::SpatialGrid;
use crate::profile::Span;
use crate::pressure::{PressureRegistry, Sample};
use crate::rng::{seed_fold, splitmix64, Rng};
use crate::sim_config::{SimConfig, SimConfigV2};
use crate::terrain::VoxelTerrain;

// Fixed brain topology: inputs → tanh hidden → tanh outputs. The genome's `brain` vector holds
// the weights (length = `genome::BRAIN_WEIGHTS` = N_INPUTS*N_HIDDEN + N_HIDDEN*N_OUTPUTS).
// Inputs: [plant_here, plant_fwd, plant_left, plant_right, energy, water_dist,
//          prey_prox, prey_bearing, threat_prox, threat_bearing, bias].
const N_INPUTS: usize = 11;
const N_HIDDEN: usize = 6;
const N_OUTPUTS: usize = 2; // [throttle (pre-squash), turn (pre-squash)]

// Seed salts (keep distinct so independent draws on the same (id, tick) don't correlate).
const SALT_FOUNDER: u64 = 0x0F00;
const SALT_MUTATE: u64 = 0x111;
const SALT_CULL: u64 = 0xC011;
const SALT_DEATH: u64 = 0xDEAD;
const SALT_BIRTH: u64 = 0xB127;
const SALT_CAMO: u64 = 0xCA30;
const SALT_TOXIN: u64 = 0x70_8127;
const SALT_MORPH: u64 = 0xD2_0F; // morphogen READ-weight mutation (PR-D2) — an INDEPENDENT stream
const SALT_GAS: u64 = 0x6A502; // oxygen-tolerance mutation (gas cycle) — an INDEPENDENT stream

/// The serialisable live sim state — the half of a save snapshot that is the creatures (the other
/// half is the terrain overlay). Restored via [`Sim::from_state`]; captured via [`Sim::to_state`].
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SimState {
    pub world_seed: u64,
    pub next_id: u64,
    pub births: u64,
    pub deaths: u64,
    pub kills: u64,
    pub cfg: SimConfig,
    pub creatures: Vec<Creature>,
}

/// One creature. Its `genome` (developmental GRN + brain weights) is grown once into `pheno`
/// (the cell body) at creation; biomass and the stat modifiers below read from `pheno`.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Creature {
    pub id: u64,
    pub founder: u64,
    pub pos: Vec2, // world (x, z) over the ground plane; column = (x/VOX, z/VOX)
    pub heading: f32,
    pub energy: f32,
    pub age: u32,
    alive: bool,
    genome: Genome,
    pub pheno: Phenotype,
}

impl Creature {
    /// Biomass in integer cells = the developed cell count (Kleiber metabolism scales with it,
    /// and it is the energy a predator gets in C2).
    pub fn biomass(&self) -> u32 {
        self.pheno.n_cells
    }

    /// Evolved body coloration `[0,1]` (for the camouflage render tint).
    pub fn coloration(&self) -> f32 {
        self.genome.coloration
    }

    /// The developed body as `(x, y, cell_type)` lattice cells — for RENDER ONLY (drawing the
    /// organism's shape at close zoom). Re-derived from the (private) genome via the shared
    /// morphogenesis core; nothing spatial is stored on the creature. `cell_type`: 0 = structural,
    /// 1..=7 = effector / storage / sensor / predator / flight / burrow / photo.
    pub fn body_layout_for_render(&self) -> Vec<(i16, i16, u8)> {
        self.genome.body_layout()
    }

    /// Top speed = stratum drift + powered thrust. Powered motility is the muscle FRACTION
    /// (`thrust = effector organ power / sqrt(n_cells)`): thrust grows with effector cells (count +
    /// coherence bonus), drag rises with the body's linear size (√area in 2D), so a body that is
    /// mostly muscle is fast at any size while dead-weight bulk slows it. A body with no effectors
    /// only drifts — almost nothing on land, more in a fluid (water/air carries it). `layer` is the
    /// creature's stratum this tick (passed from the snapshot phase).
    fn speed(&self, layer: Stratum) -> f32 {
        let drift = match layer {
            Stratum::Surface | Stratum::Underground => DRIFT_GROUND,
            Stratum::Water => DRIFT_WATER,
            Stratum::Air => DRIFT_AIR,
        };
        let thrust = self.pheno.organ_power(0) / (self.pheno.n_cells as f32).max(1.0).sqrt();
        CREATURE_SPEED * (drift + LOCO_GAIN * thrust)
    }

    /// Energy capacity: storage cells enlarge the buffer (survive lean spells, bigger broods). A
    /// coherent storage ORGAN holds more than the same cells scattered (PR-C organ_power).
    fn max_energy(&self) -> f32 {
        MAX_ENERGY + STORAGE_PER_CELL * self.pheno.organ_power(1)
    }

    /// Energy as a fraction `[0,1]` of this body's own capacity — for the inspector vitals bar
    /// (`max_energy` is private, so this is the public read).
    pub fn energy_frac(&self) -> f32 {
        (self.energy / self.max_energy().max(1e-6)).clamp(0.0, 1.0)
    }

    /// Sensing reach as a multiple of the base range, driven by the sensor ORGAN power (count +
    /// coherence). `1.0` with no sensor cells (no nerf); rises with sensory tissue and is capped so
    /// the spatial-grid query stays local. Scales BOTH prey/threat detection and the food-gradient
    /// sampling — so it benefits herbivores (food gradient) as well as predators/prey (detection).
    fn sense_mult(&self) -> f32 {
        (SENSE_FLOOR + SENSE_GAIN * self.pheno.organ_power(2)).min(SENSE_CAP)
    }

    /// Grazing throughput: a bigger body crops a little faster (sublinear, so size isn't free).
    fn intake(&self) -> f32 {
        EAT_RATE * (self.pheno.n_cells as f32).sqrt()
    }

    /// Forward brain pass: inputs → tanh hidden → tanh outputs. Returns `(throttle∈[0,1],
    /// turn∈[-1,1])`. Plain matmul (ported shape from the archived `brain.rs`).
    fn think(&self, inputs: &[f32; N_INPUTS]) -> (f32, f32) {
        let w = &self.genome.brain;
        let mut hidden = [0.0f32; N_HIDDEN];
        for (h, hv) in hidden.iter_mut().enumerate() {
            let mut sum = 0.0;
            for (i, &iv) in inputs.iter().enumerate() {
                sum += iv * w[h * N_INPUTS + i];
            }
            *hv = sum.tanh();
        }
        let base = N_INPUTS * N_HIDDEN;
        let mut out = [0.0f32; N_OUTPUTS];
        for (o, ov) in out.iter_mut().enumerate() {
            let mut sum = 0.0;
            for (h, &hv) in hidden.iter().enumerate() {
                sum += hv * w[base + o * N_HIDDEN + h];
            }
            *ov = sum.tanh();
        }
        ((out[0] + 1.0) * 0.5, out[1])
    }
}

/// Frozen ANM2 `Creature` shape (its `genome` is the pre-`oxygen_tolerance` `GenomeV2`) for save
/// migration ([`crate::persist`] v2). NEVER edit. `pheno` is unchanged (oxygen is a genome trait).
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct CreatureV2 {
    id: u64,
    founder: u64,
    pos: Vec2,
    heading: f32,
    energy: f32,
    age: u32,
    alive: bool,
    genome: GenomeV2,
    pheno: Phenotype,
}

impl CreatureV2 {
    fn migrate(self) -> Creature {
        Creature {
            id: self.id,
            founder: self.founder,
            pos: self.pos,
            heading: self.heading,
            energy: self.energy,
            age: self.age,
            alive: self.alive,
            genome: self.genome.migrate(),
            pheno: self.pheno,
        }
    }
}

/// Frozen ANM2 `SimState` (its `cfg`/`creatures` are the frozen V2 shapes) for save migration.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct SimStateV2 {
    world_seed: u64,
    next_id: u64,
    births: u64,
    deaths: u64,
    kills: u64,
    cfg: SimConfigV2,
    creatures: Vec<CreatureV2>,
}

impl SimStateV2 {
    /// ANM2 → current: migrate the config (oxygen feature off, continuity) + each creature (genome
    /// gains `oxygen_tolerance = 0`). Scalars carry over unchanged.
    pub(crate) fn migrate(self) -> SimState {
        SimState {
            world_seed: self.world_seed,
            next_id: self.next_id,
            births: self.births,
            deaths: self.deaths,
            kills: self.kills,
            cfg: self.cfg.migrate(),
            creatures: self.creatures.into_iter().map(CreatureV2::migrate).collect(),
        }
    }
}

#[cfg(test)]
impl SimState {
    /// Down-convert to the frozen ANM2 shape — migration-test support only.
    pub(crate) fn to_v2(&self) -> SimStateV2 {
        SimStateV2 {
            world_seed: self.world_seed,
            next_id: self.next_id,
            births: self.births,
            deaths: self.deaths,
            kills: self.kills,
            cfg: self.cfg.to_v2(),
            creatures: self
                .creatures
                .iter()
                .map(|c| CreatureV2 {
                    id: c.id,
                    founder: c.founder,
                    pos: c.pos,
                    heading: c.heading,
                    energy: c.energy,
                    age: c.age,
                    alive: c.alive,
                    genome: c.genome.to_v2(),
                    pheno: c.pheno,
                })
                .collect(),
        }
    }
}

/// A reproduction queued by the serial apply phase, awaiting parallel body development. Holds the
/// already-mutated child genome plus the parent state needed to finish the birth. `rng` is the SAME
/// per-parent stream `mutate` advanced — resumed (inside the parallel closure) for the child's
/// pos/heading, so the draw sequence is byte-identical to the old inline path. `genome`/`rng` are
/// MOVED (not cloned).
struct PendingBirth {
    genome: Genome,
    rng: Rng,
    parent_pos: Vec2,
    parent_energy: f32,
    founder: u64,
}

/// A fully-developed child produced by the parallel develop phase, ready for the serial id-assigning
/// append. `None` (vs this) marks a stillbirth (the parent couldn't afford the body) — drawn exactly
/// when the old code drew, so no RNG is consumed for a stillborn child.
struct BornChild {
    genome: Genome,
    pheno: Phenotype,
    pos: Vec2,
    heading: f32,
    energy: f32,
    founder: u64,
}

/// Minimum queued births to run development in parallel — below it the rayon fork/join overhead isn't
/// worth it, so develop serially. Determinism is unaffected either way (`develop` is pure; the serial
/// and parallel paths produce identical results in identical order).
const PAR_DEVELOP_THRESHOLD: usize = 16;

/// Grow a queued birth's body and finish it into a [`BornChild`], or `None` if the parent can't afford
/// the developed body (stillborn). Pure + self-contained so it runs safely on a rayon worker: the only
/// RNG is the parent's OWN moved `rng`, drawn here (after develop, conditional on viability) exactly as
/// the old inline path did. `dev` mirrors `Features::development` (off ⇒ the trivial one-cell body, no
/// growth, RNG stream unchanged).
fn develop_birth(p: PendingBirth, dev: bool, maxx: f32, maxy: f32) -> Option<BornChild> {
    let pheno = if dev {
        p.genome.develop()
    } else {
        Phenotype { n_cells: 1, structural: 1, ..Default::default() }
    };
    let build = CELL_BIOMASS_COST * pheno.n_cells.saturating_sub(1) as f32;
    let child_energy = p.parent_energy - build;
    if child_energy <= 0.0 {
        return None; // stillborn — the parent already paid half its energy; no pos/heading drawn
    }
    let mut rng = p.rng;
    let pos = vec2(
        (p.parent_pos.x + rng.signed() * 2.0).clamp(0.0, maxx),
        (p.parent_pos.y + rng.signed() * 2.0).clamp(0.0, maxy),
    );
    let heading = rng.unit() * std::f32::consts::TAU;
    Some(BornChild { genome: p.genome, pheno, pos, heading, energy: child_energy, founder: p.founder })
}

/// The whole creature population + the deterministic id counter and cumulative stats.
pub struct Sim {
    pub creatures: Vec<Creature>,
    world_seed: u64,
    next_id: u64,
    pub births: u64,
    pub deaths: u64,
    pub kills: u64,
    /// Reused spatial index over creature positions, rebuilt each tick for prey/threat queries.
    grid: SpatialGrid,
    /// The active environmental selection pressures (climate / autotrophy / metabolism), composed
    /// per creature each tick. Behaviour config, not state — not part of `state_checksum`.
    registry: PressureRegistry,
    /// Runtime feature toggles + parameters. Part of the sim's INPUT (with the seed); the golden is
    /// at `SimConfig::default()`. Behaviour config, not state.
    cfg: SimConfig,
    /// Per-phase wall-clock profiler. A pure MEASUREMENT (reads `Instant`, influences no sim value) —
    /// like `grid`/`registry` it is NOT part of `state_checksum`, so the golden is identical whether
    /// profiling is on or off.
    profiler: crate::profile::Profiler,
}

/// Dimensions of the species feature vector: 7 cell-type fractions (the developmental body plan)
/// plus normalised size. Speciation is by BODY, per the plan ("topological speciation on the
/// developmental bodies") — climate/colour are continuous within-species niche traits, not here.
const FEATURES: usize = 8;

/// The body-plan feature vector a creature is clustered by into a species — its cell-type
/// composition and size. Each component is ~`[0,1]`.
fn feature(c: &Creature) -> [f32; FEATURES] {
    let p = &c.pheno;
    [
        p.effector as f32 / p.n_cells as f32,
        p.storage as f32 / p.n_cells as f32,
        p.sensor as f32 / p.n_cells as f32,
        p.predator as f32 / p.n_cells as f32,
        p.flight as f32 / p.n_cells as f32,
        p.burrow as f32 / p.n_cells as f32,
        p.photo as f32 / p.n_cells as f32,
        (p.n_cells as f32 / crate::genome::MAX_CELLS as f32).min(1.0),
    ]
}

/// Squared Euclidean distance between two feature vectors.
fn feature_dist2(a: &[f32; FEATURES], b: &[f32; FEATURES]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y).powi(2)).sum()
}

/// Pearson correlation of two equal-length samples (`0` if undefined). Shared by the niche
/// metrics — how well an evolved trait tracks the local environment (allopatry, crypsis).
fn pearson(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len() as f32;
    if a.len() < 2 {
        return 0.0;
    }
    let (ma, mb) = (a.iter().sum::<f32>() / n, b.iter().sum::<f32>() / n);
    let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
    for (&x, &y) in a.iter().zip(b) {
        cov += (x - ma) * (y - mb);
        va += (x - ma).powi(2);
        vb += (y - mb).powi(2);
    }
    let denom = (va * vb).sqrt();
    if denom > 1e-6 {
        cov / denom
    } else {
        0.0
    }
}

/// Closeness `[0,1]` (1 = adjacent, 0 = at/over the sense range) and the left/right bearing of
/// a target relative to a creature's heading — the two cues the brain needs to steer to/from it.
/// `range` is the SENSING creature's own reach (`SENSE_RANGE · sense_mult`) so prox is normalised
/// against how far *this* body can perceive: a sharper-sensed body reads a far target as nearer.
fn rel(from: Vec2, heading: f32, target: Vec2, range: f32) -> (f32, f32) {
    let d = target - from;
    let dist = d.length();
    let prox = (1.0 - dist / range).clamp(0.0, 1.0);
    let bearing = (d.y.atan2(d.x) - heading).sin();
    (prox, bearing)
}

/// Clamp a continuous world position to an in-world column index (single conversion point —
/// out of bounds would otherwise panic or silently corrupt a neighbour row via `graze`).
pub fn column_index(pos: Vec2) -> (usize, usize) {
    let x = (pos.x / VOX).floor().clamp(0.0, (COLS - 1) as f32) as usize;
    let y = (pos.y / VOX).floor().clamp(0.0, (ROWS - 1) as f32) as usize;
    (x, y)
}

/// Vertical strata (C3). Which one a creature occupies is set by its morphology + where it
/// stands: flight cells → Air, burrow cells → Underground, fins over a water column → Water,
/// else the Surface base layer. Each is a distinct niche — its own food source and a predator
/// refuge (predators only hunt within their own stratum).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Stratum {
    Underground,
    Surface,
    Air,
    Water,
}

impl Stratum {
    pub fn idx(self) -> usize {
        match self {
            Stratum::Underground => 0,
            Stratum::Surface => 1,
            Stratum::Air => 2,
            Stratum::Water => 3,
        }
    }

    /// Human label for the inspector "strata" row.
    pub fn name(self) -> &'static str {
        match self {
            Stratum::Underground => "Under",
            Stratum::Surface => "Surface",
            Stratum::Air => "Air",
            Stratum::Water => "Water",
        }
    }

    /// Total non-surface foraging yield (energy / sim-second) of this stratum, split among its
    /// occupants (so an empty stratum richly rewards the first colonisers, then self-limits).
    /// Surface returns 0 here — it feeds from the positioned S3 plant field instead.
    fn capacity(self) -> f32 {
        match self {
            Stratum::Underground => UNDERGROUND_CAPACITY,
            Stratum::Air => AIR_CAPACITY,
            Stratum::Water => WATER_CAPACITY,
            Stratum::Surface => 0.0,
        }
    }
}

/// The stratum a creature occupies, from its body and whether its column is water. Priority
/// Air > Underground > Water > Surface (a body able to fly uses the air even over water).
pub fn stratum_of(pheno: &Phenotype, is_water_col: bool) -> Stratum {
    if pheno.flight_frac() > STRATUM_THETA {
        Stratum::Air
    } else if pheno.burrow_frac() > STRATUM_THETA {
        Stratum::Underground
    } else if is_water_col && pheno.fin_frac() > STRATUM_THETA {
        Stratum::Water
    } else {
        Stratum::Surface
    }
}

/// Light available to photosynthesis at a creature's stratum, latitude row and tick, in `[0,1]`.
/// Activates the dormant S2 day/night (a sinusoid in `day_frac` with a dim-night floor) and the
/// S1 latitude (poles dimmer). Underground = no light; water = attenuated; surface/air = full.
fn light_for(stratum: Stratum, cy: usize, tick: u64) -> f32 {
    if stratum == Stratum::Underground {
        return 0.0;
    }
    let day_frac = (tick as f64 * TICK_LEN as f64 / DAY_LEN as f64).fract() as f32;
    let daylight = LIGHT_NIGHT_FLOOR
        + (1.0 - LIGHT_NIGHT_FLOOR) * (0.5 + 0.5 * (std::f32::consts::TAU * day_frac).sin());
    let lat = 1.0 - (2.0 * cy as f32 / ROWS as f32 - 1.0).abs(); // 0 poles .. 1 equator
    let l = daylight * (0.4 + 0.6 * lat);
    if stratum == Stratum::Water {
        l * WATER_LIGHT_MULT
    } else {
        l
    }
}

impl Sim {
    /// Spawn the founder population with the default config (all features on) — the golden path.
    pub fn new(world_seed: u64, terrain: &VoxelTerrain) -> Self {
        Self::with_config(world_seed, terrain, SimConfig::default())
    }

    /// Spawn the founder population on land columns, deterministically from `world_seed`, under a
    /// given runtime config. `cfg` is part of the sim's input, so `(seed, cfg)` replays exactly.
    pub fn with_config(world_seed: u64, terrain: &VoxelTerrain, cfg: SimConfig) -> Self {
        let mut creatures = Vec::with_capacity(START_CREATURES);
        for i in 0..START_CREATURES as u64 {
            let mut rng = Rng::new(seed_fold(world_seed, &[SALT_FOUNDER, i]));
            // Place on land: a few tries to dodge water, else accept (clamped) wherever.
            let mut pos = vec2(0.0, 0.0);
            for _ in 0..FOUNDER_PLACE_TRIES {
                pos = vec2(rng.unit() * COLS as f32 * VOX, rng.unit() * ROWS as f32 * VOX);
                let (cx, cy) = column_index(pos);
                if !terrain.is_water(cx, cy) {
                    break;
                }
            }
            let genome = Genome::founder(&mut rng); // empty GRN → single cell (== C0)
            let pheno = genome.develop();
            creatures.push(Creature {
                id: i,
                founder: i,
                pos,
                heading: rng.unit() * std::f32::consts::TAU,
                energy: START_ENERGY,
                age: 0,
                alive: true,
                genome,
                pheno,
            });
        }
        Sim {
            creatures,
            world_seed,
            next_id: START_CREATURES as u64,
            births: 0,
            deaths: 0,
            kills: 0,
            grid: SpatialGrid::default(),
            registry: PressureRegistry::build(&cfg.features, &cfg.params),
            cfg,
            profiler: crate::profile::Profiler::default(),
        }
    }

    /// Capture the full live sim state for a save snapshot. Everything `state_checksum` folds plus
    /// the counters and config needed to resume; the `grid` (rebuilt each tick) and `registry`
    /// (rebuilt from `cfg`) are derived, so they are not stored.
    pub fn to_state(&self) -> SimState {
        SimState {
            world_seed: self.world_seed,
            next_id: self.next_id,
            births: self.births,
            deaths: self.deaths,
            kills: self.kills,
            cfg: self.cfg,
            creatures: self.creatures.clone(),
        }
    }

    /// Rebuild a `Sim` from a restored snapshot state. The pressure registry is rebuilt from the
    /// restored `cfg` and the spatial grid starts empty (repopulated on the next `step`), so the
    /// resumed run is bit-identical to the saved one (verified by a `state_checksum` round-trip test).
    pub fn from_state(state: SimState) -> Self {
        let registry = PressureRegistry::build(&state.cfg.features, &state.cfg.params);
        Sim {
            creatures: state.creatures,
            world_seed: state.world_seed,
            next_id: state.next_id,
            births: state.births,
            deaths: state.deaths,
            kills: state.kills,
            grid: SpatialGrid::default(),
            registry,
            cfg: state.cfg,
            profiler: crate::profile::Profiler::default(),
        }
    }

    /// Per-phase timing report (`(span, mean_ms, max_ms)` over the profiler's window) — for the
    /// headless `--profile` table, the HUD perf panel and the dev-bridge. Pure measurement.
    pub fn profile_report(&self) -> Vec<(crate::profile::Span, f32, f32)> {
        self.profiler.report()
    }

    /// Live Amdahl split `(serial_ms, parallel_ms, serial_fraction)` of a tick — the speedup ceiling
    /// from more cores is `1 / serial_fraction`, and it tightens as population/complexity grow.
    pub fn profile_amdahl(&self) -> (f32, f32, f32) {
        self.profiler.amdahl()
    }

    /// Enable/disable the phase profiler (default on). Off ⇒ the timing windows freeze; never affects
    /// determinism either way.
    pub fn set_profiling(&mut self, on: bool) {
        self.profiler.set_enabled(on);
    }

    /// The current runtime config (features + params).
    pub fn config(&self) -> SimConfig {
        self.cfg
    }

    /// Replace the runtime config live (e.g. from the dev bridge): updates the feature gates and
    /// rebuilds the pressure registry, so a changed membership / parameter takes effect from the
    /// next tick. `(seed, cfg)` still replays — the new cfg is simply the input from here on.
    pub fn set_config(&mut self, cfg: SimConfig) {
        self.cfg = cfg;
        self.registry = PressureRegistry::build(&cfg.features, &cfg.params);
    }

    /// One fixed sim tick. Multi-phase so the outcome is independent of iteration order:
    /// (a) snapshot the world + a spatial index, every creature senses (plant field + nearest
    /// prey/threat) and decides; (b) predation pass — resolve hunts by snapshot index, flagging
    /// eaten prey dead and crediting predators (trophic transfer); (c) apply per survivor in
    /// index order (move, graze — diet-scaled, mutates the terrain — metabolise, deaths,
    /// births); (d) compact dead out, append births, cull to the cap deterministically.
    pub fn step(&mut self, terrain: &mut VoxelTerrain, tick: u64) {
        let n = self.creatures.len();
        let (maxx, maxy) = (COLS as f32 * VOX, ROWS as f32 * VOX);
        // Feature toggles + params, read once as `Copy` locals so the hot closures don't borrow
        // `self`. All-on/default reproduces the pre-config behaviour bit-for-bit.
        let feat = self.cfg.features;
        let camo_base = self.cfg.params.camo_base_detect;
        // Seasonal phase this tick in [-1, 1] (a pure function of the tick). Computed always; only
        // the (default-off) seasonality pressure reads it, so it's inert unless enabled.
        let season_phase =
            (std::f32::consts::TAU * tick as f32 * TICK_LEN / self.cfg.params.season_len.max(1e-3)).sin();
        // (a) snapshot + decide — reads only, terrain unmutated. Snapshot arrays feed the grid
        // predicates without borrowing `self.creatures` inside the closures.
        let t_snapshot = Instant::now();
        let pos: Vec<Vec2> = self.creatures.iter().map(|c| c.pos).collect();
        let bm: Vec<u32> = self.creatures.iter().map(|c| c.biomass()).collect();
        let carn: Vec<f32> = self.creatures.iter().map(|c| c.pheno.carnivory()).collect();
        let color: Vec<f32> = self.creatures.iter().map(|c| c.genome.coloration).collect();
        // Ground tone each creature stands on (camouflage background, snapshot — read-only).
        let bg: Vec<f32> = self
            .creatures
            .iter()
            .map(|c| {
                let (cx, cy) = column_index(c.pos);
                terrain.ground_tone_at(cx, cy)
            })
            .collect();
        // Each creature's stratum (from its body + whether its column is water) and the per-stratum
        // headcount (for the density-split non-surface food). A stratum is a predator refuge:
        // hunting only reaches prey in the SAME stratum.
        let strata: Vec<Stratum> = self
            .creatures
            .iter()
            .map(|c| {
                if !feat.strata {
                    return Stratum::Surface; // strata off ⇒ one flat layer
                }
                let (cx, cy) = column_index(c.pos);
                stratum_of(&c.pheno, terrain.is_water(cx, cy))
            })
            .collect();
        let mut stratum_count = [0.0f32; 4];
        for s in &strata {
            stratum_count[s.idx()] += 1.0;
        }
        // Autotroph self-shading (F3: computed in the snapshot phase, order-independent). Light is
        // a finite flux, so more autotrophs ⇒ less photosynthesis per head ⇒ the niche self-limits.
        let n_auto = self.creatures.iter().filter(|c| c.pheno.photo_frac() > PHOTO_THETA).count();
        let autotroph_shading = 1.0 / (1.0 + n_auto as f32 / PHOTO_SOFTCAP);
        self.profiler.record(Span::Snapshot, t_snapshot.elapsed());
        let t_grid = Instant::now();
        self.grid.rebuild(&pos, maxx, maxy, GRID_CELL);
        self.profiler.record(Span::GridRebuild, t_grid.elapsed());
        // Decision per creature: (throttle, turn, optional prey index to attack THIS tick).
        // This phase is READ-ONLY (snapshots + the rebuilt grid + the terrain getters), so it runs
        // in parallel across cores. `par_iter().map().collect()` writes each `decisions[i]` from an
        // independent task and collects IN INDEX ORDER, and the only randomness is `seed_fold`ed per
        // `(i, j, tick)` — so the result is BIT-IDENTICAL to the serial loop (the golden holds).
        // Shared reborrows so the parallel closure captures `&Sim`/`&VoxelTerrain` (both `Sync`),
        // never the `&mut` from `step`.
        let t_decide = Instant::now();
        let this: &Sim = self;
        let terrain_ref: &VoxelTerrain = terrain;
        let decisions: Vec<(f32, f32, Option<usize>)> = (0..n)
            .into_par_iter()
            .map(|i| {
                // Predation off ⇒ no creature targets prey ⇒ the predation pass resolves nothing.
                let predator = feat.predation && carn[i] > CARNIVORE_THRESHOLD;
                let self_bm = bm[i];
                let self_layer = strata[i];
                // Nearest edible prey (only if predatory) AND nearest threatening predator, one pass.
                // Both restricted to the SAME stratum — a creature in another layer is out of reach.
                // Camouflage gates DETECTION: a predator only sees (and so only targets) a prey it
                // spots, with probability rising in the contrast of the prey's coloration vs its
                // ground. Deterministic per (predator, prey, tick).
                let detected = |j: usize| {
                    if !feat.camouflage {
                        return true; // camouflage off ⇒ prey always detectable
                    }
                    let contrast = (color[j] - bg[j]).abs();
                    let p = camo_base + (1.0 - camo_base) * contrast;
                    Rng::new(seed_fold(this.world_seed, &[SALT_CAMO, i as u64, j as u64, tick])).unit() <= p
                };
                // Per-creature sensing reach (sensor ORGAN power): a sharper-sensed body detects
                // prey/threats farther AND feels the food gradient farther (inside `sense`).
                let c = &this.creatures[i];
                let reach = SENSE_RANGE * c.sense_mult();
                let (prey, threat) = this.grid.nearest2_within(
                    &pos,
                    pos[i],
                    reach,
                    |j| predator && j != i && bm[j] <= self_bm && strata[j] == self_layer && detected(j),
                    |j| j != i && carn[j] > CARNIVORE_THRESHOLD && bm[j] >= self_bm && strata[j] == self_layer,
                );
                let prey_rel = prey.map(|j| rel(pos[i], c.heading, pos[j], reach));
                let threat_rel = threat.map(|j| rel(pos[i], c.heading, pos[j], reach));
                let inputs = this.sense(c, terrain_ref, tick, prey_rel, threat_rel);
                let (throttle, turn) = c.think(&inputs);
                // Attack if the targeted prey is within striking distance at snapshot positions.
                let hunt = prey.filter(|&j| (pos[j] - pos[i]).length() <= ATTACK_RANGE);
                (throttle, turn, hunt)
            })
            .collect();
        self.profiler.record(Span::Decide, t_decide.elapsed());
        // (b) predation: resolve hunts by snapshot index. Flag prey dead (never remove mid-pass
        // — indices must stay stable, F7); credit the predator with the trophic-scaled energy.
        let t_predation = Instant::now();
        for i in 0..n {
            let Some(j) = decisions[i].2 else { continue };
            if !self.creatures[i].alive || !self.creatures[j].alive {
                continue; // predator died, or prey already eaten by a lower-index predator
            }
            // (Camouflage already gated targeting at sensing — a chosen prey was detectable.)
            let gain = (bm[j] as f32 * CELL_BIOMASS_COST + self.creatures[j].energy.max(0.0))
                * MEAT_EFFICIENCY
                * carn[i];
            self.creatures[j].alive = false;
            self.deaths += 1;
            self.kills += 1;
            let cap = self.creatures[i].max_energy();
            self.creatures[i].energy = (self.creatures[i].energy + gain).min(cap);
            // The predator gained ENERGY; the prey's MATTER returns to the nutrient pool at its
            // column (a kill site fertilises the ground — energy and matter are separate currencies).
            let (dx, dy) = column_index(pos[j]);
            terrain.deposit_nutrient(dx, dy, bm[j] as f32 * NUTRIENT_PER_CELL, tick);
        }
        self.profiler.record(Span::Predation, t_predation.elapsed());
        // (c) apply per surviving creature in index order.
        // Logistic birth gate from the population at the START of the tick (deterministic;
        // doesn't shift as births accrue). On the over-provisioned map this aggregate
        // competition term — not food — sets the equilibrium near `SOFT_CAP`.
        let t_apply = Instant::now();
        // Reproductions queued here (genome mutated, energy paid) but NOT yet developed — `develop()`
        // is the costly body-growth and is run in a parallel batch AFTER this serial loop. The loop
        // holds `&mut self.creatures`, so the heavy work must leave it.
        let mut pending: Vec<PendingBirth> = Vec::new();
        let birth_gate = (1.0 - n as f32 / SOFT_CAP).clamp(0.0, 1.0);
        for (idx, c) in self.creatures.iter_mut().enumerate() {
            if !c.alive {
                continue; // eaten in the predation pass
            }
            let (throttle, turn, _) = decisions[idx];
            // Move. `layer` is the snapshot stratum (start-of-tick position) — it also drives the
            // drift term in `speed()` and is reused below for food/light. `spd` is captured once so
            // the movement cost can be charged per distance travelled (`MOVE_COST·throttle·spd`).
            let layer = strata[idx];
            let spd = c.speed(layer);
            c.heading += turn * TURN_RATE * TICK_LEN;
            let step = throttle * spd * TICK_LEN;
            c.pos.x += c.heading.cos() * step;
            c.pos.y += c.heading.sin() * step;
            // Map edge = wall (reflect), via the single clamp helper for the column.
            if c.pos.x < 0.0 {
                c.pos.x = 0.0;
                c.heading = std::f32::consts::PI - c.heading;
            } else if c.pos.x > maxx {
                c.pos.x = maxx;
                c.heading = std::f32::consts::PI - c.heading;
            }
            if c.pos.y < 0.0 {
                c.pos.y = 0.0;
                c.heading = -c.heading;
            } else if c.pos.y > maxy {
                c.pos.y = maxy;
                c.heading = -c.heading;
            }
            let (cx, cy) = column_index(c.pos);
            // Environmental selection pressures (climate / autotrophy / metabolism) compose into
            // one Effect for this creature; its channels plug into the energy budget below, bit-for-
            // bit as the former inline formulas (food_mult ← climate, energy_add ← photosynthesis,
            // metab_mult ← stratum cost). Predation, camouflage and the nutrient cycle are MECHANIC
            // pressures (a multi-creature pass / a per-pair detection roll / terrain mutation), not
            // pure per-creature channel effects, so they stay explicit phases in `step`.
            let sample = Sample {
                pheno: &c.pheno,
                genome: &c.genome,
                layer,
                temperature: terrain.temperature_at(cx, cy),
                light: light_for(layer, cy, tick),
                toxicity: terrain.toxicity_at(cx, cy),
                oxygen: terrain.oxygen_at(cx, cy, tick),
                season_phase,
                autotroph_shading,
            };
            let eff = self.registry.eval_all(&sample);
            let food = if layer == Stratum::Surface {
                // Surface feeds on the positioned S3 plant field; intake scales with body size, a
                // carnivore digests plants poorly (efficiency = 1 − carnivory).
                let taken = terrain.graze(cx, cy, c.intake() * TICK_LEN, tick);
                taken * PLANT_BIOMASS_TO_ENERGY * (1.0 - c.pheno.carnivory())
            } else {
                // Non-surface strata: a fixed foraging capacity split among occupants (density-
                // dependent → an empty stratum richly rewards colonisers, then self-limits).
                layer.capacity() / stratum_count[layer.idx()].max(1.0) * TICK_LEN
            };
            c.energy = (c.energy + food * eff.food_mult + eff.energy_add).min(c.max_energy());
            // O2 production (gas cycle Phase 1): photosynthesis emits oxygen into this column as an
            // obligate byproduct, keyed on the ISOLATED `photo_yield` (not composed energy_add — F2).
            // Serial (this `&mut terrain`) + f32 store ⇒ deterministic, gentle deposits accumulate.
            if self.cfg.features.oxygen && eff.photo_yield > 0.0 {
                terrain.deposit_oxygen(cx, cy, eff.photo_yield * OXYGEN_PER_PHOTO, tick);
            }
            // Metabolism: Kleiber (biomass^0.75) × stratum cost (metab_mult) + movement effort
            // (charged per distance travelled: `MOVE_COST · throttle · spd`, so drift/idle is ~free).
            let kleiber = (c.biomass() as f32).powf(0.75);
            c.energy -= (SIM_BASE_METABOLISM * kleiber * eff.metab_mult + MOVE_COST * throttle * spd) * TICK_LEN;
            c.age += 1;
            // Toxic death (C3 abiotic): ground toxicity beyond the creature's resistance is a
            // per-tick death hazard (the `mortality_add` channel). Deterministic per (id, tick).
            // Like the other deaths, the matter returns to the nutrient pool at the death site.
            if eff.mortality_add > 0.0
                && Rng::new(seed_fold(self.world_seed, &[SALT_TOXIN, c.id, tick])).unit() < eff.mortality_add
            {
                c.alive = false;
                self.deaths += 1;
                let (dx, dy) = column_index(c.pos);
                terrain.deposit_nutrient(dx, dy, c.biomass() as f32 * NUTRIENT_PER_CELL, tick);
                continue;
            }
            // Death by starvation. The creature's matter returns to the nutrient pool here
            // (decomposition) — closing the cycle and re-fertilising the death site.
            if c.energy <= 0.0 {
                c.alive = false;
                self.deaths += 1;
                let (dx, dy) = column_index(c.pos);
                terrain.deposit_nutrient(dx, dy, c.biomass() as f32 * NUTRIENT_PER_CELL, tick);
                continue;
            }
            // Death by senescence: old-age probability rising with age² gives demographic
            // turnover. Scaled by 1/biomass — bigger bodies live longer (a real size benefit),
            // so multicellularity has a gradient to climb against its build + Kleiber costs.
            let sp = SENESCENCE_RATE * (c.age as f32 / LIFESPAN).powi(2) / c.biomass() as f32;
            if sp > 0.0 && Rng::new(seed_fold(self.world_seed, &[SALT_DEATH, c.id, tick])).unit() < sp {
                c.alive = false;
                self.deaths += 1;
                let (dx, dy) = column_index(c.pos);
                terrain.deposit_nutrient(dx, dy, c.biomass() as f32 * NUTRIENT_PER_CELL, tick);
                continue;
            }
            // Reproduction: bud a mutated child, splitting energy in half. Gated by the logistic
            // birth term (population self-limits near SOFT_CAP) on top of the energy threshold.
            let lucky = birth_gate >= 1.0
                || Rng::new(seed_fold(self.world_seed, &[SALT_BIRTH, c.id, tick])).unit() < birth_gate;
            if c.energy >= REPRO_ENERGY && lucky {
                let mut rng = Rng::new(seed_fold(self.world_seed, &[SALT_MUTATE, c.id, tick]));
                // Morphogen READ weights evolve on their OWN stream (PR-D2) so the coupling's activation
                // leaves `rng` — and thus the child's pos/heading later — byte-identical to the inert sim.
                let mut morph_rng = Rng::new(seed_fold(self.world_seed, &[SALT_MORPH, c.id, tick]));
                // O2 tolerance evolves on its OWN independent stream too (gas cycle F9), so adding the
                // gene consumes zero draws from `rng` (child pos/heading byte-identical to no-gas sim).
                let mut gas_rng = Rng::new(seed_fold(self.world_seed, &[SALT_GAS, c.id, tick]));
                // Mutate the genome (brain + GRN + morphogen + gas); the parent pays half its energy now.
                // The body is grown later, in the parallel develop phase (it doesn't touch the RNG), so
                // the post-mutate `rng` is queued to finish the birth there — same draw stream, same bits.
                let genome = c.genome.mutate(&mut rng, &mut morph_rng, &mut gas_rng, MUTATION_STD, GRN_MUTATION_STD);
                c.energy *= 0.5;
                pending.push(PendingBirth {
                    genome,
                    rng,
                    parent_pos: c.pos,
                    parent_energy: c.energy,
                    founder: c.founder,
                });
            }
        }
        self.profiler.record(Span::Apply, t_apply.elapsed());
        // (c2) develop the queued children — the parallel phase. DETERMINISM: `develop()` is a pure
        // function of the genome (no RNG, no shared/mutable state, no cross-body reduction), and each
        // parent owns an INDEPENDENT `rng`, so doing this across threads is bit-identical to the old
        // inline path. `into_par_iter().collect()` (Vec is an indexed parallel iterator) preserves
        // index order, and the pos/heading draws stay co-located with — and conditional on — the
        // develop result (a stillbirth draws nothing, exactly as before). The serial id/births
        // assignment is deferred to the ordered pass below. Do NOT move id assignment in here.
        let t_develop = Instant::now();
        let dev = feat.development;
        let born: Vec<Option<BornChild>> = if dev && pending.len() >= PAR_DEVELOP_THRESHOLD {
            pending.into_par_iter().map(|p| develop_birth(p, dev, maxx, maxy)).collect()
        } else {
            pending.into_iter().map(|p| develop_birth(p, dev, maxx, maxy)).collect()
        };
        self.profiler.record(Span::Develop, t_develop.elapsed());
        // (c3) append the survivors in index order, assigning the deterministic id sequence.
        let mut births: Vec<Creature> = Vec::with_capacity(born.len());
        for b in born.into_iter().flatten() {
            births.push(Creature {
                id: self.next_id,
                founder: b.founder,
                pos: b.pos,
                heading: b.heading,
                energy: b.energy,
                age: 0,
                alive: true,
                genome: b.genome,
                pheno: b.pheno,
            });
            self.next_id += 1;
            self.births += 1;
        }
        // (d) compact: drop the dead, append births, cull to the cap.
        let t_compact = Instant::now();
        self.creatures.retain(|c| c.alive);
        self.creatures.append(&mut births);
        self.cull_to_cap(tick);
        self.profiler.record(Span::Compact, t_compact.elapsed());
        self.profiler.commit_tick();
    }

    /// Build the brain inputs: the plant-biomass field ahead / left / right (a gradient to
    /// climb), own energy, the column's water distance, the nearest prey + threat cues (closeness
    /// and left/right bearing), and a bias. Read-only on the terrain.
    fn sense(
        &self,
        c: &Creature,
        terrain: &VoxelTerrain,
        tick: u64,
        prey: Option<(f32, f32)>,
        threat: Option<(f32, f32)>,
    ) -> [f32; N_INPUTS] {
        // Sample the food gradient at the creature's own reach: a sharper-sensed body feels biomass
        // farther ahead and steers toward food sooner (this is what makes the sensor organ pay off
        // for herbivores, which have no prey/threat to detect).
        let radius = SENSE_RADIUS * c.sense_mult();
        let sample = |angle: f32| {
            let p = vec2(c.pos.x + angle.cos() * radius, c.pos.y + angle.sin() * radius);
            let (cx, cy) = column_index(p);
            terrain.biomass_at(cx, cy, tick)
        };
        let (cx, cy) = column_index(c.pos);
        let (prey_prox, prey_bearing) = prey.unwrap_or((0.0, 0.0));
        let (threat_prox, threat_bearing) = threat.unwrap_or((0.0, 0.0));
        [
            terrain.biomass_at(cx, cy, tick),
            sample(c.heading),
            sample(c.heading + std::f32::consts::FRAC_PI_2),
            sample(c.heading - std::f32::consts::FRAC_PI_2),
            (c.energy / REPRO_ENERGY).min(1.0),
            terrain.water_dist_at(cx, cy) as f32 / 255.0,
            prey_prox,
            prey_bearing,
            threat_prox,
            threat_bearing,
            1.0,
        ]
    }

    /// Deterministic-random cull down to `SIM_POP_CAP` (sort the living by a splitmix key, drop
    /// the lowest). NOT tail-truncation — that would systematically kill the freshest newborns
    /// and bias selection against reproduction (which is the engine of diversity).
    fn cull_to_cap(&mut self, tick: u64) {
        if self.creatures.len() <= SIM_POP_CAP {
            return;
        }
        let seed = self.world_seed ^ SALT_CULL;
        let key = |c: &Creature| splitmix64(seed.wrapping_add(splitmix64(tick).wrapping_add(c.id)));
        self.creatures.sort_unstable_by_key(key);
        let removed = self.creatures.len() - SIM_POP_CAP;
        self.deaths += removed as u64;
        self.creatures.truncate(SIM_POP_CAP);
    }

    pub fn population(&self) -> usize {
        self.creatures.len()
    }

    pub fn avg_energy(&self) -> f32 {
        if self.creatures.is_empty() {
            return 0.0;
        }
        self.creatures.iter().map(|c| c.energy).sum::<f32>() / self.creatures.len() as f32
    }

    /// Mean body size (cells) — the emergent biomass; >1 means multicellular bodies took hold.
    pub fn avg_biomass(&self) -> f32 {
        if self.creatures.is_empty() {
            return 0.0;
        }
        self.creatures.iter().map(|c| c.biomass() as f32).sum::<f32>() / self.creatures.len() as f32
    }

    /// Fraction of the population that is multicellular (biomass > 1) and complex (≥2 cell types).
    pub fn complexity_mix(&self) -> (f32, f32) {
        let n = self.creatures.len();
        if n == 0 {
            return (0.0, 0.0);
        }
        let multi = self.creatures.iter().filter(|c| c.pheno.complexity() >= 1).count();
        let complex = self.creatures.iter().filter(|c| c.pheno.complexity() == 2).count();
        (multi as f32 / n as f32, complex as f32 / n as f32)
    }

    /// Allopatry metric: Pearson correlation between each creature's evolved thermal preference
    /// and the actual temperature where it lives. ~0 = no climate adaptation (generalists
    /// everywhere); → 1 = lineages have sorted into the climate band they're suited to (habitats).
    pub fn thermal_correlation(&self, terrain: &VoxelTerrain) -> f32 {
        let n = self.creatures.len();
        if n < 2 {
            return 0.0;
        }
        let mut prefs = Vec::with_capacity(n);
        let mut temps = Vec::with_capacity(n);
        for c in &self.creatures {
            let (cx, cy) = column_index(c.pos);
            prefs.push(c.genome.thermal_pref);
            temps.push(terrain.temperature_at(cx, cy));
        }
        pearson(&prefs, &temps)
    }

    /// Toxic-adaptation metric: Pearson correlation between each creature's evolved
    /// `toxin_resistance` and the ground toxicity where it lives. ~0 = no adaptation; → 1 = resistant
    /// lineages have sorted onto the toxic ground (the toxicity pressure has bitten).
    pub fn toxin_correlation(&self, terrain: &VoxelTerrain) -> f32 {
        let n = self.creatures.len();
        if n < 2 {
            return 0.0;
        }
        let mut res = Vec::with_capacity(n);
        let mut tox = Vec::with_capacity(n);
        for c in &self.creatures {
            let (cx, cy) = column_index(c.pos);
            res.push(c.genome.toxin_resistance);
            tox.push(terrain.toxicity_at(cx, cy));
        }
        pearson(&res, &tox)
    }

    /// Fraction of the population in each stratum `[underground, surface, air, water]` — shows
    /// whether vertical niches (burrowers / fliers / swimmers) have been colonised.
    pub fn stratum_mix(&self, terrain: &VoxelTerrain) -> [f32; 4] {
        let n = self.creatures.len();
        if n == 0 {
            return [0.0; 4];
        }
        let mut m = [0.0f32; 4];
        for c in &self.creatures {
            let (cx, cy) = column_index(c.pos);
            m[stratum_of(&c.pheno, terrain.is_water(cx, cy)).idx()] += 1.0;
        }
        for v in &mut m {
            *v /= n as f32;
        }
        m
    }

    /// Mean nutrient level (`[0,1]`) at the columns creatures occupy — the realised fertility of
    /// the inhabited landscape. Falls where grazing strips the ground, rises where deaths return
    /// matter + weathering replenishes; a healthy bounded value means the cycle is self-sustaining.
    pub fn avg_nutrient(&self, terrain: &VoxelTerrain, tick: u64) -> f32 {
        if self.creatures.is_empty() {
            return 0.0;
        }
        let s: f32 = self
            .creatures
            .iter()
            .map(|c| {
                let (cx, cy) = column_index(c.pos);
                terrain.nutrient_at(cx, cy, tick)
            })
            .sum();
        s / self.creatures.len() as f32
    }

    /// Niche coverage: how many DISTINCT ecological niches are occupied — the cross-product of
    /// stratum × diet (herbivore/carnivore) × autotrophy × climate band × complexity tier, counted
    /// over distinct occupied combinations. Rises as the population radiates into the niche space
    /// C3 built; a single-niche monoculture would score ~1. Cheap (O(N) + a small set).
    pub fn niche_coverage(&self, terrain: &VoxelTerrain) -> usize {
        let mut seen = std::collections::HashSet::new();
        for c in &self.creatures {
            let (cx, cy) = column_index(c.pos);
            let stratum = stratum_of(&c.pheno, terrain.is_water(cx, cy)).idx() as u32;
            let carn = (c.pheno.carnivory() > CARNIVORE_THRESHOLD) as u32;
            let auto = (c.pheno.photo_frac() > PHOTO_THETA) as u32;
            let climate = (c.genome.thermal_pref * 2.99) as u32; // 0 cold .. 2 hot
            let cplx = c.pheno.complexity() as u32;
            seen.insert(stratum + 4 * (carn + 2 * (auto + 2 * (climate + 3 * cplx))));
        }
        seen.len()
    }

    /// Species count by LEADER clustering on a phenotype feature vector (body-type composition +
    /// size + climate + colour): a creature joins the first leader within `SPECIES_THRESHOLD`,
    /// else founds a new one. So distinct body plans / niches separate into clades. O(N × species)
    /// — call occasionally (it backs a throttled HUD readout + the dev bridge), not every tick.
    pub fn species_count(&self) -> usize {
        let mut leaders: Vec<[f32; FEATURES]> = Vec::new();
        for c in &self.creatures {
            let f = feature(c);
            if !leaders.iter().any(|l| feature_dist2(l, &f) <= SPECIES_THRESHOLD * SPECIES_THRESHOLD) {
                leaders.push(f);
            }
        }
        leaders.len()
    }

    /// Indices of the creatures that share the species (body-plan cluster) of the creature with
    /// `id`, EXCLUDING that creature itself — same `SPECIES_THRESHOLD` feature radius the species
    /// metric uses. Backs the inspector's conspecific markers. O(N): one pass once the target's
    /// feature is known; returns `[]` if `id` isn't present.
    pub fn conspecifics(&self, id: u64) -> Vec<usize> {
        let Some(target) = self.creatures.iter().find(|c| c.id == id) else {
            return Vec::new();
        };
        let tf = feature(target);
        let thr = SPECIES_THRESHOLD * SPECIES_THRESHOLD;
        self.creatures
            .iter()
            .enumerate()
            .filter(|(_, c)| c.id != id && feature_dist2(&tf, &feature(c)) <= thr)
            .map(|(i, _)| i)
            .collect()
    }

    /// Crypsis metric: Pearson correlation between each creature's coloration and the ground tone
    /// where it lives. ~0 = random colours; → 1 = creatures have evolved to match their background
    /// (camouflage), differently per habitat — a coevolutionary outcome of the detection channel.
    pub fn crypsis_correlation(&self, terrain: &VoxelTerrain) -> f32 {
        let n = self.creatures.len();
        if n < 2 {
            return 0.0;
        }
        let (mut cols, mut tones) = (Vec::with_capacity(n), Vec::with_capacity(n));
        for c in &self.creatures {
            let (cx, cy) = column_index(c.pos);
            cols.push(c.genome.coloration);
            tones.push(terrain.ground_tone_at(cx, cy));
        }
        pearson(&cols, &tones)
    }

    /// Fraction of the population that is autotrophic (photosynthesises — a producer tier inside
    /// the creature substrate, not just the exogenous plant field).
    pub fn frac_autotroph(&self) -> f32 {
        let n = self.creatures.len();
        if n == 0 {
            return 0.0;
        }
        let auto = self.creatures.iter().filter(|c| c.pheno.photo_frac() > PHOTO_THETA).count();
        auto as f32 / n as f32
    }

    /// Fraction of the population that is predatory (a second trophic level has appeared).
    pub fn frac_carnivore(&self) -> f32 {
        let n = self.creatures.len();
        if n == 0 {
            return 0.0;
        }
        let carn = self.creatures.iter().filter(|c| c.pheno.carnivory() > CARNIVORE_THRESHOLD).count();
        carn as f32 / n as f32
    }

    /// Fraction of the live population in each [`TrophicNiche`], in `TrophicNiche::ALL` order
    /// (mutually exclusive ⇒ sums to 1.0; empty population ⇒ all zero). The population panel iterates
    /// this, so a new niche variant produces a new bar with no UI change. O(N · niches).
    pub fn trophic_fractions(&self) -> Vec<(TrophicNiche, f32)> {
        let n = self.creatures.len();
        let inv = if n == 0 { 0.0 } else { 1.0 / n as f32 };
        TrophicNiche::ALL
            .iter()
            .map(|&niche| {
                let count = self
                    .creatures
                    .iter()
                    .filter(|c| TrophicNiche::classify(&c.pheno) == niche)
                    .count();
                (niche, count as f32 * inv)
            })
            .collect()
    }

    /// Fraction of the population that has developed a coherent ORGAN — a connected same-type cluster
    /// of at least `ORGAN_MIN` cells (PR-C). Founders (single cells) have none; this rises as bodies
    /// evolve real tissues.
    pub fn frac_with_organ(&self) -> f32 {
        let n = self.creatures.len();
        if n == 0 {
            return 0.0;
        }
        let with = self
            .creatures
            .iter()
            .filter(|c| c.pheno.organ.iter().any(|&l| l >= ORGAN_MIN))
            .count();
        with as f32 / n as f32
    }

    /// Fraction of the population that has developed an emergent body AXIS — `axis_order >= AXIS_MIN`
    /// (PR-D2). Founders (single cells) have `axis_order = 0`; this rises from zero only as the
    /// morphogen READ weights (`morph_w`) evolve to couple cell type to radial position.
    pub fn frac_with_axis(&self) -> f32 {
        let n = self.creatures.len();
        if n == 0 {
            return 0.0;
        }
        let with = self.creatures.iter().filter(|c| c.pheno.axis_order >= AXIS_MIN).count();
        with as f32 / n as f32
    }

    /// Mean `axis_order` over the population (PR-D2 observability) — the average strength of the
    /// type↔position coupling, `0..=255`. Rises from 0 as the morphogen coupling evolves.
    pub fn avg_axis_order(&self) -> f32 {
        let n = self.creatures.len();
        if n == 0 {
            return 0.0;
        }
        self.creatures.iter().map(|c| c.pheno.axis_order as f32).sum::<f32>() / n as f32
    }

    /// Pearson correlation between `axis_order` and body size (`n_cells`) over the population — the
    /// DECORRELATION control for the axis-emerges acceptance (PR-D2). `axis_order` is a scale-invariant
    /// η² RATIO by construction, so a body plan must be a genuine type↔position structure, NOT a
    /// by-product of growing more cells: this correlation must stay well below 1 (a big blob without a
    /// gradient scores ≈0). Read-only observer; f64 serial sum (off the determinism-critical path).
    pub fn axis_size_correlation(&self) -> f32 {
        let n = self.creatures.len();
        if n < 2 {
            return 0.0;
        }
        let (xs, ys): (Vec<f64>, Vec<f64>) =
            self.creatures.iter().map(|c| (c.pheno.axis_order as f64, c.pheno.n_cells as f64)).unzip();
        let nf = n as f64;
        let (mx, my) = (xs.iter().sum::<f64>() / nf, ys.iter().sum::<f64>() / nf);
        let (mut cov, mut vx, mut vy) = (0.0, 0.0, 0.0);
        for (x, y) in xs.iter().zip(&ys) {
            cov += (x - mx) * (y - my);
            vx += (x - mx).powi(2);
            vy += (y - my).powi(2);
        }
        if vx <= 0.0 || vy <= 0.0 {
            return 0.0; // no variance in one axis ⇒ correlation undefined ⇒ report 0
        }
        (cov / (vx.sqrt() * vy.sqrt())) as f32
    }
}

/// Full-state determinism checksum (PR1 lock, F1/F7): an integer fold of the COMPLETE
/// deterministic sim+terrain state — every creature's identity, kinematics, energy, age,
/// liveness, full genome and developed phenotype, the id/counter state, and the mutable terrain
/// fields. Floats are folded by `f32::to_bits` (never float-add — F2). This is the bit-exact lock
/// every later refactor is checked against: counts can collide by luck, a full-state hash cannot.
/// (Used by the determinism-checksum tests now; by the metrics-registry checksum metric in PR5.)
#[allow(dead_code)]
pub fn state_checksum(sim: &Sim, terrain: &VoxelTerrain) -> u64 {
    use crate::rng::{fnv_fold_u32, fnv_fold_u64, FNV_OFFSET};
    let mut h = FNV_OFFSET;
    fnv_fold_u64(&mut h, sim.next_id);
    fnv_fold_u64(&mut h, sim.births);
    fnv_fold_u64(&mut h, sim.deaths);
    fnv_fold_u64(&mut h, sim.kills);
    for c in &sim.creatures {
        fnv_fold_u64(&mut h, c.id);
        fnv_fold_u64(&mut h, c.founder);
        fnv_fold_u32(&mut h, c.pos.x.to_bits());
        fnv_fold_u32(&mut h, c.pos.y.to_bits());
        fnv_fold_u32(&mut h, c.heading.to_bits());
        fnv_fold_u32(&mut h, c.energy.to_bits());
        fnv_fold_u32(&mut h, c.age);
        fnv_fold_u64(&mut h, c.alive as u64);
        fnv_fold_u64(&mut h, c.genome.checksum());
        let p = &c.pheno;
        for v in [p.n_cells, p.effector, p.storage, p.sensor, p.predator, p.flight, p.burrow, p.photo, p.structural] {
            fnv_fold_u32(&mut h, v);
        }
        for &o in &p.organ {
            fnv_fold_u32(&mut h, o as u32); // organ coherence per type (PR-C; part of the body state)
        }
        fnv_fold_u32(&mut h, p.axis_order as u32); // axial body-plan order (PR-D1; part of the body state)
    }
    fnv_fold_u64(&mut h, terrain.mut_state_checksum());
    h
}

/// Golden checksum for `Sim::new(42)` stepped 300 fixed ticks on `VoxelTerrain::new(1)`. Pinned
/// in PR1 so any later change that perturbs the trajectory is caught at the introducing PR.
/// **Profile-specific:** debug and release do NOT produce bit-identical floats (LLVM fuses a*b+c
/// into an FMA in release, not debug) — the trajectory and this hash differ by profile. The sim is
/// deterministic *within* a profile, so we pin one golden per profile via `cfg!(debug_assertions)`.
/// Canonical verification profile is **release** (acceptance corridors are tuned there).
#[allow(dead_code)]
pub const GOLDEN_CHECKSUM_SEED42_300: u64 = if cfg!(debug_assertions) {
    7589643348835578897 // debug profile (re-pinned: terrain-gen base × movement rebalance)
} else {
    10375473682301875586 // release profile (re-pinned: terrain-gen base × movement rebalance)
};

/// Multi-cell determinism lock: `Sim::new(1)` stepped 8000 ticks grows complex MULTICELLULAR bodies,
/// so it catches FP-reassociation in the develop / reproduction path that the unicellular-dominated
/// seed-42/300 golden cannot (that path's float math only matters once `n_cells > 1`). Pinned when the
/// develop phase was parallelised. **Release only** — the canonical profile; an 8000-tick debug run is
/// too slow for routine testing. Re-pin (with a why-comment) only for an intended trajectory change.
#[cfg(not(debug_assertions))]
#[allow(dead_code)]
pub const GOLDEN_CHECKSUM_SEED1_8000: u64 = 2744380606710956587; // re-pinned: terrain-gen base × movement rebalance

#[cfg(test)]
#[path = "sim_tests.rs"]
mod tests;

