//! Tunable constants — window + the voxel **spatial metrics** (the coordinate
//! contract everything else builds on; fixed first, on purpose, so there are no
//! magic numbers smeared across the renderer).

// ---- Window ----
pub const WIN_W: i32 = 1100;
pub const WIN_H: i32 = 760;

// ---- Voxel spatial metrics (coordinate contract) ----
// macroquad 3D is **y-up**. A logical voxel `(gx, gy, gz)` maps to world space as
// `world = (gx*VOX, gz*VOX, gy*VOX)` — x to the right, **y up = height**, z into
// the scene. One block edge is `VOX` world units in every axis (cubic blocks), so
// the grid is trivially 3D-ready. Vertices are always built as `(g as f32)*VOX`
// (never accumulated `+= VOX`) so shared edges between chunks match bit-for-bit.

/// Block edge in world units (cubic). The orthographic camera scales the view, so
/// keeping this at 1.0 means logical and world coordinates differ only by axis
/// remap — simplest possible contract.
pub const VOX: f32 = 1.0;

/// World footprint in columns (x) × rows (z).
pub const COLS: usize = 138;
pub const ROWS: usize = 95;

/// Chunk side in columns. Stored ghost-padded to `CHUNK+2` so a chunk's mesh build
/// is self-contained (no cross-chunk reads, no bounds checks in the hot loop).
/// Consumed by phase-1 worldgen.
#[allow(dead_code)]
pub const CHUNK: usize = 16;

// ---- Vertical level budget (consumed by phase-1 worldgen) ----
/// Underground strata shown on cliff/edge cross-sections.
#[allow(dead_code)]
pub const UNDERGROUND_LEVELS: u8 = 4;
/// Surface height range above sea level (mountains rise up to this).
#[allow(dead_code)]
pub const SURFACE_RANGE: u8 = 6;
/// Water fills columns whose surface sits below this level.
#[allow(dead_code)]
pub const SEA_LEVEL: u8 = 2;
