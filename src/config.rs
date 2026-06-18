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

/// Single knob to scale the whole map. The base footprint is 138×95 columns (metres);
/// the **eventual target is ×16 per side** (×256 area) — keep at 1 for now, because
/// `MAP_SCALE = 16` is 2208×1520 ≈ 3.36M columns and will need chunk *streaming*
/// (don't hold every chunk mesh at once) + aggressive culling, a separate phase.
pub const MAP_SCALE: usize = 8;
const BASE_COLS: usize = 138;
const BASE_ROWS: usize = 95;

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
