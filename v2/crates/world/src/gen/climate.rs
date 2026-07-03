//! W-2: deterministic INTEGER climate fields — the second world-gen pipeline stage (RnD
//! `sim/world/03`). **Pure integer / fixed-point throughout — no `f32`/`f64` anywhere in this
//! file** (enforced by the recursive glob guard, `world/tests/no_float_guard_gen.rs`, extended in
//! this slice to also fail on bare float literals and scientific notation).
//!
//! **W-6 status:** [`climate_at`]'s extracted core, [`climate_from_height`], is now called by
//! production via `gen::caps::classify_and_caps` (re-classifying the biome on the post-erosion
//! surface); `climate_at` itself remains available for any per-position (infinite-domain) use.
//!
//! ## Fixed-point scale (documented, critic F3/F4/F5)
//!
//! - **Temperature `T`** is in **centidegrees Celsius**: `T = 1500` means 15.00 °C. Range is
//!   unbounded in principle (integer arithmetic never saturates here) but in practice spans
//!   roughly `-3600..1600` (-36.00 °C .. 16.00 °C) for the constants below at `hmax=200`.
//! - **Precipitation `P`** is in **integer millimeters per year**: `P = 900` means 900 mm/yr.
//!   Clamped to `≥ 0` (negative precipitation is not physical).
//! - **Cells-to-km scale**: this module does NOT fix a cells↔km mapping — `LAT_PERIOD` (the
//!   pole-to-pole gradient period, in world cells) and `WIND_DX` (the orographic sampling offset,
//!   in world cells) are the only length-scale constants, and are documented at their definition.
//!   Both are `implementer's call` constants (critic F9/F15/F18) locked by the golden-vector test.
//!
//! ## Algorithm (locked by the golden-vector test, re-derivable from this doc — critic F10)
//!
//! `T(x,z) = latitude_term(z) + altitude_lapse(height_at(x,z)) + noise_term(x,z, SALT_T)`
//! `P(x,z) = P_BASE + orographic_term(height_at(x,z), height_at(x−WIND_DX,z)) + noise_term(x,z, SALT_P)`, clamped `≥0`.
//!
//! - **Latitude gradient**: a triangular wave over `z` with period [`LAT_PERIOD`] cells — warmest
//!   ("equator") at `z ≡ 0 (mod LAT_PERIOD)`, coldest ("poles") at `z ≡ LAT_PERIOD/2`. `T` falls
//!   linearly from [`T_BASE`] at the equator to `T_BASE − LAT_AMPLITUDE` at the poles.
//! - **Altitude lapse**: `ΔT = −height · LAPSE_NUM / LAPSE_DEN` (integer divide, truncating toward
//!   zero per Rust's default `/` — critic F16). Higher terrain is colder, matching the real
//!   troposphere lapse rate (~0.65 °C/100 m), scaled onto the W-1 `height_at` integer unit.
//! - **Orographic rain-shadow**: precipitation is driven by the height SLOPE along a FIXED wind
//!   direction (+x, documented, critic F9/F18 — wind-direction scheme is the implementer's call).
//!   `Δh = height_at(x,z) − height_at(x−WIND_DX,z)`: `Δh > 0` = windward/rising air (orographic
//!   lift → more rain); `Δh < 0` = leeward/descending air (rain shadow → less rain, may go dry).
//!   `P` gets `+ Δh · OROG_NUM / OROG_DEN` (integer divide, remainder discarded).
//! - **Noise**: a deterministic hash of `(x, z, seed)` via [`sim_core::seed_fold`] (same primitive
//!   `height_at` uses) — NEVER RNG/thread/clock state (critic F8/F19) — centered at 0, added to
//!   both `T` and `P` independently (disjoint salts).
//!
//! **Ecological-realism caveat (critic F6, accepted tradeoff, per issue #209):** precipitation
//! here comes ONLY from latitude + orography — no moisture circulation until W-3 hydrology — so
//! the biome map derived from this climate may be locally implausible (e.g. a wet valley next to a
//! rain-shadow desert) until W-3 lands. This is fine: W-2's contract is determinism + arch-identity
//! of a well-defined function, not ecological plausibility (W-3/W-5 complete that).

use crate::gen::height::height_at;
use sim_core::seed_fold;

/// Pole-to-pole-to-equator gradient period, in world cells (a length-scale constant — critic F9).
/// `z ≡ 0 (mod LAT_PERIOD)` is "equator" (warmest); `z ≡ LAT_PERIOD/2` is "pole" (coldest).
const LAT_PERIOD: i64 = 4096;
/// Equator-to-pole temperature swing, in centidegrees (30.00 °C).
const LAT_AMPLITUDE: i64 = 3000;
/// Baseline sea-level equatorial temperature, in centidegrees (15.00 °C).
const T_BASE: i64 = 1500;

/// Altitude lapse rate numerator/denominator: `ΔT = −height · LAPSE_NUM / LAPSE_DEN` centidegrees
/// per `height_at` integer unit. `height_at` returns `[0, hmax]`; this constant is chosen so the
/// full-`hmax` lapse swing stays comparable to (not dominating) [`LAT_AMPLITUDE`] at the test
/// `hmax=200` fixture (a real troposphere lapse of ~0.65 °C/100 m would need a per-unit calibration
/// tied to production `hmax`, which is a W-6 concern — `hmax` stays a parameter here, never
/// hardcoded, critic F17). Deliberately gentle for this baseline: `LAPSE_NUM=10` ⇒ at `hmax=200`
/// the max lapse is `−2000` centidegrees (−20.00 °C), well inside the `±LAT_AMPLITUDE` envelope.
const LAPSE_NUM: i64 = 10;
const LAPSE_DEN: i64 = 1;

/// Windward sampling offset, in world cells, for the orographic slope (fixed wind direction: +x).
/// `pub(crate)` (not private): W-5's `classify_and_caps` re-derives the SAME upwind offset on the
/// post-erosion (finite-grid) heightmap via [`climate_from_height`], so it must read the identical
/// constant rather than risk drifting a duplicated copy. Visibility-only change — no behavior here.
pub(crate) const WIND_DX: i64 = 4;
/// Baseline precipitation at zero slope, in mm/year.
const P_BASE: i64 = 900;
/// Orographic term numerator/denominator: `+ Δh · OROG_NUM / OROG_DEN` mm/year per unit slope.
const OROG_NUM: i64 = 5;
const OROG_DEN: i64 = 1;

/// Noise amplitude added to `T` (centidegrees, symmetric around 0: `±NOISE_T_AMPLITUDE`).
const NOISE_T_AMPLITUDE: i64 = 100;
/// Noise amplitude added to `P` (mm/year, symmetric around 0).
const NOISE_P_AMPLITUDE: i64 = 100;

/// Salts distinguishing the climate noise streams from `height_at`'s `SALT_HEIGHT` and from each
/// other (T noise ≠ P noise ⇒ uncorrelated draws, R14 discipline).
const SALT_CLIMATE_T: u64 = 0x434C_4954_5F54_0000; // "CLIT_T\0\0" (ASCII, folded)
const SALT_CLIMATE_P: u64 = 0x434C_4954_5F50_0000; // "CLIT_P\0\0" (ASCII, folded)

/// Triangular latitude wave: `T_BASE` at `z ≡ 0 (mod LAT_PERIOD)`, `T_BASE − LAT_AMPLITUDE` at the
/// half-period point. `rem_euclid` gives a well-defined fold for any `i64` z (negative included).
fn latitude_term(z: i64) -> i64 {
    let half = LAT_PERIOD / 2;
    let zm = z.rem_euclid(LAT_PERIOD);
    let dist_from_equator = if zm <= half { zm } else { LAT_PERIOD - zm }; // 0=equator, half=pole
    T_BASE - (dist_from_equator * LAT_AMPLITUDE / half)
}

/// Deterministic hash-based noise term, centered at 0, in `[-amplitude, amplitude]`. A pure
/// function of `(x, z, seed, salt)` via `seed_fold` — never RNG/thread/clock state (critic F8/F19).
fn noise_term(x: i64, z: i64, seed: u64, salt: u64, amplitude: i64) -> i64 {
    if amplitude <= 0 {
        return 0;
    }
    let h = seed_fold(seed, &[salt, x as u64, z as u64]);
    let span = (2 * amplitude + 1) as u64;
    (h % span) as i64 - amplitude
}

/// Pure climate core (W-5 critic F2 extract): takes the cell's height and its upwind neighbor's
/// height EXPLICITLY (rather than calling `height_at` internally), so it can be reused on ANY
/// heightmap — in particular W-5's POST-erosion (finite `dim×dim`) height field, where `height_at`
/// (infinite-`i64`-domain) cannot be called directly. [`climate_at`] delegates here after looking
/// up both heights from `height_at`; this extract-and-delegate split is BEHAVIOR-IDENTICAL to the
/// original inline formula (verified by this module's own tests + the `w2_chain.rs` golden, which
/// must stay byte-identical).
///
/// Returns `(temperature_centidegrees, precipitation_mm_per_year)`.
pub fn climate_from_height(h_cell: i64, h_west: i64, x: i64, z: i64, seed: u64) -> (i64, i64) {
    let t_lat = latitude_term(z);
    let t_lapse = -(h_cell * LAPSE_NUM / LAPSE_DEN);
    let t_noise = noise_term(x, z, seed, SALT_CLIMATE_T, NOISE_T_AMPLITUDE);
    let t = t_lat + t_lapse + t_noise;

    let slope = h_cell - h_west; // >0 windward lift, <0 leeward rain-shadow
    let p_orog = slope * OROG_NUM / OROG_DEN;
    let p_noise = noise_term(x, z, seed, SALT_CLIMATE_P, NOISE_P_AMPLITUDE);
    let p = (P_BASE + p_orog + p_noise).max(0);

    (t, p)
}

/// Deterministic integer climate `(T, P)` at world position `(x, z)` for `seed`, reading the W-1
/// heightmap via `height_at(.., hmax)` then delegating to [`climate_from_height`]. Pure function —
/// no RNG-of-clock, no thread-dependence, no global mutable state; chunk-ready (any position
/// independently re-derivable). Bit-identical across runs and across CPU architecture (pure
/// integer — no FMA/libm divergence).
///
/// Returns `(temperature_centidegrees, precipitation_mm_per_year)`.
pub fn climate_at(x: i64, z: i64, seed: u64, hmax: i64) -> (i64, i64) {
    let h = height_at(x, z, seed, hmax);
    let h_west = height_at(x - WIND_DX, z, seed, hmax);
    climate_from_height(h, h_west, x, z, seed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: u64 = 0xA11A_2A11;
    const HMAX: i64 = 200;

    /// Re-run identity: the SAME `(x, z, seed, hmax)` always produces the SAME `(T, P)`.
    #[test]
    fn climate_at_is_deterministic_across_repeated_calls() {
        for &(x, z) in &[(0i64, 0i64), (-1, -1), (1_000_000, -1_000_000), (37, 5)] {
            let a = climate_at(x, z, SEED, HMAX);
            let b = climate_at(x, z, SEED, HMAX);
            assert_eq!(a, b, "climate_at({x},{z}) must be byte-identical across repeated calls");
        }
    }

    /// Unit test proving the LAPSE + OROGRAPHY arithmetic (not just "not-zero" — critic requirement):
    /// hand-recompute `T`/`P` from the documented formula using the SAME `height_at` the function
    /// reads, and assert `climate_at` matches exactly.
    #[test]
    fn climate_at_matches_hand_computed_lapse_and_orography() {
        let (x, z) = (10i64, 0i64);
        let h = height_at(x, z, SEED, HMAX);
        let h_west = height_at(x - WIND_DX, z, SEED, HMAX);

        let expected_t = latitude_term(z)
            - (h * LAPSE_NUM / LAPSE_DEN)
            + noise_term(x, z, SEED, SALT_CLIMATE_T, NOISE_T_AMPLITUDE);
        let slope = h - h_west;
        let expected_p = (P_BASE
            + slope * OROG_NUM / OROG_DEN
            + noise_term(x, z, SEED, SALT_CLIMATE_P, NOISE_P_AMPLITUDE))
        .max(0);

        let (t, p) = climate_at(x, z, SEED, HMAX);
        assert_eq!(t, expected_t, "T must match the hand-computed lapse+latitude+noise formula");
        assert_eq!(p, expected_p, "P must match the hand-computed orography+base+noise formula");
    }

    /// Latitude gradient: temperature falls toward the poles (z near LAT_PERIOD/2) relative to the
    /// equator (z=0), holding x/height roughly comparable via the SAME x.
    #[test]
    fn latitude_term_falls_toward_poles() {
        let equator = latitude_term(0);
        let pole = latitude_term(LAT_PERIOD / 2);
        assert!(pole < equator, "pole T ({pole}) must be colder than equator T ({equator})");
        assert_eq!(equator, T_BASE);
        assert_eq!(pole, T_BASE - LAT_AMPLITUDE);
    }

    /// Altitude lapse: taller terrain is colder, all else equal — construct two positions and
    /// compare their lapse contribution directly (isolating the term from latitude/noise).
    #[test]
    fn altitude_lapse_is_colder_at_height() {
        let height_low = 1i64;
        let height_high = 100i64;
        let lapse_low = -(height_low * LAPSE_NUM / LAPSE_DEN);
        let lapse_high = -(height_high * LAPSE_NUM / LAPSE_DEN);
        assert!(lapse_high < lapse_low, "higher terrain must have a more negative lapse contribution");
    }

    /// Precipitation never goes negative (physically required clamp).
    #[test]
    fn precipitation_is_never_negative() {
        for x in -50..50i64 {
            for z in (-50..50i64).step_by(7) {
                let (_, p) = climate_at(x, z, SEED, HMAX);
                assert!(p >= 0, "precipitation must be clamped >=0, got {p} at ({x},{z})");
            }
        }
    }

    /// Different seeds diverge (the function genuinely reads `seed`, not a constant field).
    #[test]
    fn different_seed_diverges() {
        let mut any_diff = false;
        for x in 0..8i64 {
            for z in 0..8i64 {
                if climate_at(x, z, SEED, HMAX) != climate_at(x, z, SEED ^ 0xDEAD_BEEF, HMAX) {
                    any_diff = true;
                }
            }
        }
        assert!(any_diff, "different seeds must produce different climate somewhere in the sample");
    }

    /// Golden vector: pinned exact `(T, P)` at an explicit `(seed, hmax)` over a coordinate set
    /// including negative and large coordinates. INDEPENDENT of any config's `world_dim` (critic
    /// F2) — `climate_at` is a pure per-position function like `height_at`.
    #[test]
    fn golden_vector_matches_pinned_climate() {
        const GOLDEN_SEED: u64 = 0xA11A_2A11;
        const GOLDEN_HMAX: i64 = 200;
        const CASES: &[(i64, i64, i64, i64)] = &[
            (0, 0, 243, 911),
            (1, 1, 203, 892),
            (-1, -1, 149, 848),
            (-1000, 2000, -2803, 831),
            (1_000_000_000, -1_000_000_000, -1647, 892),
        ];
        for &(x, z, expected_t, expected_p) in CASES {
            let (t, p) = climate_at(x, z, GOLDEN_SEED, GOLDEN_HMAX);
            assert_eq!((t, p), (expected_t, expected_p), "golden drift at ({x},{z})");
        }
    }
}
