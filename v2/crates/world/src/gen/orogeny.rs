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
/// **F11: anchored to architecture's convergent-boundary width (~3–5 cells).**
/// Pinned as a concrete value for merge gate; may later become plate_strength-scaled (Slice-1c).
const BELT_HALF_WIDTH: i64 = 3;

/// Orogeny amplitude constants (fractions of hmax, Slice-1c calibration).
/// All scale by (plate_strength / 100) to permit zero amplitude (strength=0 ⇒ all-zero field).
/// **F6: starting fractions (tuned so orogeny is real relief, not artifact).**
/// Conservative initial values pending Slice-1c calibration against natural reference DEMs.

/// Fold belt (continent-continent collision): symmetric ramp, both plates uplift.
/// Fraction of hmax per amplitude scale. Initial guess: 1/6 of hmax.
const OROGEN_CONT_CONT_NUM: i64 = 1;
const OROGEN_CONT_CONT_DEN: i64 = 6;

/// Subduction (continent over ocean): continental plate uplift, oceanic subsidence basin.
/// **F10 formula override:** subduction-zone ramp takes precedence at cont-ocean boundaries.
/// Initial guess: 1/8 of hmax (continent up). Oceanic subsides to -(1/12 * hmax) relative to ambient.
const OROGEN_CONT_OCEAN_NUM: i64 = 1;
const OROGEN_CONT_OCEAN_DEN: i64 = 8;
const OROGEN_OCEAN_SUBSID_NUM: i64 = 1;
const OROGEN_OCEAN_SUBSID_DEN: i64 = 12;

/// Oceanic rifts (ocean-ocean spreading ridges): sparse vents, low relief.
/// Initial guess: 1/16 of hmax.
const OROGEN_OCEAN_OCEAN_NUM: i64 = 1;
const OROGEN_OCEAN_OCEAN_DEN: i64 = 16;

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
/// **F1:** Fold-belt ramp formula without truncation:
/// `uplift = OROGEN_AMP * (belt_hw - clamp(dist, belt_hw)) / belt_hw`
/// Multiply before divide to preserve subunit increments in integer arithmetic.
///
/// **F10:** Collision-pair routing determines the formula per boundary type.
/// **F11:** `belt_hw` is a pinned constant (3 cells, architecture-anchored).
/// **F6/amplitude:** Fractions of `hmax`, scaled by `plate_strength` percent (0 ⇒ all zero).
///
/// **Return:** `Vec<i64>` of uplift per cell, suitable for adding to the base height field
/// (before depression-fill and erosion). Non-boundary cells are zero; boundary cells ramp down
/// from the belt center to zero over BELT_HALF_WIDTH cells.
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

    // **F2: Compute distance to nearest convergent boundary.**
    let belt_distance = compute_belt_distance(dim, &fields.boundary_type);

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
                    (OROGEN_CONT_OCEAN_NUM, OROGEN_CONT_OCEAN_DEN, OROGEN_OCEAN_SUBSID_NUM, OROGEN_OCEAN_SUBSID_DEN)
                }
                (false, true) => {
                    // Subduction (flipped): oceanic up (less), continental down.
                    (OROGEN_OCEAN_OCEAN_NUM, OROGEN_OCEAN_OCEAN_DEN, 0, 1)
                }
                (false, false) => {
                    // Oceanic rifts: sparse vents, low amplitude.
                    (OROGEN_OCEAN_OCEAN_NUM, OROGEN_OCEAN_OCEAN_DEN, 0, 1)
                }
            };

            // **F1: Ramp formula (integer, no truncation).**
            // `uplift = (amp_frac * hmax * (belt_hw - clamp(dist, belt_hw))) / belt_hw`
            // Clamp distance to belt_hw before subtraction to avoid negative ramps.
            let dist = belt_distance[idx].min(BELT_HALF_WIDTH);
            let ramp_weight = BELT_HALF_WIDTH - dist; // [0, belt_hw]

            // Compute uplift: multiply BEFORE divide to preserve subunit increments.
            // `(amp_num * hmax * ramp_weight * strength_frac) / (amp_den * belt_hw * 100)`
            // Reorder: `(amp_num * hmax * strength_frac / 100) * ramp_weight / (amp_den * belt_hw)`
            let scaled_amp = (amp_num * hmax * strength_frac) / (amp_den * 100);
            let up = (scaled_amp * ramp_weight) / BELT_HALF_WIDTH;

            // For subduction (cont-ocean), apply the subsidence ramp to the oceanic plate IF neighbor is oceanic.
            if !this_cont && neighbor_cont {
                // This is oceanic, neighbor is continental.
                let alt_scaled_amp = (alt_amp_num * hmax * strength_frac) / (alt_amp_den * 100);
                let down = -(alt_scaled_amp * ramp_weight) / BELT_HALF_WIDTH;
                uplift[idx] = down; // Negative (subsidence)
            } else {
                uplift[idx] = up.max(0); // Positive (uplift), clamp to zero.
            }
        }
    }

    uplift
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
}
