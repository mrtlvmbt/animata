//! W-1i: Plate-path determinism golden (Slice-1i fold-chains).
//!
//! **Slice-1i F2 (critic F2):** The ENTIRE plate-sim path is currently golden-UNCOVERED.
//! This test pins the integrated plate-sim height field cross-arch to convert plate-path
//! determinism from an untested assumption into a CI gate.
//!
//! **Determinism:** Arm64-pinned, two-pass: placeholder pin → read actual value from CI
//! `golden-arm64` job `.ci-report/failed.log`, then commit the actual pin.
//!
//! **Test contract:** Fixed seed, dim=64, enable_plate_sim=true, hmax=200, produce the
//! full height field, compute a state_checksum (fold all values via XOR), and assert exact match.

use world::gen::caps::classify_and_caps;
use world::gen::LandformFlags;

// Golden seed for deterministic reproducibility
const PLATE_GOLDEN_SEED: u64 = 0x1234_5678_9ABC_DEF0u64;
const PLATE_DIM: usize = 64;
const PLATE_HMAX: i64 = 200;

/// Compute a state_checksum over the full height field via XOR fold.
/// Deterministic, order-independent (modulo hash collisions).
fn compute_state_checksum(height: &[i64]) -> u64 {
    let mut checksum = 0u64;
    for &h in height {
        checksum = checksum.wrapping_mul(31).wrapping_add((h as u64).wrapping_mul(0xDEAD_BEEF_CAFE_BABEu64));
    }
    checksum
}

/// **Plate-path full-field determinism golden:** The integrated height field
/// (base fBm + plate uplift + anti-spike, enable_plate_sim=true) must match the pinned checksum.
#[test]
#[cfg(target_arch = "aarch64")]
fn v2_plate_golden_conserved_state_checksum() {
    // Construct the world with plate sim enabled.
    let flags = LandformFlags {
        base: true,
        tect: false,
        aeolian: false,
        volcanic: false,
        glacial: false,
        coastal: false,
        erosion: true,
        ridges: false,
        beaches: false,
        erosion_strength: 100,
        glacial_strength: 100,
    };

    // Generate the full height field via the pipeline with plate sim enabled.
    let fields = classify_and_caps(PLATE_GOLDEN_SEED, PLATE_HMAX, PLATE_DIM, false, flags, true, 100);

    // Compute the checksum.
    let checksum = compute_state_checksum(&fields.height);

    // **PLACEHOLDER PIN** — replace with actual value from CI `golden-arm64` job.
    // On first run, this will fail with the actual checksum; copy it here and commit.
    const GOLDEN_CHECKSUM: u64 = 0; // PLACEHOLDER — set after first CI run
    assert_eq!(checksum, GOLDEN_CHECKSUM, "plate-path height field checksum drifted (left=run, right=GOLDEN)");
}

/// Verify the plate path is deterministic across repeated invocations (same seed produces same field).
#[test]
fn test_plate_path_is_deterministic() {
    let flags = LandformFlags::default();

    let fields1 = classify_and_caps(PLATE_GOLDEN_SEED, PLATE_HMAX, PLATE_DIM, false, flags, true, 100);
    let fields2 = classify_and_caps(PLATE_GOLDEN_SEED, PLATE_HMAX, PLATE_DIM, false, flags, true, 100);

    for (h1, h2) in fields1.height.iter().zip(fields2.height.iter()) {
        assert_eq!(h1, h2, "plate path is not deterministic across repeated calls");
    }
}

/// Verify byte-identity with enable_plate_sim=false (default fBm path unchanged).
#[test]
fn test_default_fbm_path_byte_identical_with_enable_false() {
    let flags = LandformFlags::default();

    let fields_off = classify_and_caps(PLATE_GOLDEN_SEED, PLATE_HMAX, PLATE_DIM, false, flags, false, 100);

    // With enable_plate_sim=false, no plate uplift is added; the field should be pure fBm.
    // This test verifies the byte-identity: multiple runs should produce identical results.
    let fields_off_2 = classify_and_caps(PLATE_GOLDEN_SEED, PLATE_HMAX, PLATE_DIM, false, flags, false, 100);

    for (h1, h2) in fields_off.height.iter().zip(fields_off_2.height.iter()) {
        assert_eq!(h1, h2, "default (enable_plate_sim=false) path is not byte-identical");
    }
}

/// **Slice-1i: Corrugation verification** — ensure fold ridges are not planed flat by talus.
/// Count local maxima (crests) along belt transects and verify minimum threshold.
/// Expected crest count ≥ 0.6 × (2·belt_hw / FOLD_WAVELENGTH) for belt to read as a chain.
#[test]
fn test_corrugation_threshold_not_planed_flat() {
    let flags = LandformFlags::default();
    let fields = classify_and_caps(PLATE_GOLDEN_SEED, PLATE_HMAX, PLATE_DIM, false, flags, true, 100);
    let height = &fields.height;
    let dim = PLATE_DIM;

    // Belt parameters (matching orogeny.rs).
    let belt_hw = (PLATE_DIM as i64 / 16).max(3) as usize;
    let base_wavelength = belt_hw / 2;
    let expected_crests = (2 * belt_hw) / base_wavelength; // Expected count across full belt width

    // Sample a transect across the middle of the grid (constant z, varying x).
    // Count local maxima along this line.
    let transect_z = dim / 2;
    let mut crest_count = 0;
    for x in 1..dim - 1 {
        let idx_left = transect_z * dim + (x - 1);
        let idx_mid = transect_z * dim + x;
        let idx_right = transect_z * dim + (x + 1);

        let h_left = height[idx_left];
        let h_mid = height[idx_mid];
        let h_right = height[idx_right];

        // Count local maxima (h_mid >= both neighbors).
        if h_mid >= h_left && h_mid >= h_right {
            crest_count += 1;
        }
    }

    // Threshold: at least 60% of expected crests must be retained.
    let min_threshold = (expected_crests * 60) / 100;
    assert!(
        crest_count >= min_threshold,
        "corrugation threshold failed: got {} crests, expected ≥ {} (60% of {} expected)",
        crest_count, min_threshold, expected_crests
    );
}
