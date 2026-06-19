//! Voxel terrain — the render-side world model: a chunked, bit-packed, ghost-padded
//! column grid generated from noise. No simulation here. Generation is kept
//! abstract (emits `BiomeKind` + heights only); colours/meshes live in the renderer
//! so generation and representation stay separate.
//!
//! Coordinates follow the config contract: a column `(x, y)` (x in `0..COLS`,
//! y in `0..ROWS`) carries a surface height `h` in levels; the solid block stack is
//! `gz in 0..h`. World space is `(x*VOX, gz*VOX, y*VOX)` (y-up).

use crate::config::*;
use crate::tectonics::TectonicField;

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
/// Vertical biome bands as a FRACTION of the relief (not a fixed level count), so the
/// area distribution is invariant to `SURFACE_RANGE`: the top `SNOW_BAND` levels are
/// Snow, the top `MOUNTAIN_BAND` (incl. snow) are rock. Calibrated to reproduce the
/// previous fixed bands at the old `SURFACE_RANGE = 11` (snow = 1, mountain = 5 levels).
const SNOW_BAND: u8 = (SURFACE_RANGE as u32 / 9) as u8;
const MOUNTAIN_BAND: u8 = (SURFACE_RANGE as u32 * 5 / 11) as u8;
/// Fraction of the elevation field that lies under the sea — sets how much of the map
/// is water, independently of how tall the land rises (so taller peaks ≠ less sea).
const SEA_FRACTION: f32 = 0.42;
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
/// Temperature: warm at the equator (map middle), cold toward the poles (map top/bottom)
/// and cold with altitude. With moisture this drives a Whittaker biome matrix. The
/// latitude band is wiggled by noise so biome belts aren't ruler-straight.
const TEMP_LATTICE: f32 = 33.0 * MAP_SCALE as f32;
const TEMP_OCTAVES: u32 = 3;
const TEMP_WIGGLE: f32 = 0.18;
/// How much altitude cools (1.0 = a peak is a full band colder than its foot).
const TEMP_LAPSE: f32 = 0.55;
/// Ridged-noise field for mountain ridgelines (belts scale with the map). Domain-warped
/// by a broader field so ridges flow instead of forming blobs.
const RIDGE_LATTICE: f32 = 30.0 * MAP_SCALE as f32;
const RIDGE_OCTAVES: u32 = 4;
const WARP_LATTICE: f32 = 55.0 * MAP_SCALE as f32;
const WARP_AMP: f32 = 0.6;
/// Ridge amplitude in normalised `[0,1]` elevation units (crest lift / trough carve).
/// Gated by the tectonic mountainness field, so ridgelines ride on real orogenic belts.
const RIDGE_WEIGHT: f32 = 0.34;

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
    Taiga,
    Tundra,
    Savanna,
    Swamp,
    Jungle,
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
            BiomeKind::Taiga => 7,
            BiomeKind::Tundra => 8,
            BiomeKind::Savanna => 9,
            BiomeKind::Swamp => 10,
            BiomeKind::Jungle => 11,
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
            7 => BiomeKind::Taiga,
            8 => BiomeKind::Tundra,
            9 => BiomeKind::Savanna,
            10 => BiomeKind::Swamp,
            11 => BiomeKind::Jungle,
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
pub(crate) fn fbm(seed: u64, x: f32, y: f32, salt: u64, octaves: u32) -> f32 {
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

/// Combined surface elevation in `[0, 1]`. The macro base now comes from the **tectonic
/// field** (continents/ocean basins + orogenic belts) instead of plain fBm; on top ride
/// a many-octave low-amplitude detail field (local hills/roughness) and domain-warped
/// **ridged** noise gated by the tectonic `mountainness` — so ridgelines/cliffs flow
/// along the real mountain belts. One height function with global AND local extrema:
/// both height and biome read from it, so colour follows altitude.
fn elevation(seed: u64, x: f32, y: f32, macro_e: f32, mountainness: f32) -> f32 {
    let detail = fbm(seed, x / DETAIL_LATTICE, y / DETAIL_LATTICE, 5, DETAIL_OCTAVES) - 0.5;

    // Ridged noise for ridgelines/cliffs, applied only where the tectonic field is
    // orogenic (so lowlands stay rolling). Domain-warp the sample so ridges flow
    // organically instead of forming round blobs. `1 - |2n-1|` peaks at 1 along
    // ridgelines: positive lifts crests, negative carves the troughs between them
    // (which, reaching the sea, read as fjord-like inlets).
    let wx = fbm(seed, x / WARP_LATTICE, y / WARP_LATTICE, 11, 2) - 0.5;
    let wy = fbm(seed, x / WARP_LATTICE, y / WARP_LATTICE, 13, 2) - 0.5;
    let rx = x / RIDGE_LATTICE + wx * WARP_AMP;
    let ry = y / RIDGE_LATTICE + wy * WARP_AMP;
    let rn = fbm(seed, rx, ry, 3, RIDGE_OCTAVES);
    let ridged = 1.0 - (2.0 * rn - 1.0).abs();

    (macro_e + detail * DETAIL_WEIGHT + (ridged - 0.5) * RIDGE_WEIGHT * mountainness)
        .clamp(0.0, 1.0)
}

/// Deterministic per-column unit value in `[0, 1)` for placing discrete features
/// (trees, rocks…). `salt` separates independent decisions on the same column.
pub fn feature_unit(seed: u64, x: usize, y: usize, salt: u64) -> f32 {
    hash2(seed, x as i64, y as i64, salt)
}

/// Surface from elevation for one IN-WORLD column: the continuous surface level
/// (`f32`, kept so later global passes — tectonics, erosion — can carve fractional
/// levels), the biome, and the flags. The renderer rounds the level to an integer `h`.
/// Map a continuous elevation `[0,1]` to a (float) surface level: sea floor below the
/// shoreline fraction, land foot→peak above it. Shared by `classify` and the lake water
/// level so they agree.
fn elev_to_level(e: f32) -> f32 {
    if e < SEA_FRACTION {
        let f = e / SEA_FRACTION; // 0 deep .. 1 at the shoreline
        (1.0 + f * (SEA_ABS - 2) as f32).clamp(1.0, (SEA_ABS - 1) as f32)
    } else {
        let f = (e - SEA_FRACTION) / (1.0 - SEA_FRACTION); // 0 foot .. 1 peak
        (LAND_FOOT as f32 + f * SURFACE_RANGE as f32).clamp(LAND_FOOT as f32, MAX_H as f32)
    }
}

/// Continuous surface level + the LAND-style biome for a column, from elevation alone. The
/// water decision (ocean / lake / river) is NOT made here — it's applied in `VoxelTerrain::new`
/// from hydrology (connectivity), which overrides the biome to `Ocean` (seabed) where water
/// stands. A sub-sea column that ends up dry (a landlocked below-sea floor) therefore reads as
/// a low-`h` land biome (Beach), not a stray blue seabed.
fn classify(seed: u64, x: usize, y: usize, e: f32) -> (f32, BiomeKind, f32, f32) {
    let (cx, cy) = (x as f32, y as f32);
    let surf = elev_to_level(e);

    // The same field drives BOTH height and biome (altitude bands + climate matrix); a sub-sea
    // floor lands in the lowest band (`h <= LAND_FOOT` ⇒ Beach).
    let h = surf.round() as u8;

    // Climate fields, computed for EVERY column (not only lowland) so the sim can read
    // temperature/moisture anywhere — including beach/mountain/snow columns the biome
    // choice below decides on altitude alone. The biome result is unchanged: rock/snow/
    // beach still ignore climate; only the lowland `else` branch consults it.
    let moist = fbm(seed, cx / MOIST_LATTICE, cy / MOIST_LATTICE, 7, MOIST_OCTAVES);
    let temp = temperature(seed, cx, cy, h);

    // Altitude gates the vertical biomes (rock/snow only high, so colour still tracks
    // height — no "rock at low ground"); below the rock line, a TEMPERATURE × MOISTURE
    // Whittaker matrix picks the lowland biome, giving real climate variety across the
    // giant map (tundra/taiga cold, savanna/desert hot-dry, jungle hot-wet…).
    let biome = if h <= LAND_FOOT {
        BiomeKind::Beach // shore ring
    } else if h >= MAX_H - SNOW_BAND {
        BiomeKind::Snow // caps
    } else if h >= MAX_H - MOUNTAIN_BAND {
        BiomeKind::Mountain // grey massif
    } else {
        climate_biome(temp, moist, h)
    };
    (surf, biome, temp, moist)
}

/// Temperature in `[0, 1]` (0 cold .. 1 hot): warm at the equator (map middle), cooling
/// toward the poles (top/bottom edges) and with altitude, with a noise-wiggled band edge.
fn temperature(seed: u64, cx: f32, cy: f32, h: u8) -> f32 {
    let lat = 1.0 - (2.0 * cy / ROWS as f32 - 1.0).abs(); // 0 poles .. 1 equator
    let wiggle = (fbm(seed, cx / TEMP_LATTICE, cy / TEMP_LATTICE, 9, TEMP_OCTAVES) - 0.5) * TEMP_WIGGLE;
    let alt = (h.saturating_sub(LAND_FOOT)) as f32 / SURFACE_RANGE as f32; // 0 foot .. 1 peak
    (lat + wiggle - alt * TEMP_LAPSE).clamp(0.0, 1.0)
}

/// Whittaker-style lowland biome from temperature × moisture (+ altitude for swamps,
/// which want to sit low near water). Thresholds chosen so each biome occupies a sensible
/// slab of climate space.
fn climate_biome(temp: f32, moist: f32, h: u8) -> BiomeKind {
    if temp < 0.32 {
        // Cold: dry tundra ↔ wet taiga (boreal forest).
        if moist < 0.40 {
            BiomeKind::Tundra
        } else {
            BiomeKind::Taiga
        }
    } else if temp < 0.66 {
        // Temperate: grassland ↔ forest ↔ swamp (wet + low).
        if moist < 0.38 {
            BiomeKind::Plains
        } else if moist > 0.68 && h <= LAND_FOOT + 2 {
            BiomeKind::Swamp
        } else {
            BiomeKind::Forest
        }
    } else {
        // Hot: desert ↔ savanna ↔ jungle.
        if moist < 0.34 {
            BiomeKind::Desert
        } else if moist < 0.60 {
            BiomeKind::Savanna
        } else {
            BiomeKind::Jungle
        }
    }
}

// ---- Resident world model (flat per-column arrays) ----

/// The whole world as flat `COLS×ROWS` column arrays, computed once per seed and held
/// in RAM. Cheap (≈6 B/column → a few MB even at ×16), unlike the chunk meshes — so the
/// global generation passes (tectonics, erosion) can run over the full grid here, while
/// the renderer streams meshes from it. `surf` is the continuous surface level; the
/// renderer (and `height`) round it. `chunks_x/y` are kept for the mesher's tiling.
pub struct VoxelTerrain {
    pub seed: u64,
    pub chunks_x: usize,
    pub chunks_y: usize,
    surf: Vec<f32>,
    biome: Vec<u8>,
    flags: Vec<u8>,
    /// Water surface level per column (voxel levels): `SEA_ABS` for ocean, the fill level
    /// for lakes, the channel top for rivers, `0` = dry. The renderer floats a translucent
    /// plane here when it sits above the column's terrain.
    water: Vec<u8>,
    /// Climate fields the SIM reads (the renderer only uses biome). Quantised `[0,1]→[0,255]`
    /// (≈0.4% step — coarser than the climate itself; saves 3× the RAM of `f32` at ×16).
    /// `temp`: 0 cold .. 1 hot. `moist`: 0 dry .. 1 wet. Present for every column.
    temp: Vec<u8>,
    moist: Vec<u8>,
    /// Chebyshev-ish BFS distance (in columns, 4-connectivity) to the nearest water column,
    /// saturating at 255. `0` on water itself; the gradient near a shore is exact (far inland
    /// plateaus all read the 255 floor — fine for the sim's "far = far").
    water_dist: Vec<u8>,
    /// Vegetation biomass per column (S3), quantised `[0,1]→[0,255]` — the consumable base of
    /// the food chain. The value is what biomass was AS OF `last_update[i]`; the live amount
    /// is recovered LAZILY (see [`biomass_at`](Self::biomass_at)) by regrowing it toward the
    /// column's `carrying_capacity` over the ticks elapsed since. So an untouched column costs
    /// nothing — there is no per-tick global sweep over the 3.69M columns.
    biomass: Vec<u8>,
    /// The `WorldClock` tick at which `biomass[i]` was last written (by a graze). Lazy regrow
    /// reads `tick - last_update[i]`. Integer ticks (not an `f32` time) so the timestamp never
    /// drifts. `0` at generation, when biomass starts at full capacity.
    last_update: Vec<u32>,
}

/// Quantise a `[0,1]` field value into a `u8` (saturating). De-quantise with `/ 255.0`.
fn quant_unit(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Vegetation carrying capacity in `[0,1]` for a column — the biomass it tends toward when
/// undisturbed. A per-biome base (water/desert/rock low, forest/jungle high) modulated mildly
/// by moisture so wetter columns of the same biome carry a little more. Water carries nothing.
fn carrying_capacity(biome: BiomeKind, moist: f32) -> f32 {
    let base = match biome {
        BiomeKind::Ocean => 0.0, // water: no land vegetation
        BiomeKind::Snow => 0.05,
        BiomeKind::Desert => 0.10,
        BiomeKind::Beach => 0.12,
        BiomeKind::Mountain => 0.15,
        BiomeKind::Tundra => 0.22,
        BiomeKind::Savanna => 0.42,
        BiomeKind::Plains => 0.52,
        BiomeKind::Taiga => 0.62,
        BiomeKind::Swamp => 0.72,
        BiomeKind::Forest => 0.85,
        BiomeKind::Jungle => 1.0,
    };
    (base * (0.7 + 0.3 * moist)).clamp(0.0, 1.0)
}

/// Lazy regrow law: biomass `b` relaxes toward `cap` over `elapsed` sim-seconds by
/// `b' = cap − (cap − b)·e^(−RATE·elapsed)` (linear-with-saturation). Monotonic, never
/// exceeds `cap`, and crucially **recovers from 0** (`b=0 ⇒ cap·(1−e^…)`, no fixed point at
/// zero) — so a grazed-to-bare column regrows instead of staying a permanent bald patch. The
/// closed form makes the amortised (skip-the-untouched-ticks) update exact.
fn regrow(b: f32, cap: f32, elapsed: f32) -> f32 {
    let grown = cap - (cap - b) * (-BIOMASS_REGROW_RATE * elapsed).exp();
    grown.clamp(0.0, cap)
}

/// Multi-source BFS distance (in columns) from the nearest water column, over the whole
/// grid (4-connectivity, unit edges). Distance is computed in `u16` then clamped into the
/// `u8` field, so the cap (255) and the "unvisited" sentinel never collide. A map with no
/// water at all leaves every column at the 255 floor.
fn compute_water_dist(water: &[u8]) -> Vec<u8> {
    let n = COLS * ROWS;
    let mut dist = vec![u16::MAX; n];
    let mut q = std::collections::VecDeque::new();
    for (i, &w) in water.iter().enumerate() {
        if w != 0 {
            dist[i] = 0;
            q.push_back(i);
        }
    }
    while let Some(i) = q.pop_front() {
        let nd = dist[i] + 1;
        let (x, y) = (i % COLS, i / COLS);
        let step = |j: usize, dist: &mut [u16], q: &mut std::collections::VecDeque<usize>| {
            if dist[j] > nd {
                dist[j] = nd;
                q.push_back(j);
            }
        };
        if x + 1 < COLS {
            step(i + 1, &mut dist, &mut q);
        }
        if x > 0 {
            step(i - 1, &mut dist, &mut q);
        }
        if y + 1 < ROWS {
            step(i + COLS, &mut dist, &mut q);
        }
        if y > 0 {
            step(i - COLS, &mut dist, &mut q);
        }
    }
    dist.iter().map(|&d| d.min(255) as u8).collect()
}

impl VoxelTerrain {
    /// Generate a world for `seed`, blocking the calling thread. Pure CPU (no GPU), so it
    /// may run on a background thread; the result is `Send`. `progress` is called with a
    /// monotonically rising fraction in `[0, 1]` as the phases (tectonics → elevation →
    /// erosion → hydrology → classification) complete, for a UI progress bar.
    pub fn generate(seed: u64, progress: &(dyn Fn(f32) + Sync)) -> Self {
        let n = COLS * ROWS;
        // The tectonic macro layer is global (Voronoi plates + a distance transform from
        // boundaries), so it's built once up front; the per-column generator samples it.
        let tect = TectonicField::generate(seed);
        progress(0.10);
        // Build the continuous elevation field, then ERODE it globally (droplet + thermal)
        // before classifying columns into height/biome — so valleys, drainage and fjords
        // are carved into the land, and the altitude bands follow the eroded surface.
        let mut elev = vec![0.0f32; n];
        for y in 0..ROWS {
            for x in 0..COLS {
                elev[y * COLS + x] =
                    elevation(seed, x as f32, y as f32, tect.macro_at(x, y), tect.mountain_at(x, y));
            }
            progress(0.10 + 0.20 * (y + 1) as f32 / ROWS as f32);
        }
        // Erosion is the heavy pass; thread its local [0,1] progress into our 0.30..0.65 band.
        crate::erosion::erode(seed, &mut elev, &|f| progress(0.30 + 0.35 * f));
        // Hydrology (rivers via flow accumulation, lakes via depression filling) reads the
        // eroded field; it feeds the per-column water level + river/lake biomes below.
        let hydro = crate::hydrology::compute(&elev, SEA_FRACTION);
        progress(0.72);
        let mut surf = vec![0.0f32; n];
        let mut biome = vec![0u8; n];
        let mut flags = vec![0u8; n];
        let mut water = vec![0u8; n];
        let mut temp = vec![0u8; n];
        let mut moist = vec![0u8; n];
        let mut biomass = vec![0u8; n];
        for y in 0..ROWS {
            for x in 0..COLS {
                let i = y * COLS + x;
                let (mut s, mut b, ct, cm) = classify(seed, x, y, elev[i]);
                temp[i] = quant_unit(ct);
                moist[i] = quant_unit(cm);
                let mut f = 0u8;
                // Water priority on connectivity, not absolute height: ocean (sea-connected)
                // wins, else a depression lake, else a river. `hydro` already cleared lake/river
                // on ocean cells, so the `else if` chain is doubly exclusive.
                if hydro.ocean[i] {
                    // Open sea: water plane at the global sea level over the sea floor.
                    water[i] = SEA_ABS;
                    b = BiomeKind::Ocean;
                    f = FLAG_WATER;
                } else if hydro.lake[i] {
                    // Lake (incl. inland sub-sea pits, NOT pinned to sea level): a FLAT mirror at
                    // the depression fill level over the bed. The whole body shares `lvl`, so keep
                    // it flat — do NOT gate on `lvl > bed`: that dropped shallow-margin cells (depth
                    // rounding under half a voxel) to dry land and punched grey holes in the lake.
                    // Instead carve any rim cell whose rounded bed reaches the mirror down one level
                    // so the renderer (`wl > h`) still floats water over it. `bed <= lvl` always
                    // (monotone `elev_to_level`, `filled >= elev`), so the carve is at most one
                    // voxel — like a river channel.
                    let lvl = elev_to_level(hydro.filled[i]).round() as u8;
                    if s.round() as u8 >= lvl {
                        s = lvl.saturating_sub(1) as f32;
                    }
                    water[i] = lvl;
                    b = BiomeKind::Ocean; // underwater bed
                    f = FLAG_WATER;
                } else if hydro.river[i] {
                    // River: carve the channel one level and float water at the old top.
                    let top = s.round() as u8;
                    if top > LAND_FOOT {
                        s = (top - 1) as f32;
                        water[i] = top;
                        b = BiomeKind::Ocean;
                        f = FLAG_WATER;
                    }
                }
                surf[i] = s;
                biome[i] = b.id();
                flags[i] = f;
                // Vegetation starts mature (at capacity) on the FINAL biome + moisture; water
                // columns get 0. The lazy regrow keeps an untouched column here forever.
                biomass[i] = quant_unit(carrying_capacity(b, cm));
            }
            progress(0.72 + 0.28 * (y + 1) as f32 / ROWS as f32);
        }
        // Shoreline reconciliation (one pass, ALL water types). Terrain height and the water
        // surface are quantised to voxels INDEPENDENTLY (`surf.round()` vs
        // `round(elev_to_level(filled))`), and wet/dry is decided by comparing the two — so at a
        // shoreline these two correlated continuous fields can fall on opposite sides of the .5
        // boundary, leaving ±1 voxel noise: a dry cell a step BELOW the water it touches (a moat),
        // or a step ABOVE it while ringed by water (a 1-cell spit/pillar). Both are the same root;
        // a morphological CLOSE + a bank LIFT kill the whole class, uniformly for ocean/lake/river.
        let w0 = water.clone();
        let wl_at = |w: &[u8], nx: i32, ny: i32| -> u8 {
            if nx < 0 || ny < 0 || nx >= COLS as i32 || ny >= ROWS as i32 {
                0
            } else {
                w[ny as usize * COLS + nx as usize]
            }
        };
        // CLOSE: a dry cell ringed by water on >=3 sides is a 1-cell nub jutting into the water.
        // Flood it to the highest neighbour level (bed carved one below, like a lake rim) so the
        // shoreline reads convex instead of sprouting pillars. Capped to shallow nubs
        // (`h - wl <= SHORE_NUB_CAP`) so a genuine tall island / sea stack is preserved.
        const SHORE_NUB_CAP: u8 = 2;
        for y in 0..ROWS {
            for x in 0..COLS {
                let i = y * COLS + x;
                if flags[i] & FLAG_WATER != 0 {
                    continue;
                }
                let (xi, yi) = (x as i32, y as i32);
                let nbr = [wl_at(&w0, xi + 1, yi), wl_at(&w0, xi - 1, yi), wl_at(&w0, xi, yi + 1), wl_at(&w0, xi, yi - 1)];
                let water_nb = nbr.iter().filter(|&&w| w > 0).count();
                let wl = nbr.iter().copied().max().unwrap_or(0);
                if water_nb >= 3 && wl > 0 && surf[i].round() as u8 <= wl + SHORE_NUB_CAP {
                    water[i] = wl;
                    surf[i] = wl.saturating_sub(1) as f32; // bed one below the mirror ⇒ renders as water
                    flags[i] = FLAG_WATER;
                    biome[i] = BiomeKind::Ocean.id();
                }
            }
        }
        // LIFT: any still-dry cell whose top sits BELOW an adjacent water surface is raised to it,
        // closing the moat. Reads the post-close `water`. Rivers are skipped — lifting a low outlet
        // would dam the channel — and water cells are already at/above their own surface.
        for y in 0..ROWS {
            for x in 0..COLS {
                let i = y * COLS + x;
                if flags[i] & FLAG_WATER != 0 || hydro.river[i] {
                    continue;
                }
                let (xi, yi) = (x as i32, y as i32);
                let h = surf[i].round() as u8;
                let lift = [wl_at(&water, xi + 1, yi), wl_at(&water, xi - 1, yi), wl_at(&water, xi, yi + 1), wl_at(&water, xi, yi - 1)]
                    .into_iter()
                    .fold(h, u8::max);
                if lift > h {
                    surf[i] = lift as f32;
                }
            }
        }
        // Distance-to-water: a multi-source BFS from every water column over the eroded
        // grid (one O(N) pass, like hydrology). The sim reads it for hydration / water-seeking.
        // Computed AFTER shoreline reconciliation so it reflects the reconciled water mask.
        let water_dist = compute_water_dist(&water);
        VoxelTerrain {
            seed,
            chunks_x: COLS.div_ceil(CHUNK),
            chunks_y: ROWS.div_ceil(CHUNK),
            surf,
            biome,
            flags,
            water,
            temp,
            moist,
            water_dist,
            biomass,
            last_update: vec![0u32; n],
        }
    }

    /// Generate a world for `seed`, blocking, with no progress reporting. Thin wrapper over
    /// [`generate`](Self::generate). Used by tests/benches; the app uses `generate` on a
    /// background thread.
    #[allow(dead_code)]
    pub fn new(seed: u64) -> Self {
        Self::generate(seed, &|_| {})
    }

    /// Water surface level (voxel) at signed coords, `0` if dry or out of world. The
    /// renderer floats a translucent plane here where it stands above the terrain.
    pub fn water_level(&self, x: i32, y: i32) -> u8 {
        match self.index(x, y) {
            Some(i) => self.water[i],
            None => 0,
        }
    }

    /// Rounded surface height at signed column coords. **Out of the world ⇒ air (0)** —
    /// a boundary column's full side is then exposed (the thick slab edge), and the
    /// mesher samples neighbours through this, so no ghost ring is needed.
    pub fn height(&self, x: i32, y: i32) -> u8 {
        match self.index(x, y) {
            Some(i) => self.surf[i].round() as u8,
            None => 0,
        }
    }

    /// Packed cell at signed coords (0 = air out of world). The mesher reads this for
    /// each column's own cell; neighbour heights come from `height`.
    pub fn cell(&self, x: i32, y: i32) -> u16 {
        match self.index(x, y) {
            Some(i) => {
                pack_cell(self.surf[i].round() as u8, BiomeKind::from_id(self.biome[i]), self.flags[i])
            }
            None => 0,
        }
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= COLS as i32 || y >= ROWS as i32 {
            None
        } else {
            Some(y as usize * COLS + x as usize)
        }
    }
}

/// Per-column world-space queries for the tests and the future sim (which will look up
/// the terrain under a creature's position). The mesher uses `height`/`cell` directly.
#[allow(dead_code)]
impl VoxelTerrain {
    pub fn height_at(&self, x: usize, y: usize) -> u8 {
        self.height(x as i32, y as i32)
    }
    pub fn biome_at(&self, x: usize, y: usize) -> BiomeKind {
        BiomeKind::from_id(self.biome[y * COLS + x])
    }
    pub fn is_water(&self, x: usize, y: usize) -> bool {
        self.flags[y * COLS + x] & FLAG_WATER != 0
    }
}

/// Environment fields the SIM (and the debug overlay) read under a creature's column.
/// These have live callers (dev bridge + the `G` colourmap overlay), so they are not
/// `dead_code`-gated. All take in-world `0..COLS × 0..ROWS` indices.
impl VoxelTerrain {
    /// Temperature in `[0,1]` (0 cold .. 1 hot) at a column. De-quantised from the `u8` field.
    pub fn temperature_at(&self, x: usize, y: usize) -> f32 {
        self.temp[y * COLS + x] as f32 / 255.0
    }
    /// Moisture in `[0,1]` (0 dry .. 1 wet) at a column. De-quantised from the `u8` field.
    pub fn moisture_at(&self, x: usize, y: usize) -> f32 {
        self.moist[y * COLS + x] as f32 / 255.0
    }
    /// Distance (in columns) to the nearest water, saturating at 255. `0` on water.
    pub fn water_dist_at(&self, x: usize, y: usize) -> u8 {
        self.water_dist[y * COLS + x]
    }
    /// Terrain steepness in `[0,1]`: the largest absolute surface-level difference to a
    /// 4-neighbour, normalised by the land relief (`SURFACE_RANGE`) and clamped — so `1.0`
    /// is a full-relief cliff in one column, `0` is flat. Computed on demand from `surf`
    /// (not stored). An out-of-world neighbour contributes nothing (no false cliff at the
    /// map edge): it is treated as level with this column.
    pub fn slope_at(&self, x: usize, y: usize) -> f32 {
        let h0 = self.surf[y * COLS + x];
        let (ix, iy) = (x as i32, y as i32);
        let mut max_d = 0.0f32;
        for (nx, ny) in [(ix + 1, iy), (ix - 1, iy), (ix, iy + 1), (ix, iy - 1)] {
            let hn = match self.index(nx, ny) {
                Some(j) => self.surf[j],
                None => h0,
            };
            max_d = max_d.max((hn - h0).abs());
        }
        (max_d / SURFACE_RANGE as f32).clamp(0.0, 1.0)
    }

    /// Vegetation capacity `[0,1]` for a column (lazy-regrow target). Recomputed from the
    /// stored biome + moisture — a table lookup + a couple of muls, so it is not stored.
    fn cap_at(&self, i: usize) -> f32 {
        carrying_capacity(BiomeKind::from_id(self.biome[i]), self.moist[i] as f32 / 255.0)
    }

    /// Live biomass at column index `i` and clock `tick`: the stored value (as of its last
    /// update) regrown toward capacity over the elapsed ticks. Pure read — does NOT write back.
    fn current_biomass(&self, i: usize, tick: u64) -> f32 {
        let elapsed = (tick - self.last_update[i] as u64) as f32 * TICK_LEN;
        regrow(self.biomass[i] as f32 / 255.0, self.cap_at(i), elapsed)
    }

    /// Live vegetation biomass in `[0,1]` at a column for clock `tick` (the lazy regrow is
    /// applied on read but NOT persisted — a read-only estimate). `tick` is passed explicitly
    /// (never cached in the model) so the time reference can't silently desync from the clock.
    pub fn biomass_at(&self, x: usize, y: usize, tick: u64) -> f32 {
        self.current_biomass(y * COLS + x, tick)
    }

    /// Graze up to `amount` (in `[0,1]` biomass units) from a column at clock `tick`: applies
    /// the lazy regrow to now, removes what's available, and PERSISTS the new value + `tick`.
    /// Returns the biomass actually taken (≤ what was present, so over-grazing yields the rest,
    /// not negative food). Regrowth afterwards is handled lazily by the next read/graze.
    pub fn graze(&mut self, x: usize, y: usize, amount: f32, tick: u64) -> f32 {
        let i = y * COLS + x;
        let cur = self.current_biomass(i, tick);
        let taken = amount.clamp(0.0, cur);
        self.biomass[i] = quant_unit(cur - taken);
        self.last_update[i] = tick as u32;
        taken
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guard the "mountains are LOCAL" invariant: rock + snow must stay a minority of
    /// the land, so added worldgen complexity (ridged noise now; tectonics/erosion
    /// later) can't quietly turn the map into one mountain mess. Prints the fraction.
    #[test]
    fn mountains_are_a_minority() {
        for seed in 1..4 {
            let t = VoxelTerrain::new(seed);
            let (mut land, mut high) = (0u64, 0u64);
            for y in 0..ROWS {
                for x in 0..COLS {
                    if t.is_water(x, y) {
                        continue;
                    }
                    land += 1;
                    if matches!(t.biome_at(x, y), BiomeKind::Mountain | BiomeKind::Snow) {
                        high += 1;
                    }
                }
            }
            let frac = high as f64 / land.max(1) as f64;
            eprintln!("seed {seed}: mountain+snow = {:.1}% of land", frac * 100.0);
            assert!(frac < 0.35, "mountains dominate the land for seed {seed}: {:.1}%", frac * 100.0);
        }
    }

    /// Tectonic sanity: mountains should form a few large connected BELTS (chains), not
    /// scattered specks, and the land/water balance must stay reasonable per seed (the
    /// oceanic-plate layout shouldn't drown or fill the whole map). Prints both.
    #[test]
    fn tectonic_chains_and_balance() {
        for seed in 1..4 {
            let t = VoxelTerrain::new(seed);
            let n = COLS * ROWS;
            let mut high = vec![false; n];
            let (mut water, mut mtn) = (0u64, 0u64);
            for y in 0..ROWS {
                for x in 0..COLS {
                    let i = y * COLS + x;
                    if t.is_water(x, y) {
                        water += 1;
                    }
                    if matches!(t.biome_at(x, y), BiomeKind::Mountain | BiomeKind::Snow) {
                        high[i] = true;
                        mtn += 1;
                    }
                }
            }
            // Largest connected mountain component (4-connectivity, iterative flood fill).
            let mut seen = vec![false; n];
            let mut largest = 0u64;
            let mut stack = Vec::new();
            for start in 0..n {
                if !high[start] || seen[start] {
                    continue;
                }
                let mut size = 0u64;
                stack.push(start);
                seen[start] = true;
                while let Some(i) = stack.pop() {
                    size += 1;
                    let (x, y) = ((i % COLS) as i32, (i / COLS) as i32);
                    for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                        if nx < 0 || ny < 0 || nx >= COLS as i32 || ny >= ROWS as i32 {
                            continue;
                        }
                        let j = ny as usize * COLS + nx as usize;
                        if high[j] && !seen[j] {
                            seen[j] = true;
                            stack.push(j);
                        }
                    }
                }
                largest = largest.max(size);
            }
            let water_pct = water as f64 / n as f64 * 100.0;
            let chain = if mtn > 0 { largest as f64 / mtn as f64 } else { 0.0 };
            eprintln!(
                "seed {seed}: water {water_pct:.0}%, mountains in chains {:.0}% (largest/total)",
                chain * 100.0
            );
            assert!((8.0..92.0).contains(&water_pct), "extreme water balance for seed {seed}: {water_pct:.0}%");
        }
    }

    /// Debug dump (run with `--ignored`): writes grayscale PNGs of the generation fields
    /// to /tmp so the straight-cliff artifact can be located visually and traced to the
    /// field that produces it. Not a gate.
    #[test]
    #[ignore]
    fn dump_debug_fields() {
        use macroquad::color::Color;
        use macroquad::texture::Image;
        let seed = 1u64;
        let t = VoxelTerrain::new(seed);
        let tect = TectonicField::generate(seed);
        let dump = |path: &str, f: &dyn Fn(usize, usize) -> f32| {
            let mut img = Image::gen_image_color(COLS as u16, ROWS as u16, Color::new(0.0, 0.0, 0.0, 1.0));
            for y in 0..ROWS {
                for x in 0..COLS {
                    let v = f(x, y).clamp(0.0, 1.0);
                    img.set_pixel(x as u32, y as u32, Color::new(v, v, v, 1.0));
                }
            }
            img.export_png(path);
        };
        dump("/tmp/dbg_macro.png", &|x, y| tect.macro_field()[y * COLS + x]);
        dump("/tmp/dbg_mtn.png", &|x, y| tect.mountain_field()[y * COLS + x]);
        dump("/tmp/dbg_height.png", &|x, y| t.height_at(x, y) as f32 / MAX_H as f32);
        // Biome map: a distinct flat colour per biome id, so the climate distribution is
        // visible (poles cold, equator hot; dry↔wet bands).
        let pal: [(f32, f32, f32); 12] = [
            (0.13, 0.32, 0.55), (0.84, 0.78, 0.54), (0.42, 0.62, 0.30), (0.20, 0.46, 0.24),
            (0.80, 0.70, 0.44), (0.48, 0.46, 0.45), (0.93, 0.95, 0.98), (0.17, 0.38, 0.29),
            (0.62, 0.64, 0.56), (0.70, 0.66, 0.34), (0.31, 0.40, 0.25), (0.12, 0.43, 0.17),
        ];
        {
            use macroquad::color::Color;
            use macroquad::texture::Image;
            let mut img = Image::gen_image_color(COLS as u16, ROWS as u16, Color::new(0.0, 0.0, 0.0, 1.0));
            for y in 0..ROWS {
                for x in 0..COLS {
                    let (r, g, b) = pal[t.biome_at(x, y).id() as usize];
                    img.set_pixel(x as u32, y as u32, Color::new(r, g, b, 1.0));
                }
            }
            img.export_png("/tmp/dbg_biome.png");
        }
        // Hillshade of the actual terrain — reveals erosion channels/ridges far better
        // than raw height (slope-lit, sun from the NW).
        dump("/tmp/dbg_shade.png", &|x, y| {
            let xi = x as i32;
            let yi = y as i32;
            let gx = (t.height(xi + 1, yi) as f32 - t.height(xi - 1, yi) as f32) * 0.5;
            let gy = (t.height(xi, yi + 1) as f32 - t.height(xi, yi - 1) as f32) * 0.5;
            let inv = 1.0 / (gx * gx + gy * gy + 1.0).sqrt();
            // light dir (0.5, 0.5, 0.7) normalised ≈ (0.49,0.49,0.69), dot with normal
            let shade = (-gx * 0.49 - gy * 0.49 + 0.69) * inv;
            0.15 + 0.85 * shade.clamp(0.0, 1.0)
        });
        // Cliff map: the largest DOWNWARD step from a column to any 4-neighbour, in
        // levels, scaled so a ~10-level drop is white. This isolates where the knife
        // cliffs actually are, independent of biome colour.
        dump("/tmp/dbg_cliff.png", &|x, y| {
            let h = t.height(x as i32, y as i32) as i32;
            let mut drop = 0i32;
            for (nx, ny) in [(x as i32 + 1, y as i32), (x as i32 - 1, y as i32), (x as i32, y as i32 + 1), (x as i32, y as i32 - 1)] {
                drop = drop.max(h - t.height(nx, ny) as i32);
            }
            drop as f32 / 10.0
        });
        eprintln!("dumped /tmp/dbg_macro.png dbg_mtn.png dbg_height.png dbg_cliff.png");
    }

    /// Guard against KNIFE CLIFFS — the artifact where the macro field stepped a full
    /// relief in one column (root cause: taking the single NEAREST plate boundary's
    /// convergence, which flips across the medial axis between two boundaries; fixed by
    /// using a distance-weighted average convergence instead). The worst LAND-to-LAND
    /// downward step must stay a slope, not a wall. Prints the worst per seed.
    #[test]
    fn land_has_no_knife_cliffs() {
        for seed in 1..4 {
            let t = VoxelTerrain::new(seed);
            let mut worst = 0i32;
            for y in 0..ROWS as i32 {
                for x in 0..COLS as i32 {
                    let h = t.height(x, y) as i32;
                    for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                        // In-world land neighbours only (the map-edge slab to air and the
                        // shoreline drop to the sea floor are legitimate, not artifacts).
                        if nx < 0 || ny < 0 || nx >= COLS as i32 || ny >= ROWS as i32 {
                            continue;
                        }
                        let nh = t.height(nx, ny) as i32;
                        if nh == 0 || t.is_water(nx as usize, ny as usize) {
                            continue;
                        }
                        worst = worst.max(h - nh);
                    }
                }
            }
            eprintln!("seed {seed}: worst land cliff = {worst} levels (of {SURFACE_RANGE})");
            assert!(worst < 16, "knife cliff for seed {seed}: {worst}-level step in one column");
        }
    }

    /// Report the erosion preprocess cost (the heavy one-time pass). Run with `--release`
    /// for a representative number; informational, not a gate.
    #[test]
    #[ignore]
    fn report_erosion_cost() {
        let tect = TectonicField::generate(1);
        let mut elev = vec![0.0f32; COLS * ROWS];
        for y in 0..ROWS {
            for x in 0..COLS {
                elev[y * COLS + x] =
                    elevation(1, x as f32, y as f32, tect.macro_at(x, y), tect.mountain_at(x, y));
            }
        }
        let t0 = std::time::Instant::now();
        crate::erosion::erode(1, &mut elev, &|_| {});
        eprintln!(
            "erosion: {} cols, {:.0} ms (MAP_SCALE={MAP_SCALE})",
            COLS * ROWS,
            t0.elapsed().as_secs_f64() * 1000.0
        );
    }

    /// Climate must give the giant map real biome DIVERSITY: several lowland biomes
    /// present (temperature × moisture bands), none absurdly dominant. Prints the mix.
    #[test]
    fn biome_diversity() {
        let t = VoxelTerrain::new(1);
        let mut counts = [0u64; 12];
        let mut land = 0u64;
        for y in 0..ROWS {
            for x in 0..COLS {
                if t.is_water(x, y) {
                    continue;
                }
                land += 1;
                counts[t.biome_at(x, y).id() as usize] += 1;
            }
        }
        for id in 1..12u8 {
            let pct = counts[id as usize] as f64 / land as f64 * 100.0;
            if pct > 0.1 {
                eprintln!("  {:?}: {:.1}%", BiomeKind::from_id(id), pct);
            }
        }
        let present = counts.iter().filter(|&&c| c as f64 / land as f64 > 0.01).count();
        let maxf = *counts.iter().max().unwrap() as f64 / land as f64;
        eprintln!("distinct biomes (>1%): {present}, largest share {:.0}%", maxf * 100.0);
        assert!(present >= 6, "too few biomes present: {present}");
        assert!(maxf < 0.6, "one biome dominates the land: {:.0}%", maxf * 100.0);
    }

    /// Guard that hydrology actually produces both rivers and lakes (a regression in the
    /// flood routing once silently gave 0 rivers). Rebuilds the eroded field + hydrology.
    #[test]
    fn hydrology_makes_rivers_and_lakes() {
        let seed = 2u64;
        let tect = TectonicField::generate(seed);
        let mut elev = vec![0.0f32; COLS * ROWS];
        for y in 0..ROWS {
            for x in 0..COLS {
                elev[y * COLS + x] =
                    elevation(seed, x as f32, y as f32, tect.macro_at(x, y), tect.mountain_at(x, y));
            }
        }
        crate::erosion::erode(seed, &mut elev, &|_| {});
        let hydro = crate::hydrology::compute(&elev, SEA_FRACTION);
        let rivers = hydro.river.iter().filter(|&&r| r).count();
        let lakes = hydro.lake.iter().filter(|&&l| l).count();
        eprintln!("seed {seed}: {rivers} river cells, {lakes} lake cells");
        assert!(rivers > 200, "no river network: {rivers} cells");
        assert!(lakes > 50, "no lakes: {lakes} cells");
    }

    /// Guard the water model: water is never rendered below its own terrain (`misset`),
    /// and there are no swarms of 1-cell water specks (the lake-size filter). Both were
    /// artifacts reported on the 3D view; this locks the data side of the fixes.
    #[test]
    fn water_model_is_clean() {
        let t = VoxelTerrain::new(1);
        let nb = |x: i32, y: i32| [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)];
        let (mut misset, mut isolated, mut water_cols) = (0u64, 0u64, 0u64);
        for y in 0..ROWS as i32 {
            for x in 0..COLS as i32 {
                let (h, wl) = (t.height(x, y), t.water_level(x, y));
                if wl == 0 {
                    continue;
                }
                water_cols += 1;
                if wl <= h {
                    misset += 1;
                }
                if nb(x, y).iter().all(|&(a, b)| t.water_level(a, b) == 0) {
                    isolated += 1;
                }
            }
        }
        // Relative to the water area (scale-independent: ×16 has 4× the columns).
        let frac = isolated as f64 / water_cols.max(1) as f64;
        eprintln!("misset_water={misset}, isolated_water={isolated} ({:.3}% of water)", frac * 100.0);
        assert_eq!(misset, 0, "water rendered below terrain in {misset} columns");
        assert!(frac < 0.005, "too many 1-cell water specks: {:.3}% of water", frac * 100.0);
    }

    /// Diagnose the reported water/tree artifacts numerically on the FINAL world model:
    /// mis-set water (rendered where it shouldn't), terrain poking into water (dry holes →
    /// internal walls), isolated 1-cell water (specks), and land trees overhanging water.
    #[test]
    #[ignore]
    fn diagnose_water_artifacts() {
        let t = VoxelTerrain::new(1);
        let nb = |x: i32, y: i32| [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)];
        let (mut misset, mut dry_holes, mut isolated, mut trees_over_water) = (0u64, 0u64, 0u64, 0u64);
        let mut mountain_with_soil = 0u64;
        for y in 0..ROWS as i32 {
            for x in 0..COLS as i32 {
                let h = t.height(x, y);
                let wl = t.water_level(x, y);
                let watn = nb(x, y).iter().filter(|&&(a, b)| t.water_level(a, b) > 0).count();
                if wl > 0 && wl <= h {
                    misset += 1;
                }
                if wl > 0 && watn == 0 {
                    isolated += 1;
                }
                // Dry land cell mostly ringed by water (pokes up inside a water body).
                if wl == 0 && h > 0 && watn >= 3 {
                    dry_holes += 1;
                }
                // A tree-growing land column next to water → canopy overhangs the water.
                if wl == 0 && h > 0 {
                    let biome = t.biome_at(x as usize, y as usize);
                    if matches!(biome, BiomeKind::Mountain | BiomeKind::Snow) && h >= GROUND_MIN + 3 {
                        mountain_with_soil += 1; // would show a brown topsoil strata band
                    }
                }
            }
        }
        // Trees overhanging water (approximate: tree columns with a water neighbour).
        for y in 0..ROWS {
            for x in 0..COLS {
                if t.water_level(x as i32, y as i32) > 0 {
                    continue;
                }
                let b = t.biome_at(x, y);
                let grows = matches!(b, BiomeKind::Forest | BiomeKind::Jungle | BiomeKind::Taiga | BiomeKind::Plains | BiomeKind::Savanna | BiomeKind::Swamp);
                let near_water = nb(x as i32, y as i32).iter().any(|&(a, c)| t.water_level(a, c) > 0);
                if grows && near_water {
                    trees_over_water += 1;
                }
            }
        }
        eprintln!("misset_water={misset} dry_holes={dry_holes} isolated_water={isolated} trees_near_water={trees_over_water} mountain_soil_bands={mountain_with_soil}");
    }

    /// Report river/lake coverage and dump a water map (ocean / lake / river distinct).
    /// Rebuilds the eroded field + hydrology directly. Run with `--release`.
    #[test]
    #[ignore]
    fn dump_water() {
        use macroquad::color::Color;
        use macroquad::texture::Image;
        let seed = 1u64;
        let tect = TectonicField::generate(seed);
        let n = COLS * ROWS;
        let mut elev = vec![0.0f32; n];
        for y in 0..ROWS {
            for x in 0..COLS {
                elev[y * COLS + x] =
                    elevation(seed, x as f32, y as f32, tect.macro_at(x, y), tect.mountain_at(x, y));
            }
        }
        crate::erosion::erode(seed, &mut elev, &|_| {});
        let hydro = crate::hydrology::compute(&elev, SEA_FRACTION);
        let (mut land, mut river, mut lake) = (0u64, 0u64, 0u64);
        let mut img = Image::gen_image_color(COLS as u16, ROWS as u16, Color::new(0.0, 0.0, 0.0, 1.0));
        for y in 0..ROWS {
            for x in 0..COLS {
                let i = y * COLS + x;
                let sea = elev[i] < SEA_FRACTION;
                let c = if sea {
                    Color::new(0.10, 0.22, 0.42, 1.0) // ocean
                } else if hydro.lake[i] {
                    lake += 1;
                    Color::new(0.30, 0.65, 0.85, 1.0) // lake
                } else if hydro.river[i] {
                    river += 1;
                    land += 1;
                    Color::new(0.55, 0.80, 1.0, 1.0) // river
                } else {
                    land += 1;
                    let v = 0.25 + 0.5 * ((elev[i] - SEA_FRACTION) / (1.0 - SEA_FRACTION)).clamp(0.0, 1.0);
                    Color::new(v, v * 0.95, v * 0.8, 1.0)
                };
                img.set_pixel(x as u32, y as u32, c);
            }
        }
        img.export_png("/tmp/dbg_water.png");
        eprintln!(
            "rivers {:.2}% of land, lakes {} cells; dumped /tmp/dbg_water.png",
            river as f64 / land.max(1) as f64 * 100.0,
            lake
        );
    }

    /// Find the tallest water-to-lower-water step (= the height of a `push_water_side`
    /// wall). A big value explains the "vertical walls in the water" — a water cell whose
    /// neighbour's water surface is many levels lower.
    #[test]
    #[ignore]
    fn diagnose_water_walls() {
        let t = VoxelTerrain::new(1);
        let nb = |x: i32, y: i32| [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)];
        let (mut worst, mut wx, mut wy, mut wwl, mut wnwl) = (0u8, 0i32, 0i32, 0u8, 0u8);
        let mut count_tall = 0u64;
        for y in 0..ROWS as i32 {
            for x in 0..COLS as i32 {
                let wl = t.water_level(x, y);
                if wl == 0 {
                    continue;
                }
                for (nx, ny) in nb(x, y) {
                    let nwl = t.water_level(nx, ny);
                    if nwl > 0 && nwl < wl {
                        let d = wl - nwl;
                        if d >= 3 {
                            count_tall += 1;
                        }
                        if d > worst {
                            worst = d;
                            (wx, wy, wwl, wnwl) = (x, y, wl, nwl);
                        }
                    }
                }
            }
        }
        eprintln!("tallest water wall = {worst} levels at ({wx},{wy}) wl={wwl} nwl={wnwl}; cells with >=3-tall walls: {count_tall}");
    }

    /// Dump every connected WATER body coloured by its surface height (voxel level): each
    /// 4-connected component of `water_level > 0` is flood-filled and painted with a height
    /// ramp (blue=low → red=high). A flat body reads as ONE solid colour; any gradient inside
    /// a single blob is a stepped body. Land is dark grey. Writes /tmp/dbg_water_height.png.
    #[test]
    #[ignore]
    fn dump_water_height() {
        use macroquad::color::Color;
        use macroquad::texture::Image;
        let t = VoxelTerrain::new(1);
        let n = COLS * ROWS;
        // Range of water surface levels present (for normalising the colour ramp).
        let (mut lo, mut hi) = (u8::MAX, 0u8);
        for i in 0..n {
            let wl = t.water[i];
            if wl > 0 {
                lo = lo.min(wl);
                hi = hi.max(wl);
            }
        }
        let span = (hi - lo).max(1) as f32;
        // Blue(low) → cyan → green → yellow → red(high) ramp.
        let ramp = |u: f32| -> Color {
            let stops = [
                (0.00, (0.10, 0.20, 0.70)),
                (0.25, (0.10, 0.75, 0.85)),
                (0.50, (0.20, 0.80, 0.30)),
                (0.75, (0.95, 0.85, 0.15)),
                (1.00, (0.85, 0.15, 0.10)),
            ];
            for w in stops.windows(2) {
                let (a, ca) = w[0];
                let (b, cb) = w[1];
                if u <= b {
                    let f = ((u - a) / (b - a)).clamp(0.0, 1.0);
                    return Color::new(
                        ca.0 + (cb.0 - ca.0) * f,
                        ca.1 + (cb.1 - ca.1) * f,
                        ca.2 + (cb.2 - ca.2) * f,
                        1.0,
                    );
                }
            }
            Color::new(0.85, 0.15, 0.10, 1.0)
        };
        let mut img = Image::gen_image_color(COLS as u16, ROWS as u16, Color::new(0.0, 0.0, 0.0, 1.0));
        let (mut bodies, mut water_cells) = (0u64, 0u64);
        let mut seen = vec![false; n];
        let mut stack: Vec<usize> = Vec::new();
        for y in 0..ROWS {
            for x in 0..COLS {
                let i = y * COLS + x;
                let wl = t.water[i];
                let c = if wl == 0 {
                    Color::new(0.12, 0.12, 0.13, 1.0) // dry land
                } else {
                    ramp((wl - lo) as f32 / span)
                };
                img.set_pixel(x as u32, y as u32, c);
            }
        }
        // Count connected water bodies (4-connected over water_level > 0).
        for start in 0..n {
            if t.water[start] == 0 || seen[start] {
                continue;
            }
            bodies += 1;
            stack.push(start);
            seen[start] = true;
            while let Some(i) = stack.pop() {
                water_cells += 1;
                let (x, y) = ((i % COLS) as i32, (i / COLS) as i32);
                for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                    if nx < 0 || ny < 0 || nx >= COLS as i32 || ny >= ROWS as i32 {
                        continue;
                    }
                    let j = (ny * COLS as i32 + nx) as usize;
                    if t.water[j] > 0 && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        img.export_png("/tmp/dbg_water_height.png");
        eprintln!(
            "water levels {lo}..{hi}; connected bodies={bodies}, water cells={water_cells}; dumped /tmp/dbg_water_height.png"
        );
    }

    /// LOCK: no "lake inside a lake". The bug pinned inland sub-sea pits to the global sea
    /// level (`SEA_ABS`) by absolute elevation, so a deep pit rendered as an ocean-level pool
    /// embedded in a higher lake. Invariant after the fix: an ocean-level water column
    /// (`wl == SEA_ABS`) only exists in a body CONNECTED TO THE MAP BORDER — i.e. the real sea.
    /// Any landlocked `SEA_ABS` cell is the bug. Must be EXACTLY 0 (it's impossible by
    /// construction once ocean is defined by border-connectivity, not by `e < SEA_FRACTION`).
    /// RED until the classify/ocean fix lands.
    #[test]
    fn no_landlocked_ocean() {
        let t = VoxelTerrain::new(1);
        let n = COLS * ROWS;
        let mut seen = vec![false; n];
        let mut stack: Vec<usize> = Vec::new();
        let mut landlocked_ocean_cells = 0u64;
        for start in 0..n {
            if t.water[start] == 0 || seen[start] {
                continue;
            }
            stack.push(start);
            seen[start] = true;
            // Count SEA_ABS cells in this body; flag if the body ever touches the map edge.
            let (mut ocean_cells, mut touches_border) = (0u64, false);
            while let Some(i) = stack.pop() {
                if t.water[i] == SEA_ABS {
                    ocean_cells += 1;
                }
                let (x, y) = ((i % COLS) as i32, (i / COLS) as i32);
                if x == 0 || y == 0 || x == COLS as i32 - 1 || y == ROWS as i32 - 1 {
                    touches_border = true;
                }
                for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                    if nx < 0 || ny < 0 || nx >= COLS as i32 || ny >= ROWS as i32 {
                        continue;
                    }
                    let j = (ny * COLS as i32 + nx) as usize;
                    if t.water[j] > 0 && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
            // Ocean-level water in a body that never reaches the map edge = inland pit pinned
            // to SEA_ABS (the bug). The real sea always touches the border.
            if !touches_border {
                landlocked_ocean_cells += ocean_cells;
            }
        }
        eprintln!("landlocked_ocean_cells={landlocked_ocean_cells}");
        assert_eq!(
            landlocked_ocean_cells, 0,
            "{landlocked_ocean_cells} water cells sit at SEA_ABS in a landlocked body (lake-in-lake bug)"
        );
    }

    /// "Lake inside a lake": flood-fill connected WATER bodies (final model, `water > 0`) and
    /// report bodies that span >1 surface level, separating OCEAN-classified cells (`wl ==
    /// SEA_ABS`) from lake/river. Also counts LANDLOCKED ocean (a `wl == SEA_ABS` body that
    /// never touches the map border) — an inland below-sea-level pit pinned to the global sea.
    #[test]
    #[ignore]
    fn diagnose_lake_in_lake() {
        let t = VoxelTerrain::new(1);
        let n = COLS * ROWS;
        let mut seen = vec![false; n];
        let mut stack: Vec<usize> = Vec::new();
        let mut comp: Vec<usize> = Vec::new();
        let (mut bodies, mut mixed, mut worst_span) = (0u64, 0u64, 0i32);
        let (mut landlocked_ocean_bodies, mut landlocked_ocean_cells) = (0u64, 0u64);
        let mut example = (0usize, 0usize, 0i32, 0i32);
        for start in 0..n {
            if t.water[start] == 0 || seen[start] {
                continue;
            }
            comp.clear();
            stack.push(start);
            seen[start] = true;
            let (mut lo, mut hi) = (i32::MAX, i32::MIN);
            let (mut has_ocean, mut has_other, mut touches_border) = (false, false, false);
            while let Some(i) = stack.pop() {
                comp.push(i);
                let wl = t.water[i] as i32;
                lo = lo.min(wl);
                hi = hi.max(wl);
                if t.water[i] == SEA_ABS {
                    has_ocean = true;
                } else {
                    has_other = true;
                }
                let (x, y) = ((i % COLS) as i32, (i / COLS) as i32);
                if x == 0 || y == 0 || x == COLS as i32 - 1 || y == ROWS as i32 - 1 {
                    touches_border = true;
                }
                for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                    if nx < 0 || ny < 0 || nx >= COLS as i32 || ny >= ROWS as i32 {
                        continue;
                    }
                    let j = (ny * COLS as i32 + nx) as usize;
                    if t.water[j] > 0 && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
            bodies += 1;
            let span = hi - lo;
            if span > 0 {
                mixed += 1;
                if span > worst_span {
                    worst_span = span;
                    example = (start % COLS, start / COLS, lo, hi);
                }
            }
            // A wholly-ocean body that never reaches the border = inland "sea" at SEA_ABS.
            if has_ocean && !has_other && !touches_border {
                landlocked_ocean_bodies += 1;
                landlocked_ocean_cells += comp.len() as u64;
            }
        }
        eprintln!(
            "water bodies={bodies}, MIXED-level bodies={mixed}, worst span={worst_span} levels (lo={} hi={} near col={} row={}); landlocked-ocean bodies={landlocked_ocean_bodies} cells={landlocked_ocean_cells}",
            example.2, example.3, example.0, example.1
        );
    }

    /// Lake flatness: flood-fill each connected LAKE body (`hydro.lake`) and report how many
    /// distinct rendered water levels (`elev_to_level(filled).round()`) it spans. A correct
    /// lake is ONE flat mirror → span 0. Any body with span > 0 is a stepped lake (the bug).
    #[test]
    #[ignore]
    fn diagnose_lake_steps() {
        let seed = 1u64;
        let tect = TectonicField::generate(seed);
        let n = COLS * ROWS;
        let mut elev = vec![0.0f32; n];
        for y in 0..ROWS {
            for x in 0..COLS {
                elev[y * COLS + x] =
                    elevation(seed, x as f32, y as f32, tect.macro_at(x, y), tect.mountain_at(x, y));
            }
        }
        crate::erosion::erode(seed, &mut elev, &|_| {});
        let hydro = crate::hydrology::compute(&elev, SEA_FRACTION);
        // Rendered water level per lake column (same formula the world model uses).
        let lvl = |i: usize| elev_to_level(hydro.filled[i]).round() as i32;
        let mut seen = vec![false; n];
        let (mut bodies, mut stepped_bodies, mut stepped_cells, mut worst_span) = (0u64, 0u64, 0u64, 0i32);
        let mut stack: Vec<usize> = Vec::new();
        let mut comp: Vec<usize> = Vec::new();
        for start in 0..n {
            if !hydro.lake[start] || seen[start] {
                continue;
            }
            comp.clear();
            stack.push(start);
            seen[start] = true;
            let (mut lo, mut hi) = (i32::MAX, i32::MIN);
            while let Some(i) = stack.pop() {
                comp.push(i);
                let l = lvl(i);
                lo = lo.min(l);
                hi = hi.max(l);
                let (x, y) = ((i % COLS) as i32, (i / COLS) as i32);
                for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                    if nx < 0 || ny < 0 || nx >= COLS as i32 || ny >= ROWS as i32 {
                        continue;
                    }
                    let j = (ny * COLS as i32 + nx) as usize;
                    if hydro.lake[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
            bodies += 1;
            let span = hi - lo;
            if span > 0 {
                stepped_bodies += 1;
                stepped_cells += comp.len() as u64;
                worst_span = worst_span.max(span);
            }
        }
        eprintln!(
            "lake bodies={bodies}, STEPPED bodies={stepped_bodies} ({:.0}%), cells in stepped bodies={stepped_cells}, worst intra-lake span={worst_span} levels",
            stepped_bodies as f64 / bodies.max(1) as f64 * 100.0
        );
    }

    /// Shore HOLES: dry cells whose top sits BELOW an adjacent water cell's surface — the
    /// gap between bank and water the user reported. Counts them and attributes each to WHY
    /// it stayed dry (ocean/lake/river flag vs none), so we can tell a hydrology
    /// classification gap from a pure discretisation artefact. Run: `cargo test
    /// diagnose_shore_holes -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn diagnose_shore_holes() {
        for seed in [1u64, 7, 42] {
            let t = VoxelTerrain::new(seed);
            // Recompute hydrology to attribute each hole (same inputs as `VoxelTerrain::new`).
            let tect = TectonicField::generate(seed);
            let n = COLS * ROWS;
            let mut elev = vec![0.0f32; n];
            for y in 0..ROWS {
                for x in 0..COLS {
                    elev[y * COLS + x] =
                        elevation(seed, x as f32, y as f32, tect.macro_at(x, y), tect.mountain_at(x, y));
                }
            }
            crate::erosion::erode(seed, &mut elev, &|_| {});
            let hydro = crate::hydrology::compute(&elev, SEA_FRACTION);

            let (mut holes, mut as_lake, mut as_river, mut as_ocean, mut as_none) =
                (0u64, 0u64, 0u64, 0u64, 0u64);
            let mut holes_by_lake = 0u64; // holes whose deepest adjacent water is a LAKE
            let mut max_drop = 0u8;
            let mut sample = None;
            for y in 0..ROWS {
                for x in 0..COLS {
                    let i = y * COLS + x;
                    let (xi, yi) = (x as i32, y as i32);
                    if t.water_level(xi, yi) != 0 {
                        continue; // this cell is water — not a hole
                    }
                    let h = t.height(xi, yi);
                    // Deepest adjacent water surface standing ABOVE this dry cell's top, and
                    // whether the deepest such neighbour is a LAKE (vs a river channel wall).
                    let mut below_wl = 0u8;
                    let mut adj_is_lake = false;
                    for (nx, ny) in [(xi + 1, yi), (xi - 1, yi), (xi, yi + 1), (xi, yi - 1)] {
                        let nwl = t.water_level(nx, ny);
                        if nwl > h && nwl > below_wl {
                            below_wl = nwl;
                            let j = (ny as usize) * COLS + nx as usize;
                            adj_is_lake = (0..COLS as i32).contains(&nx)
                                && (0..ROWS as i32).contains(&ny)
                                && hydro.lake[j];
                        }
                    }
                    if below_wl == 0 {
                        continue; // sits above all neighbouring water — a normal bank, not a hole
                    }
                    holes += 1;
                    if adj_is_lake {
                        holes_by_lake += 1;
                    }
                    max_drop = max_drop.max(below_wl - h);
                    if hydro.ocean[i] {
                        as_ocean += 1;
                    } else if hydro.lake[i] {
                        as_lake += 1;
                    } else if hydro.river[i] {
                        as_river += 1;
                    } else {
                        as_none += 1;
                    }
                    if sample.is_none() {
                        sample = Some((x, y, h, below_wl, hydro.filled[i] - elev[i]));
                    }
                }
            }

            // SPIKES: dry cells ringed by water on >=3 sides that poke ABOVE every adjacent
            // water surface — a lone pillar standing in the water. Also flag how many were
            // pushed up by the shore-lift pass (height now exceeds the raw classify height),
            // to tell a manufactured spike from a natural island.
            let (mut spikes, mut spikes_lifted) = (0u64, 0u64);
            let mut spike_sample = None;
            for y in 0..ROWS {
                for x in 0..COLS {
                    let (xi, yi) = (x as i32, y as i32);
                    if t.water_level(xi, yi) != 0 {
                        continue;
                    }
                    let h = t.height(xi, yi);
                    let (mut water_nb, mut above_all) = (0u8, true);
                    for (nx, ny) in [(xi + 1, yi), (xi - 1, yi), (xi, yi + 1), (xi, yi - 1)] {
                        let nwl = t.water_level(nx, ny);
                        if nwl > 0 {
                            water_nb += 1;
                            if nwl >= h {
                                above_all = false; // some neighbour water reaches this cell's top
                            }
                        }
                    }
                    if water_nb >= 3 && above_all {
                        spikes += 1;
                        // Raw classify height (pre shore-lift) for this column.
                        let raw = elev_to_level(elev[y * COLS + x]).round() as u8;
                        if h > raw {
                            spikes_lifted += 1;
                        }
                        if spike_sample.is_none() {
                            spike_sample = Some((x, y, h, raw, water_nb));
                        }
                    }
                }
            }

            eprintln!(
                "seed {seed}: shore holes={holes} (dry but below adj water), of which adjacent-to-LAKE={holes_by_lake}, max drop={max_drop} lvl; \
                 hole-cell class: ocean={as_ocean} lake={as_lake} river={as_river} none={as_none}; \
                 sample (x,y,h,adjwl,filled-elev)={sample:?}"
            );
            eprintln!(
                "seed {seed}: SPIKES={spikes} (dry pillar, >=3 water nbrs, above all), of which shore-lifted={spikes_lifted}; \
                 sample (x,y,h,raw,water_nbrs)={spike_sample:?}"
            );
        }
    }

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
    fn out_of_world_is_air_and_sampling_is_consistent() {
        let t = VoxelTerrain::new(3);
        // Out of the world reads as air (height 0) on every side — that's the slab edge.
        assert_eq!(t.height(-1, 0), 0);
        assert_eq!(t.height(0, -1), 0);
        assert_eq!(t.height(COLS as i32, 0), 0);
        assert_eq!(t.height(0, ROWS as i32), 0);
        // The signed `height`/`cell` and the unsigned `height_at` agree in-world, and
        // a column read straight across a chunk seam (x = CHUNK-1 vs CHUNK) is the same
        // value whether reached as "self" or as a neighbour — the seam is just one array.
        for &(x, y) in &[(0usize, 0usize), (CHUNK - 1, 1), (CHUNK, 1), (COLS - 1, ROWS - 1)] {
            assert_eq!(t.height(x as i32, y as i32), t.height_at(x, y));
            assert_eq!(cell_height(t.cell(x as i32, y as i32)), t.height_at(x, y));
        }
    }

    /// `quant_unit` ↔ `/255.0` round-trips within one quantisation step (≤ 1/255).
    #[test]
    fn quant_roundtrip_within_one_step() {
        for k in 0..=1000u32 {
            let v = k as f32 / 1000.0;
            let back = quant_unit(v) as f32 / 255.0;
            assert!((back - v).abs() <= 1.0 / 255.0 + 1e-6, "quant({v}) round-trips to {back}");
        }
        // Saturates, doesn't wrap.
        assert_eq!(quant_unit(-0.5), 0);
        assert_eq!(quant_unit(1.5), 255);
    }

    /// S1: temperature/moisture are now populated for EVERY column (not just lowland), and
    /// temperature is a REAL latitude field everywhere — the equator row is warmer on average
    /// than the poles. (If climate were left unset on beach/mountain/snow columns, those rows
    /// would read a flat 0 and the gradient would vanish.) Also bounds every value to `[0,1]`.
    #[test]
    fn env_fields_populated_and_latitude_gradient() {
        let t = VoxelTerrain::new(1);
        let row_mean_temp = |y: usize| {
            let s: f32 = (0..COLS).map(|x| t.temperature_at(x, y)).sum();
            s / COLS as f32
        };
        let equator = row_mean_temp(ROWS / 2);
        let pole = (row_mean_temp(0) + row_mean_temp(ROWS - 1)) * 0.5;
        eprintln!("mean temp: equator {equator:.3}, poles {pole:.3}");
        assert!(equator > pole + 0.1, "no latitude temp gradient: equator {equator:.3} ≤ poles {pole:.3}");
        // Every sampled column (across all biome bands) has in-range, defined climate.
        let mut moist_seen_low = false;
        let mut moist_seen_high = false;
        for y in (0..ROWS).step_by(ROWS / 20) {
            for x in (0..COLS).step_by(COLS / 20) {
                let (te, mo) = (t.temperature_at(x, y), t.moisture_at(x, y));
                assert!((0.0..=1.0).contains(&te) && (0.0..=1.0).contains(&mo));
                moist_seen_low |= mo < 0.35;
                moist_seen_high |= mo > 0.65;
            }
        }
        assert!(moist_seen_low && moist_seen_high, "moisture field has no variety");
    }

    /// S1: distance-to-water is 0 exactly on water, grows off the shore, and is a valid BFS
    /// (every non-source column under the cap has a 4-neighbour one step closer). The far
    /// inland plateau reaching the 255 floor is expected, not a bug.
    #[test]
    fn water_dist_is_a_valid_bfs() {
        let t = VoxelTerrain::new(1);
        let mut max_d = 0u8;
        for y in 0..ROWS {
            for x in 0..COLS {
                let d = t.water_dist_at(x, y);
                // Source set agrees with the water flag (they're set together in `generate`).
                assert_eq!(d == 0, t.is_water(x, y), "water_dist 0 vs is_water disagree at ({x},{y})");
                max_d = max_d.max(d);
                // BFS invariant: an interior column with finite distance has a strictly closer
                // neighbour (skip the saturated 255 rim, where the true distance is clipped).
                if (1..255).contains(&d) && x > 0 && y > 0 && x + 1 < COLS && y + 1 < ROWS {
                    let closer = [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]
                        .iter()
                        .any(|&(a, b)| t.water_dist_at(a, b) == d - 1);
                    assert!(closer, "water_dist not a BFS at ({x},{y}): d={d}, no neighbour at d-1");
                }
            }
        }
        eprintln!("max water_dist = {max_d}");
        assert!(max_d > 1, "distance-to-water never grows (no inland?)");
    }

    /// S1: slope is in `[0,1]`, zero on flat ground (open-sea floor is flat), positive where
    /// the terrain steps, and shows NO false cliff at the map edge (out-of-world neighbours
    /// are treated as level, so the corner column isn't read as a wall).
    #[test]
    fn slope_bounds_and_no_edge_cliff() {
        let t = VoxelTerrain::new(1);
        let mut max_s = 0.0f32;
        for y in (0..ROWS).step_by(7) {
            for x in (0..COLS).step_by(7) {
                let s = t.slope_at(x, y);
                assert!((0.0..=1.0).contains(&s), "slope out of range at ({x},{y}): {s}");
                max_s = max_s.max(s);
            }
        }
        assert!(max_s > 0.0, "slope is zero everywhere (no relief?)");
        // Corners: bounded, and not a spurious full-relief cliff from the world edge.
        for &(x, y) in &[(0, 0), (COLS - 1, 0), (0, ROWS - 1), (COLS - 1, ROWS - 1)] {
            let s = t.slope_at(x, y);
            assert!((0.0..1.0).contains(&s), "edge column ({x},{y}) reads a false cliff: {s}");
        }
    }

    /// S1: the new fields are deterministic per seed (the sim must replay).
    #[test]
    fn env_fields_are_deterministic() {
        let (a, b) = (VoxelTerrain::new(7), VoxelTerrain::new(7));
        for &(x, y) in &[(0, 0), (COLS / 3, ROWS / 2), (COLS - 1, ROWS - 1), (CHUNK, CHUNK)] {
            assert_eq!(a.temperature_at(x, y), b.temperature_at(x, y));
            assert_eq!(a.moisture_at(x, y), b.moisture_at(x, y));
            assert_eq!(a.water_dist_at(x, y), b.water_dist_at(x, y));
            assert_eq!(a.slope_at(x, y), b.slope_at(x, y));
        }
    }

    // ---- S3: vegetation (pure-law tests, no world generation needed) ----

    /// The regrow law recovers from 0 (no fixed point there — the bug a logistic law would
    /// have had), is monotonic, and never overshoots capacity.
    #[test]
    fn regrow_recovers_from_zero_and_saturates() {
        let cap = 0.8;
        assert_eq!(regrow(0.0, cap, 0.0), 0.0); // no time → no growth
        let a = regrow(0.0, cap, 50.0);
        let b = regrow(0.0, cap, 100.0);
        assert!(0.0 < a && a < b && b < cap, "not monotonic toward cap: {a} {b} {cap}");
        assert!(regrow(0.0, cap, 1e6) <= cap + 1e-6, "overshot cap");
        assert!((regrow(0.0, cap, 1e6) - cap).abs() < 1e-3, "did not saturate to cap");
        // From a non-zero start it still only ever climbs to cap.
        assert!(regrow(0.5, cap, 1e6) <= cap + 1e-6);
    }

    /// Closed-form ⇒ the lazy (skip-the-untouched-ticks) update equals the stepwise one:
    /// regrowing once over `t1+t2` matches regrowing over `t1` then `t2`. This is what makes
    /// amortised regen exact, so a column the sim ignores for a million ticks is still correct.
    #[test]
    fn regrow_is_semigroup() {
        let cap = 0.9;
        let (t1, t2) = (37.0, 121.0);
        let lazy = regrow(0.1, cap, t1 + t2);
        let stepwise = regrow(regrow(0.1, cap, t1), cap, t2);
        assert!((lazy - stepwise).abs() < 1e-6, "lazy {lazy} ≠ stepwise {stepwise}");
    }

    /// Capacity: water carries nothing, wetter biomes carry more, and moisture nudges it up.
    #[test]
    fn carrying_capacity_ordering() {
        assert_eq!(carrying_capacity(BiomeKind::Ocean, 1.0), 0.0);
        let jungle = carrying_capacity(BiomeKind::Jungle, 0.5);
        let plains = carrying_capacity(BiomeKind::Plains, 0.5);
        let desert = carrying_capacity(BiomeKind::Desert, 0.5);
        assert!(jungle > plains && plains > desert, "{jungle} {plains} {desert}");
        assert!(
            carrying_capacity(BiomeKind::Forest, 0.9) > carrying_capacity(BiomeKind::Forest, 0.1),
            "moisture should raise capacity"
        );
    }

    /// S3 on the real world model (one generation): vegetation starts mature, water is bare,
    /// grazing removes ≤ what's present, a cleared column regrows from 0, and a long chain of
    /// graze→requant doesn't drift (the F5 quantisation-noise guard).
    #[test]
    fn vegetation_field_grazing_and_regrowth() {
        let mut t = VoxelTerrain::new(1);
        // Find a high-capacity land column and a water column.
        let (mut land, mut wet) = (None, None);
        'scan: for y in (0..ROWS).step_by(13) {
            for x in (0..COLS).step_by(13) {
                if land.is_none() && !t.is_water(x, y) {
                    let cap = carrying_capacity(t.biome_at(x, y), t.moisture_at(x, y));
                    if cap > 0.3 {
                        land = Some((x, y, cap));
                    }
                }
                if wet.is_none() && t.is_water(x, y) {
                    wet = Some((x, y));
                }
                if land.is_some() && wet.is_some() {
                    break 'scan;
                }
            }
        }
        let (lx, ly, cap) = land.expect("no high-capacity land column found");
        let (wx, wy) = wet.expect("no water column found");

        // Mature start: biomass ≈ capacity at tick 0 (within one quant step).
        assert!((t.biomass_at(lx, ly, 0) - cap).abs() <= 1.0 / 255.0 + 1e-6, "veg not mature at gen");
        // Water is bare at any tick.
        assert_eq!(t.biomass_at(wx, wy, 0), 0.0);
        assert_eq!(t.biomass_at(wx, wy, 100_000), 0.0);
        assert_eq!(t.graze(wx, wy, 1.0, 5), 0.0, "grazed biomass off water");

        // Clear-cut the land column: takes ≈ all of it, leaves ~0.
        let taken = t.graze(lx, ly, 1.0, 0);
        assert!((taken - cap).abs() <= 2.0 / 255.0, "clear-cut took {taken}, expected ≈{cap}");
        assert!(t.biomass_at(lx, ly, 0) <= 1.0 / 255.0, "column not bare right after clear-cut");
        // Over-graze immediately: nothing left to take.
        assert!(t.graze(lx, ly, 1.0, 0) <= 1.0 / 255.0, "over-graze produced food from nothing");

        // Regrows from 0 back toward cap, monotonically, with NO downward drift across 200
        // graze→requantise cycles (F5 quantisation-noise guard).
        let mut prev = 0.0f32;
        for k in 1..=200u64 {
            let tick = k * 50;
            t.graze(lx, ly, 0.0, tick); // take nothing, but re-quantise current at this tick
            let b = t.biomass_at(lx, ly, tick);
            assert!(b >= prev - 2.0 / 255.0, "biomass drifted DOWN at step {k}: {b} < {prev}");
            assert!(b <= cap + 1.0 / 255.0, "biomass exceeded cap at step {k}: {b} > {cap}");
            prev = b;
        }
        assert!((prev - cap).abs() < 0.05, "did not regrow to cap: {prev} vs {cap}");
    }

    /// Biomass replays deterministically: same graze sequence ⇒ same readings.
    #[test]
    fn biomass_is_deterministic() {
        let (mut a, mut b) = (VoxelTerrain::new(5), VoxelTerrain::new(5));
        let col = (COLS / 2 + 7, ROWS / 3 + 3);
        for k in 0..10u64 {
            a.graze(col.0, col.1, 0.05, k * 20);
            b.graze(col.0, col.1, 0.05, k * 20);
        }
        assert_eq!(a.biomass_at(col.0, col.1, 500), b.biomass_at(col.0, col.1, 500));
    }
}
