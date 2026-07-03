//! W-3: edaphic (valley) moisture — derived from the drainage area ([`crate::gen::drainage`]).
//! **Pure integer / fixed-point throughout — no `f32`/`f64` anywhere in this file** (enforced by
//! the recursive glob guard, `world/tests/no_float_guard_gen.rs`).
//!
//! ## Fixed-point scale (documented, critic F4)
//!
//! Moisture is a **unitless integer index in `[`[`MOISTURE_MIN`]`, `[`MOISTURE_MAX`]`]`**
//! (`0..=1000` — no physical unit, just a documented relative scale: 0 = driest, 1000 = wettest).
//! It is a pure function of drainage AREA (self + upstream cell count, from
//! [`crate::gen::drainage::kahn_accumulate`]):
//!
//! `moisture(area) = MOISTURE_MIN + min(area, SATURATION_AREA) · (MOISTURE_MAX − MOISTURE_MIN) / SATURATION_AREA`
//!
//! **Monotone (critic F4/F4.1):** non-decreasing in `area` by construction (`min(area, SAT)` is
//! non-decreasing in `area`, and the rest of the expression is a non-negative integer scale of a
//! non-decreasing quantity). Clamped implicitly: at `area=0` this is exactly `MOISTURE_MIN`; for
//! `area ≥ SATURATION_AREA` the `min` caps it, so this is exactly `MOISTURE_MAX` and never exceeds
//! it. `SATURATION_AREA=256` is the implementer's call (RnD `sim/world/09`): on a documented
//! `dim=64` grid (4096 cells), a cell whose basin covers ≥256 cells (~6% of the grid) is already a
//! substantial valley/proto-river, so moisture saturates there rather than requiring an
//! unrealistically large basin. This field is standalone here — **W-5 later reads it** to refine
//! the biome map (azonal wet biomes); it is NOT yet consumed by anything in W-3.

/// Minimum moisture index (driest).
pub const MOISTURE_MIN: i64 = 0;
/// Maximum moisture index (wettest).
pub const MOISTURE_MAX: i64 = 1000;
/// Drainage area (cell count) at which moisture saturates to [`MOISTURE_MAX`] (implementer's call,
/// documented above).
pub const SATURATION_AREA: i64 = 256;

/// Edaphic moisture index at a cell with the given drainage `area` (self + all upstream cells,
/// from [`crate::gen::drainage::kahn_accumulate`]). Pure integer, monotone non-decreasing in
/// `area`, clamped to `[`[`MOISTURE_MIN`]`, `[`MOISTURE_MAX`]`]`. `area` must be `≥ 0` (a
/// drainage-area count can never be negative — every cell has area `≥ 1`, itself).
pub fn moisture_at(area: i64) -> i64 {
    debug_assert!(area >= 0, "drainage area must be >= 0, got {area}");
    let capped = area.clamp(0, SATURATION_AREA);
    MOISTURE_MIN + capped * (MOISTURE_MAX - MOISTURE_MIN) / SATURATION_AREA
}

/// Map [`moisture_at`] over a whole drainage-area field (row-major, any length).
pub fn moisture_field(area: &[i64]) -> Vec<i64> {
    area.iter().map(|&a| moisture_at(a)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Boundary values: area=0 -> MOISTURE_MIN; area>=SATURATION_AREA -> MOISTURE_MAX.
    #[test]
    fn moisture_at_boundaries() {
        assert_eq!(moisture_at(0), MOISTURE_MIN);
        assert_eq!(moisture_at(SATURATION_AREA), MOISTURE_MAX);
        assert_eq!(moisture_at(SATURATION_AREA * 10), MOISTURE_MAX, "must clamp beyond saturation, never exceed MAX");
    }

    /// Property test (critic F4.1): moisture is non-decreasing in `area` over a broad sweep,
    /// including values well beyond `SATURATION_AREA` (the flat-clamped region — still
    /// non-decreasing, never a violation) and a disconnected/degenerate `area=1` (a lone cell with
    /// no upstream contributions, the minimum possible non-zero area).
    #[test]
    fn moisture_at_is_monotone_non_decreasing_in_area() {
        let mut prev = moisture_at(0);
        for area in 1..=(SATURATION_AREA * 20) {
            let m = moisture_at(area);
            assert!(m >= prev, "moisture must be non-decreasing: moisture({area})={m} < prev={prev}");
            prev = m;
        }
    }

    /// Result always stays within the documented `[MOISTURE_MIN, MOISTURE_MAX]` range.
    #[test]
    fn moisture_at_is_bounded() {
        for area in [0i64, 1, 50, 255, 256, 257, 1000, 100_000] {
            let m = moisture_at(area);
            assert!((MOISTURE_MIN..=MOISTURE_MAX).contains(&m), "moisture({area})={m} out of range");
        }
    }

    /// `moisture_field` maps element-wise, preserving length and matching `moisture_at` per cell.
    #[test]
    fn moisture_field_maps_element_wise() {
        let areas = vec![0i64, 1, 32, 256, 5000];
        let field = moisture_field(&areas);
        assert_eq!(field.len(), areas.len());
        for (i, &a) in areas.iter().enumerate() {
            assert_eq!(field[i], moisture_at(a));
        }
    }

    /// Golden vector: pinned exact moisture at a fixture of representative area values.
    #[test]
    fn golden_vector_matches_pinned_moisture() {
        const CASES: &[(i64, i64)] = &[
            (0, 0),
            (1, 3),
            (32, 125),
            (64, 250),
            (128, 500),
            (256, 1000),
            (1000, 1000),
        ];
        for &(area, expected) in CASES {
            let m = moisture_at(area);
            assert_eq!(m, expected, "golden drift at area={area}: got {m}, expected {expected}");
        }
    }
}
