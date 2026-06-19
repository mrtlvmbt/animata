//! Tunable constants — window + the voxel **spatial metrics** (the coordinate
//! contract everything else builds on; fixed first, on purpose, so there are no
//! magic numbers smeared across the renderer).

// ---- Window ----
pub const WIN_W: i32 = 1100;
pub const WIN_H: i32 = 760;

// ---- Voxel spatial metrics (coordinate contract) ----
// macroquad 3D is **y-up**. A logical voxel `(gx, gy, gz)` maps to world space as
// `world = (gx*VOX, gz*VOX, gy*VOX)` — x to the right, **y up = height**, z into
// the scene. Vertices are always built as `(g as f32)*VOX` (never accumulated
// `+= VOX`) so shared edges between chunks match bit-for-bit.
//
// **Physical scale: 1 voxel = 1 cubic metre** (`VOX` = 1 m edge). The future sim's
// creatures are mouse-sized (~0.12 m) and live in CONTINUOUS space on top of / inside
// the terrain — a cube is a *terrain cell*, not a creature slot. Density contract:
// up to ~8 creatures share a cube's VOLUME, ~4 share its top SURFACE (see the
// `CREATURE_*` constants). So at this scale the current map (138×95 m) holds on the
// order of `COLS*ROWS*4 ≈ 52k` surface creatures — re-tune when the sim returns.

/// Block edge in world units = **1 metre**. The orthographic camera scales the view,
/// so logical and world coordinates differ only by the axis remap above.
pub const VOX: f32 = 1.0;

/// Single knob to scale the whole map. The base footprint is 120×120 columns (metres);
/// at the **×16 per-side target** (×256 area) that is 1920×1920 = 3.69M columns, which
/// needs chunk *streaming* (don't hold every chunk mesh at once) + aggressive culling
/// (both landed in the worldgen E/F phases).
pub const MAP_SCALE: usize = 16;
// Square footprint (×16 ⇒ 1920×1920 columns = 120×120 chunks = 15×15 super-tiles).
const BASE_COLS: usize = 120;
const BASE_ROWS: usize = 120;

/// World footprint in columns (x) × rows (z) = metres. Derived from `MAP_SCALE`.
pub const COLS: usize = BASE_COLS * MAP_SCALE;
pub const ROWS: usize = BASE_ROWS * MAP_SCALE;

/// Chunk side in columns. Stored ghost-padded to `CHUNK+2` so a chunk's mesh build
/// is self-contained (no cross-chunk reads, no bounds checks in the hot loop).
pub const CHUNK: usize = 16;

// ---- Vertical level budget (metres) ----
/// Underground strata shown on cliff/edge cross-sections.
pub const UNDERGROUND_LEVELS: u8 = 4;
/// Land relief in **levels (= metres)** above the shoreline: the tallest peak stands
/// this many blocks above the lowest land (the "foot"). Raised to give erosion and
/// tectonics vertical room — deep valleys / tall ridges need resolution. Biome bands
/// in `terrain.rs` scale with this, so the area distribution stays the same, just
/// taller. Decoupled from how much of the map is water (`SEA_FRACTION` in `terrain.rs`),
/// so raising peaks doesn't drain the sea.
pub const SURFACE_RANGE: u8 = 40;
/// Water fills columns whose surface sits below this level.
pub const SEA_LEVEL: u8 = 2;

// ---- Sim time base (the WorldClock; consumed in clock.rs) ----
/// Fixed sub-step length in **sim-seconds** — the sim's time resolution. 0.1 s = 10 ticks
/// per sim-second: a compromise between integration accuracy and per-tick cost. The clock
/// counts whole ticks (`u64`), so this is the only place real and sim time meet.
pub const TICK_LEN: f32 = 0.1;
/// Spiral-of-death guard: the most sub-steps one rendered frame may run. Past this the
/// interactive clock drops the backlog instead of trying to catch up (so a lag spike can't
/// snowball). Headless `advance(n)` ignores this — it is the deterministic path.
pub const MAX_SUBSTEPS: u64 = 8;
/// Length of one in-world day in **sim-seconds**. Only feeds `day_frac()` for now (no
/// day/night visual yet — that's a later, deferred phase). Tunable.
pub const DAY_LEN: f32 = 600.0;

// ---- Vegetation (S3; consumed in terrain.rs) ----
/// Biomass regrow rate (per sim-second) for the linear-with-saturation law
/// `b' = cap − (cap − b)·e^(−RATE·elapsed)`. `0.01` ⇒ a ~100 s time-constant (a grazed
/// patch is most of the way back within a sim-day). Tunable.
pub const BIOMASS_REGROW_RATE: f32 = 0.01;

// ---- Life simulation: C0 unicellular ecosystem (consumed in sim.rs) ----
/// Founder creatures spawned at world start (on land columns).
pub const START_CREATURES: usize = 2000;
/// Hard population ceiling (deterministic random cull above it).
pub const SIM_POP_CAP: usize = 12000;
/// Energy a founder / newborn starts with.
pub const START_ENERGY: f32 = 50.0;
/// Energy at/above which a creature buds off a child (splitting its energy in half).
pub const REPRO_ENERGY: f32 = 100.0;
/// Energy is capped here so a well-fed creature that the logistic gate keeps from breeding
/// doesn't hoard energy without bound (it just sits full until it gets to reproduce).
pub const MAX_ENERGY: f32 = 200.0;
/// Base metabolic energy drain per **sim-second** at biomass 1 — scaled by `biomass^0.75`
/// (Kleiber) and by a climate factor. Movement adds on top.
pub const SIM_BASE_METABOLISM: f32 = 0.05;
/// Extra energy per sim-second at full throttle (movement effort).
pub const MOVE_COST: f32 = 0.05;
/// World units travelled per sim-second at full throttle.
pub const CREATURE_SPEED: f32 = 6.0;
/// Max turn rate (radians per sim-second).
pub const TURN_RATE: f32 = 3.0;
/// How much plant biomass (`[0,1]` field units) a creature grazes per sim-second (capped by
/// what the column holds).
pub const EAT_RATE: f32 = 0.8;
/// Energy gained per unit of grazed plant biomass (the herbivore conversion of the trophic
/// loop). Plant biomass is `[0,1]` per column; this sets its caloric worth.
pub const PLANT_BIOMASS_TO_ENERGY: f32 = 8.0;
/// Soft carrying capacity: reproduction is gated by `1 - N/SOFT_CAP`, so the population
/// self-limits HERE (well below the hard `SIM_POP_CAP` safety net). On the ×16 map the
/// vegetation supports far more than the single-thread budget, so food alone can't regulate a
/// ≤12k population — this aggregate competition term stands in until spatially food-limited
/// densities are reachable (chunked millions, a later scale phase).
pub const SOFT_CAP: f32 = 6000.0;
/// Senescence reference lifespan in ticks: per-tick old-age death probability rises as
/// `SENESCENCE_RATE·(age/LIFESPAN)²`, giving demographic turnover (so death isn't only the
/// random over-cap cull) and a real survival-to-reproduce selection pressure.
pub const LIFESPAN: f32 = 1500.0;
pub const SENESCENCE_RATE: f32 = 0.02;
/// Distance (world units) at which a creature samples the plant-biomass field to feel a
/// gradient (forward / left / right of its heading).
pub const SENSE_RADIUS: f32 = 5.0;
/// Std-dev of the Gaussian-ish weight perturbation applied to a child's brain on reproduction.
pub const MUTATION_STD: f32 = 0.12;
/// Std-dev for the developmental GRN genes on reproduction. Smaller than `MUTATION_STD` so the
/// BODY PLAN changes by rarer, gentler steps than behaviour — no mutational flood of giant
/// bodies (the body is the costly part).
pub const GRN_MUTATION_STD: f32 = 0.10;
/// C1 body-stat coupling: each effector cell adds this fraction to top speed.
pub const EFFECTOR_GAIN: f32 = 0.12;
/// Each storage cell adds this much to the energy cap.
pub const STORAGE_PER_CELL: f32 = 25.0;
/// Energy to build one cell beyond the first when budding a child — so a larger body costs
/// more to reproduce (an interior optimum) and is the mass a C2 predator will convert.
pub const CELL_BIOMASS_COST: f32 = 8.0;

// ---- C3: autotrophs (photosynthesis) ----
/// Energy per photosynthetic cell per sim-second at full light. The autotroph's income.
pub const PHOTO_RATE: f32 = 3.0;
/// Fraction of a body that must be photo cells to count as an autotroph (stats + shading).
pub const PHOTO_THETA: f32 = 0.15;
/// Self-shading soft cap: photosynthesis income is scaled by `1/(1 + n_autotrophs/PHOTO_SOFTCAP)`
/// — light is a finite flux, so autotrophs compete and the niche self-limits (like a stratum).
pub const PHOTO_SOFTCAP: f32 = 1500.0;
/// Dim-night light floor (so a global night doesn't starve every autotroph at once → wild
/// oscillation); enough day/night swing to reward STORAGE cells as a night buffer.
pub const LIGHT_NIGHT_FLOOR: f32 = 0.15;
/// Light reaching the water column (shallow-water simplification; depth-resolved later).
pub const WATER_LIGHT_MULT: f32 = 0.5;

// ---- C3: vertical strata (air / underground / water column) ----
/// Fraction of a body that must be flight / burrow / fin cells to occupy that stratum.
pub const STRATUM_THETA: f32 = 0.15;
/// Total foraging yield (energy / sim-second) of each non-surface stratum, split among its
/// occupants (density-dependent: empty strata richly reward first colonisers, then self-limit).
pub const AIR_CAPACITY: f32 = 320.0;
pub const UNDERGROUND_CAPACITY: f32 = 480.0;
pub const WATER_CAPACITY: f32 = 400.0;
/// Per-stratum metabolic multiplier — flight is dear (lift), burrowing cheap (sheltered).
pub const AIR_METAB_MULT: f32 = 1.6;
pub const UNDERGROUND_METAB_MULT: f32 = 0.7;

// ---- C3: habitat / climate niche ----
/// How steeply FOOD VALUE falls with the mismatch between local temperature and a creature's
/// evolved thermal preference (climate stress on foraging). `0.9` ⇒ a fully mismatched lineage
/// feeds at the `0.1` floor — strong pressure to adapt to the local climate band ⇒ allopatric
/// sorting. (Acts on food, the dominant energy channel, so it actually bites.)
pub const THERMAL_PENALTY: f32 = 0.9;

// ---- C2: predation / trophic web ----
/// A creature is predatory once predator cells make up this fraction of its body.
pub const CARNIVORE_THRESHOLD: f32 = 0.2;
/// World-unit radius at which a creature senses the nearest prey / threat (brain input).
pub const SENSE_RANGE: f32 = 30.0;
/// World-unit radius within which a predator actually strikes its targeted prey.
pub const ATTACK_RANGE: f32 = 3.5;
/// Spatial-grid cell size for prey/threat queries (≈ the sense range so each query touches a
/// small ring of cells).
pub const GRID_CELL: f32 = 32.0;
/// Trophic transfer efficiency: a predator gets this fraction of the prey's (structural mass +
/// reserve) energy, further scaled by the predator's carnivory. ~10% rule (real 5–20%).
pub const MEAT_EFFICIENCY: f32 = 0.35;

// ---- Creature density contract (documented now, consumed by the future sim) ----
/// Creature body size in metres (mouse-sized).
#[allow(dead_code)]
pub const CREATURE_SIZE_M: f32 = 0.12;
/// Max creatures sharing one cube's volume.
#[allow(dead_code)]
pub const CREATURES_PER_CUBE_VOLUME: u32 = 8;
/// Max creatures sharing one cube's top surface.
#[allow(dead_code)]
pub const CREATURES_PER_CUBE_SURFACE: u32 = 4;
