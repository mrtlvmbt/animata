//! Voxel terrain — the render-side world model: a chunked, bit-packed, ghost-padded
//! column grid generated from noise. No simulation here. Generation is kept
//! abstract (emits `BiomeKind` + heights only); colours/meshes live in the renderer
//! so generation and representation stay separate.
//!
//! Coordinates follow the config contract: a column `(x, y)` (x in `0..COLS`,
//! y in `0..ROWS`) carries a surface height `h` in levels; the solid block stack is
//! `gz in 0..h`. World space is `(x*VOX, gz*VOX, y*VOX)` (y-up).

use crate::config::*;

/// Baseline slab thickness in levels: even the lowest land keeps this many levels
/// below the surface, so cliff/edge cross-sections always show strata.
const GROUND_MIN: u8 = UNDERGROUND_LEVELS;
/// Absolute level the sea fills to. Columns at or below this are ocean; the water
/// surface is rendered as a translucent plane at this level (the renderer reads it).
pub const SEA_ABS: u8 = GROUND_MIN + SEA_LEVEL;
/// Lowest land level — the shoreline / mountain "foot", one above the water surface.
const LAND_FOOT: u8 = SEA_ABS + 1;
/// Tallest peak. Land relief (`SURFACE_RANGE`) is measured up from the foot.
const MAX_H: u8 = LAND_FOOT + SURFACE_RANGE;
/// Fraction of the elevation field that lies under the sea — sets how much of the map
/// is water, independently of how tall the land rises (so taller peaks ≠ less sea).
const SEA_FRACTION: f32 = 0.42;
/// Macro elevation lattice in columns (continents, mountain masses), with FEW octaves
/// → the big smooth structure. Scaled by `MAP_SCALE` so it grows with the map.
const ELEV_LATTICE: f32 = 26.0 * MAP_SCALE as f32;
const ELEV_OCTAVES: u32 = 3;
/// Detail field: MANY octaves at a fixed (absolute, not `MAP_SCALE`-scaled) coarsest
/// lattice, admixed at low amplitude. This is the "few-octave base + more-octave,
/// smaller-amplitude admix" — it adds local extrema/complexity on top of the macro
/// shape. Its high octaves carry tiny amplitude, so altitude-band edges stay clean.
const DETAIL_LATTICE: f32 = 22.0;
const DETAIL_OCTAVES: u32 = 5;
/// Detail admix amplitude in normalised `[0,1]` elevation units (≈ `±WEIGHT/2`).
const DETAIL_WEIGHT: f32 = 0.34;
/// Moisture lattice (columns) — sets how dry/wet a region is, choosing the lowland
/// biome (desert↔plains↔forest). Scaled by `MAP_SCALE` so moisture regions are big.
const MOIST_LATTICE: f32 = 21.0 * MAP_SCALE as f32;
const MOIST_OCTAVES: u32 = 3;
/// Ridged-noise field for mountain ridgelines (belts scale with the map). Domain-warped
/// by a broader field so ridges flow instead of forming blobs.
const RIDGE_LATTICE: f32 = 30.0 * MAP_SCALE as f32;
const RIDGE_OCTAVES: u32 = 4;
const WARP_LATTICE: f32 = 55.0 * MAP_SCALE as f32;
const WARP_AMP: f32 = 0.6;
/// Macro elevation at which ridges start to bite (below this the land stays rolling).
const RIDGE_ONSET: f32 = 0.55;
/// Ridge amplitude in normalised `[0,1]` elevation units (crest lift / trough carve).
const RIDGE_WEIGHT: f32 = 0.42;

/// Abstract biome class from worldgen — carries no colours (those live in the
/// render palette). Up to 16 kinds (4 bits in the packed cell).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BiomeKind {
    Ocean,
    Beach,
    Plains,
    Forest,
    Desert,
    Mountain,
    Snow,
}

impl BiomeKind {
    pub fn id(self) -> u8 {
        match self {
            BiomeKind::Ocean => 0,
            BiomeKind::Beach => 1,
            BiomeKind::Plains => 2,
            BiomeKind::Forest => 3,
            BiomeKind::Desert => 4,
            BiomeKind::Mountain => 5,
            BiomeKind::Snow => 6,
        }
    }
    pub fn from_id(id: u8) -> BiomeKind {
        match id {
            0 => BiomeKind::Ocean,
            1 => BiomeKind::Beach,
            3 => BiomeKind::Forest,
            4 => BiomeKind::Desert,
            5 => BiomeKind::Mountain,
            6 => BiomeKind::Snow,
            _ => BiomeKind::Plains,
        }
    }
}

// ---- Bit-packed cell (u16): bits 0-7 height, 8-11 biome id, 12-15 flags ----

/// Cell flag: this column is water (filled to `SEA_ABS`).
pub const FLAG_WATER: u8 = 1 << 0;

pub fn pack_cell(h: u8, biome: BiomeKind, flags: u8) -> u16 {
    (h as u16) | ((biome.id() as u16) << 8) | (((flags & 0xF) as u16) << 12)
}
pub fn cell_height(c: u16) -> u8 {
    (c & 0xFF) as u8
}
pub fn cell_biome(c: u16) -> BiomeKind {
    BiomeKind::from_id(((c >> 8) & 0xF) as u8)
}
/// Used by `is_water` (future-sim query) and the bit-pack test.
#[allow(dead_code)]
pub fn cell_flags(c: u16) -> u8 {
    ((c >> 12) & 0xF) as u8
}

// ---- Value noise (fresh, self-contained) ----

fn hash2(seed: u64, x: i64, y: i64, salt: u64) -> f32 {
    // Combine the inputs, then a FULL fmix64 (two multiplies). The earlier
    // half-fmix didn't avalanche the top bits for adjacent seeds, so seed 1 vs 2
    // produced near-identical fields.
    let mut h = seed;
    h ^= (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h = h.rotate_left(31);
    h ^= (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h = h.rotate_left(29);
    h ^= salt.wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    h = h.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    h ^= h >> 33;
    (h >> 40) as f32 / (1u64 << 24) as f32 // [0, 1)
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn value_noise(seed: u64, x: f32, y: f32, salt: u64) -> f32 {
    let x0 = x.floor();
    let y0 = y.floor();
    let (ix, iy) = (x0 as i64, y0 as i64);
    let tx = smoothstep(x - x0);
    let ty = smoothstep(y - y0);
    let c00 = hash2(seed, ix, iy, salt);
    let c10 = hash2(seed, ix + 1, iy, salt);
    let c01 = hash2(seed, ix, iy + 1, salt);
    let c11 = hash2(seed, ix + 1, iy + 1, salt);
    let top = c00 + (c10 - c00) * tx;
    let bot = c01 + (c11 - c01) * tx;
    top + (bot - top) * ty
}

/// Fractal value-noise (fBm) in `[0, 1]`: `octaves` octaves, each at double frequency
/// and half amplitude. (Value-noise fractal — same multi-scale look as Perlin fBm,
/// cheaper.) More octaves = finer detail; the highest octaves carry little amplitude.
fn fbm(seed: u64, x: f32, y: f32, salt: u64, octaves: u32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 1.0;
    let mut freq = 1.0;
    let mut norm = 0.0;
    for o in 0..octaves {
        sum += amp * value_noise(seed, x * freq, y * freq, salt.wrapping_add(o as u64 * 0x9E37_79B1));
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    (sum / norm).clamp(0.0, 1.0)
}

/// Combined surface elevation in `[0, 1]`: a few-octave macro field (continents,
/// mountain masses) admixed with a many-octave, low-amplitude detail field (local
/// hills + roughness). One height function with global AND local extrema — both
/// height and biome read from it, so colour follows altitude.
fn elevation(seed: u64, x: f32, y: f32) -> f32 {
    let macro_e = fbm(seed, x / ELEV_LATTICE, y / ELEV_LATTICE, 1, ELEV_OCTAVES);
    let detail = fbm(seed, x / DETAIL_LATTICE, y / DETAIL_LATTICE, 5, DETAIL_OCTAVES) - 0.5;

    // Ridged noise for mountain ridgelines/cliffs, applied only where the macro field
    // is already high (so lowlands stay rolling). Domain-warp the sample so ridges
    // flow organically instead of forming round blobs. `1 - |2n-1|` peaks at 1 along
    // ridgelines: positive lifts crests, negative carves the troughs between them
    // (which, reaching the sea, read as fjord-like inlets). True long parallel CHAINS
    // come later from the tectonic layer; this gives ridge texture + sharper relief.
    let wx = fbm(seed, x / WARP_LATTICE, y / WARP_LATTICE, 11, 2) - 0.5;
    let wy = fbm(seed, x / WARP_LATTICE, y / WARP_LATTICE, 13, 2) - 0.5;
    let rx = x / RIDGE_LATTICE + wx * WARP_AMP;
    let ry = y / RIDGE_LATTICE + wy * WARP_AMP;
    let rn = fbm(seed, rx, ry, 3, RIDGE_OCTAVES);
    let ridged = 1.0 - (2.0 * rn - 1.0).abs();
    let mountainness = ((macro_e - RIDGE_ONSET) / (1.0 - RIDGE_ONSET)).clamp(0.0, 1.0);

    (macro_e + detail * DETAIL_WEIGHT + (ridged - 0.5) * RIDGE_WEIGHT * mountainness)
        .clamp(0.0, 1.0)
}

/// Deterministic per-column unit value in `[0, 1)` for placing discrete features
/// (trees, rocks…). `salt` separates independent decisions on the same column.
pub fn feature_unit(seed: u64, x: usize, y: usize, salt: u64) -> f32 {
    hash2(seed, x as i64, y as i64, salt)
}

/// Generate one column's packed cell. Pure function of `(seed, x, y)`. Coordinates
/// outside the world return **air** (height 0): a boundary column's full side is then
/// exposed, which is what gives the world its thick slab edge (strata cross-section).
/// This is also what the ghost border samples.
fn gen_cell(seed: u64, x: i32, y: i32) -> u16 {
    if x < 0 || y < 0 || x >= COLS as i32 || y >= ROWS as i32 {
        return 0; // air
    }
    let (cx, cy) = (x as f32, y as f32);
    let e = elevation(seed, cx, cy);

    if e < SEA_FRACTION {
        // Sea floor with real depth: deeper offshore, shallowing toward the shore. The
        // renderer floats a translucent water plane at `SEA_ABS` above it.
        let f = e / SEA_FRACTION; // 0 deep .. 1 at the shoreline
        let h = (1.0 + f * (SEA_ABS - 2) as f32).round();
        let h = (h as i32).clamp(1, SEA_ABS as i32 - 1) as u8;
        return pack_cell(h, BiomeKind::Ocean, FLAG_WATER);
    }

    // Land: map elevation onto `LAND_FOOT..=MAX_H`. The same field drives BOTH the
    // height and the biome, so biomes are altitude bands — colour follows height, and
    // the detail octaves give local hills (a rise in the plains can crest into the
    // forest or rock band) without speckle, since the field is smooth.
    let f = (e - SEA_FRACTION) / (1.0 - SEA_FRACTION); // 0 foot .. 1 peak
    let h = (LAND_FOOT as f32 + f * SURFACE_RANGE as f32).round();
    let h = (h as i32).clamp(LAND_FOOT as i32, MAX_H as i32) as u8;

    // Altitude gates the vertical biomes (rock/snow only high, so colour still tracks
    // height — no "rock at low ground"); below the rock line, MOISTURE picks the
    // lowland biome, which is a natural same-height transition (dry desert ↔ grassland
    // ↔ wet forest), not the old spilled-paint look.
    let biome = if h <= LAND_FOOT {
        BiomeKind::Beach // shore ring
    } else if h >= MAX_H - 1 {
        BiomeKind::Snow // caps (top 2 levels)
    } else if h >= MAX_H - 5 {
        BiomeKind::Mountain // grey massif (4 levels)
    } else {
        let moist = fbm(seed, cx / MOIST_LATTICE, cy / MOIST_LATTICE, 7, MOIST_OCTAVES);
        if moist < 0.38 {
            BiomeKind::Desert
        } else if moist > 0.60 {
            BiomeKind::Forest
        } else {
            BiomeKind::Plains
        }
    };
    pack_cell(h, biome, 0)
}

// ---- Chunked storage (ghost-padded) ----

const PAD: usize = CHUNK + 2; // 18: 16 interior + 1 ghost ring each side

/// One chunk: a ghost-padded `PAD×PAD` block of packed cells. The 1-cell border is
/// a copy of neighbouring columns so a chunk's mesh build (phase 2) is fully
/// self-contained — no cross-chunk reads, no bounds checks in the hot loop.
pub struct Chunk {
    cells: [u16; PAD * PAD],
}

impl Chunk {
    fn local(lx: usize, ly: usize) -> usize {
        ly * PAD + lx
    }
    /// Interior cell at chunk-local `(0..CHUNK, 0..CHUNK)` (skips the ghost ring).
    pub fn interior(&self, lx: usize, ly: usize) -> u16 {
        self.cells[Self::local(lx + 1, ly + 1)]
    }
    /// Any padded cell incl. the ghost ring, `(0..PAD, 0..PAD)`. Used by the
    /// chunk mesher (and the ghost-ring test).
    pub fn padded(&self, plx: usize, ply: usize) -> u16 {
        self.cells[Self::local(plx, ply)]
    }
}

/// The whole world: a grid of chunks covering `COLS×ROWS` columns.
pub struct VoxelTerrain {
    pub seed: u64,
    pub chunks_x: usize,
    pub chunks_y: usize,
    chunks: Vec<Chunk>,
}

impl VoxelTerrain {
    pub fn new(seed: u64) -> Self {
        let chunks_x = COLS.div_ceil(CHUNK);
        let chunks_y = ROWS.div_ceil(CHUNK);
        let mut chunks = Vec::with_capacity(chunks_x * chunks_y);
        for cy in 0..chunks_y {
            for cx in 0..chunks_x {
                let mut cells = [0u16; PAD * PAD];
                // Fill interior + the ghost ring in one pass via the pure generator.
                for ply in 0..PAD {
                    for plx in 0..PAD {
                        let gx = (cx * CHUNK) as i32 + plx as i32 - 1;
                        let gy = (cy * CHUNK) as i32 + ply as i32 - 1;
                        cells[ply * PAD + plx] = gen_cell(seed, gx, gy);
                    }
                }
                chunks.push(Chunk { cells });
            }
        }
        VoxelTerrain { seed, chunks_x, chunks_y, chunks }
    }

    pub fn chunk(&self, cx: usize, cy: usize) -> &Chunk {
        &self.chunks[cy * self.chunks_x + cx]
    }
}

/// Per-column world-space queries. The renderer builds meshes straight from chunk
/// cells, so these aren't used there — they exist for the tests and the future sim
/// (which will look up the terrain under a creature's position).
#[allow(dead_code)]
impl VoxelTerrain {
    fn cell_at(&self, x: usize, y: usize) -> u16 {
        let (cx, cy) = (x / CHUNK, y / CHUNK);
        self.chunk(cx, cy).interior(x % CHUNK, y % CHUNK)
    }
    pub fn height_at(&self, x: usize, y: usize) -> u8 {
        cell_height(self.cell_at(x, y))
    }
    pub fn biome_at(&self, x: usize, y: usize) -> BiomeKind {
        cell_biome(self.cell_at(x, y))
    }
    pub fn is_water(&self, x: usize, y: usize) -> bool {
        cell_flags(self.cell_at(x, y)) & FLAG_WATER != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_pack_roundtrips() {
        for &h in &[1u8, 4, 7, 10, 200] {
            for b in [BiomeKind::Ocean, BiomeKind::Forest, BiomeKind::Snow] {
                for &f in &[0u8, FLAG_WATER, 0xF] {
                    let c = pack_cell(h, b, f);
                    assert_eq!(cell_height(c), h);
                    assert_eq!(cell_biome(c), b);
                    assert_eq!(cell_flags(c), f & 0xF);
                }
            }
        }
    }

    #[test]
    fn generation_is_deterministic() {
        let a = VoxelTerrain::new(42);
        let b = VoxelTerrain::new(42);
        for y in (0..ROWS).step_by(7) {
            for x in (0..COLS).step_by(7) {
                assert_eq!(a.height_at(x, y), b.height_at(x, y));
                assert_eq!(a.biome_at(x, y), b.biome_at(x, y));
            }
        }
    }

    #[test]
    fn different_seeds_differ() {
        let a = VoxelTerrain::new(1);
        let b = VoxelTerrain::new(2);
        let mut diff = 0;
        for y in 0..ROWS {
            for x in 0..COLS {
                if a.height_at(x, y) != b.height_at(x, y) {
                    diff += 1;
                }
            }
        }
        assert!(diff > (COLS * ROWS) / 10, "seeds barely differ: {diff}");
    }

    #[test]
    fn heights_in_range_and_mixed_water_land() {
        let t = VoxelTerrain::new(7);
        let mut water = 0;
        let total = COLS * ROWS;
        for y in 0..ROWS {
            for x in 0..COLS {
                let h = t.height_at(x, y);
                assert!((1..=MAX_H).contains(&h), "height {h} out of range");
                if t.is_water(x, y) {
                    water += 1;
                }
            }
        }
        assert!(water > 0 && water < total, "expected mix of water/land, got {water}/{total}");
    }

    #[test]
    fn ghost_ring_matches_neighbour_interior() {
        let t = VoxelTerrain::new(3);
        // The right ghost column of chunk (0,0) must equal the left interior column
        // of chunk (1,0), when that neighbour exists.
        if t.chunks_x >= 2 {
            let a = t.chunk(0, 0);
            let b = t.chunk(1, 0);
            for ly in 0..CHUNK {
                // a's ghost at padded x = CHUNK+1 (one past its last interior col)
                let ghost = a.padded(CHUNK + 1, ly + 1);
                let neighbour = b.interior(0, ly);
                assert_eq!(cell_height(ghost), cell_height(neighbour));
            }
        }
    }
}
