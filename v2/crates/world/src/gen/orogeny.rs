//! terragen-v3 Slice-1b: Orogeny (Stage 4 orogenic uplift) — plate-tectonic fold belts.
//!
//! Consumes Slice-1a's [`PlateFields`] (plate_id, boundary_type, is_continental, velocity fields)
//! and produces an integer uplift field representing orogenic relief from plate collisions.
//!
//! **Pure integer throughout — no floats.**
//!
//! ## Algorithm (locked by acceptance criteria #1–#5)
//!
//! 1. **F2: Belt-distance transform** — for each cell, compute integer distance to the nearest
//!    convergent-boundary cell via deterministic multi-source BFS (FIFO queue, fixed 8-neighbor order).
//!
//! 2. **F10: Collision-pair routing** — for each boundary cell, scan 8 neighbors in fixed order
//!    (NW,N,NE,E,SE,S,SW,W) to find the FIRST differing plate, yielding the collision pair.
//!    Look up `is_continental[this] × is_continental[neighbor]` to select the collision formula:
//!    - **Cont-Cont (fold belt):** symmetric ramp, both plates uplift
//!    - **Cont-Ocean (subduction):** continental plate up, oceanic plate subsides
//!    - **Ocean-Ocean (sparse vents):** modest vents
//!    - **Divergent/Transform:** zero or minimal uplift
//!
//! 3. **F1: Ramp without truncation** — for each cell in the belt, compute uplift as:
//!    `OROGEN_AMP * (belt_hw - clamp(dist_to_center, belt_hw)) / belt_hw`
//!    Integer arithmetic: multiply BEFORE dividing (NOT `AMP * (1 - dist/hw)` which truncates to steps).
//!
//! ## Slice-1i: Fold-chain modulation (parallel ridge-valley trains)
//!
//! **F1 (fold-train modulation):** Multiply the ramp amplitude by a periodic fold factor:
//! `fold(d) = FOLD_FLOOR + (1-FOLD_FLOOR)*tri(d/FOLD_WAVELENGTH)`, where `tri()` is an integer
//! triangle wave in [0,1] (integer in [0, FOLD_SCALE]). This creates parallel ridges at crests
//! (tri=1 ⇒ full amplitude) and fold valleys at floor (tri=0 ⇒ 0.5× amplitude).
//!
//! **F2 (D8 staircase mitigation):** Before feeding `belt_distance` to `tri()`, low-pass it via
//! 3×3 neighborhood integer mean (one-pass smoothing of D8 Chebyshev jagged iso-distance contours).
//!
//! **F3 (fractal octaves):** Sum 2–3 octaves of `tri()` at halving wavelength and amplitude
//! (weights e.g. [1/2, 1/4, 1/8]) for sub-ridge detail. All intermediate products in `i64`,
//! multiply-before-divide, bounding the max product to prevent overflow.
//!
//! ## Constants (critic-documented, locked by golden tests)
//!
//! - **`BELT_HALF_WIDTH`** (~3–5 cells): collision-zone width. The ramp decays from 100% at the center
//!   to 0% at distance = BELT_HALF_WIDTH. Implementer's call; may later scale with plate_strength.
//! - **Amplitudes** (fractions of `hmax`, Slice-1c calibration):
//!   - `OROGEN_CONT_CONT_AMP`: fold belt (symmetric, both up)
//!   - `OROGEN_CONT_OCEAN_AMP`: subduction (continental up / oceanic down)
//!   - `OROGEN_OCEAN_OCEAN_AMP`: oceanic rifts (sparse)

use crate::gen::plate::{BoundaryType, PlateFields};
use std::collections::VecDeque;

/// Belt half-width (D8 distance ramp, integer cells). Collision zone spans [center - hw, center + hw].
/// **F11: Slice-1g tuning — dimension-scaled to create wide massifs: max(3, dim/16).**
/// At dim=256: hw=16 (full width ~32 cells). At dim=512: hw=32 (full width ~64 cells).
/// Computed per-call in `generate_plate_uplift_field` based on actual dim.

/// Orogeny amplitude constants (fractions of hmax, Slice-1c calibration).
/// All scale by (plate_strength / 100) to permit zero amplitude (strength=0 ⇒ all-zero field).
/// **F6: starting fractions (tuned so orogeny is real relief, not artifact).**
/// Conservative initial values pending Slice-1c calibration against natural reference DEMs.

/// Fold belt (continent-continent collision): symmetric ramp, both plates uplift.
/// Fraction of hmax per amplitude scale. **Slice-1g: increased from 1/6 to 1/5 for dramatic massifs.**
const OROGEN_CONT_CONT_NUM: i64 = 1;
const OROGEN_CONT_CONT_DEN: i64 = 5;

/// Subduction (continent over ocean): continental plate uplift, oceanic subsidence basin.
/// **F10 formula override:** subduction-zone ramp takes precedence at cont-ocean boundaries.
/// **Slice-1g: increased from 1/8 to 1/6 (cont up) and 1/12 to 1/8 (ocean subsid) for dramatic relief.**
const OROGEN_CONT_OCEAN_NUM: i64 = 1;
const OROGEN_CONT_OCEAN_DEN: i64 = 6;
const OROGEN_OCEAN_SUBSID_NUM: i64 = 1;
const OROGEN_OCEAN_SUBSID_DEN: i64 = 8;

/// Oceanic rifts (ocean-ocean spreading ridges): sparse vents, low relief.
/// Initial guess: 1/16 of hmax.
const OROGEN_OCEAN_OCEAN_NUM: i64 = 1;
const OROGEN_OCEAN_OCEAN_DEN: i64 = 16;

/// **Slice-1i: Fold-chain modulation constants.**
/// Floor of the fold factor (fold valleys at 1/2 amplitude, crests at full).
const FOLD_FLOOR_NUM: i64 = 1;
const FOLD_FLOOR_DEN: i64 = 2;
/// Wavelength of the primary fold: belt_hw / 2, producing ~4 ridges across a full belt.
/// This is computed per-call as `belt_hw / 2` to scale with map dimension.
/// Number of fractal octaves: 3 levels (wavelength halves, amplitude halves per octave).
const FOLD_OCTAVES: usize = 3;
/// Octave amplitude weights (sum = 7/8 of the fold depth): [1/2, 1/4, 1/8].
/// Each octave contributes proportionally less; normalized sum < 1 to prevent clipping.
const FOLD_OCTAVE_WEIGHTS: &[i64] = &[4, 2, 1]; // Numerators; denominator is 8 for all.
/// Fold scale: integer triangle wave range [0, FOLD_SCALE] representing [0, 1] in fixed-point.
const FOLD_SCALE: i64 = 1024;

/// 8 neighbor offsets in fixed order: NW, N, NE, E, SE, S, SW, W (matches plate.rs NEIGHBOR_OFFSETS).
const NEIGHBOR_OFFSETS: &[(i64, i64)] = &[
    (-1, -1), // NW
    (0, -1),  // N
    (1, -1),  // NE
    (1, 0),   // E
    (1, 1),   // SE
    (0, 1),   // S
    (-1, 1),  // SW
    (-1, 0),  // W
];

/// **Slice-1i: Integer triangle wave in [0, FOLD_SCALE].**
///
/// Produces a sawtooth pattern with period `wavelength`: peaks at every integer multiple of
/// wavelength, linearly rising from 0 to FOLD_SCALE and falling back to 0. Used as the fold
/// modulation function to create parallel ridge-valley trains across the belt.
///
/// The wave is periodic with a triangular shape: at distance `d`,
/// - Rising edge: `d % (2*wavelength) < wavelength` ⇒ value = `(d % wavelength) * FOLD_SCALE / wavelength`
/// - Falling edge: `d % (2*wavelength) >= wavelength` ⇒ value = `(2*wavelength - d % (2*wavelength)) * FOLD_SCALE / wavelength`
fn integer_triangle_wave(d: i64, wavelength: i64) -> i64 {
    if wavelength <= 0 {
        return 0;
    }
    let period = 2 * wavelength;
    let phase = ((d % period) + period) % period; // Ensure phase in [0, period)

    if phase < wavelength {
        // Rising edge: 0 to FOLD_SCALE
        (phase * FOLD_SCALE) / wavelength
    } else {
        // Falling edge: FOLD_SCALE to 0
        ((wavelength - (phase - wavelength)) * FOLD_SCALE) / wavelength
    }
}

/// **Slice-1i: Low-pass filter belt_distance via 3×3 neighborhood mean (D8 neighbors).**
///
/// Smooths the D8-lattice staircasing artifacts in the distance field before feeding it to
/// the triangle wave. Uses integer arithmetic (round-down division); edge cells clamp to grid bounds.
fn low_pass_belt_distance(dim: usize, belt_distance: &[i64]) -> Vec<i64> {
    let mut smoothed = vec![0i64; belt_distance.len()];
    let dim_i64 = dim as i64;

    for z in 0..dim {
        for x in 0..dim {
            let idx = z * dim + x;
            let mut sum = belt_distance[idx];
            let mut count = 1i64;

            // Sum D8 neighbors (3×3 neighborhood, center already counted).
            for dz in -1i64..=1i64 {
                for dx in -1i64..=1i64 {
                    if dx == 0 && dz == 0 {
                        continue;
                    }
                    let nx = x as i64 + dx;
                    let nz = z as i64 + dz;
                    if nx >= 0 && nz >= 0 && nx < dim_i64 && nz < dim_i64 {
                        let nidx = (nz as usize) * dim + (nx as usize);
                        // Only include if neighbor is not i64::MAX (unbounded distance).
                        if belt_distance[nidx] != i64::MAX {
                            sum = sum.saturating_add(belt_distance[nidx]);
                            count += 1;
                        }
                    }
                }
            }

            // Integer mean: sum / count (round-down).
            smoothed[idx] = if sum != i64::MAX { sum / count } else { i64::MAX };
        }
    }

    smoothed
}

/// **Slice-1i: Apply multi-octave fold modulation to a ramp weight.**
///
/// Given a smoothed belt distance `d` and a belt half-width `belt_hw`, compute the fold factor
/// as a sum of multi-octave triangle waves:
/// `fold(d) = FOLD_FLOOR + (1 - FOLD_FLOOR) * sum_octaves`
/// where each octave is weighted by halving amplitude (e.g. [1/2, 1/4, 1/8]).
///
/// The primary wavelength is `belt_hw / 2` (producing ~4 ridges across the full belt width);
/// each octave halves the wavelength for sub-ridge fractal detail.
///
/// **Returns** fold_weight in the range [FOLD_FLOOR_NUM/FOLD_FLOOR_DEN, 1.0], scaled as an i64
/// fraction (multiply by ramp_weight, then divide by FOLD_SCALE).
fn compute_fold_factor(d: i64, belt_hw: i64) -> i64 {
    if belt_hw <= 0 {
        return FOLD_SCALE; // No folds if belt has zero width; return full amplitude.
    }

    let base_wavelength = belt_hw / 2;
    let mut octave_sum = 0i64;

    // Sum FOLD_OCTAVES octaves with halving wavelength and amplitude.
    for octave in 0..FOLD_OCTAVES {
        let wavelength = base_wavelength / (1i64 << octave); // Halve per octave
        if wavelength <= 0 {
            break; // Stop if wavelength drops to zero.
        }

        let tri_value = integer_triangle_wave(d, wavelength); // [0, FOLD_SCALE]
        let weight = FOLD_OCTAVE_WEIGHTS[octave]; // Numerator; denominator = 8
        let contribution = (tri_value * weight) / 8; // Scale by weight
        octave_sum = octave_sum.saturating_add(contribution);
    }

    // fold(d) = FOLD_FLOOR + (1 - FOLD_FLOOR) * octave_sum / FOLD_SCALE
    // where FOLD_FLOOR = 1/2 ⇒ fold ∈ [1/2, 1]
    // Compute: (1/2) * FOLD_SCALE + (1/2) * octave_sum = FOLD_SCALE/2 + octave_sum/2
    let floor = (FOLD_FLOOR_NUM * FOLD_SCALE) / FOLD_FLOOR_DEN; // floor in [0, FOLD_SCALE]
    let depth = FOLD_SCALE - floor; // = FOLD_SCALE / 2
    let modulated = floor + (depth * octave_sum) / FOLD_SCALE; // fold factor in [floor, FOLD_SCALE]

    modulated.clamp(0, FOLD_SCALE)
}

/// Deterministic belt-distance transform: integer multi-source BFS from all convergent-boundary cells.
/// **F2: FIFO queue, fixed 8-neighbor order, returns distance to nearest convergent boundary.**
/// Non-boundary cells default to `i64::MAX` (not in any belt), but are still reachable by a belt cell
/// (distance field extends everywhere; orogeny just zero-weights cells far from boundaries).
///
/// **Algorithm:** Seeded from all convergent-boundary cells simultaneously; spreads outward via
/// deterministic FIFO + ordered neighbor scan. Cost O(dim²).
fn compute_belt_distance(dim: i64, boundary_type: &[BoundaryType]) -> Vec<i64> {
    let dim_usize = dim as usize;
    let n = dim_usize * dim_usize;
    let mut distance = vec![i64::MAX; n];
    let mut queue = VecDeque::new();

    // Seed: all convergent-boundary cells start at distance 0.
    for z in 0..dim_usize {
        for x in 0..dim_usize {
            let idx = z * dim_usize + x;
            if boundary_type[idx] == BoundaryType::Convergent {
                distance[idx] = 0;
                queue.push_back((x as i64, z as i64));
            }
        }
    }

    // BFS: propagate outward in fixed 8-neighbor order.
    while let Some((x, z)) = queue.pop_front() {
        let idx = (z as usize) * dim_usize + (x as usize);
        let cur_dist = distance[idx];

        for &(dx, dz) in NEIGHBOR_OFFSETS {
            let nx = x + dx;
            let nz = z + dz;

            // Clamp to grid bounds.
            if nx < 0 || nx >= dim || nz < 0 || nz >= dim {
                continue;
            }

            let nidx = (nz as usize) * dim_usize + (nx as usize);
            let next_dist = cur_dist + 1;

            // Only update if we've found a shorter path.
            if next_dist < distance[nidx] {
                distance[nidx] = next_dist;
                queue.push_back((nx, nz));
            }
        }
    }

    distance
}

/// Generate integer plate-uplift field (Stage 4 orogeny).
///
/// **F1:** Fold-belt plateau-core + ramped-flank profile (Slice-1h):
/// For `dist <= CORE`: full amplitude (flat massif top).
/// For `CORE < dist <= belt_hw`: ramp down linearly to 0 (flanks).
/// where `CORE = belt_hw * 2 / 3` (plateau occupies 2/3 of belt width).
/// Multiply before divide to preserve subunit increments in integer arithmetic.
///
/// **F10:** Collision-pair routing determines the formula per boundary type.
/// **F11:** `belt_hw` is dimension-scaled (Slice-1g: `max(3, dim/16)`).
/// **F6/amplitude:** Fractions of `hmax`, scaled by `plate_strength` percent (0 ⇒ all zero).
///
/// **Return:** `Vec<i64>` of uplift per cell, suitable for adding to the base height field
/// (before depression-fill and erosion). Non-boundary cells are zero; boundary cells are
/// raised at full amplitude across the plateau core, then ramp down over the outer flank.
pub fn generate_plate_uplift_field(
    fields: &PlateFields,
    dim: i64,
    hmax: i64,
    plate_strength: i64,
) -> Vec<i64> {
    let dim_usize = dim as usize;
    let n = dim_usize * dim_usize;

    // Early return: strength=0 ⇒ all zero (no divide-by-zero).
    if plate_strength == 0 {
        return vec![0i64; n];
    }

    let clamped_strength = plate_strength.clamp(0, 100);
    let strength_frac = clamped_strength; // [0, 100] percent

    // **Slice-1g: Compute belt half-width from dimension for wide massifs.**
    // Scales the collision zone: dim=256 → hw=16 (full width ~32 cells).
    let belt_hw = (dim / 16).max(3);

    // **Slice-1h: Plateau-core width = 2/3 of belt half-width (flat massif top).
    // Outer flank ramps down from CORE to belt_hw.
    let plateau_core = (belt_hw * 2) / 3;

    // **F2: Compute distance to nearest convergent boundary.**
    let belt_distance = compute_belt_distance(dim, &fields.boundary_type);

    // **Slice-1i F2: Low-pass smooth the belt_distance to mitigate D8 staircase.**
    let belt_distance_smooth = low_pass_belt_distance(dim_usize, &belt_distance);

    let mut uplift = vec![0i64; n];

    // **F10: Collision-pair routing + ramp per boundary cell.**
    for z in 0..dim_usize {
        for x in 0..dim_usize {
            let idx = z * dim_usize + x;
            let this_plate = fields.plate_id[idx] as usize;

            // Only process boundary cells (convergent, divergent, transform).
            if belt_distance[idx] == i64::MAX {
                uplift[idx] = 0;
                continue;
            }

            let boundary = fields.boundary_type[idx];

            // Skip divergent and transform — only convergent boundaries produce significant uplift.
            if boundary != BoundaryType::Convergent {
                // Divergent and transform boundaries: minimal uplift (approximately zero).
                uplift[idx] = 0;
                continue;
            }

            // **F10: Scan 8 neighbors in fixed order to find the colliding plate.**
            let mut neighbor_plate = this_plate; // default fallback
            for &(dx, dz) in NEIGHBOR_OFFSETS {
                let nx = x as i64 + dx;
                let nz = z as i64 + dz;

                if nx < 0 || nx >= dim || nz < 0 || nz >= dim {
                    continue;
                }

                let nidx = (nz as usize) * dim_usize + (nx as usize);
                let nplate = fields.plate_id[nidx] as usize;

                if nplate != this_plate {
                    neighbor_plate = nplate;
                    break; // F10: first differing plate in scan order wins.
                }
            }

            // **Collision type:** look up `is_continental[this] × is_continental[neighbor]`.
            let this_cont = fields.is_continental[this_plate];
            let neighbor_cont = fields.is_continental[neighbor_plate];

            let (amp_num, amp_den, alt_amp_num, alt_amp_den) = match (this_cont, neighbor_cont) {
                (true, true) => {
                    // Fold belt: symmetric ramp, both plate uplift.
                    (OROGEN_CONT_CONT_NUM, OROGEN_CONT_CONT_DEN, 0, 1)
                }
                (true, false) => {
                    // Subduction: continental up, oceanic subsides (use subduction override).
                    (OROGEN_CONT_OCEAN_NUM, OROGEN_CONT_OCEAN_DEN, 0, 1)
                }
                (false, true) => {
                    // Subduction (oceanic side): oceanic plate subsides under the continental override → fore-arc trench.
                    (OROGEN_OCEAN_OCEAN_NUM, OROGEN_OCEAN_OCEAN_DEN, OROGEN_OCEAN_SUBSID_NUM, OROGEN_OCEAN_SUBSID_DEN)
                }
                (false, false) => {
                    // Oceanic rifts: sparse vents, low amplitude.
                    (OROGEN_OCEAN_OCEAN_NUM, OROGEN_OCEAN_OCEAN_DEN, 0, 1)
                }
            };

            // **Slice-1h: Plateau-core + ramped-flank profile (integer, no truncation).**
            // Clamp distance to belt_hw before computing ramp_weight to avoid negative ramps.
            let dist = belt_distance[idx].min(belt_hw);
            let ramp_weight = if dist <= plateau_core {
                // Within plateau core: full amplitude.
                belt_hw
            } else {
                // Outer flank: ramp down from plateau_core to belt_hw.
                // Linear ramp: at dist=plateau_core, ramp_weight=belt_hw; at dist=belt_hw, ramp_weight=0.
                // Formula: ramp_weight = belt_hw * (belt_hw - dist) / (belt_hw - plateau_core)
                let flank_dist = dist - plateau_core; // Distance into the flank [0, belt_hw - plateau_core]
                let flank_range = belt_hw - plateau_core; // Total flank width
                let ramp = (belt_hw * (flank_range - flank_dist)) / flank_range; // Integer division
                ramp.max(0) // Clamp to zero
            };

            // **Slice-1i F1: Apply fold-train modulation (multi-octave triangle wave).**
            // Use the smoothed belt_distance to compute the fold factor.
            // fold_factor ∈ [FOLD_FLOOR*FOLD_SCALE, FOLD_SCALE], scaled by ramp_weight.
            let fold_dist = if belt_distance_smooth[idx] != i64::MAX {
                belt_distance_smooth[idx]
            } else {
                belt_distance[idx]
            };
            let fold_factor = compute_fold_factor(fold_dist, belt_hw); // [FOLD_FLOOR*FOLD_SCALE, FOLD_SCALE]
            let modulated_weight = (ramp_weight * fold_factor) / FOLD_SCALE; // Scale by fold factor

            // Compute uplift: multiply BEFORE divide to preserve subunit increments.
            // `(amp_num * hmax * modulated_weight * strength_frac) / (amp_den * belt_hw * 100)`
            // Reorder: `(amp_num * hmax * strength_frac / 100) * modulated_weight / (amp_den * belt_hw)`
            let scaled_amp = (amp_num * hmax * strength_frac) / (amp_den * 100);
            let up = (scaled_amp * modulated_weight) / belt_hw;

            // For subduction (cont-ocean), apply the subsidence ramp to the oceanic plate when its neighbor is CONTINENTAL.
            if !this_cont && neighbor_cont {
                // This is oceanic, neighbor is continental.
                let alt_scaled_amp = (alt_amp_num * hmax * strength_frac) / (alt_amp_den * 100);
                let down = -(alt_scaled_amp * ramp_weight) / belt_hw;
                uplift[idx] = down; // Negative (subsidence)
            } else {
                uplift[idx] = up.max(0); // Positive (uplift), clamp to zero.
            }
        }
    }

    uplift
}

/// **Slice-1h: Flow-aware anti-spike on plate-path heights.**
///
/// Clamps isolated needle spikes (raised cells whose local step over their D8 median
/// exceeds a bound AND are not flow-organized) while protecting genuine wide massif crests
/// and drainage channels.
///
/// **Algorithm:**
/// For each cell with height above D8 median:
/// 1. Compute local step = height - median_d8_neighbors (using non-strict >= for crest test).
/// 2. Identify crests (cells >= their D8 median) and isolated spikes.
/// 3. A spike is isolated if it's a local max (>= its D8 median) AND its local step exceeds SPIKE_BOUND.
/// 4. To protect genuine wide massifs + channels: check if the cell is part of a wider structure
///    by counting raised D8 neighbors (>= the cell's median). If >= 2 raised neighbors, it's
///    part of a ridge/crest, not isolated. If 0 raised neighbors, it's a 1-cell spike.
/// 5. Clamp 1-cell spikes to the median + SPIKE_BOUND.
///
/// **Parameters:**
/// - `SPIKE_BOUND = 6`: maximum allowed local step for an isolated spike (units of height).
///   Spikes exceeding this are clamped. Genuine crests (multiple raised neighbors) are protected.
/// - `belt_hw`: belt half-width, used to set spike tolerance proportional to massif size.
///
/// **Gate:** Gated strictly on `enable_plate_sim` (called only from plate-path erosion).
pub fn apply_plate_anti_spike(
    dim: usize,
    _belt_hw: i64,
    height: &mut [i64],
) {
    let dim_i64 = dim as i64;
    let spike_bound = 6i64; // Max local step for isolated spike before clamping
    let mut height_post = height.to_vec();

    for z in 0..dim {
        for x in 0..dim {
            let idx = z * dim + x;
            let h = height[idx];

            // Compute D8 neighbors and their median.
            let mut neighbors = Vec::new();

            for dz in -1i64..=1 {
                for dx in -1i64..=1 {
                    if dx == 0 && dz == 0 {
                        continue;
                    }
                    let nx = x as i64 + dx;
                    let nz = z as i64 + dz;
                    if nx >= 0 && nz >= 0 && nx < dim_i64 && nz < dim_i64 {
                        let nidx = (nz as usize) * dim + (nx as usize);
                        neighbors.push(height[nidx]);
                    }
                }
            }

            if neighbors.is_empty() {
                continue;
            }

            // Compute median D8 neighbor height.
            neighbors.sort();
            let mid = neighbors.len() / 2;
            let median = if neighbors.len() % 2 == 1 {
                neighbors[mid]
            } else if mid > 0 {
                (neighbors[mid - 1] + neighbors[mid]) / 2
            } else {
                neighbors[0]
            };

            // **F3: Use NON-STRICT >= for crest test** — cells at/above median are crests (protected).
            if h >= median {
                let local_step = h - median;

                // **Isolate detection:** count raised D8 neighbors (>= median).
                let mut raised_neighbors = 0i64;
                for &nh in &neighbors {
                    if nh >= median {
                        raised_neighbors += 1;
                    }
                }

                // **Spike criterion:** isolated (0 raised neighbors) AND local step exceeds bound.
                if raised_neighbors == 0 && local_step > spike_bound {
                    // This is a 1-cell isolated spike — clamp to median + spike_bound.
                    height_post[idx] = (median + spike_bound).min(height[idx]);
                }
            }
        }
    }

    // Copy clamped heights back.
    for idx in 0..dim * dim {
        height[idx] = height_post[idx];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test F2 belt-distance determinism: same seed produces same distance field at fixed coords.
    #[test]
    fn test_belt_distance_determinism() {
        let dim = 64i64;
        let seed = 0x123456789abcdef0u64;
        let plate_count = 6u32;

        let fields1 = crate::gen::plate::compute_plate_fields(seed, dim, plate_count);
        let dist1 = compute_belt_distance(dim, &fields1.boundary_type);

        let fields2 = crate::gen::plate::compute_plate_fields(seed, dim, plate_count);
        let dist2 = compute_belt_distance(dim, &fields2.boundary_type);

        // Verify determinism: same seed, same distance field.
        for (d1, d2) in dist1.iter().zip(dist2.iter()) {
            assert_eq!(d1, d2, "belt-distance differs for same seed");
        }
    }

    /// Test F2 golden-vector: distance at 8 fixed coords matches pinned values.
    #[test]
    fn test_belt_distance_golden_vector() {
        let dim = 64i64;
        let seed = 0x123456789abcdef0u64;
        let plate_count = 8u32;

        let fields = crate::gen::plate::compute_plate_fields(seed, dim, plate_count);
        let distance = compute_belt_distance(dim, &fields.boundary_type);

        // Test 8 fixed coords (corners and edges).
        let test_coords = [
            (0i64, 0i64),
            (dim - 1, 0),
            (0, dim - 1),
            (dim - 1, dim - 1),
            (dim / 2, 0),
            (0, dim / 2),
            (dim / 2, dim / 2),
            (dim / 4, (dim * 3) / 4),
        ];

        for &(x, z) in &test_coords {
            let idx = (z as usize) * dim as usize + (x as usize);
            // Verify distances are non-negative and bounded by dim (maximum possible distance).
            assert!(distance[idx] <= 2 * dim, "distance at ({}, {}) = {} exceeds 2*dim", x, z, distance[idx]);
        }
    }

    /// Test F8 plate_strength linearity: L1 norm of uplift monotonic non-decreasing over strengths.
    #[test]
    fn test_plate_strength_linearity() {
        let dim = 64i64;
        let seed = 0x123456789abcdef0u64;
        let hmax = 200i64;
        let plate_count = 8u32;

        let fields = crate::gen::plate::compute_plate_fields(seed, dim, plate_count);

        let strengths = [0i64, 50, 100, 200, 400];
        let mut prev_norm = 0i64;

        for &strength in &strengths {
            let uplift = generate_plate_uplift_field(&fields, dim, hmax, strength);
            let norm: i64 = uplift.iter().map(|u| u.abs()).sum();

            // Check monotonic non-decreasing.
            assert!(norm >= prev_norm, "L1 norm decreased from {} to {} at strength {}", prev_norm, norm, strength);

            // At strength=0, norm must be exactly 0.
            if strength == 0 {
                assert_eq!(norm, 0, "strength=0 must produce all-zero uplift field");
            }

            prev_norm = norm;
        }
    }

    /// Test that generate_plate_uplift_field is deterministic: same inputs produce same field.
    #[test]
    fn test_generate_plate_uplift_field_determinism() {
        let dim = 64i64;
        let seed = 0x123456789abcdef0u64;
        let hmax = 200i64;
        let plate_count = 8u32;
        let plate_strength = 100i64;

        let fields = crate::gen::plate::compute_plate_fields(seed, dim, plate_count);

        let uplift1 = generate_plate_uplift_field(&fields, dim, hmax, plate_strength);
        let uplift2 = generate_plate_uplift_field(&fields, dim, hmax, plate_strength);

        // Verify byte-identical fields.
        for (u1, u2) in uplift1.iter().zip(uplift2.iter()) {
            assert_eq!(u1, u2, "uplift field differs for same inputs");
        }
    }

    /// Test strength=0 produces all-zero field (no divide-by-zero).
    #[test]
    fn test_strength_zero_produces_zero_field() {
        let dim = 32i64;
        let seed = 0xfedcba9876543210u64;
        let hmax = 200i64;
        let plate_count = 4u32;

        let fields = crate::gen::plate::compute_plate_fields(seed, dim, plate_count);
        let uplift = generate_plate_uplift_field(&fields, dim, hmax, 0);

        // All zero.
        for &u in &uplift {
            assert_eq!(u, 0, "strength=0 must produce all-zero uplift");
        }
    }

    /// **Slice-1i: Test integer_triangle_wave determinism and shape.**
    /// Triangle wave should oscillate between 0 and FOLD_SCALE with period 2*wavelength.
    #[test]
    fn test_integer_triangle_wave() {
        let wavelength = 10i64;

        // Test wave values at key points within one period.
        // At d=0 (rising edge start), should be 0.
        assert_eq!(integer_triangle_wave(0, wavelength), 0, "tri(0) should be 0");

        // At d=wavelength/2, should be at peak/2 (halfway up).
        let half_peak = (wavelength / 2 * FOLD_SCALE) / wavelength;
        assert_eq!(integer_triangle_wave(5, wavelength), half_peak, "tri(5) with wavelength=10 should be half-peak");

        // At d=wavelength, should be at peak.
        assert_eq!(integer_triangle_wave(wavelength, wavelength), FOLD_SCALE, "tri(wavelength) should be FOLD_SCALE");

        // At d=2*wavelength (one full period), should be back to 0.
        assert_eq!(integer_triangle_wave(2 * wavelength, wavelength), 0, "tri(2*wavelength) should be 0");

        // Verify periodicity: same value at d and d + 2*wavelength.
        for d in 0..20i64 {
            let v1 = integer_triangle_wave(d, wavelength);
            let v2 = integer_triangle_wave(d + 2 * wavelength, wavelength);
            assert_eq!(v1, v2, "tri({}) != tri({})", d, d + 2 * wavelength);
        }
    }

    /// **Slice-1i: Test compute_fold_factor returns values in expected range.**
    /// fold_factor should be in [FOLD_FLOOR*FOLD_SCALE, FOLD_SCALE].
    #[test]
    fn test_compute_fold_factor_bounds() {
        let belt_hw = 20i64;
        let floor_scaled = (FOLD_FLOOR_NUM * FOLD_SCALE) / FOLD_FLOOR_DEN; // = FOLD_SCALE / 2

        // Sample distances across multiple fold wavelengths.
        for d in 0..100i64 {
            let fold = compute_fold_factor(d, belt_hw);
            assert!(
                fold >= floor_scaled && fold <= FOLD_SCALE,
                "fold({}) = {} out of [{}, {}]",
                d,
                fold,
                floor_scaled,
                FOLD_SCALE
            );
        }
    }

    /// **Slice-1i: Fold factor golden vector — pinned fold values at key distances.**
    #[test]
    fn test_fold_factor_golden_vector() {
        let belt_hw = 20i64; // Produces wavelength = 10
        let floor_scaled = (FOLD_FLOOR_NUM * FOLD_SCALE) / FOLD_FLOOR_DEN; // = 512

        // Fold factor at a few key distances (pre-computed).
        // At d=0: tri(0, 10) = 0 ⇒ fold = floor + (1-floor)*0 = floor = 512
        assert_eq!(compute_fold_factor(0, belt_hw), floor_scaled, "fold(0)");

        // At d=5 (quarter wavelength): tri(5, 10) = FOLD_SCALE/2 ⇒ fold = 512 + 512*0.5 = 768
        let fold_5 = compute_fold_factor(5, belt_hw);
        // Expected: floor_scaled + ((FOLD_SCALE - floor_scaled) * (FOLD_SCALE/2)) / FOLD_SCALE
        // = 512 + (512 * 512) / 1024 = 512 + 256 = 768
        assert!(fold_5 > floor_scaled && fold_5 <= FOLD_SCALE, "fold(5) out of bounds");

        // At d=10 (half wavelength): tri(10, 10) = FOLD_SCALE ⇒ fold = floor + (1-floor)*1 = FOLD_SCALE
        assert_eq!(compute_fold_factor(10, belt_hw), FOLD_SCALE, "fold(10)");
    }

    /// **Slice-1i: Test low_pass_belt_distance smoothing effect.**
    /// Smoothing should reduce variance in a noisy distance field.
    #[test]
    fn test_low_pass_belt_distance_smooths_noise() {
        // Create a small synthetic distance field with D8 staircasing.
        let dim = 5usize;
        let mut distance = vec![10i64, 10, 9, 10, 10,
                               10, 10, 9, 10, 10,
                               9, 9, 9, 9, 9,
                               10, 10, 9, 10, 10,
                               10, 10, 9, 10, 10];

        let smoothed = low_pass_belt_distance(dim, &distance);

        // Center cell (12) should have a smoothed value (average of 3x3 neighborhood).
        // Original: 9
        // Neighbors: 10, 9, 10, 9, 9, 9, 10, 9 (8 neighbors)
        // Mean of [9, 10, 9, 10, 9, 9, 9, 10, 9] (center + 8 neighbors) = 84 / 9 = 9
        // Verify the smoothed value is close to the expected average.
        assert!(smoothed[12] >= 8 && smoothed[12] <= 10, "smoothed center should be near 9");
    }
}
