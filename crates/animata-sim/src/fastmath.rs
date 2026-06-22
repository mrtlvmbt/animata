//! Deterministic fast approximation of the brain's `tanh` activation.
//!
//! The 200k-creature `step` is compute-bound on per-creature transcendentals — chiefly the brain's
//! `tanh` activations (10 per creature per tick: 8 hidden + 2 output). `f32::tanh` calls libm
//! (~40–60 cycles) and dominates `decide`. [`tanh`] replaces it with pure `f32` arithmetic (built on
//! the [`exp`] helper below — no libm call, no table), ~2× cheaper, and **fully deterministic**: the
//! same input gives the same bits on every thread and every run, so the parallel `decide` stays
//! replay-exact within a profile (debug vs release still differ via FMA, as everywhere — each has its
//! own golden).
//!
//! This is an APPROXIMATION, so swapping it into the brain is an INTENDED trajectory change — the
//! golden moves and is re-pinned. Accuracy is kept high (max error below) so the dynamics are
//! preserved: the decision surface shifts only at the ~1e-3 level. The ecosystem stays a living,
//! multicellular boom-bust oscillator (verified across seeds); see
//! `multicellularity_emerges_under_selection`. The terrain's `exp` relaxation laws are deliberately
//! left on libm — that economy is a finely tuned equilibrium and is not the throughput bottleneck.

/// Hyperbolic tangent, max abs error ≈ 3e-4. Exact at `x = 0`, saturates cleanly to ±1.
///
/// `tanh(x) = 1 − 2/(e^{2x} + 1)`, built on the fast [`exp`] below so the accuracy follows exp's.
/// Robust in the tails: `e^{2x} → +inf` gives `1`, `e^{2x} → 0` gives `−1` (no clamp needed). This
/// is the brain activation — the sigmoidal, ±1-saturating shape is what the decision surface needs,
/// and it matches `tanh` to within a few ten-thousandths everywhere.
#[inline]
pub fn tanh(x: f32) -> f32 {
    let e = exp(2.0 * x);
    1.0 - 2.0 / (e + 1.0)
}

/// Natural exponential, max relative error ≈ 2e-4. Exact at `x = 0` (returns `1.0`).
///
/// `e^x = 2^(x·log2 e)`, split into an integer part (built directly into the float exponent bits)
/// and a fractional part in `[0,1)` evaluated by a 5th-order polynomial (the `2^f` series in powers
/// of `ln 2`). All hot callers pass `x ≤ 0` (the relaxation laws `e^(−rate·elapsed)`), where the
/// integer part is `≤ 0` and underflows cleanly to `0` for very stale columns — exactly the
/// `e^(−∞) → 0` the lazy regrow relies on. Defined for `x > 0` too (kept correct for completeness).
#[inline]
pub fn exp(x: f32) -> f32 {
    const LOG2E: f32 = std::f32::consts::LOG2_E;
    let y = x * LOG2E;
    let yf = y.floor();
    let n = yf as i32;
    // Exponent out of `f32`'s normal range: underflow to 0 (stale column) / overflow to +inf.
    if n < -126 {
        return 0.0;
    }
    if n > 127 {
        return f32::INFINITY;
    }
    let f = y - yf; // fractional part in [0, 1)
    // 2^f via the Maclaurin series of 2^f = e^(f·ln2) (coefficients (ln2)^k / k!), Horner form.
    let p = 1.0 + f * (std::f32::consts::LN_2 + f * (0.240_226_5 + f * (0.055_504_1 + f * (0.009_618_1 + f * 0.001_333_4))));
    // 2^n by constructing the IEEE-754 exponent field directly (n in [-126, 127] ⇒ a normal float).
    let pow2n = f32::from_bits(((n + 127) as u32) << 23);
    pow2n * p
}

#[cfg(test)]
#[path = "fastmath_tests.rs"]
mod tests;
