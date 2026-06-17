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
/// Noise lattice spacing in columns (bigger → broader features). Scaled by `MAP_SCALE`
/// so biomes grow with the map instead of fragmenting into noise on a giant map.
const ELEV_LATTICE: f32 = 26.0 * MAP_SCALE as f32;
const MOIST_LATTICE: f32 = 18.0 * MAP_SCALE as f32;

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

/// Two-octave fBm in `[0, 1]`.
fn fbm(seed: u64, x: f32, y: f32, salt: u64) -> f32 {
    let base = value_noise(seed, x, y, salt);
    let det = value_noise(seed, x * 2.0, y * 2.0, salt ^ 0x9999);
    (base * 0.7 + det * 0.3).clamp(0.0, 1.0)
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
    let elev = fbm(seed, cx / ELEV_LATTICE, cy / ELEV_LATTICE, 1);

    if elev < SEA_FRACTION {
        // Sea floor with real depth (not flattened to sea level): deeper offshore,
        // shallowing toward the shore. The renderer floats a translucent water plane
        // at `SEA_ABS` above it.
        let f = elev / SEA_FRACTION; // 0 deep .. 1 at the shoreline
        let h = (1.0 + f * (SEA_ABS - 2) as f32).round() as u8;
        return pack_cell(h.clamp(1, SEA_ABS - 1), BiomeKind::Ocean, FLAG_WATER);
    }

    // Land: map the above-sea elevation onto `LAND_FOOT..=MAX_H` so peaks stand
    // `SURFACE_RANGE` blocks above the foot.
    let f = (elev - SEA_FRACTION) / (1.0 - SEA_FRACTION); // 0 foot .. 1 peak
    let h = (LAND_FOOT as f32 + f * SURFACE_RANGE as f32).round() as u8;
    let h = h.clamp(LAND_FOOT, MAX_H);

    let moist = fbm(seed, cx / MOIST_LATTICE, cy / MOIST_LATTICE, 7);
    let biome = if h == LAND_FOOT {
        BiomeKind::Beach
    } else if h >= MAX_H - 1 {
        BiomeKind::Snow // top 2 levels: snow caps
    } else if h >= MAX_H - 6 {
        BiomeKind::Mountain // the tall grey massif below the caps
    } else if moist < 0.35 {
        BiomeKind::Desert
    } else if moist > 0.66 {
        BiomeKind::Forest
    } else {
        BiomeKind::Plains
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
