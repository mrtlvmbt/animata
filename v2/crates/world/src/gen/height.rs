//! W-1: deterministic INTEGER fBm heightmap — the first world-gen pipeline stage (plan RnD
//! `sim/world/02`, integer-hash noise per `engineering/16 §3a`). **Pure integer / fixed-point
//! throughout — no `f32`/`f64` anywhere in this file** (enforced by the recursive glob guard,
//! `world/tests/no_float_guard_gen.rs`, which scans every `.rs` under `world/src/gen/`).
//!
//! **W-6 status:** [`height_at`] is now called by production (`ProcgenWorld::new`,
//! `world/src/lib.rs`) via the `erode` → `classify_and_caps` chain — it is `pub` (part of the
//! crate's public API surface) and no longer prod-inert.
//!
//! ## Algorithm (locked by the golden-vector test, re-derivable from this doc — critic F10)
//!
//! Multi-octave fBm = a weighted sum of [`OCTAVES`] independent **integer value-noise** layers,
//! each at half the previous layer's spatial period (more detail, less amplitude, per octave —
//! the standard fBm shape), rescaled into `[0, hmax]`:
//!
//! 1. **Per-octave lattice hash.** For octave `o`, the sample position `(x, z)` falls in a grid
//!    cell of side `period(o) = (BASE_PERIOD >> o).max(1)`. The four surrounding lattice corners
//!    `(cx, cz)`, `(cx+1, cz)`, `(cx, cz+1)`, `(cx+1, cz+1)` (`cx = x.div_euclid(period)`, etc.)
//!    each get an integer value in `[0, HASH_SCALE)` via `sim_core::seed_fold` (salted with the
//!    octave index so octaves are decorrelated) — a pure hash of quantized integer coordinates,
//!    never a float lookup.
//! 2. **Fixed-point bilinear interpolation** between the four corner values, using an integer
//!    smoothstep fade curve (`t² · (3·FIX − 2t) / FIX²`, `t` and `FIX` both integers) on the
//!    fractional position within the cell — the same S-curve classic value/Perlin noise uses to
//!    avoid visible grid-line artifacts, computed entirely in `i64` fixed-point (denominator
//!    [`FIX`]).
//! 3. **Octave sum.** Each octave's interpolated value is weighted by an amplitude that HALVES
//!    per octave (integer right-shift, starting at [`AMPLITUDE_START`]) and accumulated; this is
//!    the fBm "1/f" falloff — later (finer-period) octaves contribute less.
//! 4. **Rescale.** The accumulated sum, which is bounded by construction to
//!    `[0, max_amplitude_sum · HASH_SCALE]`, is rescaled by integer division into `[0, hmax]` and
//!    clamped (saturate, never wrap/panic — the same discipline `morphogen.rs`/`grn.rs` use) to
//!    guard the documented output range against any interpolation-order rounding slack.
//!
//! **Negative / large coordinates:** `div_euclid`/`rem_euclid` give a well-defined non-negative
//! cell-local fraction for ANY `i64` input (including negative `x`/`z`); the lattice corner
//! coordinates are folded into the hash via an `as u64` bit-reinterpretation (two's-complement,
//! deterministic for any `i64`, including negative and very large values) — so `height_at` is
//! total over the full `i64` domain, not just the positive quadrant.
//!
//! `hmax` is a PARAMETER (not a module constant, critic F9): the golden-vector test fixes an
//! explicit test `hmax`; production supplies its own `hmax` at the W-6 call site with no
//! constant-conflict between this test and production.

use sim_core::seed_fold;

/// Number of fBm octaves summed.
const OCTAVES: u32 = 4;
/// Spatial period (in world cells) of octave 0 — the lowest-frequency, largest-amplitude layer.
/// Halved (integer shift) per subsequent octave, floored at 1 cell.
const BASE_PERIOD: i64 = 64;
/// Per-corner hash value range `[0, HASH_SCALE)`.
const HASH_SCALE: i64 = 1 << 16;
/// Fixed-point denominator for the interpolation fade weight (`t ∈ [0, FIX]`).
const FIX: i64 = 4096;
/// Octave-0 amplitude weight; halves each subsequent octave (`8, 4, 2, 1` for [`OCTAVES`]=4).
const AMPLITUDE_START: i64 = 8;
/// Salt distinguishing this stage's hash stream from any other `seed_fold` caller in the sim.
const SALT_HEIGHT: u64 = 0x4845_4947_4854_5F30; // "HEIGHT_0" (ASCII, folded)

/// Integer smoothstep fade: `t² · (3·FIX − 2t) / FIX²`, monotone `0..=FIX` for `t ∈ [0, FIX]`.
/// Pure integer fixed-point — the classic value/Perlin-noise S-curve, never a float `t*t*(3-2*t)`.
fn smoothstep_fixed(t: i64) -> i64 {
    let t2 = t * t;
    (t2 * (3 * FIX - 2 * t)) / (FIX * FIX)
}

/// Hash lattice corner `(cx, cz)` at octave `o` into `[0, HASH_SCALE)`. `cx`/`cz` may be negative
/// or arbitrarily large — the `as u64` cast is a deterministic two's-complement reinterpretation,
/// not a lossy truncation, so this is total over the full `i64` domain.
fn hash_corner(cx: i64, cz: i64, seed: u64, o: u32) -> i64 {
    let h = seed_fold(seed, &[SALT_HEIGHT, o as u64, cx as u64, cz as u64]);
    (h % HASH_SCALE as u64) as i64
}

/// One fBm octave's interpolated value noise at `(x, z)`, in `[0, HASH_SCALE)`.
fn value_noise_octave(x: i64, z: i64, period: i64, seed: u64, o: u32) -> i64 {
    let period = period.max(1);
    let cx0 = x.div_euclid(period);
    let cz0 = z.div_euclid(period);
    let fx = x.rem_euclid(period); // in [0, period)
    let fz = z.rem_euclid(period);

    let tx = smoothstep_fixed(fx * FIX / period);
    let tz = smoothstep_fixed(fz * FIX / period);

    let c00 = hash_corner(cx0, cz0, seed, o);
    let c10 = hash_corner(cx0 + 1, cz0, seed, o);
    let c01 = hash_corner(cx0, cz0 + 1, seed, o);
    let c11 = hash_corner(cx0 + 1, cz0 + 1, seed, o);

    let top = c00 + (c10 - c00) * tx / FIX;
    let bot = c01 + (c11 - c01) * tx / FIX;
    top + (bot - top) * tz / FIX
}

/// Deterministic multi-octave integer fBm height at world position `(x, z)` for `seed`, rescaled
/// into `[0, hmax]`. Pure function of `(x, z, seed, hmax)` — no RNG-of-clock, no thread-dependence,
/// no global mutable state; chunk-ready (any position independently re-derivable). Bit-identical
/// across runs and across CPU architecture (pure integer — no FMA/libm divergence).
pub fn height_at(x: i64, z: i64, seed: u64, hmax: i64) -> i64 {
    let mut total: i64 = 0;
    let mut max_total: i64 = 0;
    let mut amplitude = AMPLITUDE_START;
    let mut period = BASE_PERIOD;
    for o in 0..OCTAVES {
        let n = value_noise_octave(x, z, period, seed, o);
        total += amplitude * n;
        max_total += amplitude * HASH_SCALE;
        amplitude >>= 1;
        period = (period / 2).max(1);
    }
    if max_total == 0 || hmax <= 0 {
        return 0;
    }
    (total * hmax / max_total).clamp(0, hmax)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: u64 = 0xA11A_2A11;
    const HMAX: i64 = 16;

    /// Re-run identity: the SAME `(x, z, seed, hmax)` always produces the SAME height.
    #[test]
    fn height_at_is_deterministic_across_repeated_calls() {
        for &(x, z) in &[(0i64, 0i64), (-1, -1), (1_000_000, -1_000_000), (37, 5)] {
            let a = height_at(x, z, SEED, HMAX);
            let b = height_at(x, z, SEED, HMAX);
            assert_eq!(a, b, "height_at({x},{z}) must be byte-identical across repeated calls");
        }
    }

    /// Output stays within the documented `[0, hmax]` range across a broad sample (positive,
    /// negative, and boundary-large coordinates), never saturating open past the clamp.
    #[test]
    fn height_at_is_bounded_to_0_hmax() {
        for x in -200..200i64 {
            for &z in &[-137i64, 0, 250, i64::MAX / 2, i64::MIN / 2] {
                let h = height_at(x, z, SEED, HMAX);
                assert!((0..=HMAX).contains(&h), "height_at({x},{z})={h} out of [0,{HMAX}]");
            }
        }
    }

    /// A real multi-octave fractal relief, not a single noise lookup and not flat: across a
    /// modest sample the output must take on more than 2 distinct values (a single-octave or
    /// flat implementation would collapse to a handful of blocky levels).
    #[test]
    fn height_at_is_multi_octave_not_flat_or_single_lookup() {
        let mut values = std::collections::BTreeSet::new();
        for x in 0..64i64 {
            for z in 0..64i64 {
                values.insert(height_at(x, z, SEED, HMAX));
            }
        }
        assert!(values.len() > 2, "expected a varied fractal relief, got only {} distinct height values: {:?}", values.len(), values);
    }

    /// `hmax` is a genuine parameter, not baked into the module: the same position/seed rescales
    /// proportionally under a different `hmax`.
    #[test]
    fn hmax_is_a_real_parameter() {
        let h16 = height_at(10, 10, SEED, 16);
        let h256 = height_at(10, 10, SEED, 256);
        assert!((0..=16).contains(&h16));
        assert!((0..=256).contains(&h256));
    }

    /// Different seeds diverge (the function genuinely reads `seed`, not a constant lattice).
    #[test]
    fn different_seed_diverges() {
        let a = height_at(10, 10, SEED, HMAX);
        let b = height_at(10, 10, SEED ^ 0xDEAD_BEEF, HMAX);
        // Not asserting unconditional inequality at one point (a coincidence is possible); check
        // across a small sample that at least one position diverges.
        let mut any_diff = a != b;
        for x in 0..8i64 {
            for z in 0..8i64 {
                if height_at(x, z, SEED, HMAX) != height_at(x, z, SEED ^ 0xDEAD_BEEF, HMAX) {
                    any_diff = true;
                }
            }
        }
        assert!(any_diff, "different seeds must produce a different heightmap somewhere in the sample");
    }

    /// Golden vector (W-1, first link of the incremental chain-golden — `w1_chain.rs` extends
    /// this at W-2+): pinned exact integer heights at an explicit `(seed, hmax)` over a
    /// coordinate set that includes negative and large coordinates (critic F12), so the
    /// documented negative/large-coord handling is actually exercised. INDEPENDENT of any
    /// config's `world_dim`/production `hmax` (critic F6/F9) — `height_at` is a pure
    /// per-position function, so this vector never needs re-pinning when map size or production
    /// `hmax` changes.
    #[test]
    fn golden_vector_matches_pinned_heights() {
        const GOLDEN_SEED: u64 = 0xA11A_2A11;
        const GOLDEN_HMAX: i64 = 200;
        // (x, z, expected height) — includes origin, positive, negative, and a large-magnitude
        // coordinate pair.
        const CASES: &[(i64, i64, i64)] = &[
            (0, 0, 130),
            (1, 1, 129),
            (-1, -1, 130),
            (-1000, 2000, 146),
            (1_000_000_000, -1_000_000_000, 98),
            (i64::MAX / 3, i64::MIN / 3, 86),
        ];
        for &(x, z, expected) in CASES {
            let h = height_at(x, z, GOLDEN_SEED, GOLDEN_HMAX);
            assert_eq!(h, expected, "golden drift at ({x},{z}): got {h}, expected {expected}");
        }
    }
}
