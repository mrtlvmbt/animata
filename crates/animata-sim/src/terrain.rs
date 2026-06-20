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
pub fn fbm(seed: u64, x: f32, y: f32, salt: u64, octaves: u32) -> f32 {
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
    /// Inorganic nutrient pool per column (C3 nutrient cycle), `[0,255]`. Plant capacity is
    /// scaled by it (Liebig limit), grazing CARRIES it away with the herbivore, creature death
    /// returns it (decomposition), and it weathers toward a geology baseline. Lazy like biomass:
    /// updated only on events (graze/deposit), weathered closed-form from `nutrient_update`.
    nutrient: Vec<u8>,
    nutrient_update: Vec<u32>,
}

impl VoxelTerrain {
    /// Fold the MUTABLE terrain state into a determinism checksum (PR1 lock). Geometry
    /// (`surf`/`biome`/`flags`/`water`/`temp`/`moist`/`water_dist`) is fixed after worldgen and
    /// reproducible from `seed`, so only the sim-mutated fields (vegetation + nutrient pools and
    /// their lazy-update timestamps) need hashing here. Integer-only fold (F2).
    /// (Used by the determinism-checksum tests now; by the metrics-registry checksum metric in PR5.)
    #[allow(dead_code)]
    pub fn mut_state_checksum(&self) -> u64 {
        let mut h = crate::rng::FNV_OFFSET;
        crate::rng::fnv_fold_u64(&mut h, self.seed);
        for &b in &self.biomass {
            crate::rng::fnv_fold_u32(&mut h, b as u32);
        }
        for &u in &self.last_update {
            crate::rng::fnv_fold_u32(&mut h, u);
        }
        for &n in &self.nutrient {
            crate::rng::fnv_fold_u32(&mut h, n as u32);
        }
        for &u in &self.nutrient_update {
            crate::rng::fnv_fold_u32(&mut h, u);
        }
        h
    }
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

/// Geology nutrient baseline for a column in `[0,1]`: lowlands fertile, high ground poor (thin,
/// leached soils), water beds moderate. The level weathering relaxes toward — the slow abiotic
/// anchor that keeps the cycle from running down or up without bound.
fn nutrient_baseline(biome: BiomeKind, h: u8) -> f32 {
    let alt = (h.saturating_sub(LAND_FOOT)) as f32 / SURFACE_RANGE as f32; // 0 foot .. 1 peak
    let base = match biome {
        BiomeKind::Mountain | BiomeKind::Snow => 0.25,
        BiomeKind::Desert => 0.35,
        BiomeKind::Swamp | BiomeKind::Jungle => 0.85, // rich wet lowlands
        _ => 0.6,
    };
    (base * (1.0 - 0.5 * alt)).clamp(0.05, 1.0)
}

/// Closed-form weathering: nutrient `n` relaxes toward the geology `baseline` over `elapsed`
/// sim-seconds — `n' = base + (n − base)·e^(−RATE·elapsed)`. Same lazy shape as regrow (so it is
/// updated only on events, never per-tick), and bounded both ways (a death-enriched column decays
/// back down, a leached one weathers back up) → total matter stays anchored, not drifting.
fn weather(n: f32, baseline: f32, elapsed: f32) -> f32 {
    (baseline + (n - baseline) * (-NUTRIENT_WEATHER_RATE * elapsed).exp()).clamp(0.0, 1.0)
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
        let mut nutrient = vec![0u8; n];
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
                // Nutrient starts at the geology baseline (the level weathering relaxes toward).
                let nb = nutrient_baseline(b, s.round() as u8);
                nutrient[i] = quant_unit(nb);
                // Vegetation starts mature at the NUTRIENT-LIMITED capacity (carrying capacity ×
                // nutrient fraction) on the FINAL biome + moisture; water columns get 0. Starting
                // at the un-limited cap would overshoot and clamp down on the first read.
                biomass[i] = quant_unit(carrying_capacity(b, cm) * nb);
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
            nutrient,
            nutrient_update: vec![0u32; n],
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

    /// Vegetation capacity `[0,1]` for a column (lazy-regrow target): the biome/moisture base
    /// SCALED by the column's nutrient pool (Liebig limit — a nutrient-poor column grows less
    /// plant). Uses the STORED nutrient (frozen since the last event), so the cap is constant
    /// between events and the lazy biomass regrow stays a valid closed form (F4 discipline).
    fn cap_at(&self, i: usize) -> f32 {
        let base = carrying_capacity(BiomeKind::from_id(self.biome[i]), self.moist[i] as f32 / 255.0);
        base * (self.nutrient[i] as f32 / 255.0)
    }

    /// Geology nutrient baseline (`[0,1]`) for a column — what weathering relaxes toward.
    fn nutrient_base_at(&self, i: usize) -> f32 {
        nutrient_baseline(BiomeKind::from_id(self.biome[i]), self.surf[i].round() as u8)
    }

    /// Materialise the lazy nutrient weathering up to `tick` and persist it (an event touched
    /// this column). Between events the column is frozen, so this is the only place nutrient
    /// moves on its own — no per-tick sweep.
    fn materialize_nutrient(&mut self, i: usize, tick: u64) {
        let elapsed = (tick - self.nutrient_update[i] as u64) as f32 * TICK_LEN;
        let n = weather(self.nutrient[i] as f32 / 255.0, self.nutrient_base_at(i), elapsed);
        self.nutrient[i] = quant_unit(n);
        self.nutrient_update[i] = tick as u32;
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
        // Weather the nutrient to now FIRST, so `cap_at` (and thus the regrow target) reflects
        // the current pool; then grow + graze.
        self.materialize_nutrient(i, tick);
        let cur = self.current_biomass(i, tick);
        let taken = amount.clamp(0.0, cur);
        self.biomass[i] = quant_unit(cur - taken);
        self.last_update[i] = tick as u32;
        // The grazed plant matter LEAVES the column with the herbivore (carried, to be returned
        // as nutrient where the creature later dies) — the nutrient pool drops accordingly. This
        // is what makes heavily-grazed ground go nutrient-poor (and depend on the death recycle).
        let drained = taken * NUTRIENT_PER_BIOMASS;
        self.nutrient[i] = quant_unit((self.nutrient[i] as f32 / 255.0 - drained).max(0.0));
        taken
    }

    /// Deposit `amount` (`[0,1]` nutrient units) into a column — the decomposition return when a
    /// creature dies here (its locked matter goes back to the inorganic pool). Weathers first so
    /// the addition lands on the up-to-date pool.
    pub fn deposit_nutrient(&mut self, x: usize, y: usize, amount: f32, tick: u64) {
        let i = y * COLS + x;
        self.materialize_nutrient(i, tick);
        self.nutrient[i] = quant_unit((self.nutrient[i] as f32 / 255.0 + amount).min(1.0));
    }

    /// Live nutrient level `[0,1]` at a column (weathered to `tick`, read-only — does not persist,
    /// like `biomass_at`). For the sim's foraging-quality sense + observability.
    pub fn nutrient_at(&self, x: usize, y: usize, tick: u64) -> f32 {
        let i = y * COLS + x;
        let elapsed = (tick - self.nutrient_update[i] as u64) as f32 * TICK_LEN;
        weather(self.nutrient[i] as f32 / 255.0, self.nutrient_base_at(i), elapsed)
    }

    /// Ground tone `[0,1]` (dark .. light) a creature is seen AGAINST at a column — the camouflage
    /// background (C3). Derived from the biome (sand/snow light, forest/jungle dark, …) so a prey
    /// whose coloration matches the local ground is hard for a predator to spot.
    pub fn ground_tone_at(&self, x: usize, y: usize) -> f32 {
        match BiomeKind::from_id(self.biome[y * COLS + x]) {
            BiomeKind::Snow => 0.95,
            BiomeKind::Beach | BiomeKind::Desert => 0.82,
            BiomeKind::Tundra | BiomeKind::Savanna => 0.6,
            BiomeKind::Plains | BiomeKind::Mountain => 0.5,
            BiomeKind::Ocean => 0.4,
            BiomeKind::Swamp | BiomeKind::Taiga => 0.3,
            BiomeKind::Forest => 0.25,
            BiomeKind::Jungle => 0.2,
        }
    }
}

#[cfg(test)]
#[path = "terrain_tests.rs"]
mod tests;
