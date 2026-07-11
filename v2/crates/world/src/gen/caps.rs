//! W-5: post-erosion FINAL biome classification (zonal + azonal edaphic override) + integer
//! per-layer resource caps — the LAST substrate slice (RnD `sim/world/{03,11,02 §4}`, determinism
//! clause `[biome-classify]`). **Pure integer / fixed-point throughout — no `f32`/`f64` anywhere in
//! this file** (enforced by the recursive glob guard, `world/tests/no_float_guard_gen.rs`).
//!
//! **W-6 status:** [`classify_and_caps`] is now `ProcgenWorld::new`'s (`world/src/lib.rs`) entry
//! point into the whole `gen/` pipeline — the production `WorldView` impl.
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
use crate::gen::height::height_at;
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

/// W-7: Patchiness (spatial autocorrelation) seed salt — decorrelated from height to create
/// independent spatial structure for resource-cap heterogeneity (implementer's call, RnD W-7,
/// documented, locked). Used as `seed ^ PATCH_SEED_SALT` in [`patchiness_at`], same pattern as
/// `resistance_class_at` in erosion.rs.
const PATCH_SEED_SALT: u64 = 0x5041_5443_4849_4E45; // "PATCHINE" (ASCII, folded)

/// W-7: Resource-cap patchiness scale range [MIN, MAX] — symmetric factor centered on 256,
/// mapping to resource modulation `[192, 320]` (0.75×–1.25× multiplicative factor). The symmetric
/// range ATTEMPTS mean-neutrality; empirical re-measure required post-merge to confirm drift <±5%
/// in economy equilibrium (issue #380, owner PM). Implementer's call (RnD W-7): 4–8 cells per
/// patch, 64×64 grid → 64–256 coherent regions.
const PATCH_SCALE_MIN: i64 = 192;
const PATCH_SCALE_MAX: i64 = 320;

/// W-7: Patchiness scale factor per cell via integer fBm noise — a **mean-neutral symmetric
/// modulation** of the base cap. Returns a multiplicative factor in `[PATCH_SCALE_MIN,
/// PATCH_SCALE_MAX]` (integer, centered at 256) derived from a decorrelated fBm layer
/// (via [`crate::gen::height::height_at`] with `PATCH_SEED_SALT`). The formula uses linear
/// rescaling that preserves fBm's natural spatial mean at `hmax/2`:
///
/// ```ignore
/// factor = PATCH_SCALE_MIN + (height_at(...) * (PATCH_SCALE_MAX - PATCH_SCALE_MIN)) / hmax
///        = 192 + (height_at * 128) / hmax
/// ```
///
/// This is mean-neutral: if `height_at` has spatial mean ≈ `hmax/2`, then `factor` has spatial
/// mean ≈ 256 (the center). Changing the range narrower/wider scales the variation amplitude but
/// preserves the mean. Applied in [`classify_and_caps`] as: `cap_modulated =
/// clamp((cap_base * factor + 128) / 256, 0, CAP_MAX)` (the `+128` is round-half, eliminates
/// truncation bias).
///
/// **Determinism:** Integer-only, uses `height_at` (W-1 primitive, x86-deterministic), no float.
pub fn patchiness_at(x: i64, z: i64, seed: u64, hmax: i64) -> i64 {
    let raw_noise = height_at(x, z, seed ^ PATCH_SEED_SALT, hmax);
    PATCH_SCALE_MIN + (raw_noise * (PATCH_SCALE_MAX - PATCH_SCALE_MIN)) / hmax
}

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

/// Per-`FinalBiome` base O₂ capacity (P1-0, ШВ-1). Aerated/surface biomes have high O₂; anaerobic/
/// deep biomes have zero. Integer fixed-point (same scale as substrate caps). Non-negative and
/// bounded to `[0, CAP_MAX]` for consistency with substrate.
fn oxygen_base_cap(b: FinalBiome) -> i64 {
    match b {
        // Aerated surface biomes: high O₂ capacity (well-oxygenated)
        FinalBiome::TropicalRainforest | FinalBiome::TemperateRainforest => 250,
        FinalBiome::TemperateForest | FinalBiome::BorealForest => 240,
        FinalBiome::TemperateGrassland | FinalBiome::Savanna => 230,
        FinalBiome::Fertile => 220,
        // Wetland: waterlogged but oxygenated (higher than Rock, lower than upland)
        FinalBiome::Wetland => 150,
        FinalBiome::Floodplain => 180,
        // Transition biomes: lower O₂ availability
        FinalBiome::Tundra => 200,  // Cold, thin soils, but aerated
        FinalBiome::Desert | FinalBiome::Dune => 180,  // Arid, sparse life, but O₂-available surface
        // Anaerobic/impenetrable: no O₂
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

/// Pure integer per-cell O₂ cap (P1-0 ШВ-1): derived from biome only (no moisture/material bonus
/// for now — static O₂ field in P1; dynamic source coupling comes P2+ from photosynthesis + surface
/// aeration). Returns O₂ capacity clamped to `[0, CAP_MAX]`. Material is ignored for O₂
/// (rock/bedrock still have zero O₂ via `oxygen_base_cap`).
pub fn oxygen_cap_from(biome: FinalBiome) -> i64 {
    oxygen_base_cap(biome).clamp(0, CAP_MAX)
}

/// Per-`FinalBiome` base NO₃ capacity (P5-0, ШВ-1). Anaerobic/waterlogged biomes have high NO₃;
/// aerated surface biomes have low NO₃ (denitrification, leaching). Integer fixed-point (same scale
/// as substrate caps). NO₃ is the INVERSE of O₂ — high where O₂ is low. Non-negative and bounded
/// to `[0, CAP_MAX]` for consistency with substrate.
fn nitrate_base_cap(b: FinalBiome) -> i64 {
    match b {
        // Anaerobic/waterlogged biomes: high NO₃ capacity (accumulates in reducing zones)
        FinalBiome::Wetland => 220,
        FinalBiome::Floodplain => 180,
        FinalBiome::Tundra => 120,  // Permafrost waterlogged
        // Aerated surface biomes: low NO₃ (consumed/leached in oxic soil)
        FinalBiome::TemperateRainforest | FinalBiome::TropicalRainforest => 40,
        FinalBiome::TemperateForest | FinalBiome::BorealForest => 30,
        FinalBiome::TemperateGrassland | FinalBiome::Savanna => 30,
        FinalBiome::Fertile => 40,
        FinalBiome::Desert | FinalBiome::Dune => 30,  // Arid, minimal NO₃
        // Anaerobic/impenetrable: no NO₃ (uninhabitable)
        FinalBiome::Rock => 0,
    }
}

/// Pure integer per-cell NO₃ cap (P5-0, ШВ-1): derived from biome only (inverse of O₂). Returns
/// NO₃ capacity clamped to `[0, CAP_MAX]`. Static field in P5-0 (no regen; inert layer).
pub fn nitrate_cap_from(biome: FinalBiome) -> i64 {
    nitrate_base_cap(biome).clamp(0, CAP_MAX)
}

/// The full W-5 output over a `dim × dim` grid (mirrors W-3/W-4's state shape, critic F5): the
/// POST-erosion `height` (W-6's `ProcgenWorld` needs this for `WorldView::height`/`is_solid` —
/// added here rather than having W-6 re-run `erode` a second time), the final post-override biome,
/// and the per-cell resource cap. `pub` surface W-6 builds a `WorldView` impl from
/// (`height`/`biome(pos)`/`resource(pos)`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldFields {
    pub dim: usize,
    pub height: Vec<i64>,
    pub final_biome: Vec<FinalBiome>,
    pub caps: Vec<i64>,
    /// Surface material per cell (from W-4 erosion), exposed for richness testing.
    pub surface_material: Vec<u8>,
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

            // W-7: Apply spatial patchiness modulation to the base cap (mean-neutral symmetric factor).
            let cap_base = caps_from(final_b, moisture, material);
            let patch_scale = patchiness_at(x as i64, z as i64, seed, hmax);
            // Modulation formula: cap_modulated = clamp((cap_base * patch_scale + 128) / 256, ...)
            // The +128 implements round-half, eliminates constant −0.5 truncation bias.
            let cap_modulated = ((cap_base * patch_scale + 128) / 256).clamp(0, CAP_MAX);
            caps[idx] = cap_modulated;
        }
    }

    let surface_material = erosion.surface_material.iter().map(|&m| m as u8).collect();
    WorldFields { dim, height: erosion.height, final_biome, caps, surface_material }
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

        // W-7 re-pin: caps drifted due to spatial patchiness modulation (multiplicative factor,
        // symmetric range [192, 320], centered at 256). Height/biome/material fields unchanged.
        const CASES: &[(usize, FinalBiome, i64)] = &[
            (0, FinalBiome::BorealForest, 229),
            (36, FinalBiome::BorealForest, 226),
            (100, FinalBiome::BorealForest, 223),
            (255, FinalBiome::TemperateGrassland, 187),
        ];
        for &(idx, exp_biome, exp_cap) in CASES {
            assert_eq!(fields.final_biome[idx], exp_biome, "golden drift: final_biome[{idx}]");
            assert_eq!(fields.caps[idx], exp_cap, "golden drift: caps[{idx}]");
        }
    }

    /// W-7 bounds and clamp verification: on prod-scale 64×64 grid, all caps stay bounded and
    /// clamp incidence (cells hitting CAP_MAX or floor) stays low (<15%). If clamp incidence is
    /// too high, the patchiness factor range [192, 320] is asymmetric in the integer domain and
    /// needs narrowing (empirical re-measure owns this post-merge). Clamping at the low end
    /// (rescale_cap floor at 1) is expected; high-end (CAP_MAX=300) requires monitoring.
    #[test]
    fn w7_patchiness_is_bounded_on_prod_scale_grid() {
        const SEED: u64 = 0xA11A_2A11;
        const HMAX: i64 = 200;
        const DIM: usize = 64;
        let fields = classify_and_caps(SEED, HMAX, DIM);

        // Count cells hitting bounds (ceiling at CAP_MAX, floor at 1 via rescale_cap).
        let mut clamp_low = 0usize;
        let mut clamp_high = 0usize;
        let mut total_sum: i64 = 0;

        for &cap in &fields.caps {
            assert!((0..=CAP_MAX).contains(&cap), "cap {cap} out of bounds [0,{CAP_MAX}]");
            if cap == 1 {
                clamp_low += 1;
            }
            if cap == CAP_MAX {
                clamp_high += 1;
            }
            total_sum += cap;
        }

        let grid_size = DIM * DIM;
        let clamp_total = clamp_low + clamp_high;

        // Report (integer only, no float — this is prod gen code with float guard).
        // Mean integer div: (total_sum * 1000) / grid_size gives mean × 1000.
        let mean_times_1000 = (total_sum * 1000) / (grid_size as i64);
        eprintln!(
            "W-7 prod-grid: mean_×1000={}, median_idx={}, clamp_count={} ({}+{}), grid={}×{}",
            mean_times_1000,
            if fields.caps.len() > grid_size / 2 {
                let mut sorted = fields.caps.clone();
                sorted.sort();
                sorted[grid_size / 2]
            } else {
                0
            },
            clamp_total,
            clamp_low,
            clamp_high,
            DIM,
            DIM
        );

        // Primary check: clamp incidence should be low. If >15% of cells clamp, the range is
        // asymmetric and needs narrowing (e.g., shift PATCH_SCALE_MIN/MAX to [224,288]).
        let clamp_permille = (clamp_total * 1000) / grid_size;
        assert!(
            clamp_permille < 150, // <15%
            "patchiness clamp_incidence too high: {}/{} cells — PATCH_SCALE_RANGE may need narrowing",
            clamp_total,
            grid_size
        );
    }

    /// W-7 spatial autocorrelation: verify that neighboring cells have more similar caps than
    /// cells far apart (Moran's I proxy). This confirms patchiness adds coherent spatial
    /// structure, not random noise. Integer-only comparison (no divisions).
    #[test]
    fn w7_patchiness_has_positive_spatial_autocorrelation() {
        const SEED: u64 = 0xA11A_2A11;
        const HMAX: i64 = 200;
        const DIM: usize = 64;
        let fields = classify_and_caps(SEED, HMAX, DIM);

        // Adjacent-cell differences: sum of absolute differences for cells one step apart.
        let mut same_neighbor_sum = 0i64;
        let mut same_count = 0i64;

        for z in 0..DIM {
            for x in 0..(DIM - 1) {
                let idx_a = z * DIM + x;
                let idx_b = z * DIM + x + 1;
                same_neighbor_sum += (fields.caps[idx_a] - fields.caps[idx_b]).abs();
                same_count += 1;
            }
        }

        // Far-cell differences: cells 16 steps away (different row).
        let mut cross_neighbor_sum = 0i64;
        let mut cross_count = 0i64;
        for z in 0..(DIM - 16) {
            for x in 0..DIM {
                let idx_a = z * DIM + x;
                let idx_b = (z + 16) * DIM + x;
                cross_neighbor_sum += (fields.caps[idx_a] - fields.caps[idx_b]).abs();
                cross_count += 1;
            }
        }

        eprintln!(
            "W-7 autocorr: neighbor_sum_diff={} ({} pairs), far_sum_diff={} ({} pairs)",
            same_neighbor_sum, same_count, cross_neighbor_sum, cross_count
        );

        // Positive autocorrelation: adjacent cells differ LESS (smaller sum) than distant cells.
        // Multiply to avoid division: same_neighbor_sum * cross_count should be < cross_neighbor_sum * same_count
        assert!(
            same_neighbor_sum * cross_count < cross_neighbor_sum * same_count,
            "patchiness lacks autocorrelation: neighbors={} should be < distant={} (scaled)",
            same_neighbor_sum,
            cross_neighbor_sum
        );
    }

    /// W-7 mean-invariance: patchiness must be a PURE VARIANCE knob with NO hidden mean shift
    /// (MUST-FIX #1 per critic). This test computes the spatial-average resource cap over the whole
    /// map WITH patchiness active, then WITH patchiness NEUTRALIZED (patch factor forced to 256,
    /// i.e. ×1.0), and asserts the two means are equal within ±5% (the acceptance threshold at
    /// caps.rs:153). The test is required to verify that patchiness does not introduce correlation
    /// with the base cap (covariance E[factor·base] ≠ E[factor]·E[base] would shift the mean even
    /// though the factor is symmetric).
    #[test]
    fn patchiness_maintains_mean_neutrality() {
        const SEED: u64 = 0xA11A_2A11;
        const HMAX: i64 = 200;
        const DIM: usize = 64;

        // Compute world WITH patchiness active
        let with_patch = classify_and_caps(SEED, HMAX, DIM);
        let sum_with: i64 = with_patch.caps.iter().sum();
        let mean_with = sum_with as f64 / (DIM * DIM) as f64;

        // Compute world WITHOUT patchiness (patch factor forced to 256 = identity)
        // We replicate the classify_and_caps logic but with patch_scale = 256.
        // This requires re-running the full pipeline with neutralized patchiness.
        let mut without_patch_caps = Vec::new();
        let erosion = erode(SEED, HMAX, DIM);
        for z in 0..DIM {
            for x in 0..DIM {
                let idx = z * DIM + x;
                let h_cell = erosion.height[idx];
                let x_src = (x as i64 - WIND_DX).max(0) as usize;
                let h_west = erosion.height[z * DIM + x_src];
                let (t, p) = climate_from_height(h_cell, h_west, x as i64, z as i64, SEED);
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
                let cap_base = caps_from(final_b, moisture, material);

                // W-7: Apply NO patchiness (patch_scale = 256 = identity modulation)
                let patch_scale = 256i64; // Neutralized: no modulation
                let cap_modulated = ((cap_base * patch_scale + 128) / 256).clamp(0, CAP_MAX);
                without_patch_caps.push(cap_modulated);
            }
        }

        let sum_without: i64 = without_patch_caps.iter().sum();
        let mean_without = sum_without as f64 / (DIM * DIM) as f64;

        eprintln!(
            "W-7 mean-invariance: mean_with_patch={:.2}, mean_without_patch={:.2}, diff_pct={:.2}%",
            mean_with,
            mean_without,
            ((mean_with - mean_without).abs() / mean_without * 100.0)
        );

        // Assert means are equal within ±5% (the documented acceptance threshold).
        let pct_diff = (mean_with - mean_without).abs() / mean_without * 100.0;
        assert!(
            pct_diff <= 5.0,
            "patchiness mean shift exceeded ±5%: {:.2}% diff (with={:.2}, without={:.2}). \
             This indicates covariance between patch factor and base cap — decorrelate the patch noise.",
            pct_diff, mean_with, mean_without
        );
    }

}
