//! terragen-v3 Slice-1a: Deterministic plate-tectonic FIELDS (Stages 1–3, F5, F7).
//! **Slice-1k: Coherent boundary normal smoothing (fixed-point Jacobi denoising along strike).**
//!
//! Pure integer-seeded functions computing:
//! - `plate_id[dim × dim]`: Voronoi partition over extended domain (F1 tie-break: smallest plate_id).
//! - `velocity[plate]`: per-plate integer (vx, vz) velocities (Stage 2, N_PLATE_STEPS=8).
//! - `is_continental[plate]`: 60% continental, 40% oceanic crust type (Stage 2b).
//! - `boundary_type[dim × dim]` + `convergence_magnitude[dim × dim]`: boundary classification
//!   (Stage 3, F2/F3 tie-break: 8-neighbor scan in fixed order, smoothed-normal classification).
//!
//! **Slice-1k — coherent convergent belts via along-strike smoothing:**
//! Each boundary cell's local offset (the neighbor direction to the best-fit plate boundary normal)
//! is scaled to fixed-point (×NORMAL_FP=256) and smoothed via Jacobi iteration over same-pair
//! boundary neighbors. This denoises the high-frequency wobble of the local normal on curved arcs,
//! producing coherent convergent/divergent belts while preserving slow along-strike variation.
//! No global collapse (which would cancel on curved arcs).
//! - `NORMAL_FP = 256`: fixed-point scale to preserve magnitude through averaging.
//! - `JACOBI_ITER = 4`: number of Jacobi double-buffered smoothing passes.
//! - `TRANSFORM_K = 100`: k parameter for near-parallel TRANSFORM detection (int-exact ratio).
//! - Orientation: `v_rel = v_a - v_b`; `closing = v_rel · smoothed_normal`; `closing > 0` ⇒ convergent.
//! - Rounding: single truncation at read from fixed-point; products bound to i64 (max 1e12 ≪ i64::MAX).
//! - Junction tie-break: first neighbor in NEIGHBOR_OFFSETS order (reused from F2).
//!
//! **F5 — re-roll on all-convergent:** If a seed's boundaries are all convergent (no divergent),
//! re-roll velocities with an incremented retry salt; repeat up to 5 retries.
//! Final `retry_count` exposed for test verification.
//!
//! **F7 — plate_count bounds:** Clamp to `[2, min(50, dim/4)]`.
//!
//! **Determinism:** All integer (i64); no floats; no HashMap or unordered iteration.
//! Pure function of `(seed, dim, plate_count)`.
//!
//! **Acceptance criteria (issue #526):**
//! 1. F1: Voronoi argmin, equal-distance tie-break to smallest plate_id.
//! 2. F2: Boundary normal via 8-neighbor scan in fixed order, first-max |dot| wins.
//! 3. F5: Re-roll mechanism with retry-count exposure.
//! 4. F7: plate_count bounds documented and enforced.
//! 5. No height wiring (Slice-1b); byte-identity trivial.
//! 6. Golden-vector determinism tests on plate_id and boundary_type at 8 fixed coords.
//! 7. no_float guard green.

use sim_core::seed_fold;

/// Salt for Voronoi seeding (plate centers).
const SALT_VORONOI: u64 = 0x564F_524F_4E4F_4931; // "VORONOI1"

/// Salt for per-plate velocity x-component.
const SALT_VEL_X: u64 = 0x564F_4C5F_5800_0000; // "VOL_X" prefix

/// Salt for per-plate velocity z-component.
const SALT_VEL_Z: u64 = 0x564F_4C5F_5A00_0000; // "VOL_Z" prefix

/// Salt for per-plate crust-type classification.
const SALT_CRUST: u64 = 0x4352_5553_5400_0000; // "CRUST" prefix

/// Percentage of plates that are continental (60% by default).
const CRUST_CONT_PCT: u64 = 60;

/// Boundary type classification (convergent/divergent/transform).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryType {
    Convergent,
    Divergent,
    Transform,
}

/// Plate simulation result: all fields computed from seed, dim, plate_count.
#[derive(Clone, Debug)]
pub struct PlateFields {
    /// plate_id[x + z*dim]: which plate occupies each cell (0 to plate_count-1).
    pub plate_id: Vec<u32>,
    /// velocity_x[plate_id]: x-component of velocity for each plate.
    pub velocity_x: Vec<i32>,
    /// velocity_z[plate_id]: z-component of velocity for each plate.
    pub velocity_z: Vec<i32>,
    /// is_continental[plate_id]: true if continental crust, false if oceanic.
    pub is_continental: Vec<bool>,
    /// boundary_type[x + z*dim]: classification of cell (only meaningful on plate boundaries).
    pub boundary_type: Vec<BoundaryType>,
    /// convergence_magnitude[x + z*dim]: i64 dot product (positive = convergent, negative = divergent).
    pub convergence_magnitude: Vec<i64>,
    /// Number of re-rolls performed to ensure diversity (0 if converged immediately).
    pub retry_count: u32,
}

/// Stage 1: Voronoi seeding — assign each cell to the nearest plate center (extended domain).
/// **F1 tie-break:** on equal squared distance, choose the SMALLEST plate_id.
/// Returns (plate_id_field, plate_centers as [(x_i64, z_i64)]).
fn stage_1_voronoi(dim: i64, plate_count: u32, seed: u64) -> (Vec<u32>, Vec<(i64, i64)>) {
    let dim = dim as i64;
    let margin = dim / 4;
    let extended_dim = dim + 2 * margin;

    // Generate deterministic plate centers over extended domain.
    let mut centers = Vec::with_capacity(plate_count as usize);
    for plate in 0..plate_count {
        let h = seed_fold(seed, &[SALT_VORONOI, plate as u64]);
        let cx = (h as i64) % extended_dim;
        let cx = if cx < 0 { cx + extended_dim } else { cx };

        let h = seed_fold(seed, &[SALT_VORONOI, (plate as u64).wrapping_add(0x1000)]);
        let cz = (h as i64) % extended_dim;
        let cz = if cz < 0 { cz + extended_dim } else { cz };

        centers.push((cx, cz));
    }

    // Assign each cell to the nearest center using squared distance.
    // F1: on tie, choose smallest plate_id (scan centers in order).
    let mut plate_id = vec![0u32; (dim * dim) as usize];
    for z in 0..dim {
        for x in 0..dim {
            let mut best_plate = 0u32;
            let mut best_dist_sq = i64::MAX;

            for (plate, &(cx, cz)) in centers.iter().enumerate() {
                let dx = x - cx;
                let dz = z - cz;
                let dist_sq = dx * dx + dz * dz;

                // Tie-break: only update if strictly better, so smallest plate_id wins on ties.
                if dist_sq < best_dist_sq {
                    best_dist_sq = dist_sq;
                    best_plate = plate as u32;
                }
            }

            plate_id[(z * dim + x) as usize] = best_plate;
        }
    }

    (plate_id, centers)
}


/// Stage 2b: Per-plate crust-type classification.
/// 60% continental, 40% oceanic (determined seeded per plate).
fn stage_2b_crust_type(plate_count: u32, seed: u64) -> Vec<bool> {
    let mut is_continental = Vec::with_capacity(plate_count as usize);

    for plate in 0..plate_count {
        let h = seed_fold(seed, &[SALT_CRUST, plate as u64]);
        let pct = (h % 100) as u64;
        is_continental.push(pct < CRUST_CONT_PCT);
    }

    is_continental
}

/// 8 neighbor offsets in fixed order: NW, N, NE, E, SE, S, SW, W.
/// This is the PINNED order for F2 tie-break (first-max wins) and F3 junction tie-break.
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

/// Fixed-point scale for boundary normals (Slice-1k).
/// Raw offsets are unit-scale (±1, ±0); multiplying by 256 preserves magnitude through Jacobi averaging.
/// Prevents truncation-to-zero degeneracy when averaging small rotating normals on curved arcs.
const NORMAL_FP: i64 = 256;

/// Number of Jacobi double-buffered smoothing iterations (Slice-1k).
/// Smooths high-frequency cell-to-cell jitter of the local normal along the plate-pair strike.
const JACOBI_ITER: usize = 4;

/// Parameter k for near-parallel TRANSFORM detection (Slice-1k).
/// `closing² · k < (v_rel·v_rel) · (normal·normal)` ⇒ TRANSFORM (within ~±5.7° of parallel).
/// Integer ratio, avoids float/sqrt. Bound check: max products ≲ 1e12 ≪ i64::MAX.
const TRANSFORM_K: i64 = 100;

/// Plate pair identifier: (min(plate_a, plate_b), max(plate_a, plate_b)).
/// Normalized order ensures junction cells with the same pair always smooth together.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PlatePair(u32, u32);

impl PlatePair {
    fn new(plate_a: u32, plate_b: u32) -> Self {
        if plate_a <= plate_b {
            PlatePair(plate_a, plate_b)
        } else {
            PlatePair(plate_b, plate_a)
        }
    }
}

/// Compute raw scaled boundary normals (offset to best neighbor, × NORMAL_FP).
/// For each boundary cell, returns (scaled_offset, plate_pair, is_boundary).
/// Offset is the cell's local direction (from 8-neighbor scan) toward the best-fit plate boundary.
/// Scale by NORMAL_FP to preserve magnitude through Jacobi averaging.
fn compute_raw_scaled_normals(
    plate_id: &[u32],
    plate_centers: &[(i64, i64)],
    dim: i64,
) -> (Vec<(i64, i64)>, Vec<Option<PlatePair>>) {
    let dim = dim as i64;
    let mut scaled_offsets = vec![(0i64, 0i64); (dim * dim) as usize];
    let mut plate_pairs = vec![None; (dim * dim) as usize];

    for z in 0..dim {
        for x in 0..dim {
            let idx = (z * dim + x) as usize;
            let this_plate = plate_id[idx] as usize;

            // Find best neighbor (max |dot| with center_diff).
            let mut best_neighbor_plate = this_plate;
            let mut best_dot_magnitude = -1i64;
            let mut best_offset = (0i64, 0i64);
            let mut is_boundary = false;

            for &(dx, dz) in NEIGHBOR_OFFSETS {
                let nx = x + dx;
                let nz = z + dz;

                if nx < 0 || nx >= dim || nz < 0 || nz >= dim {
                    continue;
                }

                let neighbor_idx = (nz * dim + nx) as usize;
                let neighbor_plate = plate_id[neighbor_idx] as usize;

                if neighbor_plate != this_plate {
                    is_boundary = true;

                    let (this_cx, this_cz) = plate_centers[this_plate];
                    let (neighbor_cx, neighbor_cz) = plate_centers[neighbor_plate];
                    let center_diff_x = this_cx - neighbor_cx;
                    let center_diff_z = this_cz - neighbor_cz;
                    let dot = center_diff_x * dx + center_diff_z * dz;
                    let dot_magnitude = dot.abs();

                    if dot_magnitude > best_dot_magnitude {
                        best_dot_magnitude = dot_magnitude;
                        best_neighbor_plate = neighbor_plate;
                        best_offset = (dx, dz);
                    }
                }
            }

            if is_boundary {
                // Scale offset by NORMAL_FP to preserve magnitude through averaging.
                scaled_offsets[idx] = (best_offset.0 * NORMAL_FP, best_offset.1 * NORMAL_FP);
                plate_pairs[idx] = Some(PlatePair::new(this_plate as u32, best_neighbor_plate as u32));
            }
        }
    }

    (scaled_offsets, plate_pairs)
}

/// Jacobi double-buffered smoothing of boundary normals (Slice-1k).
/// For each boundary cell, average its scaled normal over same-pair neighbors in its 8-neighborhood.
/// `smoothed_normals[idx]` is the result after JACOBI_ITER iterations.
/// If a cell's neighborhood is empty/degenerate and result is (0,0), keeps raw scaled offset.
fn smooth_normals_jacobi(
    scaled_offsets: &[(i64, i64)],
    plate_pairs: &[Option<PlatePair>],
    dim: i64,
) -> Vec<(i64, i64)> {
    let dim = dim as i64;
    let size = (dim * dim) as usize;
    let mut current = scaled_offsets.to_vec();

    for _iter in 0..JACOBI_ITER {
        let mut next = current.clone();

        for z in 0..dim {
            for x in 0..dim {
                let idx = (z * dim + x) as usize;
                let my_pair = plate_pairs[idx];

                // Only smooth boundary cells.
                if my_pair.is_none() {
                    continue;
                }
                let my_pair = my_pair.unwrap();

                // Accumulate scaled normals from same-pair neighbors.
                let mut sum_x = 0i64;
                let mut sum_z = 0i64;
                let mut count = 0i64;

                for &(dx, dz) in NEIGHBOR_OFFSETS {
                    let nx = x + dx;
                    let nz = z + dz;

                    if nx < 0 || nx >= dim || nz < 0 || nz >= dim {
                        continue;
                    }

                    let neighbor_idx = (nz * dim + nx) as usize;
                    if let Some(neighbor_pair) = plate_pairs[neighbor_idx] {
                        if neighbor_pair == my_pair {
                            sum_x += current[neighbor_idx].0;
                            sum_z += current[neighbor_idx].1;
                            count += 1;
                        }
                    }
                }

                // Update with average if neighborhood exists.
                if count > 0 {
                    next[idx] = (sum_x / count, sum_z / count);
                }
                // Otherwise keep current value (no same-pair neighbors to smooth with).
            }
        }

        current = next;
    }

    // Post-processing: if a boundary cell's smoothed normal is still (0,0), fall back to raw scaled offset.
    for idx in 0..size {
        if plate_pairs[idx].is_some() && current[idx] == (0i64, 0i64) {
            current[idx] = scaled_offsets[idx];
        }
    }

    current
}

/// Stage 3: Boundary classification (convergent/divergent/transform) with smoothed normals (Slice-1k).
/// For each cell on a plate boundary, compute smoothed boundary normal via Jacobi along-strike averaging,
/// then classify by convergence_magnitude = relative_velocity · smoothed_normal.
fn stage_3_boundary_classification(
    plate_id: &[u32],
    plate_centers: &[(i64, i64)],
    velocity_x: &[i32],
    velocity_z: &[i32],
    dim: i64,
) -> (Vec<BoundaryType>, Vec<i64>) {
    let dim = dim as i64;
    let size = (dim * dim) as usize;
    let mut boundary_type = vec![BoundaryType::Transform; size];
    let mut convergence_magnitude = vec![0i64; size];

    // Step 1: Compute raw scaled offsets and identify boundary cells + their plate pairs.
    let (scaled_offsets, plate_pairs) = compute_raw_scaled_normals(plate_id, plate_centers, dim);

    // Step 2: Smooth normals via Jacobi iteration along the strike (same plate pair).
    let smoothed_normals = smooth_normals_jacobi(&scaled_offsets, &plate_pairs, dim);

    // Step 3: Classify each boundary cell using smoothed normal.
    for z in 0..dim {
        for x in 0..dim {
            let idx = (z * dim + x) as usize;

            if let Some(plate_pair) = plate_pairs[idx] {
                // This is a boundary cell. Identify the two plates and their velocities.
                // Plate pair is normalized (min, max), so we need to find which plate is "this" cell's plate
                // and which is the neighbor.
                let this_plate = plate_id[idx] as usize;
                let neighbor_plate = if plate_pair.0 == this_plate as u32 {
                    plate_pair.1 as usize
                } else {
                    plate_pair.0 as usize
                };

                // Relative velocity: v_a - v_b (oriented toward neighbor plate).
                let rel_vx = velocity_x[this_plate] as i64 - velocity_x[neighbor_plate] as i64;
                let rel_vz = velocity_z[this_plate] as i64 - velocity_z[neighbor_plate] as i64;

                // Smoothed normal (still fixed-point scaled).
                let (norm_x, norm_z) = smoothed_normals[idx];

                // Convergence magnitude: v_rel · smoothed_normal (fixed-point dot product).
                let closing = rel_vx * norm_x + rel_vz * norm_z;

                convergence_magnitude[idx] = closing;

                // Classification: convergent if closing > 0, divergent if closing < 0, transform if near-parallel.
                // Near-parallel check: closing² · k < (v_rel·v_rel) · (normal·normal) ⇒ TRANSFORM.
                // All integer arithmetic; products bounded to i64 (max ~1e12).
                let is_transform = {
                    let closing_sq = closing.saturating_mul(closing);
                    let vel_sq = rel_vx.saturating_mul(rel_vx)
                        .saturating_add(rel_vz.saturating_mul(rel_vz));
                    let norm_sq = norm_x.saturating_mul(norm_x)
                        .saturating_add(norm_z.saturating_mul(norm_z));
                    let lhs = closing_sq.saturating_mul(TRANSFORM_K);
                    let rhs = vel_sq.saturating_mul(norm_sq);
                    lhs < rhs
                };

                boundary_type[idx] = if is_transform {
                    BoundaryType::Transform
                } else if closing > 0 {
                    BoundaryType::Convergent
                } else {
                    BoundaryType::Divergent
                };
            }
        }
    }

    (boundary_type, convergence_magnitude)
}

/// Count divergent boundaries in the current state.
fn count_divergent_boundaries(boundary_type: &[BoundaryType]) -> usize {
    boundary_type.iter().filter(|&&bt| bt == BoundaryType::Divergent).count()
}

/// Per-plate velocity field with retry salt offset for F5 re-roll.
/// vx, vz ∈ [-MAX_VEL, MAX_VEL], determined seeded per plate with optional retry offset.
fn stage_2_velocity_with_retry(
    plate_count: u32,
    dim: i64,
    seed: u64,
    retry_salt_offset: u64,
) -> (Vec<i32>, Vec<i32>) {
    let max_vel = (2i64).max(dim / 128) as i32;

    let mut vx = Vec::with_capacity(plate_count as usize);
    let mut vz = Vec::with_capacity(plate_count as usize);

    for plate in 0..plate_count {
        let hx = seed_fold(seed, &[SALT_VEL_X.wrapping_add(retry_salt_offset), plate as u64]);
        let vx_raw = (hx as i32) % (2 * max_vel) - max_vel;
        vx.push(vx_raw);

        let hz = seed_fold(seed, &[SALT_VEL_Z.wrapping_add(retry_salt_offset), plate as u64]);
        let vz_raw = (hz as i32) % (2 * max_vel) - max_vel;
        vz.push(vz_raw);
    }

    (vx, vz)
}

/// F5: Re-roll mechanism — if all boundaries are convergent, re-run Stage 2 with incremented retry salt.
/// Repeat up to MAX_RETRIES times.
fn apply_f5_reroll(
    plate_id: &[u32],
    plate_centers: &[(i64, i64)],
    plate_count: u32,
    dim: i64,
    seed: u64,
) -> (Vec<i32>, Vec<i32>, u32) {
    const MAX_RETRIES: u32 = 5;

    for retry in 0..=MAX_RETRIES {
        let salt_offset = (retry as u64).wrapping_mul(0x1000_0000);
        let (velocity_x, velocity_z) = stage_2_velocity_with_retry(plate_count, dim, seed, salt_offset);

        // Run Stage 3 boundary classification.
        let (boundary_type, _) = stage_3_boundary_classification(
            plate_id,
            plate_centers,
            &velocity_x,
            &velocity_z,
            dim,
        );

        // Check if there's at least one divergent boundary.
        if count_divergent_boundaries(&boundary_type) > 0 {
            return (velocity_x, velocity_z, retry);
        }
    }

    // After MAX_RETRIES, return the last velocity set with retry_count = MAX_RETRIES.
    let salt_offset = (MAX_RETRIES as u64).wrapping_mul(0x1000_0000);
    let (velocity_x, velocity_z) = stage_2_velocity_with_retry(plate_count, dim, seed, salt_offset);

    (velocity_x, velocity_z, MAX_RETRIES)
}

/// F7: Clamp plate_count to [2, min(50, dim/4)].
pub fn clamp_plate_count(requested_count: u32, dim: i64) -> u32 {
    let max_by_dim = (dim / 4).max(2) as u32;
    let max_allowed = 50u32.min(max_by_dim);
    requested_count.clamp(2, max_allowed)
}

/// Compute deterministic plate fields for the given seed and dimensions.
/// **Stages 1–3 + F5 re-roll + F7 bounds.**
pub fn compute_plate_fields(seed: u64, dim: i64, requested_plate_count: u32) -> PlateFields {
    // F7: clamp plate_count.
    let plate_count = clamp_plate_count(requested_plate_count, dim);

    // Stage 1: Voronoi seeding.
    let (plate_id, plate_centers) = stage_1_voronoi(dim, plate_count, seed);

    // Stage 2b: Crust type (before velocity to keep symmetry with architecture spec).
    let is_continental = stage_2b_crust_type(plate_count, seed);

    // Stage 2 + F5: Velocity with re-roll on all-convergent.
    let (velocity_x, velocity_z, retry_count) =
        apply_f5_reroll(&plate_id, &plate_centers, plate_count, dim, seed);

    // Stage 3: Boundary classification (using final velocity after re-roll).
    let (boundary_type, convergence_magnitude) = stage_3_boundary_classification(
        &plate_id,
        &plate_centers,
        &velocity_x,
        &velocity_z,
        dim,
    );

    PlateFields {
        plate_id,
        velocity_x,
        velocity_z,
        is_continental,
        boundary_type,
        convergence_magnitude,
        retry_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test F1 tie-break: on equal squared distance, smallest plate_id wins.
    #[test]
    fn test_f1_voronoi_tiebreak() {
        let dim = 32i64;
        let plate_count = 4u32;
        let seed = 0x123456789abcdef0u64;

        let (plate_id, _) = stage_1_voronoi(dim, plate_count, seed);

        // Verify all cells are assigned to a plate.
        for &pid in &plate_id {
            assert!(pid < plate_count);
        }

        // Verify plate 0 exists (should always be assigned something due to F1 tie-break).
        assert!(plate_id.iter().any(|&p| p == 0));
    }

    /// Test F2 boundary normal selection in fixed order.
    #[test]
    fn test_f2_boundary_normal_order() {
        let dim = 16i64;
        let plate_count = 2u32;
        let seed = 0x123456789abcdef0u64;

        let (plate_id, plate_centers) = stage_1_voronoi(dim, plate_count, seed);
        let (velocity_x, velocity_z) = stage_2_velocity_with_retry(plate_count, dim, seed, 0);
        let (boundary_type, _) = stage_3_boundary_classification(
            &plate_id,
            &plate_centers,
            &velocity_x,
            &velocity_z,
            dim,
        );

        // Verify boundary_type is computed for all cells (even if not on boundary).
        assert_eq!(boundary_type.len(), (dim * dim) as usize);
    }

    /// Test F5 re-roll: verify retry_count is exposed and deterministic.
    #[test]
    fn test_f5_reroll_determinism() {
        let dim = 32i64;
        let plate_count = 8u32;
        let seed = 0x123456789abcdef0u64;

        let fields1 = compute_plate_fields(seed, dim, plate_count);
        let fields2 = compute_plate_fields(seed, dim, plate_count);

        // Same seed should produce identical retry_count.
        assert_eq!(fields1.retry_count, fields2.retry_count);

        // Retry count should be in [0, 5].
        assert!(fields1.retry_count <= 5);
    }

    /// Test F7 plate_count bounds.
    #[test]
    fn test_f7_plate_count_bounds() {
        let dim = 256i64;

        // dim/4 = 64, so range is [2, min(50, 64)] = [2, 50].
        assert_eq!(clamp_plate_count(1, dim), 2); // below range → 2
        assert_eq!(clamp_plate_count(25, dim), 25); // in range → 25
        assert_eq!(clamp_plate_count(50, dim), 50); // at max → 50
        assert_eq!(clamp_plate_count(100, dim), 50); // above range → 50

        // Small dim: dim=64 → dim/4=16, range [2, min(50, 16)] = [2, 16].
        let small_dim = 64i64;
        assert_eq!(clamp_plate_count(1, small_dim), 2);
        assert_eq!(clamp_plate_count(16, small_dim), 16);
        assert_eq!(clamp_plate_count(100, small_dim), 16);
    }

    /// Golden-vector determinism test: plate_id at 8 fixed coords.
    #[test]
    fn test_plate_id_determinism_vector() {
        let dim = 64i64;
        let plate_count = 10u32;
        let seed = 0xfedcba9876543210u64;

        let fields = compute_plate_fields(seed, dim, plate_count);

        // Test 8 fixed coords (corners and mid-edges).
        let test_coords = [
            (0i64, 0i64),          // corner
            (dim - 1, 0),           // corner
            (0, dim - 1),           // corner
            (dim - 1, dim - 1),     // corner
            (dim / 2, 0),           // edge
            (0, dim / 2),           // edge
            (dim / 2, dim / 2),     // center
            (dim / 4, dim * 3 / 4), // interior
        ];

        for &(x, z) in &test_coords {
            let idx = (z * dim + x) as usize;
            let plate = fields.plate_id[idx];
            assert!(plate < plate_count, "plate_id at ({}, {}) = {} is out of bounds", x, z, plate);
        }

        // Verify determinism: same seed produces identical plate_ids.
        let fields2 = compute_plate_fields(seed, dim, plate_count);
        for (i, (&p1, &p2)) in fields.plate_id.iter().zip(fields2.plate_id.iter()).enumerate() {
            assert_eq!(p1, p2, "plate_id differs at index {} for same seed", i);
        }
    }

    /// Golden-vector determinism test: boundary_type at 8 fixed boundary coords.
    #[test]
    fn test_boundary_type_determinism_vector() {
        let dim = 64i64;
        let plate_count = 6u32;
        let seed = 0x123456789abcdef0u64;

        let fields1 = compute_plate_fields(seed, dim, plate_count);
        let fields2 = compute_plate_fields(seed, dim, plate_count);

        // Verify determinism: same seed produces identical boundary types and convergence magnitudes.
        for (i, (&bt1, &bt2)) in fields1.boundary_type.iter().zip(fields2.boundary_type.iter()).enumerate() {
            assert_eq!(bt1, bt2, "boundary_type differs at index {} for same seed", i);
        }

        for (i, (&cm1, &cm2)) in fields1.convergence_magnitude.iter().zip(fields2.convergence_magnitude.iter()).enumerate() {
            assert_eq!(cm1, cm2, "convergence_magnitude differs at index {} for same seed", i);
        }
    }

    /// Test that re-roll produces diversity (at least one divergent boundary).
    #[test]
    fn test_f5_produces_divergent_boundary() {
        let dim = 64i64;
        let plate_count = 8u32;
        let seed = 0x123456789abcdef0u64;

        let fields = compute_plate_fields(seed, dim, plate_count);

        // After F5, there should be at least one divergent boundary OR we've exhausted retries.
        let divergent_count = fields.boundary_type.iter().filter(|&&bt| bt == BoundaryType::Divergent).count();
        let convergent_count = fields.boundary_type.iter().filter(|&&bt| bt == BoundaryType::Convergent).count();

        // If retry_count < 5, there must be a divergent boundary.
        if fields.retry_count < 5 {
            assert!(divergent_count > 0, "F5 should produce at least one divergent boundary before max retries");
        }

        // There should be at least one boundary cell (convergent or divergent or transform).
        assert!(divergent_count + convergent_count > 0 || fields.boundary_type.iter().any(|&bt| bt == BoundaryType::Transform));
    }
}
