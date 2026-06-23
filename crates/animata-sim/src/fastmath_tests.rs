use super::*;

/// `fastmath::tanh` tracks `f32::tanh` to within ~1e-3 across the brain's working range, stays in
/// `[-1, 1]`, is odd, and saturates at ±1 in the tail — the properties the activation relies on.
#[test]
fn tanh_matches_libm_within_tolerance() {
    let mut max_err = 0.0f32;
    let mut x = -8.0f32;
    while x <= 8.0 {
        let a = tanh(x);
        let e = x.tanh();
        max_err = max_err.max((a - e).abs());
        assert!((-1.0..=1.0).contains(&a), "tanh({x}) = {a} out of [-1,1]");
        // Approximately odd (the fast exp isn't exactly reciprocal, so allow approx-level slack).
        assert!((tanh(-x) + a).abs() < 2e-3, "tanh not odd at {x}");
        x += 0.01;
    }
    assert!(max_err < 1.5e-3, "fast tanh max error {max_err} too large");
    // Saturation tail.
    assert!((tanh(20.0) - 1.0).abs() < 1e-6 && (tanh(-20.0) + 1.0).abs() < 1e-6);
}

/// `fastmath::exp` tracks `f32::exp` to within ~3e-4 relative over the hot callers' domain
/// (`x ≤ 0`, the relaxation laws), is exact at 0, and underflows to 0 for very negative x.
#[test]
fn exp_matches_libm_on_nonpositive_domain() {
    let mut max_rel = 0.0f32;
    let mut x = 0.0f32;
    while x >= -30.0 {
        let a = exp(x);
        let e = x.exp();
        if e > 1e-12 {
            max_rel = max_rel.max((a - e).abs() / e);
        }
        assert!(a >= 0.0, "exp({x}) = {a} negative");
        x -= 0.001;
    }
    assert!(max_rel < 5e-4, "fast exp max relative error {max_rel} too large");
    assert!((exp(0.0) - 1.0).abs() < 1e-6, "exp(0) must be 1");
    assert_eq!(exp(-200.0), 0.0, "deeply negative exp must underflow to 0");
}

/// `kleiber075` must be BIT-IDENTICAL to the `powf(0.75)` it replaces over the whole biomass domain
/// (`0..=MAX_CELLS`) AND for an out-of-range value (the defensive `powf` fallback) — this is a pure
/// throughput swap, NOT an approximation, so the determinism golden must not move.
#[test]
fn kleiber_lut_is_bit_identical_to_powf() {
    // `black_box` forces a RUNTIME libm `powf` (no const-fold / vectorised codegen), matching the
    // dynamic call the apply path makes — that is the value the golden was pinned against.
    for n in 0..=(crate::genome::MAX_CELLS as u32) {
        let direct = (std::hint::black_box(n) as f32).powf(0.75);
        assert_eq!(
            kleiber075(n).to_bits(),
            direct.to_bits(),
            "kleiber075({n}) must equal a runtime (n as f32).powf(0.75) bit-for-bit"
        );
    }
    // Out-of-range falls through to a direct powf — still exact.
    let big = crate::genome::MAX_CELLS as u32 + 7;
    let direct = (std::hint::black_box(big) as f32).powf(0.75);
    assert_eq!(kleiber075(big).to_bits(), direct.to_bits());
}

/// Determinism: the approximations are pure functions — identical bits on repeat calls (the
/// property the parallel replay depends on).
#[test]
fn approximations_are_bit_stable() {
    for &x in &[-3.3f32, -1.0, -0.25, 0.0, 0.5, 2.7] {
        assert_eq!(tanh(x).to_bits(), tanh(x).to_bits());
        assert_eq!(exp(x).to_bits(), exp(x).to_bits());
    }
}
