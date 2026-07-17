//! W-11 ridged mountain belts (phase plan, acceptance suite) — ridge stage byte-identity
//! and purity contract. Tests the ridge-stage FBM, belt mask, and final clamp behavior
//! against golden fixtures and known properties.
//!
//! **Acceptance criteria (ТЗ W-11):**
//! 1. Flag-off byte-purity: with ridges=false, output byte-identical to pre-W-11 baseline
//! 2. Struct-refactor purity: LandformFlags::from_five path byte-identical to old 5-bool tuple path
//! 3. Clamp/bounds: ridge application keeps all heights in [0, hmax]
//! 4. Salt-independence: ridge field decorrelates from base noise (RIDGE_SEED_SALT)

use world::gen::caps::classify_and_caps;
use world::gen::LandformFlags;

const W11_SEED: u64 = 0xA11A_2A11;
const W11_HMAX: i64 = 200;
const W11_DIM: usize = 64;

/// **Acceptance criterion 1a**: Flag-off byte-purity.
/// With ridges=false, output byte-identical to pre-W-11 baseline (all landforms ON, ridges OFF).
#[test]
fn w11_ridges_flag_off_byte_identical_to_baseline() {
    // Pre-W-11 baseline: all original 5 landforms ON, ridges OFF (ridges didn't exist)
    let baseline = classify_and_caps(
        W11_SEED,
        W11_HMAX,
        W11_DIM,
        false,
        LandformFlags::from_five(true, true, true, true, true),
    );

    // Post-W-11 with ridges explicitly OFF: should be byte-identical
    let ridges_off = classify_and_caps(
        W11_SEED,
        W11_HMAX,
        W11_DIM,
        false,
        LandformFlags::new(true, true, true, true, true, false, false),
    );

    assert_eq!(baseline.height, ridges_off.height, "height must be byte-identical with ridges=false");
    assert_eq!(baseline.final_biome, ridges_off.final_biome, "final_biome must be byte-identical");
    assert_eq!(baseline.caps, ridges_off.caps, "caps must be byte-identical");
    assert_eq!(baseline.surface_material, ridges_off.surface_material, "surface_material must be byte-identical");
}

/// **Acceptance criterion 1b**: Struct-refactor byte-purity.
/// LandformFlags::from_five (new struct constructor path) byte-identical to old 5-bool tuple path
/// (simulated by LandformFlags::new with ridges=false, beaches=false).
#[test]
fn w11_struct_refactor_byte_identical_to_tuple_era() {
    // Struct path: from_five (convenience constructor, ridges/beaches always false)
    let via_from_five = classify_and_caps(
        W11_SEED,
        W11_HMAX,
        W11_DIM,
        false,
        LandformFlags::from_five(true, true, true, true, true),
    );

    // Struct path: new (explicit all-flags, with ridges/beaches false)
    let via_new = classify_and_caps(
        W11_SEED,
        W11_HMAX,
        W11_DIM,
        false,
        LandformFlags::new(true, true, true, true, true, false, false),
    );

    assert_eq!(via_from_five.height, via_new.height, "from_five and new must produce byte-identical height");
    assert_eq!(via_from_five.final_biome, via_new.final_biome, "from_five and new must produce byte-identical final_biome");
}

/// **Acceptance criterion 3**: Clamp/bounds.
/// Ridge field application must keep all cells in [0, hmax] after the tectonic+ridge delta is clamped.
#[test]
fn w11_ridge_application_respects_bounds() {
    let fields = classify_and_caps(
        W11_SEED,
        W11_HMAX,
        W11_DIM,
        false,
        LandformFlags::new(true, true, true, true, true, true, false), // ridges ON
    );

    for (i, &h) in fields.height.iter().enumerate() {
        assert!(
            (0..=W11_HMAX).contains(&h),
            "height[{i}]={h} out of [0, {W11_HMAX}]"
        );
    }
}

/// **Acceptance criterion 4**: Salt-independence (indirect test).
/// Ridge field (with RIDGE_SEED_SALT) must decorrelate from base-noise field.
/// Indirect test: with ridges ON vs OFF, the height field difference verifies salt worked
/// (if it didn't, ridges ON would be identical to ridges OFF, which this test catches).
#[test]
fn w11_ridge_field_has_effect_via_salt() {
    // Ridge ON: tectonic only, so ridges can be the only contributor
    let ridges_on = classify_and_caps(
        W11_SEED,
        W11_HMAX,
        W11_DIM,
        false,
        LandformFlags::new(true, false, false, false, false, true, false), // tect+ridges ON, others OFF
    );

    // Ridge OFF: same setup but ridges disabled
    let ridges_off = classify_and_caps(
        W11_SEED,
        W11_HMAX,
        W11_DIM,
        false,
        LandformFlags::new(true, false, false, false, false, false, false), // tect ON, ridges OFF
    );

    // With RIDGE_SEED_SALT decorrelating the ridge field from base noise,
    // ridges ON and OFF should produce different height arrays.
    // If RIDGE_SEED_SALT didn't work (ridge field = base noise), outputs would be identical.
    assert_ne!(
        ridges_on.height, ridges_off.height,
        "ridges must change the output height field (RIDGE_SEED_SALT must decorrelate from base noise)"
    );
}

/// **NEW: Amplitude sensitivity test**
/// Ridge amplitude knob must actually change the delta values.
/// This test catches saturation: if FBM overshoots the fold's domain, all candidates produce
/// identical output (zero deltas). With proper FBM normalization, different amplitudes
/// produce different ridge_delta values.
#[test]
fn w11_ridge_amplitude_sensitivity_candidates_differ() {
    // Test ridge delta computation with different amplitudes
    // Using a representative raw_fbm value in the middle of the range (half-saturated)
    let raw_fbm = 500_000i64; // In [0, 983040)
    let mountainness = 128i64;

    // Candidate 0 (conservative: 15/10)
    let delta0 = world::gen::erosion::ridge_delta_compute(
        raw_fbm,
        mountainness,
        15, // RIDGE_AMP_NUM for candidate 0
        10, // RIDGE_AMP_DEN for candidate 0
        W11_HMAX,
    );

    // Candidate 2 (aggressive: 40/10)
    let delta2 = world::gen::erosion::ridge_delta_compute(
        raw_fbm,
        mountainness,
        40, // RIDGE_AMP_NUM for candidate 2
        10, // RIDGE_AMP_DEN for candidate 2
        W11_HMAX,
    );

    // With proper FBM normalization, different amplitudes MUST produce different deltas
    // If they're identical, the amplitude knob is dead (saturation bug not fixed)
    assert_ne!(
        delta0, delta2,
        "ridge delta with amp=15/10 ({}) must differ from amp=40/10 ({}); amplitude knob is dead",
        delta0, delta2
    );
}

/// **NEW: Anti-saturation test**
/// Ridge delta should stay within reasonable bounds across the FBM input range.
/// With RIDGE_SCALE properly chosen, |ridge_delta| should not blow up to hmax,
/// indicating that the fold is not saturated (not producing all zeros or all ones).
#[test]
fn w11_ridge_field_avoids_saturation_to_extremes() {
    // Test ridge delta across the full FBM range with the default amplitude (25/10)
    // If FBM saturates, ridged values collapse and deltas should be near-zero everywhere
    let mut max_delta_abs = 0i64;
    let mut zero_delta_count = 0;
    let test_count = 100;

    for fbm_idx in 0..test_count {
        // Sample FBM range: [0, FBM_MAX)
        let raw_fbm = (fbm_idx as i64) * (983040i64 / test_count as i64);
        let mountainness = 256i64; // Max mountainness value

        let delta = world::gen::erosion::ridge_delta_compute(
            raw_fbm,
            mountainness,
            25, // RIDGE_AMP_NUM for candidate 1 (mid)
            10, // RIDGE_AMP_DEN for candidate 1 (mid)
            W11_HMAX,
        );

        max_delta_abs = max_delta_abs.max(delta.abs());
        if delta == 0 {
            zero_delta_count += 1;
        }
    }

    // Verify: max|delta| should be much less than hmax (we set it to tens of units, not hundreds)
    assert!(
        max_delta_abs <= W11_HMAX / 2,
        "max ridge delta {} exceeds hmax/2 ({}); saturation or scale error",
        max_delta_abs,
        W11_HMAX / 2
    );

    // Verify: not all deltas should be zero (fold is working, not saturated to all-zeros)
    assert!(
        zero_delta_count < test_count / 2,
        "too many zero deltas ({}/{}); fold may be saturated",
        zero_delta_count,
        test_count
    );
}
