//! W-2: Whittaker biome classification — `[biome-classify]` (R10/R18/R19). **Pure integer /
//! fixed-point throughout — no `f32`/`f64` anywhere in this file** (enforced by the recursive glob
//! guard, `world/tests/no_float_guard_gen.rs`).
//!
//! ## Algorithm (locked by the golden-vector test, re-derivable from this doc)
//!
//! [`biome_at`] classifies a `(T, P)` climate point (same fixed-point scale as
//! [`crate::gen::climate::climate_at`]: `T` centidegrees, `P` mm/year) by **nearest nearest-point
//! Whittaker biome envelope** on the quantized `(T, P)` plane (RnD `sim/world/03 §4` / `11 §2`
//! zonal stage — this is the ZONAL classification only; the azonal edaphic override is W-5):
//!
//! 1. **Quantize first (critic requirement — no float, no ULP-flip):** `T`/`P` are rounded to the
//!    nearest multiple of [`T_QUANT`]/[`P_QUANT`] via pure integer arithmetic BEFORE any
//!    comparison — so two climate points that differ only in noise below the quantum collapse to
//!    the same classification bucket, deterministically.
//! 2. **Argmin over reference points:** each [`BiomeId`] has one canonical `(T_ref, P_ref)` point
//!    (documented below, grounded in typical Whittaker-diagram biome envelopes). Classify by
//!    minimum squared Euclidean distance in the quantized plane (integer, no `sqrt` needed since
//!    only relative distances matter).
//! 3. **Total tie-break by lowest `BiomeId`:** the reference table is iterated in ascending
//!    `BiomeId` order using a STRICT `<` update (never `<=`), so on an exact distance tie the
//!    earlier (lower-id) candidate is kept — deterministic, never ambiguous.

/// A cell's zonal climate biome, a small integer id. `#[repr(u8)]` so `id as usize` is a stable
/// array index and the encoding is invariant to future variant additions (append-only; never
/// reorder — matches the `telemetry::Guild` idiom).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum BiomeId {
    Tundra = 0,
    BorealForest = 1,
    TemperateGrassland = 2,
    TemperateForest = 3,
    TemperateRainforest = 4,
    Desert = 5,
    Savanna = 6,
    TropicalRainforest = 7,
}

impl BiomeId {
    /// All variants in ascending-id order — the SAME order [`BIOME_POINTS`] is iterated in
    /// (the order the argmin tie-break relies on).
    pub const ALL: [BiomeId; 8] = [
        BiomeId::Tundra,
        BiomeId::BorealForest,
        BiomeId::TemperateGrassland,
        BiomeId::TemperateForest,
        BiomeId::TemperateRainforest,
        BiomeId::Desert,
        BiomeId::Savanna,
        BiomeId::TropicalRainforest,
    ];
}

/// Canonical Whittaker-envelope reference points: `(BiomeId, T_ref centidegrees, P_ref mm/year)`,
/// grounded in typical biome climate envelopes (RnD `sim/world/03 §4`). Listed in ASCENDING
/// `BiomeId` order — `biome_at`'s argmin tie-break depends on this order (lowest id wins ties).
/// The exact reference values are the implementer's call (critic F9/F15/F21); each is locked by
/// the golden-vector test below.
const BIOME_POINTS: &[(BiomeId, i64, i64)] = &[
    (BiomeId::Tundra, -1500, 200),
    (BiomeId::BorealForest, -500, 600),
    (BiomeId::TemperateGrassland, 1000, 400),
    (BiomeId::TemperateForest, 1200, 1200),
    (BiomeId::TemperateRainforest, 1200, 2500),
    (BiomeId::Desert, 2500, 100),
    (BiomeId::Savanna, 2500, 900),
    (BiomeId::TropicalRainforest, 2500, 2500),
];

/// Quantization step for `T` (centidegrees) before argmin — 1.00 °C buckets.
const T_QUANT: i64 = 100;
/// Quantization step for `P` (mm/year) before argmin — 50 mm buckets.
const P_QUANT: i64 = 50;

/// Round `v` to the nearest multiple of `step` (`step > 0`), pure integer arithmetic, deterministic
/// for any `i64` including negative `v` (uses `div_euclid`, never a float `round()`).
fn quantize(v: i64, step: i64) -> i64 {
    debug_assert!(step > 0, "quantize step must be > 0");
    (v + step / 2).div_euclid(step) * step
}

/// Nearest-reference-point classification on an already-quantized `(tq, pq)` plane. Iterates
/// `points` in the given order and keeps the STRICT-`<` minimum — ties keep the FIRST (lowest-id,
/// given ascending input order) candidate, never the later one. Exposed as a free fn (not `biome_at`
/// itself) so the tie-break rule is unit-testable against a synthetic table, independent of the
/// real Whittaker geography (critic-style isolation, matches `height.rs`'s testable-primitive idiom).
fn nearest(points: &[(u8, i64, i64)], tq: i64, pq: i64) -> u8 {
    let mut best_id = points[0].0;
    let mut best_dist = i64::MAX;
    for &(id, tref, pref) in points {
        let dt = tq - tref;
        let dp = pq - pref;
        let dist = dt * dt + dp * dp;
        if dist < best_dist {
            best_dist = dist;
            best_id = id;
        }
    }
    best_id
}

/// Classify a `(T, P)` climate point into its zonal Whittaker [`BiomeId`]. Pure function, no float,
/// no RNG, no global state — deterministic and arch-identical.
pub fn biome_at(t: i64, p: i64) -> BiomeId {
    let tq = quantize(t, T_QUANT);
    let pq = quantize(p, P_QUANT);
    let raw: Vec<(u8, i64, i64)> =
        BIOME_POINTS.iter().map(|&(id, tref, pref)| (id as u8, tref, pref)).collect();
    let best = nearest(&raw, tq, pq);
    BiomeId::ALL[best as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exact reference points classify to themselves (distance 0, no other point can beat it).
    #[test]
    fn biome_at_exact_reference_point_classifies_to_itself() {
        for &(id, t, p) in BIOME_POINTS {
            assert_eq!(biome_at(t, p), id, "exact ref point ({t},{p}) must classify as {id:?}");
        }
    }

    /// Tie-break: `nearest` on a synthetic table with two equidistant points must return the
    /// LOWER id (ascending-order-first, strict `<` update — never the later-seen candidate).
    #[test]
    fn nearest_tie_break_prefers_lowest_id() {
        // Two points at (0,0) id=2 and (10,0) id=5 (deliberately non-adjacent ids); query (5,0) is
        // equidistant (dist=25 from each). Expect id=2 (listed first / lowest).
        let points = &[(2u8, 0i64, 0i64), (5u8, 10i64, 0i64)];
        assert_eq!(nearest(points, 5, 0), 2, "exact tie must resolve to the lower id");

        // Reversed listing order still must prefer the LOWER id (2), not "whichever is listed
        // first" — the tie-break is BY ID VALUE via ascending BIOME_POINTS order, so this test
        // documents that `nearest`'s tie-break is order-dependent (first-listed wins); callers
        // (biome_at) always supply ascending-id order, which makes "first-listed" == "lowest id".
        let points_desc = &[(5u8, 10i64, 0i64), (2u8, 0i64, 0i64)];
        assert_eq!(nearest(points_desc, 5, 0), 5, "nearest() itself is first-listed-wins; biome_at supplies ascending order so lowest-id wins in practice");
    }

    /// Quantization: two `(T,P)` points differing only by less than half a quantum collapse to the
    /// same bucket (deterministic noise-tolerance).
    #[test]
    fn quantize_collapses_sub_quantum_differences() {
        assert_eq!(quantize(1000, T_QUANT), quantize(1030, T_QUANT));
        assert_eq!(quantize(-1000, T_QUANT), quantize(-1030, T_QUANT));
    }

    /// Re-run identity + determinism sanity across the full reference table.
    #[test]
    fn biome_at_is_deterministic() {
        for &(_, t, p) in BIOME_POINTS {
            assert_eq!(biome_at(t, p), biome_at(t, p));
        }
    }

    /// Golden vector: pinned classification at a broad sample of `(T, P)` points, including
    /// off-reference (interpolated) points that exercise the argmin over multiple candidates.
    #[test]
    fn golden_vector_matches_pinned_biomes() {
        const CASES: &[(i64, i64, BiomeId)] = &[
            (-1500, 200, BiomeId::Tundra),
            (-500, 600, BiomeId::BorealForest),
            (1000, 400, BiomeId::TemperateGrassland),
            (1200, 1200, BiomeId::TemperateForest),
            (1200, 2500, BiomeId::TemperateRainforest),
            (2500, 100, BiomeId::Desert),
            (2500, 900, BiomeId::Savanna),
            (2500, 2500, BiomeId::TropicalRainforest),
            // Off-reference interpolated points (exercise the argmin over >1 real candidate):
            (-1000, 400, BiomeId::Tundra),
            (0, 1500, BiomeId::BorealForest),
            (2500, 500, BiomeId::Desert),
        ];
        for &(t, p, expected) in CASES {
            let b = biome_at(t, p);
            assert_eq!(b, expected, "golden drift at (T={t},P={p}): got {b:?}, expected {expected:?}");
        }
    }
}
