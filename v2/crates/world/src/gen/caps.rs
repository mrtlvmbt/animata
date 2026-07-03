//! W-5: post-erosion FINAL biome classification (zonal + azonal edaphic override) + integer
//! per-layer resource caps — the LAST substrate slice (RnD `sim/world/{03,11,02 §4}`, determinism
//! clause `[biome-classify]`). **Pure integer / fixed-point throughout — no `f32`/`f64` anywhere in
//! this file** (enforced by the recursive glob guard, `world/tests/no_float_guard_gen.rs`).
//!
//! **Prod-inert (W-5 scope, like W-1…W-4):** [`classify_and_caps`] is `pub` but called by NO
//! `WorldView` impl and NOT by `build_sim` — production classification doesn't exist until W-6
//! assembles the pipeline. This module changes zero runtime behavior on its own.
//!
//! ## Pipeline
//!
//! 1. **Final ZONAL biome, re-classified on the POST-erosion surface.** `climate.rs::climate_at`
//!    cannot be reused directly on the eroded height field (it calls `height_at` internally, which
//!    is an infinite-domain function — the eroded field is a finite `dim×dim` array). W-5 uses the
//!    extracted pure core [`crate::gen::climate::climate_from_height`] instead, feeding it the
//!    POST-erosion heights directly. **Border rule (critic F2b, PINNED here — `climate_at` has no
//!    border since `height_at` is infinite-domain):** the upwind sample at `x < WIND_DX` clamps to
//!    the grid edge: `x_src = (x − WIND_DX).max(0)`. The resulting `(T,P)` feeds
//!    [`crate::gen::biome::biome_at`] (reused as-is — it never touched height).
//! 2. **Azonal edaphic override** ([`override_biome`]) — a fixed INTEGER PRIORITY CASCADE over W-3
//!    moisture ([`crate::gen::moisture::moisture_at`] on W-4's final drainage area) + W-4
//!    `surface_material` + a post-erosion slope (here: the raw height drop to the cell's own D8
//!    receiver, `crate::gen::drainage::DrainageState.downstream` — the implementer's call for
//!    "how slope is derived", documented and locked by the golden-vector): waterlogged → `Wetland`,
//!    riparian → `Floodplain`, bedrock/steep → `Rock`, moist soil (alluvium) → `Fertile`, sand →
//!    `Dune`, else the zonal biome passes through unchanged. **No double-count:** the override
//!    function ONLY produces a [`FinalBiome`] tag from the RAW signals — it never derives or caches
//!    a modified moisture/material; [`caps_from`] is always called with the SAME raw
//!    `(moisture, material)` that fed the override, so a cell classified `Wetland` via high
//!    moisture gets its cap from that raw moisture ONCE (via [`caps_from`]'s single moisture-bonus
//!    term), never a second time through the tag.
//! 3. **`caps_from(biome, moisture, material)`** — a pure integer per-cell resource cap: a
//!    documented per-biome base value, moisture-scaled bonus (bounded), material multiplier
//!    (`Bedrock`/`Air` → 0, softer materials scale down), clamped to `[0, `[`CAP_MAX`]`]`.
//!
//! **`FinalBiome` vs `gen::biome::BiomeId` (why a NEW type, not new `BiomeId` variants):**
//! golden-neutrality forbids editing `gen/biome.rs` (not in this slice's allowed-edits list), so the
//! five azonal outcomes (`Wetland`/`Floodplain`/`Rock`/`Fertile`/`Dune`) live in a SEPARATE
//! `FinalBiome` enum here, whose first 8 discriminants mirror `BiomeId`'s 8 zonal variants
//! (`From<BiomeId>` pass-through) and append the azonal ones — `biome.rs` stays byte-for-byte
//! untouched.
//!
//! **Interior-sink isolation (W-3/W-4 carry-forward, critic F7) — a documented tradeoff, not a
//! bug:** a flat-plateau interior cell whose D8 direction is `None` (isolated from the true outlet,
//! per W-3's linear-index tie-break) simply has LOW local drainage area → low moisture; it
//! classifies NORMALLY on that low moisture plus its material/slope — no special case, no crash.
//! The bounded-caps property test below covers the whole prod grid (interior-sink cells included).
//!
//! ## Public output shape for W-6
//!
//! [`classify_and_caps(seed, hmax, dim)`] returns a [`WorldFields`] with `final_biome` + `caps` —
//! the shape W-6 wires into `WorldView::biome`/`resource`. `hmax` threads into `erode` (which needs
//! it for `height_at`); `climate_from_height` takes no `hmax` (it consumes explicit eroded heights
//! already in the `[0,hmax]` range).

use crate::gen::biome::{biome_at, BiomeId};
use crate::gen::climate::{climate_from_height, WIND_DX};
use crate::gen::drainage::is_river;
use crate::gen::erosion::erode;
use crate::gen::material::MaterialId;
use crate::gen::moisture::moisture_at;

/// The FINAL post-override biome id (zonal pass-through + azonal outcomes). `#[repr(u8)]`,
/// append-only (matches `BiomeId`'s idiom) — the first 8 discriminants intentionally mirror
/// `BiomeId` 1:1 (see [`From<BiomeId>`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FinalBiome {
    Tundra = 0,
    BorealForest = 1,
    TemperateGrassland = 2,
    TemperateForest = 3,
    TemperateRainforest = 4,
    Desert = 5,
    Savanna = 6,
    TropicalRainforest = 7,
    // Azonal edaphic override outcomes (RnD 11 §3) — appended, never reorder.
    Wetland = 8,
    Floodplain = 9,
    Rock = 10,
    Fertile = 11,
    Dune = 12,
}

impl From<BiomeId> for FinalBiome {
    fn from(b: BiomeId) -> Self {
        match b {
            BiomeId::Tundra => FinalBiome::Tundra,
            BiomeId::BorealForest => FinalBiome::BorealForest,
            BiomeId::TemperateGrassland => FinalBiome::TemperateGrassland,
            BiomeId::TemperateForest => FinalBiome::TemperateForest,
            BiomeId::TemperateRainforest => FinalBiome::TemperateRainforest,
            BiomeId::Desert => FinalBiome::Desert,
            BiomeId::Savanna => FinalBiome::Savanna,
            BiomeId::TropicalRainforest => FinalBiome::TropicalRainforest,
        }
    }
}

/// Azonal override priority-cascade thresholds (implementer's call, RnD 11 §3, documented, locked
/// by the golden-vector tests). Moisture is on `moisture.rs`'s `[0,1000]` scale.
const WETLAND_MOISTURE_THRESHOLD: i64 = 700;
const FERTILE_MOISTURE_THRESHOLD: i64 = 400;
/// Slope (raw height units to the D8 receiver) at/above which a cell is "steep" enough for `Rock`.
/// Calibrated against W-4's measured relief (adjacent-cell slopes on this fBm terrain are only
/// 0–5 units — a naive large threshold would never fire, the same lesson W-4's `REPOSE_THRESHOLD`
/// recalibration already learned).
const ROCK_SLOPE_THRESHOLD: i64 = 4;

/// Azonal edaphic override: a fixed, documented INTEGER PRIORITY CASCADE (deterministic, no
/// `HashMap` iteration) over EXPLICIT signals — never re-derives them internally, so the caller
/// (both `classify_and_caps` and the golden-fixture test) controls exactly what's tested. Each
/// branch is mutually exclusive by priority order (checked top-to-bottom, first match wins):
///
/// 1. Waterlogged (`moisture ≥ WETLAND_MOISTURE_THRESHOLD`) → `Wetland`.
/// 2. Riparian (`is_river`) → `Floodplain`.
/// 3. Bedrock or steep (`material == Bedrock || slope ≥ ROCK_SLOPE_THRESHOLD`) → `Rock`.
/// 4. Alluvium (`material == Soil && moisture ≥ FERTILE_MOISTURE_THRESHOLD`) → `Fertile`.
/// 5. Sand (`material == Sand`) → `Dune`.
/// 6. Else: the zonal biome passes through unchanged (`From<BiomeId>`).
pub fn override_biome(zonal: BiomeId, moisture: i64, material: MaterialId, slope: i64, is_riv: bool) -> FinalBiome {
    if moisture >= WETLAND_MOISTURE_THRESHOLD {
        return FinalBiome::Wetland;
    }
    if is_riv {
        return FinalBiome::Floodplain;
    }
    if material == MaterialId::Bedrock || slope >= ROCK_SLOPE_THRESHOLD {
        return FinalBiome::Rock;
    }
    if material == MaterialId::Soil && moisture >= FERTILE_MOISTURE_THRESHOLD {
        return FinalBiome::Fertile;
    }
    if material == MaterialId::Sand {
        return FinalBiome::Dune;
    }
    FinalBiome::from(zonal)
}

/// Maximum resource cap (documented ceiling, critic F4 — "bounded" needs a named const to be
/// testable). Matches `NoiseWorld`'s typical `resource_base` scale (see `world/src/lib.rs`'s
/// `resource_nonneg_and_bounded` test, `resource_base=300`).
pub const CAP_MAX: i64 = 300;

/// Per-`FinalBiome` base resource cap (implementer's call, RnD 02 §4, documented, locked).
fn biome_base_cap(b: FinalBiome) -> i64 {
    match b {
        FinalBiome::TropicalRainforest | FinalBiome::TemperateRainforest => 280,
        FinalBiome::Fertile | FinalBiome::Floodplain => 260,
        FinalBiome::TemperateForest | FinalBiome::BorealForest => 220,
        FinalBiome::Wetland => 200,
        FinalBiome::TemperateGrassland | FinalBiome::Savanna => 180,
        FinalBiome::Tundra => 80,
        FinalBiome::Desert | FinalBiome::Dune => 40,
        FinalBiome::Rock => 0,
    }
}

/// Per-`MaterialId` cap multiplier (numerator/denominator — integer-domain, never a float scale).
fn material_mult(m: MaterialId) -> (i64, i64) {
    match m {
        MaterialId::Bedrock | MaterialId::Air => (0, 1),
        MaterialId::Sand => (1, 2),
        MaterialId::Permafrost => (3, 4),
        MaterialId::Soil => (1, 1),
    }
}

/// Pure integer per-cell resource cap: `(base_cap + moisture_bonus) · material_mult`, clamped to
/// `[0, CAP_MAX]`. `moisture_bonus` scales the base cap up to +50% at maximum moisture (integer
/// truncating division — `moisture.rs`'s `MOISTURE_MAX` denominator). Non-negative and bounded BY
/// CONSTRUCTION (the final `.clamp`), locked by the golden-vector + the property test below.
pub fn caps_from(biome: FinalBiome, moisture: i64, material: MaterialId) -> i64 {
    let base = biome_base_cap(biome);
    let moisture_bonus = moisture * base / (2 * crate::gen::moisture::MOISTURE_MAX);
    let (mnum, mden) = material_mult(material);
    let raw = (base + moisture_bonus) * mnum / mden;
    raw.clamp(0, CAP_MAX)
}

/// The full W-5 output over a `dim × dim` grid (mirrors W-3/W-4's state shape, critic F5): the
/// final post-override biome + the per-cell resource cap. `pub` surface W-6 builds a `WorldView`
/// impl from (`biome(pos)`/`resource(pos)`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldFields {
    pub dim: usize,
    pub final_biome: Vec<FinalBiome>,
    pub caps: Vec<i64>,
}

/// Sample `erode(seed, hmax, dim)` (W-4) and classify the FINAL biome + caps per cell: zonal biome
/// on the post-erosion surface (via `climate_from_height` + `biome_at`) → azonal override (via
/// moisture/material/slope/is_river) → `caps_from`. Pure function of `(seed, hmax, dim)` — no
/// RNG-of-clock, no thread-dependence, no global mutable state.
pub fn classify_and_caps(seed: u64, hmax: i64, dim: usize) -> WorldFields {
    let erosion = erode(seed, hmax, dim);
    let n = dim * dim;
    let mut final_biome = Vec::with_capacity(n);
    let mut caps = vec![0i64; n];

    for z in 0..dim {
        for x in 0..dim {
            let idx = z * dim + x;
            let h_cell = erosion.height[idx];
            // Border rule (critic F2b): clamp the upwind sample to the grid edge.
            let x_src = (x as i64 - WIND_DX).max(0) as usize;
            let h_west = erosion.height[z * dim + x_src];
            let (t, p) = climate_from_height(h_cell, h_west, x as i64, z as i64, seed);
            let zonal = biome_at(t, p);

            let area = erosion.drainage.area[idx];
            let moisture = moisture_at(area);
            let material = erosion.surface_material[idx];
            let riparian = is_river(area);
            let slope = match erosion.drainage.downstream[idx] {
                Some(d) => (erosion.height[idx] - erosion.height[d]).max(0),
                None => 0,
            };

            let final_b = override_biome(zonal, moisture, material, slope, riparian);
            final_biome.push(final_b);
            caps[idx] = caps_from(final_b, moisture, material);
        }
    }

    WorldFields { dim, final_biome, caps }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: u64 = 0xA11A_2A11;
    const HMAX: i64 = 200;

    // ── FinalBiome / BiomeId mirroring ───────────────────────────────────────────────────────────

    #[test]
    fn final_biome_from_biome_id_is_a_pass_through() {
        for &b in &BiomeId::ALL {
            let f: FinalBiome = b.into();
            assert_eq!(f as u8, b as u8, "FinalBiome discriminant must mirror BiomeId's exactly");
        }
    }

    // ── override_biome cascade — every branch hit, hand-placed synthetic inputs ─────────────────

    #[test]
    fn override_biome_waterlogged_becomes_wetland() {
        let f = override_biome(BiomeId::TemperateForest, 800, MaterialId::Soil, 0, false);
        assert_eq!(f, FinalBiome::Wetland);
    }

    #[test]
    fn override_biome_riparian_becomes_floodplain() {
        let f = override_biome(BiomeId::Desert, 100, MaterialId::Soil, 0, true);
        assert_eq!(f, FinalBiome::Floodplain);
    }

    #[test]
    fn override_biome_bedrock_or_steep_becomes_rock() {
        let f_bedrock = override_biome(BiomeId::TemperateForest, 100, MaterialId::Bedrock, 0, false);
        assert_eq!(f_bedrock, FinalBiome::Rock);
        let f_steep = override_biome(BiomeId::TemperateForest, 100, MaterialId::Soil, 10, false);
        assert_eq!(f_steep, FinalBiome::Rock);
    }

    #[test]
    fn override_biome_moist_soil_becomes_fertile() {
        let f = override_biome(BiomeId::Desert, 500, MaterialId::Soil, 0, false);
        assert_eq!(f, FinalBiome::Fertile);
    }

    #[test]
    fn override_biome_sand_becomes_dune() {
        let f = override_biome(BiomeId::TemperateGrassland, 100, MaterialId::Sand, 0, false);
        assert_eq!(f, FinalBiome::Dune);
    }

    #[test]
    fn override_biome_plain_cell_passes_through_zonal_unchanged() {
        let f = override_biome(BiomeId::TemperateGrassland, 100, MaterialId::Soil, 0, false);
        assert_eq!(f, FinalBiome::TemperateGrassland);
    }

    /// Priority order sanity: waterlogged beats riparian/rock/fertile/sand (checked first).
    #[test]
    fn override_biome_priority_wetland_beats_all_others() {
        let f = override_biome(BiomeId::Desert, 900, MaterialId::Sand, 10, true);
        assert_eq!(f, FinalBiome::Wetland, "wetland must win even when every other condition also holds");
    }

    // ── no-double-count property (critic F3) ────────────────────────────────────────────────────

    /// A cell whose override fires on moisture (Wetland) must get its cap from the SAME single
    /// `caps_from(final_biome, raw_moisture, raw_material)` computation any other biome tag would
    /// use at that moisture — i.e. the tag itself carries no SEPARATE stacked moisture bonus.
    /// Verified by hand-recomputing the documented formula components directly (the
    /// `climate_at_matches_hand_computed_lapse_and_orography` idiom) and asserting `caps_from`
    /// matches exactly — a hidden double-application would diverge from this hand computation.
    #[test]
    fn caps_from_applies_moisture_bonus_exactly_once() {
        let moisture = 800i64; // triggers Wetland via override_biome
        let material = MaterialId::Soil;
        let zonal = BiomeId::Desert; // deliberately a LOW-cap zonal biome, overridden by moisture
        let final_biome = override_biome(zonal, moisture, material, 0, false);
        assert_eq!(final_biome, FinalBiome::Wetland);

        let cap = caps_from(final_biome, moisture, material);

        // Hand-recompute via the documented single-application formula.
        let base = biome_base_cap(FinalBiome::Wetland);
        let expected_bonus = moisture * base / (2 * crate::gen::moisture::MOISTURE_MAX);
        let (mnum, mden) = material_mult(material);
        let expected = ((base + expected_bonus) * mnum / mden).clamp(0, CAP_MAX);
        assert_eq!(cap, expected, "cap must equal base+bonus applied ONCE, no hidden extra term");
    }

    // ── caps_from bounds ─────────────────────────────────────────────────────────────────────────

    /// Property test (critic F4): every cap over a wide sweep of (biome, moisture, material) stays
    /// within the documented [0, CAP_MAX] range.
    #[test]
    fn caps_from_is_nonneg_and_bounded() {
        const ALL_BIOMES: [FinalBiome; 13] = [
            FinalBiome::Tundra, FinalBiome::BorealForest, FinalBiome::TemperateGrassland,
            FinalBiome::TemperateForest, FinalBiome::TemperateRainforest, FinalBiome::Desert,
            FinalBiome::Savanna, FinalBiome::TropicalRainforest, FinalBiome::Wetland,
            FinalBiome::Floodplain, FinalBiome::Rock, FinalBiome::Fertile, FinalBiome::Dune,
        ];
        const ALL_MATERIALS: [MaterialId; 5] = [
            MaterialId::Air, MaterialId::Sand, MaterialId::Permafrost, MaterialId::Soil, MaterialId::Bedrock,
        ];
        for &b in &ALL_BIOMES {
            for &m in &ALL_MATERIALS {
                for moisture in (0..=1000i64).step_by(37) {
                    let cap = caps_from(b, moisture, m);
                    assert!((0..=CAP_MAX).contains(&cap), "caps_from({b:?},{moisture},{m:?})={cap} out of [0,{CAP_MAX}]");
                }
            }
        }
    }

    // ── classify_and_caps end-to-end ─────────────────────────────────────────────────────────────

    #[test]
    fn classify_and_caps_is_deterministic_across_repeated_calls() {
        let a = classify_and_caps(SEED, HMAX, 16);
        let b = classify_and_caps(SEED, HMAX, 16);
        assert_eq!(a, b, "classify_and_caps must be byte-identical across repeated calls");
    }

    /// Interior-sink isolation (critic F7): a cell with no D8 receiver (`downstream=None`) must
    /// still classify + cap WITHOUT panicking, over the whole prod-scale grid.
    #[test]
    fn classify_and_caps_is_well_defined_grid_wide() {
        const DIM: usize = 64;
        let fields = classify_and_caps(SEED, HMAX, DIM);
        assert_eq!(fields.final_biome.len(), DIM * DIM);
        assert_eq!(fields.caps.len(), DIM * DIM);
        for &c in &fields.caps {
            assert!((0..=CAP_MAX).contains(&c), "cap {c} out of [0,{CAP_MAX}] somewhere on the prod grid");
        }
    }

    #[test]
    fn golden_vector_matches_pinned_classify_and_caps_fixture() {
        const GOLDEN_SEED: u64 = 0xA11A_2A11;
        const GOLDEN_HMAX: i64 = 200;
        const DIM: usize = 16;
        let fields = classify_and_caps(GOLDEN_SEED, GOLDEN_HMAX, DIM);

        const CASES: &[(usize, FinalBiome, i64)] = &[
            (0, FinalBiome::BorealForest, 220),
            (36, FinalBiome::BorealForest, 220),
            (100, FinalBiome::BorealForest, 220),
            (255, FinalBiome::TemperateGrassland, 180),
        ];
        for &(idx, exp_biome, exp_cap) in CASES {
            assert_eq!(fields.final_biome[idx], exp_biome, "golden drift: final_biome[{idx}]");
            assert_eq!(fields.caps[idx], exp_cap, "golden drift: caps[{idx}]");
        }
    }
}
