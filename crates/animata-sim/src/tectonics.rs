//! Plate tectonics — the macro layer of worldgen. Voronoi plates each carry a type
//! (continental/oceanic) and a motion vector; where two plates CONVERGE the boundary
//! uplifts into a mountain BELT/chain, where they DIVERGE it rifts. The result is a
//! normalised macro-elevation field (continents vs ocean basins, with real chains) plus
//! a "mountainness" field that the noise layer (ridged ridgelines) rides on top of.
//!
//! Global by nature: plate assignment is a Voronoi over the whole grid, and the uplift
//! reaches inland via a distance transform from the boundaries — both need the full map.
//! So it's computed once per seed into flat `COLS×ROWS` arrays that the per-column
//! generator samples. Pure function of the seed (deterministic).

use crate::config::*;

/// Average plate footprint in columns — scales with the map, so the plate COUNT stays
/// roughly constant (~12-18) while plates grow with a gigantic map (big structures).
const PLATE_CELL: usize = 30 * MAP_SCALE;
/// Fraction of plates that are continental (land); the rest are oceanic basins. Sets the
/// rough land/water balance together with `SEA_FRACTION`.
const CONTINENTAL_FRAC: f32 = 0.52;
const CONTINENTAL_BASE: f32 = 0.58;
const OCEANIC_BASE: f32 = 0.20;
/// How far (columns) orogenic uplift reaches inland from a convergent boundary — the
/// half-width of a mountain belt. Scales with the map.
const UPLIFT_REACH: f32 = 13.0 * MAP_SCALE as f32;
const UPLIFT_AMP: f32 = 0.45;
const RIFT_AMP: f32 = 0.16;
/// Low-frequency variation added to plate interiors so they aren't dead flat.
const MACRO_LATTICE: f32 = 38.0 * MAP_SCALE as f32;
const MACRO_OCTAVES: u32 = 3;
const MACRO_VAR: f32 = 0.13;
/// Domain warp applied to the Voronoi lookup so plate boundaries (and the coastlines /
/// mountain walls / rifts that follow them) MEANDER organically instead of being the
/// dead-straight perpendicular bisectors of raw Voronoi. The amplitude is a good
/// fraction of the plate size, sampled fractally (multi-scale wiggle).
const PLATE_WARP_LATTICE: f32 = 15.0 * MAP_SCALE as f32;
const PLATE_WARP_OCTAVES: u32 = 4;
const PLATE_WARP_AMP: f32 = 10.0 * MAP_SCALE as f32;
/// Blur radius (columns) for the plate base step → continental-shelf slope width. Scales
/// with the map so the shelf looks the same at any `MAP_SCALE`.
const SHELF_RADIUS: i32 = 2 * MAP_SCALE as i32;

fn hashf(seed: u64, a: i64, b: i64, salt: u64) -> f32 {
    let mut h = seed ^ 0xD1B5_4A32_D192_ED03;
    h ^= (a as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h = h.rotate_left(31);
    h ^= (b as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h = h.rotate_left(29);
    h ^= salt.wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    h = h.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    h ^= h >> 33;
    (h >> 40) as f32 / (1u64 << 24) as f32 // [0, 1)
}

struct Plate {
    sx: f32,
    sy: f32,
    mx: f32,
    my: f32,
    base: f32,
}

/// The tectonic macro fields, one value per column.
pub struct TectonicField {
    macro_elev: Vec<f32>,
    mountainness: Vec<f32>,
    uplift: Vec<f32>,
}

impl TectonicField {
    /// Normalised macro elevation in `[0, 1]` (continents/basins + orogenic belts).
    pub fn macro_at(&self, x: usize, y: usize) -> f32 {
        self.macro_elev[y * COLS + x]
    }
    /// How orogenic this column is in `[0, 1]` — high in mountain belts. Gates the
    /// ridged-noise amplitude so ridgelines/cliffs concentrate on real belts.
    pub fn mountain_at(&self, x: usize, y: usize) -> f32 {
        self.mountainness[y * COLS + x]
    }
    /// The orogenic **uplift-rate** field in `[0, 1]`: the distance-weighted plate
    /// CONVERGENCE (not the static elevation), high where plates collide and falling off
    /// inland over the belt reach. This is the `U(x)` a stream-power LEM ([`crate::lem`])
    /// balances against fluvial incision — a real rate, not the `mountainness` proxy.
    pub fn uplift_field(&self) -> &[f32] {
        &self.uplift
    }

    #[cfg(test)]
    pub fn macro_field(&self) -> &[f32] {
        &self.macro_elev
    }
    #[cfg(test)]
    pub fn mountain_field(&self) -> &[f32] {
        &self.mountainness
    }

    pub fn generate(seed: u64) -> Self {
        let n = COLS * ROWS;
        // ---- Plates: jittered grid of seed points (count ~constant across MAP_SCALE) ----
        let gx = (COLS / PLATE_CELL).max(3);
        let gy = (ROWS / PLATE_CELL).max(3);
        let (cell_w, cell_h) = (COLS as f32 / gx as f32, ROWS as f32 / gy as f32);
        let mut plates = Vec::with_capacity(gx * gy);
        for cy in 0..gy {
            for cx in 0..gx {
                let (ix, iy) = (cx as i64, cy as i64);
                let sx = (cx as f32 + hashf(seed, ix, iy, 1)) * cell_w;
                let sy = (cy as f32 + hashf(seed, ix, iy, 2)) * cell_h;
                let ang = hashf(seed, ix, iy, 3) * std::f32::consts::TAU;
                let speed = 0.5 + 0.5 * hashf(seed, ix, iy, 4);
                let base = if hashf(seed, ix, iy, 5) < CONTINENTAL_FRAC {
                    CONTINENTAL_BASE
                } else {
                    OCEANIC_BASE
                };
                plates.push(Plate { sx, sy, mx: ang.cos() * speed, my: ang.sin() * speed, base });
            }
        }

        // ---- Voronoi: nearest plate per column, with a domain-warped lookup so the
        // boundaries meander instead of being straight perpendicular bisectors ----
        let mut plate_id = vec![0u16; n];
        for y in 0..ROWS {
            for x in 0..COLS {
                let wx = crate::terrain::fbm(
                    seed,
                    x as f32 / PLATE_WARP_LATTICE,
                    y as f32 / PLATE_WARP_LATTICE,
                    811,
                    PLATE_WARP_OCTAVES,
                ) - 0.5;
                let wy = crate::terrain::fbm(
                    seed,
                    x as f32 / PLATE_WARP_LATTICE,
                    y as f32 / PLATE_WARP_LATTICE,
                    823,
                    PLATE_WARP_OCTAVES,
                ) - 0.5;
                let fx = x as f32 + wx * PLATE_WARP_AMP;
                let fy = y as f32 + wy * PLATE_WARP_AMP;
                let mut best = 0usize;
                let mut best_d = f32::MAX;
                for (p, pl) in plates.iter().enumerate() {
                    let d = (pl.sx - fx).powi(2) + (pl.sy - fy).powi(2);
                    if d < best_d {
                        best_d = d;
                        best = p;
                    }
                }
                plate_id[y * COLS + x] = best as u16;
            }
        }

        // ---- Boundaries + convergence between the meeting plates ----
        // Convergence = relative approach speed across the boundary: project the relative
        // plate velocity onto the A→B direction. Positive ⇒ converging ⇒ uplift; negative
        // ⇒ diverging ⇒ rift.
        let mut boundary = vec![false; n];
        let mut conv = vec![0f32; n];
        for y in 0..ROWS as i32 {
            for x in 0..COLS as i32 {
                let i = y as usize * COLS + x as usize;
                let p = plate_id[i] as usize;
                for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                    if nx < 0 || ny < 0 || nx >= COLS as i32 || ny >= ROWS as i32 {
                        continue;
                    }
                    let q = plate_id[ny as usize * COLS + nx as usize] as usize;
                    if q != p {
                        let (a, b) = (&plates[p], &plates[q]);
                        let (dx, dy) = (b.sx - a.sx, b.sy - a.sy);
                        let len = (dx * dx + dy * dy).sqrt().max(1e-3);
                        boundary[i] = true;
                        conv[i] = ((a.mx - b.mx) * dx + (a.my - b.my) * dy) / len;
                        break;
                    }
                }
            }
        }

        // ---- Distance to the NEAREST boundary (any), via a two-pass CHAMFER (3,4)
        // transform ≈ Euclidean. This is a continuous field (smooth falloff), used only
        // for HOW FAR a column is from a plate edge — NOT for which boundary, so it has no
        // discontinuity. Distance is in thirds of a column. ----
        const INF: i32 = i32::MAX / 4;
        let (w, h) = (COLS as i32, ROWS as i32);
        let mut dist = vec![INF; n];
        for i in 0..n {
            if boundary[i] {
                dist[i] = 0;
            }
        }
        let relax = |dist: &mut [i32], x: i32, y: i32, mask: &[(i32, i32, i32)]| {
            let i = (y * w + x) as usize;
            let mut bd = dist[i];
            for &(dx, dy, cost) in mask {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= w || ny >= h {
                    continue;
                }
                bd = bd.min(dist[(ny * w + nx) as usize] + cost);
            }
            dist[i] = bd;
        };
        let fwd = [(-1, 0, 3), (-1, -1, 4), (0, -1, 3), (1, -1, 4)];
        let bwd = [(1, 0, 3), (1, 1, 4), (0, 1, 3), (-1, 1, 4)];
        for y in 0..h {
            for x in 0..w {
                relax(&mut dist, x, y, &fwd);
            }
        }
        for y in (0..h).rev() {
            for x in (0..w).rev() {
                relax(&mut dist, x, y, &bwd);
            }
        }

        // ---- Smooth convergence field. The OLD bug: taking the convergence of the single
        // NEAREST boundary makes that value flip discontinuously across the medial axis
        // between two boundaries (uplift 0.83 → 0 in one column ⇒ a full-relief knife
        // cliff). Instead, average the convergence of ALL nearby boundaries, distance-
        // weighted, by blurring the boundary convergence and the boundary mask over the
        // belt reach and dividing — a continuous field, so the macro elevation is smooth.
        let mut conv_src = vec![0f32; n];
        let mut mask_src = vec![0f32; n];
        for i in 0..n {
            if boundary[i] {
                conv_src[i] = conv[i];
                mask_src[i] = 1.0;
            }
        }
        let r = UPLIFT_REACH as i32;
        let conv_blur = box_blur(&conv_src, r);
        let mask_blur = box_blur(&mask_src, r);
        let smooth_conv: Vec<f32> =
            (0..n).map(|i| conv_blur[i] / mask_blur[i].max(1e-4)).collect();

        // ---- Plate BASE field (continental shelf vs ocean basin) + interior variation.
        // This is the part with the hard plate-to-plate STEP, so it's blurred into a
        // graded continental shelf — turning knife cliffs at the coast into slopes —
        // BEFORE the (sharp) orogenic uplift is added, so mountain belts stay crisp.
        let mut base_field = vec![0f32; n];
        for y in 0..ROWS {
            for x in 0..COLS {
                let i = y * COLS + x;
                let var = crate::terrain::fbm(
                    seed,
                    x as f32 / MACRO_LATTICE,
                    y as f32 / MACRO_LATTICE,
                    701,
                    MACRO_OCTAVES,
                ) - 0.5;
                base_field[i] = plates[plate_id[i] as usize].base + var * MACRO_VAR;
            }
        }
        let base_field = box_blur(&base_field, SHELF_RADIUS);

        // ---- Compose macro elevation + mountainness + uplift-rate ----
        let mut macro_elev = vec![0f32; n];
        let mut mountainness = vec![0f32; n];
        let mut uplift_field = vec![0f32; n];
        for i in 0..n {
            // Chamfer distance is in thirds of a column → /3 for columns. SMOOTHSTEP the
            // falloff so the belt rises and FOOTS as a graded slope (no step at REACH).
            // Both inputs are now continuous — the distance falloff AND the smoothed
            // convergence — so the product, hence the macro elevation, is continuous.
            let d = dist[i] as f32 / 3.0;
            let t = (1.0 - d / UPLIFT_REACH).clamp(0.0, 1.0);
            let fall = t * t * (3.0 - 2.0 * t);
            let c = smooth_conv[i];
            let uplift = c.max(0.0) * fall;
            let rift = (-c).max(0.0) * fall;
            macro_elev[i] =
                (base_field[i] + uplift * UPLIFT_AMP - rift * RIFT_AMP).clamp(0.0, 1.0);
            mountainness[i] = uplift.clamp(0.0, 1.0);
            uplift_field[i] = uplift.clamp(0.0, 1.0);
        }

        TectonicField { macro_elev, mountainness, uplift: uplift_field }
    }
}

/// Separable box blur (prefix-sum, O(n)) with clamped edges. Used to grade the plate
/// base step into a shelf; one-time, so the per-row/col scratch allocation is fine.
fn box_blur(src: &[f32], radius: i32) -> Vec<f32> {
    if radius <= 0 {
        return src.to_vec();
    }
    let (w, h) = (COLS as i32, ROWS as i32);
    let mut horiz = vec![0f32; src.len()];
    let mut pref = vec![0f32; (w.max(h) + 1) as usize];
    for y in 0..h {
        let base = (y * w) as usize;
        for x in 0..w {
            pref[(x + 1) as usize] = pref[x as usize] + src[base + x as usize];
        }
        for x in 0..w {
            let lo = (x - radius).max(0);
            let hi = (x + radius + 1).min(w);
            horiz[base + x as usize] = (pref[hi as usize] - pref[lo as usize]) / (hi - lo) as f32;
        }
    }
    let mut out = vec![0f32; src.len()];
    for x in 0..w {
        for y in 0..h {
            pref[(y + 1) as usize] = pref[y as usize] + horiz[(y * w + x) as usize];
        }
        for y in 0..h {
            let lo = (y - radius).max(0);
            let hi = (y + radius + 1).min(h);
            out[(y * w + x) as usize] = (pref[hi as usize] - pref[lo as usize]) / (hi - lo) as f32;
        }
    }
    out
}
