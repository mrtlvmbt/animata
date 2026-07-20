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

/// Measure corrugation: count crests (local maxima) along belt transects.
/// Compare to expected count based on fold wavelength.
fn measure_corrugation_threshold(
    dim: usize,
    height: &[i64],
    plate_fields: &world::gen::plate::PlateFields,
) -> (i64, i64, f64) {
    let dim_i = dim as i64;
    let hmax = 200i64;

    // Expected wavelength: belt_hw / 2
    // belt_hw = max(3, dim/16) from orogeny.rs
    let belt_hw = (dim_i / 16 + 3).max(3);
    let fold_wavelength = belt_hw / 2;

    // Expected crest count across full belt width
    // Crests spaced at wavelength intervals: 2*belt_hw / wavelength
    let expected_crest_count = (2 * belt_hw) / fold_wavelength;

    // Measure crest count across belt transects (every other column, take max)
    let mut total_crests = 0i64;
    let mut transect_count = 0i64;

    // Scan vertical transects (along z axis) at various x positions
    for x in (0..dim).step_by(2) {
        let x_i = x as i64;
        let mut crests_in_transect = 0i64;

        // Find cells in this transect that are in a convergent belt
        for z in 1..(dim - 1) {
            let z_i = z as i64;
            let idx = z * dim + x;
            let idx_prev = (z - 1) * dim + x;
            let idx_next = (z + 1) * dim + x;

            // Only count cells in convergent boundary (plate_fields.boundary_type)
            if plate_fields.boundary_type[idx] == world::gen::plate::BoundaryType::Convergent {
                // Check if local maximum (crest)
                if height[idx] > height[idx_prev] && height[idx] > height[idx_next] {
                    crests_in_transect += 1;
                }
            }
        }

        if crests_in_transect > 0 {
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
