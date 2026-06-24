//! animata v2 `brain` — **fixed-point INTEGER neural inference** (M3 / D-Brain-1,3).
//!
//! The hot path is PURE INTEGER (no `f32`/`f64` anywhere in `src/` — enforced by the zero-float guard
//! test, the same shield M0 put on `sim-core`). Integer multiply-add is associative ⇒ the sum order
//! is irrelevant absent overflow ⇒ batched/threaded inference replays bit-for-bit on every arch
//! (R19). Float would not: x86↔arm64 disagree on FMA fusing and on a per-element dequant multiply.
//!
//! ## Topology (fixed; only the weights evolve — D-Brain-1)
//! `I=BRAIN_INPUTS` sensor inputs → `H=BRAIN_HIDDEN` **recurrent** hidden units → `O=BRAIN_OUTPUTS`
//! motor outputs. The per-creature weight vector ([`sim_core::BRAIN_WEIGHTS`] `int8`s) packs three
//! dense blocks, laid out so [`weight_index`] addresses them without a per-tick repack:
//! * `W_ih[j][i]`  (`H·I`) — input→hidden,           index `j*I + i`
//! * `W_hh[j][k]`  (`H·H`) — hidden→hidden RECURRENT, index `H*I + j*H + k`
//! * `W_ho[o][j]`  (`O·H`) — hidden→output,           index `H*I + H*H + o*H + j`
//!
//! ## Fixed-point representation (D-Brain-3)
//! * weights `int8`, Q1.7 (real ∈ [−1, +0.992]);
//! * inputs / hidden / outputs `FixedI16`, **Q8.8** (real = value/256);
//! * a product `w·x` is Q1.7·Q8.8 = scale `2^15`; the dot product accumulates in a wide **`i64`**;
//! * **rescale is an integer SHIFT** `acc >> BRAIN_SHIFT` (`BRAIN_SHIFT == 7`), returning Q8.8 — NOT a
//!   float multiplier (F11). `127·256` (=`2^15`) `>> 7` = `2^8`, i.e. Q8.8 back.
//!
//! ## Execution scheme — full double-buffer (D-Brain-2)
//! All RECURRENT edges read `h_old` and write `h_new`; no topological sort, trivially order-free. The
//! feed-forward output block reads the just-computed `h_new` (well-defined within one `infer` call,
//! still deterministic — there is no cycle there). The caller swaps `h_old`⇄`h_new` only on Brain
//! ticks (1/K). Cost: a signal crosses one layer per Brain tick (acceptable).
//!
//! ## Overflow — proven impossible by width (D-Brain-5 / F3)
//! Peak hidden accumulator = `fan_in · max|w| · max|x|` with `fan_in = I + H = 14`, `max|w| = 127`,
//! and `max|x| = 32767` (a full `i16`): `14 · 127 · 32767 = 58_259_726 < 2^31` — fits even an `i32`,
//! so `i64` has ≈157 bits of headroom. The output block (`fan_in = H = 8`) is smaller still. Hence
//! overflow is **impossible** for this topology; the guard is a `debug_assert!` (an always-on panic
//! would itself be a non-deterministic run-killer; saturate/wrap would break associativity → R19),
//! backed by ONE always-on test at the calibrated maximum input ([`tests`]).

mod lut;

pub use lut::{LUT_BIN, PREACT_MAX, PREACT_MIN, TANH_LUT, TANH_LUT_SHA256};

use sim_core::{
    brain_w_hh as w_hh, brain_w_ho as w_ho, brain_w_ih as w_ih, Brain, BRAIN_HIDDEN as H,
    BRAIN_INPUTS as I, BRAIN_OUTPUTS as O, BRAIN_SHIFT, BRAIN_WEIGHTS,
};

/// Conservative `i64` accumulator bound for the `debug_assert!` overflow guard (see module docs). The
/// real peak is far below this; we assert against a generous, obviously-safe ceiling.
const ACC_BOUND: i64 = (I as i64 + H as i64) * 127 * (i16::MAX as i64) + 1;

/// Activation φ = tanh via the committed integer LUT. `preact` is Q8.8; out-of-range CLAMPS (never
/// wraps) to the table ends; the result is Q8.8 tanh ∈ [−256, 256].
#[inline]
pub fn activate(preact: i64) -> i16 {
    let clamped = preact.clamp(PREACT_MIN, PREACT_MAX);
    let idx = ((clamped - PREACT_MIN) / LUT_BIN) as usize;
    TANH_LUT[idx]
}

/// The fixed-topology integer brain. Zero-sized — all state is per-creature (`weights`, `h`), passed
/// in by the caller; one boxed instance is shared by the whole population (R1).
#[derive(Clone, Copy, Debug, Default)]
pub struct FixedBrain;

impl FixedBrain {
    pub const fn new() -> Self {
        FixedBrain
    }
}

impl Brain for FixedBrain {
    fn infer(
        &self,
        inputs: &[i16; I],
        h_old: &[i16; H],
        weights: &[i8; BRAIN_WEIGHTS],
        h_new: &mut [i16; H],
        out: &mut [i16; O],
    ) {
        // Recurrent hidden layer: every edge reads `h_old`, writes `h_new` (full double-buffer).
        for j in 0..H {
            let mut acc: i64 = 0;
            for i in 0..I {
                acc += weights[w_ih(j, i)] as i64 * inputs[i] as i64;
            }
            for k in 0..H {
                acc += weights[w_hh(j, k)] as i64 * h_old[k] as i64;
            }
            debug_assert!(acc.abs() < ACC_BOUND, "brain hidden accumulator overflow guard (j={j})");
            h_new[j] = activate(acc >> BRAIN_SHIFT);
        }
        // Feed-forward output layer: reads the just-computed `h_new` (no cycle → order-deterministic).
        for o in 0..O {
            let mut acc: i64 = 0;
            for j in 0..H {
                acc += weights[w_ho(o, j)] as i64 * h_new[j] as i64;
            }
            debug_assert!(acc.abs() < ACC_BOUND, "brain output accumulator overflow guard (o={o})");
            out[o] = activate(acc >> BRAIN_SHIFT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    /// CI LUT verification (F6): a pure integer SHA-256 over the LE-`i16` bytes equals the committed
    /// checksum. The float generator never runs here — this is arch-identical and runs on both arches.
    #[test]
    fn lut_checksum_matches_committed() {
        let mut hasher = Sha256::new();
        for v in TANH_LUT {
            hasher.update(v.to_le_bytes());
        }
        let digest = hasher.finalize();
        assert_eq!(format!("{digest:x}"), TANH_LUT_SHA256, "TANH_LUT drifted from its committed checksum");
    }

    /// The LUT domain constants are self-consistent (exactly 512 bins span [PREACT_MIN, PREACT_MAX]).
    #[test]
    fn lut_domain_is_consistent() {
        assert_eq!((PREACT_MAX - PREACT_MIN) / LUT_BIN, (TANH_LUT.len() - 1) as i64);
        assert_eq!(activate(0), 0); // tanh(0) = 0
        assert_eq!(activate(PREACT_MIN), TANH_LUT[0]);
        assert_eq!(activate(PREACT_MAX), TANH_LUT[TANH_LUT.len() - 1]);
        assert_eq!(activate(i64::MIN), TANH_LUT[0], "below-range clamps, never wraps");
        assert_eq!(activate(i64::MAX), TANH_LUT[TANH_LUT.len() - 1], "above-range clamps, never wraps");
    }

    /// Always-on overflow witness (D-Brain-5): drive EVERY weight and input to its calibrated extreme
    /// and confirm the `i64` accumulator stays inside the proven bound (no wrap, no panic).
    #[test]
    fn accumulator_never_overflows_at_calibrated_max() {
        let weights = [i8::MAX; BRAIN_WEIGHTS];
        let inputs = [i16::MAX; I];
        let h_old = [i16::MAX; H];
        // Reproduce the worst-case hidden accumulation explicitly and assert the proven ceiling.
        let mut peak: i64 = 0;
        for i in 0..I {
            peak += weights[w_ih(0, i)] as i64 * inputs[i] as i64;
        }
        for k in 0..H {
            peak += weights[w_hh(0, k)] as i64 * h_old[k] as i64;
        }
        assert!(peak < ACC_BOUND, "calibrated-max accumulator {peak} exceeded bound {ACC_BOUND}");
        assert!(peak < i32::MAX as i64, "even i32 must suffice for this topology (headroom proof)");
        // And the real call must not panic at the extreme.
        let mut h_new = [0i16; H];
        let mut out = [0i16; O];
        FixedBrain.infer(&inputs, &h_old, &weights, &mut h_new, &mut out);
    }

    /// Inference is a pure function of (inputs, h_old, weights): batching/reordering cannot change a
    /// creature's result, because each creature's dot products are independent and integer-associative.
    /// (The sim-side 1-vs-N batch determinism test rides on this; here we prove the kernel itself.)
    #[test]
    fn infer_is_deterministic_and_order_free() {
        let mut weights = [0i8; BRAIN_WEIGHTS];
        for (n, w) in weights.iter_mut().enumerate() {
            *w = ((n as i64 * 37 - 50) % 100 - 50) as i8;
        }
        let inputs = [120i16, -90, 30, -200, 256, -16];
        let h_old = [10i16, -20, 30, -40, 50, -60, 70, -80];

        let run = || {
            let mut hn = [0i16; H];
            let mut out = [0i16; O];
            FixedBrain.infer(&inputs, &h_old, &weights, &mut hn, &mut out);
            (hn, out)
        };
        assert_eq!(run(), run(), "infer must be a pure deterministic function");

        // Parallel batch of identical inputs ⇒ identical results regardless of thread scheduling.
        use rayon::prelude::*;
        let serial: Vec<_> = (0..256).map(|_| run()).collect();
        let parallel: Vec<_> = (0..256).into_par_iter().map(|_| run()).collect();
        assert_eq!(serial, parallel, "batched inference diverged from serial (R19 kernel)");
    }
}
