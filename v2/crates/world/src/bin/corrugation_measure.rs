//! Corrugation-threshold measurement: count crests along belt transects
//! Verifies that fold-chain modulation produces ≥60% of expected ridge crests.
//!
//! PRE-REGISTERED VALIDITY GATE:
//! - Capability: measures fold-train ridges on plate-tectonics uplift fields
//! - Regime: dim=256, enable_plate_sim=true, erosion=true, two seeds (1234567890, 9876543210)
//! - Metric: crest count along full-width belt transect vs. expected (2·belt_hw/FOLD_WAVELENGTH)
//! - Anti-forcing: if < 60%, folds are being planed—report and stop (don't paper over)
//!
//! Usage:
//!   cargo run --release --bin corrugation_measure
//!
//! Output:
//!   - Crest counts per seed and transect
//!   - Pass/fail verdict on 60% threshold

use std::collections::HashSet;

fn main() {
    const HMAX: i64 = 200;
    const DIM: usize = 256;
    const PLATE_STRENGTH: i64 = 100; // Standard strength

    let seeds = [1234567890u64, 9876543210u64];

    println!("=== Fold-Chain Corrugation Threshold Measurement ===");
    println!("Config: dim={}, hmax={}, enable_plate_sim=true, erosion=true", DIM, HMAX);
    println!("Plate strength: {}", PLATE_STRENGTH);
    println!();

    let mut all_pass = true;

    for &seed in &seeds {
        println!("Seed: {}", seed);

        // Compute plate fields and uplift
        let plate_count = 15u32;
        let plate_count_clamped = world::gen::plate::clamp_plate_count(plate_count, DIM as i64);
        let plate_fields =
            world::gen::plate::compute_plate_fields(seed, DIM as i64, plate_count_clamped);
        let plate_uplift =
            world::gen::orogeny::generate_plate_uplift_field(&plate_fields, DIM as i64, HMAX, PLATE_STRENGTH);

        // Create flat base + uplift
        let flat_base = HMAX / 2;
        let mut height = vec![flat_base; DIM * DIM];
        for i in 0..DIM * DIM {
            height[i] = (height[i] + plate_uplift[i]).clamp(0, HMAX);
        }

        // Apply erosion (with fold structures intact before erosion)
        let resistance = compute_resistance(seed, DIM, HMAX);
        let erosion = world::gen::erosion::erode_from_fields(
            seed, HMAX, DIM, height, resistance, true, PLATE_STRENGTH, 0,
        );

        // Measure crests along multiple transects (across the belt)
        let (crest_count, expected_count, fraction_pass) =
            measure_corrugation_threshold(DIM, &erosion.height, &plate_fields);

        let pass = fraction_pass >= 0.6;
        all_pass &= pass;

        println!(
            "  Measured crests: {} / Expected: {} / Fraction: {:.2} / {} threshold",
            crest_count,
            expected_count,
            fraction_pass,
            if pass { "PASS" } else { "FAIL" }
        );
        println!();
    }

    if all_pass {
        println!("✓ GATE PASS: All seeds meet ≥60% corrugation threshold");
        std::process::exit(0);
    } else {
        println!("✗ GATE FAIL: Folds are being planed flat (< 60% threshold)");
        std::process::exit(1);
    }
}

fn compute_resistance(seed: u64, dim: usize, hmax: i64) -> Vec<i64> {
    const SALT: u64 = 0x5245_5349_5354_414E;
    const N_CLASSES: i64 = 4;

    let mut resistance = Vec::with_capacity(dim * dim);
    for idx in 0..dim * dim {
        let x = (idx % dim) as i64;
        let z = (idx / dim) as i64;
        let h = world::gen::height::height_at(x, z, seed ^ SALT, hmax);
        let class = ((h * N_CLASSES) / hmax).min(N_CLASSES - 1);
        resistance.push(class);
    }
    resistance
}

/// Compute belt distance via BFS from convergent boundary cells.
/// Returns distance[idx] = min distance to nearest convergent boundary (0 = boundary cell itself).
fn compute_belt_distance(
    dim: usize,
    boundary_type: &[world::gen::plate::BoundaryType],
) -> Vec<i64> {
    use std::collections::VecDeque;

    let dim_i = dim as i64;
    let mut distance = vec![i64::MAX; dim * dim];
    let mut queue = VecDeque::new();

    // Seed: convergent boundary cells start at distance 0.
    for z in 0..dim {
        for x in 0..dim {
            let idx = z * dim + x;
            if boundary_type[idx] == world::gen::plate::BoundaryType::Convergent {
                distance[idx] = 0;
                queue.push_back((x as i64, z as i64));
            }
        }
    }

    // BFS: propagate outward (8-neighbor).
    const NEIGHBOR_OFFSETS: &[(i64, i64)] = &[
        (-1, -1), (0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0),
    ];

    while let Some((x, z)) = queue.pop_front() {
        let idx = (z as usize) * dim + (x as usize);
        let cur_dist = distance[idx];

        for &(dx, dz) in NEIGHBOR_OFFSETS {
            let nx = x + dx;
            let nz = z + dz;

            if nx < 0 || nx >= dim_i || nz < 0 || nz >= dim_i {
                continue;
            }

            let nidx = (nz as usize) * dim + (nx as usize);
            let next_dist = cur_dist + 1;

            if next_dist < distance[nidx] {
                distance[nidx] = next_dist;
                queue.push_back((nx, nz));
            }
        }
    }

    distance
}

/// Measure corrugation: count crests (local maxima) ACROSS THE BELT, not just at boundary.
/// **CORRECTED METHODOLOGY (was broken):**
/// The fold ridges are distributed at belt_distance 0..belt_hw from the convergent boundary line.
/// The old code only counted at boundary cells (1-cell-wide line) → max 1-2 crests per transect.
/// This counts within the belt band, capturing the fold ridge distribution.
fn measure_corrugation_threshold(
    dim: usize,
    height: &[i64],
    plate_fields: &world::gen::plate::PlateFields,
) -> (i64, i64, f64) {
    let dim_i = dim as i64;

    // Expected wavelength: belt_hw / 2
    // belt_hw = max(3, dim/16) from orogeny.rs
    let belt_hw = ((dim_i / 16).max(3)) as usize;
    let fold_wavelength = belt_hw / 2;

    // Expected crest count across full belt width
    // Crests spaced at wavelength intervals: 2*belt_hw / wavelength
    let expected_crest_count = (2 * belt_hw as i64) / (fold_wavelength as i64);

    // Compute belt distance: min distance from each cell to nearest convergent boundary
    let belt_distance = compute_belt_distance(dim, &plate_fields.boundary_type);

    // Measure crest count across belt transects
    let mut total_crests = 0i64;
    let mut transect_count = 0i64;

    // Scan vertical transects (along z axis) at various x positions
    for x in (0..dim).step_by(2) {
        let mut crests_in_transect = 0i64;
        let mut belt_cells_in_transect = 0i64;

        // Find cells in this transect that are WITHIN the belt (belt_distance <= belt_hw)
        for z in 1..(dim - 1) {
            let idx = z * dim + x;
            let idx_prev = (z - 1) * dim + x;
            let idx_next = (z + 1) * dim + x;

            // Only count cells within the belt distance
            if belt_distance[idx] <= belt_hw as i64 {
                belt_cells_in_transect += 1;

                // Check if local maximum (crest) within belt
                if height[idx] > height[idx_prev] && height[idx] > height[idx_next] {
                    crests_in_transect += 1;
                }
            }
        }

        // Only count transects that actually cross a belt (non-empty)
        if belt_cells_in_transect > 0 {
            transect_count += 1;
            total_crests += crests_in_transect;
        }
    }

    let measured_count = if transect_count > 0 {
        total_crests / transect_count.max(1)
    } else {
        0
    };

    let fraction = if expected_crest_count > 0 {
        (measured_count as f64) / (expected_crest_count as f64)
    } else {
        0.0
    };

    (measured_count, expected_crest_count, fraction)
}
