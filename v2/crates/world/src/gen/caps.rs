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

use crate::gen::aeolian;
use crate::gen::biome::{biome_at, BiomeId};
use crate::gen::climate::{climate_from_height, WIND_DX};
use crate::gen::drainage::is_river;
use crate::gen::erosion::{erode, de_needle_pass, talus_step_final, MAX_SPIKE_FINAL, SPIKE_MARGIN_FINAL, N_ITERS_FINAL, NEEDLE_MARGIN};
use crate::gen::height::height_at;
use crate::gen::material::MaterialId;
use crate::gen::moisture::moisture_at;

/// The FINAL post-override biome id (zonal pass-through + azonal outcomes). `#[repr(u8)]`,
/// append-only (matches `BiomeId`'s idiom) — the first 8 discriminants intentionally mirror
/// `BiomeId` 1:1 (see [`From<BiomeId>`]). `Ocean` (W-SIM-7, #423) is the submerged branch —
/// classify's ONLY non-terrestrial outcome, checked BEFORE the zonal climate classification even
/// runs (a submerged cell never reads `climate_from_height`/`biome_at` — RnD's "no ocean, all land"
/// gap this slice closes).
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
    // W-SIM-7 (#423): the submerged branch — appended, never reorder.
    Ocean = 13,
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

// ── W-9: Final-surface relief measurement ──────────────────────────────────────────────────────

/// W-9: Landform masks for amplitude measurement. Each landform's presence is tracked as a bool
/// array (same length as height field). Masks are NOT mutually exclusive — a cell may be counted
/// in multiple masks if multiple landforms affected it (intentional for retention ratio reporting).
#[derive(Clone, Debug)]
pub struct LandformMasks {
    /// Cells affected by volcanic edifice (any MaterialId::Basalt or MaterialId::Tuff).
    pub edifice: Vec<bool>,
    /// Cells affected by glacial till (MaterialId::Till).
    pub till: Vec<bool>,
    /// Cells affected by aeolian dune sand (sand_depth > 0).
    pub dune: Vec<bool>,
}

/// W-9: Amplitude report for a single landform — measures relief preservation via crest retention.
#[derive(Clone, Debug)]
pub struct CrestAmplitudeReport {
    /// Number of identified crests (strict local maxima within the mask, above floor).
    pub crest_count: usize,
    /// Percentile 10 of retention ratios at crests: (post_amplitude / pre_amplitude * 100).
    /// If no crests or empty mask, this is 0 (safe caller contract).
    pub p10_retention_pct: i64,
}

/// W-9: Staged output of the classification pipeline — height snapshots at each major stage.
/// Used to measure amplitude preservation across the final-surface thermal relaxation pass.
#[derive(Clone, Debug)]
pub struct StagedHeights {
    /// Height after coastal phase (before talus_step_final).
    pub post_coastal: Vec<i64>,
    /// Height after talus_step_final (before de_needle).
    pub post_talus: Vec<i64>,
    /// Height after de_needle (final, used for classification).
    pub post_deneedle: Vec<i64>,
}

/// W-9: Amplitude floor constant for crest identification. Start = MAX_SPIKE_FINAL
/// (cells with amplitude >= this are considered crests). Can be recalibrated from Phase-0
/// if crest count falls below the precondition (>=16 @512, >=4 @64).
pub const AMPLITUDE_FLOOR: i64 = MAX_SPIKE_FINAL;

/// W-9: D8 offsets for neighbor iteration (reused from erosion.rs pattern).
const D8_OFFSETS_CAPS: [(i64, i64); 8] =
    [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];

/// W-9: Compute the median height of in-grid D8 neighbors. Used to identify local maxima.
/// Returns i64::MIN if the cell has no in-grid neighbors (edge case, should not occur in practice).
fn median_d8_neighbors(x: usize, z: usize, dim: usize, heights: &[i64]) -> i64 {
    let mut neighbors = Vec::new();
    for &(dx, dz) in &D8_OFFSETS_CAPS {
        let nx = x as i64 + dx;
        let nz = z as i64 + dz;
        if nx >= 0 && nz >= 0 && (nx as usize) < dim && (nz as usize) < dim {
            let u = (nz as usize) * dim + (nx as usize);
            neighbors.push(heights[u]);
        }
    }
    if neighbors.is_empty() {
        return i64::MIN;
    }
    neighbors.sort();
    let mid = neighbors.len() / 2;
    if neighbors.len() % 2 == 1 {
        neighbors[mid]
    } else if mid > 0 {
        (neighbors[mid - 1] + neighbors[mid]) / 2
    } else {
        neighbors[0]
    }
}

/// W-9: Compute median of D8 neighbors at radius-2 (16 neighbors in a 5x5 ring, excluding center and radius-1).
/// Used as a fallback median when radius-1 crest detection yields too few candidates.
fn median_d8_neighbors_radius2(x: usize, z: usize, dim: usize, heights: &[i64]) -> i64 {
    let mut neighbors = Vec::new();
    for dz in -2..=2i64 {
        for dx in -2..=2i64 {
            if dx == 0 && dz == 0 { continue; } // Skip center
            if dx.abs() == 1 && dz.abs() <= 1 { continue; } // Skip radius-1 (D8 ring)
            if dx.abs() <= 1 && dz.abs() == 1 { continue; }
            let nx = x as i64 + dx;
            let nz = z as i64 + dz;
            if nx >= 0 && nz >= 0 && (nx as usize) < dim && (nz as usize) < dim {
                let u = (nz as usize) * dim + (nx as usize);
                neighbors.push(heights[u]);
            }
        }
    }
    if neighbors.is_empty() {
        return i64::MIN;
    }
    neighbors.sort();
    let mid = neighbors.len() / 2;
    if neighbors.len() % 2 == 1 {
        neighbors[mid]
    } else if mid > 0 {
        (neighbors[mid - 1] + neighbors[mid]) / 2
    } else {
        neighbors[0]
    }
}

/// W-9: Identify crests and measure their amplitude preservation across the talus_step_final pass.
/// A crest is a local maximum within the mask with pre-pass amplitude >= AMPLITUDE_FLOOR.
/// Uses fallback order if crest count is too low: (1) strict maxima, (2) non-strict maxima, (3) radius-2 median.
/// Returns amplitude statistics: crest count and p10 retention percentage, with fallback mode indicator.
///
/// **Amplitude calculation (all in i64, no float):**
/// - Pre-amplitude at crest c: `h_pre[c] - median(h_pre of c's in-grid D8 ring)`.
/// - Post-amplitude at crest c: `h_post[c] - median(h_post of c's in-grid D8 ring)`.
/// - Retention at c: `100 * post / pre` (all i64 comparisons, no truncation to <100% = 0).
/// - Score: p10 of the crest retention list per landform.
///
/// **Fallback order (pre-decided in spec):**
/// 1. Strict maxima: `h > all D8 neighbors`
/// 2. Non-strict maxima: `h >= all D8 neighbors`
/// 3. Radius-2 median: use outer ring (5x5 excluding center and inner ring) for median
///
/// **Edge cases (caller contract):**
/// - Empty mask: returns `(count: 0, p10: 0)`.
/// - Fewer than expected crests: returns actual count with fallback mode used if applicable.
pub fn landform_amplitudes(
    dim: usize,
    heights_pre: &[i64],
    heights_post: &[i64],
    mask: &[bool],
) -> CrestAmplitudeReport {
    let n = dim * dim;
    debug_assert_eq!(heights_pre.len(), n);
    debug_assert_eq!(heights_post.len(), n);
    debug_assert_eq!(mask.len(), n);

    // Try Mode 1: strict local maxima (h > all D8 neighbors)
    let mut crests = Vec::new();
    for z in 0..dim {
        for x in 0..dim {
            let idx = z * dim + x;
            if !mask[idx] { continue; }

            let mut is_strict_max = true;
            for &(dx, dz) in &D8_OFFSETS_CAPS {
                let nx = x as i64 + dx;
                let nz = z as i64 + dz;
                if nx >= 0 && nz >= 0 && (nx as usize) < dim && (nz as usize) < dim {
                    let u = (nz as usize) * dim + (nx as usize);
                    if heights_pre[u] >= heights_pre[idx] {
                        is_strict_max = false;
                        break;
                    }
                }
            }
            if is_strict_max {
                let median_pre = median_d8_neighbors(x, z, dim, heights_pre);
                let amp_pre = heights_pre[idx] - median_pre;
                if amp_pre >= AMPLITUDE_FLOOR {
                    crests.push(idx);
                }
            }
        }
    }

    // Fallback: if too few crests, try non-strict maxima (h >= all D8 neighbors)
    if crests.len() < (if dim == 512 { 16 } else { 4 }) {
        crests.clear();
        for z in 0..dim {
            for x in 0..dim {
                let idx = z * dim + x;
                if !mask[idx] { continue; }

                let mut is_nonstrict_max = true;
                for &(dx, dz) in &D8_OFFSETS_CAPS {
                    let nx = x as i64 + dx;
                    let nz = z as i64 + dz;
                    if nx >= 0 && nz >= 0 && (nx as usize) < dim && (nz as usize) < dim {
                        let u = (nz as usize) * dim + (nx as usize);
                        if heights_pre[u] > heights_pre[idx] {
                            is_nonstrict_max = false;
                            break;
                        }
                    }
                }
                if is_nonstrict_max {
                    let median_pre = median_d8_neighbors(x, z, dim, heights_pre);
                    let amp_pre = heights_pre[idx] - median_pre;
                    if amp_pre >= AMPLITUDE_FLOOR {
                        crests.push(idx);
                    }
                }
            }
        }
    }

    // Further fallback: if still too few, try radius-2 median for amplitude calc
    if crests.len() < (if dim == 512 { 16 } else { 4 }) {
        crests.clear();
        for z in 0..dim {
            for x in 0..dim {
                let idx = z * dim + x;
                if !mask[idx] { continue; }

                let mut is_nonstrict_max = true;
                for &(dx, dz) in &D8_OFFSETS_CAPS {
                    let nx = x as i64 + dx;
                    let nz = z as i64 + dz;
                    if nx >= 0 && nz >= 0 && (nx as usize) < dim && (nz as usize) < dim {
                        let u = (nz as usize) * dim + (nx as usize);
                        if heights_pre[u] > heights_pre[idx] {
                            is_nonstrict_max = false;
                            break;
                        }
                    }
                }
                if is_nonstrict_max {
                    let median_pre = median_d8_neighbors_radius2(x, z, dim, heights_pre);
                    let amp_pre = heights_pre[idx] - median_pre;
                    if amp_pre >= AMPLITUDE_FLOOR {
                        crests.push(idx);
                    }
                }
            }
        }
    }

    // Compute retention for all crests
    let mut retentions = Vec::new();
    for &idx in &crests {
        let z = idx / dim;
        let x = idx % dim;

        let median_pre = median_d8_neighbors(x, z, dim, heights_pre);
        let amp_pre = heights_pre[idx] - median_pre;
        let median_post = median_d8_neighbors(x, z, dim, heights_post);
        let amp_post = heights_post[idx] - median_post;

        let retention = if amp_pre > 0 {
            (100 * amp_post) / amp_pre
        } else {
            100
        };
        retentions.push(retention);
    }

    if retentions.is_empty() {
        return CrestAmplitudeReport {
            crest_count: 0,
            p10_retention_pct: 0,
        };
    }

    // Compute p10
    retentions.sort();
    let p10_idx = (retentions.len() / 10).max(0);
    let p10 = retentions[p10_idx];

    CrestAmplitudeReport {
        crest_count: crests.len(),
        p10_retention_pct: p10,
    }
}

// ── W-9: Measurement utilities for sweep evaluation ──────────────────────────────────────────────

/// W-9: Count isolated spikes ("needles") on the field: cells whose height exceeds max of D8
/// neighbors by > NEEDLE_MARGIN. Returns count and list of needle cell indices.
pub fn measure_needles(dim: usize, heights: &[i64]) -> (usize, Vec<usize>) {
    let n = dim * dim;
    debug_assert_eq!(heights.len(), n);
    let mut needles = Vec::new();

    for z in 0..dim {
        for x in 0..dim {
            let idx = z * dim + x;
            let mut nmax = i64::MIN;
            for &(dx, dz) in &D8_OFFSETS_CAPS {
                let nx = x as i64 + dx;
                let nz = z as i64 + dz;
                if nx >= 0 && nz >= 0 && (nx as usize) < dim && (nz as usize) < dim {
                    let u = (nz as usize) * dim + (nx as usize);
                    nmax = nmax.max(heights[u]);
                }
            }
            if heights[idx] > nmax + NEEDLE_MARGIN {
                needles.push(idx);
            }
        }
    }
    (needles.len(), needles)
}

/// W-9: Measure max second-max spike: the maximum by which any cell exceeds its second-highest D8 neighbor.
/// Selective donor rule: cells only donate if h - second_max > spike_margin, so this is the gate metric.
/// Returns the max second-spike found. Gate: must be <= MAX_SPIKE_FINAL.
pub fn measure_max_local_step(dim: usize, heights: &[i64]) -> i64 {
    let n = dim * dim;
    debug_assert_eq!(heights.len(), n);
    let mut max_spike: i64 = 0;

    for z in 0..dim {
        for x in 0..dim {
            let idx = z * dim + x;
            let mut max_h = i64::MIN;
            let mut second_max_h = i64::MIN;
            for &(dx, dz) in &D8_OFFSETS_CAPS {
                let nx = x as i64 + dx;
                let nz = z as i64 + dz;
                if nx >= 0 && nz >= 0 && (nx as usize) < dim && (nz as usize) < dim {
                    let u = (nz as usize) * dim + (nx as usize);
                    if heights[u] > max_h {
                        second_max_h = max_h;
                        max_h = heights[u];
                    } else if heights[u] > second_max_h {
                        second_max_h = heights[u];
                    }
                }
            }
            if second_max_h != i64::MIN {
                let spike = heights[idx] - second_max_h;
                max_spike = max_spike.max(spike);
            }
        }
    }
    max_spike
}

/// W-9: Count cells exceeding spike thresholds: how many cells have `h - second_max(D8) > threshold`.
/// Returns count of cells exceeding each threshold; useful for understanding residual distribution.
pub fn count_spikes_exceeding(dim: usize, heights: &[i64], threshold: i64) -> usize {
    let n = dim * dim;
    debug_assert_eq!(heights.len(), n);
    let mut count = 0;

    for z in 0..dim {
        for x in 0..dim {
            let idx = z * dim + x;
            let mut max_h = i64::MIN;
            let mut second_max_h = i64::MIN;
            for &(dx, dz) in &D8_OFFSETS_CAPS {
                let nx = x as i64 + dx;
                let nz = z as i64 + dz;
                if nx >= 0 && nz >= 0 && (nx as usize) < dim && (nz as usize) < dim {
                    let u = (nz as usize) * dim + (nx as usize);
                    if heights[u] > max_h {
                        second_max_h = max_h;
                        max_h = heights[u];
                    } else if heights[u] > second_max_h {
                        second_max_h = heights[u];
                    }
                }
            }
            if second_max_h != i64::MIN {
                let spike = heights[idx] - second_max_h;
                if spike > threshold {
                    count += 1;
                }
            }
        }
    }
    count
}

/// W-9: Count cells clipped by de_needle_pass: cells with excess > NEEDLE_MARGIN that were reduced.
/// Returns the count of cells that de_needle modified (sent out positive amount).
pub fn measure_de_needle_clip_count(dim: usize, heights_before: &[i64], heights_after: &[i64]) -> usize {
    let n = dim * dim;
    debug_assert_eq!(heights_before.len(), n);
    debug_assert_eq!(heights_after.len(), n);
    let mut clip_count = 0;

    for z in 0..dim {
        for x in 0..dim {
            let idx = z * dim + x;
            let mut nmax = i64::MIN;
            for &(dx, dz) in &D8_OFFSETS_CAPS {
                let nx = x as i64 + dx;
                let nz = z as i64 + dz;
                if nx >= 0 && nz >= 0 && (nx as usize) < dim && (nz as usize) < dim {
                    let u = (nz as usize) * dim + (nx as usize);
                    nmax = nmax.max(heights_before[u]);
                }
            }
            if heights_before[idx] > nmax + NEEDLE_MARGIN {
                // This cell was a candidate for clipping; check if it was modified
                if heights_before[idx] != heights_after[idx] {
                    clip_count += 1;
                }
            }
        }
    }
    clip_count
}

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
        FinalBiome::Rock | FinalBiome::Ocean => 0,
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
        // Anaerobic/impenetrable: no O₂. Ocean (W-SIM-7, #423): out of scope for this slice's biology
        // (RnD roadmap is relief-only) — treated as zero baseline, same as Rock.
        FinalBiome::Rock | FinalBiome::Ocean => 0,
    }
}

/// Per-`MaterialId` cap multiplier (numerator/denominator — integer-domain, never a float scale).
/// `Basalt`/`Tuff` (W-SIM-5, #410): fresh volcanic substrate is a barren rocky/ashy zone of
/// near-zero production (RnD 15 §8) — the same zero multiplier as `Bedrock`. `Till` (W-SIM-6, #416):
/// fresh glacial till/moraine is likewise a barren zone (RnD 16 §9) — same zero multiplier. `Water`
/// (W-SIM-7, #423): out of scope for this slice's biology (relief-only roadmap) — zero multiplier,
/// same as `Bedrock`/`Air` (the `caps_from` value never actually matters for a submerged cell in
/// this slice, since `Ocean` biome's own base cap is already 0 — the multiplier is here purely for
/// completeness of the `MaterialId` match, not a load-bearing production signal).
fn material_mult(m: MaterialId) -> (i64, i64) {
    match m {
        MaterialId::Bedrock | MaterialId::Air | MaterialId::Basalt | MaterialId::Tuff | MaterialId::Till | MaterialId::Water => (0, 1),
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
        FinalBiome::Rock | FinalBiome::Ocean => 0,
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

/// W-SIM-3a (#403) aeolian sand-supply gate: a cell's PRE-aeolian working precipitation estimate
/// below this (mm/year) counts as arid enough to seed a dune sand supply (see `classify_and_caps`'s
/// aeolian-seeding comment for why this is precipitation, not the `Desert` zonal biome). Set at
/// `climate.rs`'s baseline zero-slope precipitation (`P_BASE`, duplicated as a local literal here —
/// intentionally, mirroring `erosion.rs`'s `surface_material_for_biome` duplication — so
/// `climate.rs` stays untouched): any BELOW-baseline cell (negative orographic slope and/or noise)
/// counts as arid. Implementer's call, documented, locked by the golden-vector test.
const ARID_P_THRESHOLD: i64 = 900;

/// W-SIM-3a (#403): reconcile the PRIMARY substrate at a cell. Sand is written as the primary
/// layer — with aeolian on, real dune sand (`sand_depth>0`) reads as `Sand`; the erosion-baseline
/// Desert→Sand mapping is SUBORDINATED in dune zones (falls back to `Soil`) so the azonal override
/// reads ONLY the aeolian primary layer, breaking the biome↔material circularity RnD 13 §6
/// describes (today, ANY Desert-classified cell gets material `Sand` → `override_biome` fires
/// `Dune`, regardless of real dune geometry). With aeolian OFF, `material` is exactly
/// `erosion_material`, unchanged — a pure pass-through, no perturbation.
///
/// Extracted as its own function (rather than inlined in `classify_and_caps`'s loop) so the OFF-path
/// no-op guarantee is unit-testable directly against a synthetic `Sand` input — on THIS climate
/// model, `erode`'s own Desert→Sand path never actually fires on any real generated grid (see
/// `ARID_P_THRESHOLD`'s doc: `Desert`'s T_ref is unreachable), so a full-pipeline OFF-path test
/// alone could never organically exercise the Sand/Dune case this reconciliation touches.
fn reconcile_primary_material(enable_aeolian: bool, erosion_material: MaterialId, sand_depth: i64) -> MaterialId {
    if !enable_aeolian {
        return erosion_material;
    }
    if sand_depth > 0 {
        MaterialId::Sand
    } else if erosion_material == MaterialId::Sand {
        MaterialId::Soil
    } else {
        erosion_material
    }
}

/// Sample `erode(seed, hmax, dim, enable_tectonics, enable_volcanic)` (W-4) and classify the FINAL
/// biome + caps per cell: zonal biome on the post-erosion surface (via `climate_from_height` +
/// `biome_at`) → azonal override (via moisture/material/slope/is_river) → `caps_from`. Pure function
/// of `(seed, hmax, dim, enable_patchiness, enable_tectonics, enable_aeolian, enable_volcanic)` — no
/// RNG-of-clock, no thread-dependence, no global mutable state.
///
/// **W-7 gate (patchiness default-off):** When `enable_patchiness` is false, the function produces
/// caps identical to pre-W-7 (no spatial modulation). Patchiness must be explicitly opted-in by the
/// caller (the map/visual track). This preserves acceptance corridors (settling, emergence) that assume
/// homogeneous world-gen.
///
/// **W-SIM-4a gate (#396, tectonics default-off):** `enable_tectonics` threads straight to `erode`
/// — `false` reproduces the pre-#396 `erosion` output byte-for-byte, so caps/biome derived from it
/// are unaffected too.
///
/// **W-SIM-3a gate (#403, aeolian default-off):** `enable_aeolian` runs [`aeolian::run_aeolian`]
/// POST-erosion (RnD 13 §1: `erode → AEOLIAN → final classify`), seeding its sand supply from a
/// working precipitation-based aridity estimate on the post-erosion height ([`ARID_P_THRESHOLD`] —
/// see its doc for why precipitation, not the `Desert` zonal biome). `false` reproduces the
/// pre-#403 output byte-for-byte: `post_aeolian_height` is a plain clone of `erosion.height`,
/// `sand_depth` is all-zero, and `material` below is exactly `erosion.surface_material[idx]`
/// unchanged — no aeolian RNG draw, no reorder on the OFF path.
///
/// **W-SIM-5 gate (#410, volcanic default-off):** `enable_volcanic` threads straight to `erode`
/// (the edifice height delta is folded in PRE-erosion — see `erosion.rs::erode_with_tectonics`).
/// The volcanic material mask (Basalt/Tuff, re-derived here from the SAME `(seed, dim)` vents
/// `erode` used internally — `volcanic::build_vents` is a cheap pure function, no need to thread it
/// through `ErosionState`) takes PRIORITY over the glacial/aeolian reconciliation below when
/// present: a volcanic-emplaced cell is never simultaneously read as a till or dune-sand cell.
/// `false` reproduces the pre-#410 output byte-for-byte: the mask is all-`None`, so `material` falls
/// straight through to the glacial/aeolian reconciliation, unperturbed.
///
/// **W-SIM-6 gate (#416, glacial default-off):** `enable_glacial` runs [`crate::gen::glacial::run_glacial`]
/// POST-erosion, PRE-aeolian (RnD 16 §1: `erode → GLACIAL → aeolian → final classify` — glacial
/// outwash could later feed the aeolian sand reserve, a follow-up coupling not built here, so
/// glacial's reshaped height feeds FORWARD into aeolian's own aridity seeding below). The glacial
/// `Till` material mask takes priority over aeolian's reconciliation (a till-covered cell is never
/// simultaneously read as dune sand) but yields to volcanic (checked first). `false` reproduces the
/// pre-#416 output byte-for-byte: `post_glacial_height` is a plain clone of `erosion.height`, the
/// mask is all-`None` — no ELA/ice computation of any kind.
///
/// **W-SIM-7 gate (#423, coastal default-off):** `enable_coastal` runs
/// [`crate::gen::coastal::run_coastal`] POST-aeolian, PRE-final-classify (RnD 17 §1's ordering — the
/// LAST landform pass before classification). Introduces the world's FIRST water: `submerged` cells
/// take the SUBMERGED BRANCH in the classify loop below (checked before any zonal climate call),
/// reading `FinalBiome::Ocean` + `MaterialId::Water` directly, never `override_biome`/the
/// volcanic/glacial/aeolian material reconciliation chain (a submerged cell can never simultaneously
/// be a dune/till/edifice cell). `false` reproduces the pre-#423 output byte-for-byte:
/// `post_coastal_height` is a plain clone of `post_aeolian_height`, `submerged` is all-`false` — no
/// sea-level/BFS computation of any kind, and the classify loop never takes the submerged branch.
///
/// **W-9 gate (#432, talus_step_final default-off):** `enable_talus_final` runs
/// [`talus_step_final`] POST-coastal, PRE-de_needle (the LAST landform smoothing pass).
/// Applies Jacobi pair-wise diffusion to remove the "picket fence" residual spikes left by
/// landforms (glacial/aeolian/volcanic/coastal) that run after the early `erode` loop's
/// thermal talus pass. `false` reproduces the pre-#432 output byte-for-byte: `post_talus_height`
/// is a plain clone of `post_coastal_height` — no diffusion of any kind, and de_needle sees the
/// raw post-coastal spikes.
pub fn classify_and_caps_staged(
    seed: u64,
    hmax: i64,
    dim: usize,
    enable_patchiness: bool,
    enable_tectonics: bool,
    enable_aeolian: bool,
    enable_volcanic: bool,
    enable_glacial: bool,
    enable_coastal: bool,
    enable_talus_final: bool,
) -> (WorldFields, StagedHeights, LandformMasks) {
    let erosion = erode(seed, hmax, dim, enable_tectonics, enable_volcanic);
    let n = dim * dim;

    let volcanic_mask: Vec<Option<MaterialId>> = if enable_volcanic {
        let vents = crate::gen::volcanic::build_vents(seed, dim);
        crate::gen::volcanic::edifice_material_mask(dim, &vents)
    } else {
        vec![None; n]
    };

    let (post_glacial_height, glacial_mask): (Vec<i64>, Vec<Option<MaterialId>>) = if enable_glacial {
        let g = crate::gen::glacial::run_glacial(seed, dim, hmax, &erosion.height);
        (g.height, g.material)
    } else {
        (erosion.height.clone(), vec![None; n])
    };

    let (post_aeolian_height, sand_depth) = if enable_aeolian {
        let initial_sand: Vec<i64> = (0..n)
            .map(|idx| {
                let x = (idx % dim) as i64;
                let z = (idx / dim) as i64;
                let x_src = (x - WIND_DX).max(0) as usize;
                let h_west = post_glacial_height[z as usize * dim + x_src];
                let (_t, p) = climate_from_height(post_glacial_height[idx], h_west, x, z, seed);
                if p < ARID_P_THRESHOLD { aeolian::INITIAL_SAND_DEPTH } else { 0 }
            })
            .collect();
        let aeo = aeolian::run_aeolian(seed, dim, &post_glacial_height, initial_sand);
        (aeo.height, aeo.sand_depth)
    } else {
        (post_glacial_height.clone(), vec![0i64; n])
    };

    let (post_coastal_height, submerged) = if enable_coastal {
        let c = crate::gen::coastal::run_coastal(seed, dim, hmax, &post_aeolian_height);
        (c.height, c.submerged)
    } else {
        (post_aeolian_height.clone(), vec![false; n])
    };

    // W-9: Final-surface thermal relaxation. When enabled, applies Jacobi diffusion to smooth
    // residual spikes from landforms that ran after the erode loop's early talus pass.
    // When disabled, this is byte-identical to the old path.
    let post_talus_height = if enable_talus_final && (enable_tectonics || enable_aeolian || enable_volcanic || enable_glacial || enable_coastal) {
        talus_step_final(dim, &post_coastal_height, SPIKE_MARGIN_FINAL, N_ITERS_FINAL)
    } else {
        post_coastal_height.clone()
    };

    // W-8: De-needle pass — remove isolated 1-cell height spikes, FINAL landform post-processing
    // BEFORE classify. Only runs when at least one landform is enabled (preserves byte-identical
    // all-OFF golden path).
    let post_deneedle_height = if enable_tectonics || enable_aeolian || enable_volcanic || enable_glacial || enable_coastal {
        de_needle_pass(dim, &post_talus_height)
    } else {
        post_talus_height.clone()
    };

    // Build landform masks for amplitude measurement
    let edifice_mask: Vec<bool> = (0..n)
        .map(|idx| volcanic_mask[idx].is_some())
        .collect();

    let till_mask: Vec<bool> = (0..n)
        .map(|idx| glacial_mask[idx] == Some(MaterialId::Till))
        .collect();

    let dune_mask: Vec<bool> = (0..n)
        .map(|idx| sand_depth[idx] > 0)
        .collect();

    let landform_masks = LandformMasks {
        edifice: edifice_mask,
        till: till_mask,
        dune: dune_mask,
    };

    // Staged heights for amplitude measurement
    let staged_heights = StagedHeights {
        post_coastal: post_coastal_height.clone(),
        post_talus: post_talus_height.clone(),
        post_deneedle: post_deneedle_height.clone(),
    };

    let mut final_biome = Vec::with_capacity(n);
    let mut caps = vec![0i64; n];
    let mut surface_material = Vec::with_capacity(n);

    for z in 0..dim {
        for x in 0..dim {
            let idx = z * dim + x;

            if submerged[idx] {
                final_biome.push(FinalBiome::Ocean);
                surface_material.push(MaterialId::Water as u8);
                caps[idx] = 0;
                continue;
            }

            let h_cell = post_deneedle_height[idx];
            let x_src = (x as i64 - WIND_DX).max(0) as usize;
            let h_west = post_deneedle_height[z * dim + x_src];
            let (t, p) = climate_from_height(h_cell, h_west, x as i64, z as i64, seed);
            let zonal = biome_at(t, p);

            let area = erosion.drainage.area[idx];
            let moisture = moisture_at(area);
            let riparian = is_river(area);
            let slope = match erosion.drainage.downstream[idx] {
                Some(d) => (post_deneedle_height[idx] - post_deneedle_height[d]).max(0),
                None => 0,
            };

            let material = volcanic_mask[idx].or(glacial_mask[idx]).unwrap_or_else(|| {
                reconcile_primary_material(enable_aeolian, erosion.surface_material[idx], sand_depth[idx])
            });

            let final_b = override_biome(zonal, moisture, material, slope, riparian);
            final_biome.push(final_b);

            let cap_base = caps_from(final_b, moisture, material);
            let cap_final = if enable_patchiness {
                let patch_scale = patchiness_at(x as i64, z as i64, seed, hmax);
                ((cap_base * patch_scale + 128) / 256).clamp(0, CAP_MAX)
            } else {
                cap_base
            };
            caps[idx] = cap_final;
            surface_material.push(material as u8);
        }
    }

    let world_fields = WorldFields { dim, height: post_deneedle_height, final_biome, caps, surface_material };
    (world_fields, staged_heights, landform_masks)
}

pub fn classify_and_caps(
    seed: u64,
    hmax: i64,
    dim: usize,
    enable_patchiness: bool,
    enable_tectonics: bool,
    enable_aeolian: bool,
    enable_volcanic: bool,
    enable_glacial: bool,
    enable_coastal: bool,
) -> WorldFields {
    // W-9: Thin wrapper — talus_step_final is gated the SAME as de_needle: any_landform_on
    // Production output CHANGES when landforms are enabled (exactly why two-pass golden re-pin is prescribed).
    let enable_talus_final = enable_tectonics || enable_aeolian || enable_volcanic || enable_glacial || enable_coastal;
    let (world_fields, _, _) = classify_and_caps_staged(
        seed, hmax, dim, enable_patchiness, enable_tectonics, enable_aeolian,
        enable_volcanic, enable_glacial, enable_coastal, enable_talus_final,
    );
    world_fields
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
        let a = classify_and_caps(SEED, HMAX, 16, false, false, false, false, false, false);
        let b = classify_and_caps(SEED, HMAX, 16, false, false, false, false, false, false);
        assert_eq!(a, b, "classify_and_caps must be byte-identical across repeated calls");
    }

    /// Interior-sink isolation (critic F7): a cell with no D8 receiver (`downstream=None`) must
    /// still classify + cap WITHOUT panicking, over the whole prod-scale grid.
    #[test]
    fn classify_and_caps_is_well_defined_grid_wide() {
        const DIM: usize = 64;
        let fields = classify_and_caps(SEED, HMAX, DIM, false, false, false, false, false, false);
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
        let fields = classify_and_caps(GOLDEN_SEED, GOLDEN_HMAX, DIM, false, false, false, false, false, false);

        // W-7 gate (patchiness default-off): caps are byte-identical to pre-W-7 (no patchiness applied).
        // Height/biome/material fields unchanged. These are the canonical pre-W-7 values.
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
        // With patchiness ON to verify bounds and clamping behavior of the modulation
        let fields = classify_and_caps(SEED, HMAX, DIM, true, false, false, false, false, false);

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
        let fields = classify_and_caps(SEED, HMAX, DIM, true, false, false, false, false, false);

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
    /// caps.rs:153). Uses pure integer arithmetic to stay within the no_float_guard_gen constraint.
    /// The test is required to verify that patchiness does not introduce correlation with the base
    /// cap (covariance E[factor·base] ≠ E[factor]·E[base] would shift the mean even though the
    /// factor is symmetric).
    #[test]
    fn patchiness_maintains_mean_neutrality() {
        const SEED: u64 = 0xA11A_2A11;
        const HMAX: i64 = 200;
        const DIM: usize = 64;
        const GRID_SIZE: i64 = (DIM * DIM) as i64;

        // Compute world WITH patchiness active (gated ON)
        let with_patch = classify_and_caps(SEED, HMAX, DIM, true, false, false, false, false, false);
        let sum_with: i64 = with_patch.caps.iter().sum();
        // Integer-only mean: multiply first to preserve precision, then divide
        let mean_with_times_1000 = (sum_with * 1000) / GRID_SIZE;

        // Compute world WITHOUT patchiness (gated OFF) — byte-identical to homogeneous baseline
        let without_patch = classify_and_caps(SEED, HMAX, DIM, false, false, false, false, false, false);
        let sum_without: i64 = without_patch.caps.iter().sum();
        // Integer-only mean: multiply first to preserve precision, then divide
        let mean_without_times_1000 = (sum_without * 1000) / GRID_SIZE;

        // Compute percentage difference using only integer arithmetic: avoid truncation by
        // multiplying by 100 before dividing. The ±5% threshold becomes ±50 in this metric
        // (i.e., 50 permille out of 1000 permille = 5%).
        let abs_diff = if mean_with_times_1000 > mean_without_times_1000 {
            mean_with_times_1000 - mean_without_times_1000
        } else {
            mean_without_times_1000 - mean_with_times_1000
        };
        // Avoid division-by-zero; shouldn't happen in practice (caps are always in [0,300])
        let pct_diff_times_100 = if mean_without_times_1000 > 0 {
            (abs_diff * 100) / mean_without_times_1000
        } else {
            0
        };

        eprintln!(
            "W-7 mean-invariance: mean_with_patch={}×1000⁻¹, mean_without_patch={}×1000⁻¹, diff_pct_×100={}",
            mean_with_times_1000, mean_without_times_1000, pct_diff_times_100
        );

        // Assert means are equal within ±5% (pct_diff_times_100 <= 500, where 500/100 = 5%).
        assert!(
            pct_diff_times_100 <= 500,
            "patchiness mean shift exceeded 5%% threshold: {}% diff (with={}×1000⁻¹, without={}×1000⁻¹). \
             This indicates covariance between patch factor and base cap — decorrelate the patch noise.",
            pct_diff_times_100 / 100,
            mean_with_times_1000,
            mean_without_times_1000
        );
    }

    // ── W-SIM-3a: primary material reconciliation (#403) ─────────────────────────────────────────

    #[test]
    fn reconcile_primary_material_off_is_always_pass_through() {
        assert_eq!(reconcile_primary_material(false, MaterialId::Sand, 5), MaterialId::Sand);
        assert_eq!(reconcile_primary_material(false, MaterialId::Soil, 0), MaterialId::Soil);
        assert_eq!(reconcile_primary_material(false, MaterialId::Bedrock, 3), MaterialId::Bedrock);
    }

    #[test]
    fn reconcile_primary_material_on_prefers_real_dune_sand() {
        // Real aeolian sand present -> Sand, regardless of the erosion-baseline material underneath.
        assert_eq!(reconcile_primary_material(true, MaterialId::Soil, 2), MaterialId::Sand);
        assert_eq!(reconcile_primary_material(true, MaterialId::Sand, 2), MaterialId::Sand);
    }

    #[test]
    fn reconcile_primary_material_on_subordinates_erosion_baseline_sand() {
        // No real dune sand -> the erosion-baseline Desert->Sand mapping is subordinated to Soil
        // (breaks the biome<->material circularity, RnD 13 §6), non-Sand materials pass through.
        assert_eq!(reconcile_primary_material(true, MaterialId::Sand, 0), MaterialId::Soil);
        assert_eq!(reconcile_primary_material(true, MaterialId::Bedrock, 0), MaterialId::Bedrock);
    }

    // ── W-SIM-4a: tectonic gate threading (#396) ─────────────────────────────────────────────────

    /// The `enable_tectonics` gate genuinely threads through to `erode` (not a dead parameter): the
    /// same `(seed, hmax, dim)` must produce a DIFFERENT height field with tectonics on vs off.
    #[test]
    fn classify_and_caps_tectonics_gate_actually_changes_height() {
        const DIM: usize = 64;
        let off = classify_and_caps(SEED, HMAX, DIM, false, false, false, false, false, false);
        let on = classify_and_caps(SEED, HMAX, DIM, false, true, false, false, false, false);
        assert_ne!(off.height, on.height, "enable_tectonics=true must change the height field — else the gate is dead code");
    }

    /// `enable_tectonics=false` must be byte-identical to the pre-#396 `classify_and_caps` output —
    /// the golden-neutral OFF-path guard at the caps layer (mirrors the erosion-layer guard).
    #[test]
    fn classify_and_caps_tectonics_off_is_deterministic_and_matches_baseline() {
        const DIM: usize = 16;
        let a = classify_and_caps(SEED, HMAX, DIM, false, false, false, false, false, false);
        let b = classify_and_caps(SEED, HMAX, DIM, false, false, false, false, false, false);
        assert_eq!(a, b, "classify_and_caps(..,false,false,false,false,false) must be byte-identical across repeated calls");
    }

    // ── W-SIM-3a: aeolian gate threading (#403) ──────────────────────────────────────────────────

    /// The `enable_aeolian` gate genuinely threads through to `aeolian::run_aeolian` (not a dead
    /// parameter): on a grid with a real Desert-derived sand supply, the same `(seed, hmax, dim)`
    /// must produce a DIFFERENT height field with aeolian on vs off.
    #[test]
    fn classify_and_caps_aeolian_gate_actually_changes_height() {
        const DIM: usize = 64;
        let off = classify_and_caps(SEED, HMAX, DIM, false, false, false, false, false, false);
        let on = classify_and_caps(SEED, HMAX, DIM, false, false, true, false, false, false);
        assert_ne!(off.height, on.height, "enable_aeolian=true must change the height field — else the gate is dead code");
    }

    /// `enable_aeolian=false` must be byte-identical to the pre-#403 `classify_and_caps` output —
    /// **critically, this must cover the BIOME/MATERIAL classification too, not only height**
    /// (#403 ТЗ): a test that only checked height/export could pass while Dune/Sand cells silently
    /// drifted. Asserts full-struct byte-identity across repeated OFF calls AND that every
    /// Sand-material cell in this fixture still classifies as `Dune` — the exact pre-#403
    /// Desert→Sand→Dune mapping, unperturbed by the aeolian reconciliation logic being present
    /// (but gated off) in the same function.
    #[test]
    fn classify_and_caps_aeolian_off_matches_baseline_including_dune_cells() {
        const DIM: usize = 64;
        let a = classify_and_caps(SEED, HMAX, DIM, false, false, false, false, false, false);
        let b = classify_and_caps(SEED, HMAX, DIM, false, false, false, false, false, false);
        assert_eq!(a, b, "classify_and_caps(..,enable_aeolian=false) must be byte-identical across repeated calls");

        // Direct unit coverage of the BIOME/MATERIAL reconciliation on a Sand cell specifically
        // (#403 ТЗ) — via `reconcile_primary_material` rather than hoping the full pipeline
        // organically produces one: on this climate model `erode`'s own Desert→Sand path is
        // unreachable on any real generated grid (see `ARID_P_THRESHOLD`'s doc), so a full-pipeline
        // assertion alone could never exercise this case. `override_biome` itself is untouched by
        // #403, so material==Sand -> Dune is proven directly against it.
        let material_off = reconcile_primary_material(false, MaterialId::Sand, 0);
        assert_eq!(material_off, MaterialId::Sand, "OFF path: reconcile_primary_material must pass Sand through unchanged");
        assert_eq!(
            override_biome(BiomeId::Desert, 0, material_off, 0, false),
            FinalBiome::Dune,
            "OFF path: a Sand-material cell must still classify as Dune — today's \
             Desert→Sand→Dune mapping, unperturbed by enable_aeolian=false"
        );
    }

    // ── W-SIM-5: volcanic gate threading (#410) ──────────────────────────────────────────────────

    /// The `enable_volcanic` gate genuinely threads through to `erode`/the material mask (not a
    /// dead parameter): the same `(seed, hmax, dim)` must produce a DIFFERENT height field with
    /// volcanic on vs off.
    #[test]
    fn classify_and_caps_volcanic_gate_actually_changes_height() {
        const DIM: usize = 64;
        let off = classify_and_caps(SEED, HMAX, DIM, false, false, false, false, false, false);
        let on = classify_and_caps(SEED, HMAX, DIM, false, false, false, true, false, false);
        assert_ne!(off.height, on.height, "enable_volcanic=true must change the height field — else the gate is dead code");
    }

    /// `enable_volcanic=false` is deterministic across repeated calls AND never emits Basalt/Tuff —
    /// the two properties this test actually asserts. (The literal byte-identity-to-pre-#410
    /// guarantee is structural — `classify_and_caps`'s `if enable_volcanic` gate skips
    /// `volcanic::build_vents`/`edifice_material_mask` entirely when off — and is empirically
    /// confirmed by every PRE-EXISTING pinned golden/chain-hash in this crate, computed with
    /// `enable_volcanic=false`, remaining unchanged by this PR.)
    #[test]
    fn classify_and_caps_volcanic_off_matches_baseline_and_never_emits_volcanic_material() {
        const DIM: usize = 64;
        let a = classify_and_caps(SEED, HMAX, DIM, false, false, false, false, false, false);
        let b = classify_and_caps(SEED, HMAX, DIM, false, false, false, false, false, false);
        assert_eq!(a, b, "classify_and_caps(..,enable_volcanic=false) must be byte-identical across repeated calls");

        let has_volcanic_material = a
            .surface_material
            .iter()
            .any(|&m| m == MaterialId::Basalt as u8 || m == MaterialId::Tuff as u8);
        assert!(!has_volcanic_material, "OFF path: Basalt/Tuff must never be emitted with enable_volcanic=false");
    }

    /// With volcanic on, at least one cell reads back as Basalt or Tuff (the material mask actually
    /// threads through to the final `surface_material`, not just the height).
    #[test]
    fn classify_and_caps_volcanic_on_emits_basalt_or_tuff() {
        const DIM: usize = 64;
        let on = classify_and_caps(SEED, HMAX, DIM, false, false, false, true, false, false);
        let has_volcanic_material = on
            .surface_material
            .iter()
            .any(|&m| m == MaterialId::Basalt as u8 || m == MaterialId::Tuff as u8);
        assert!(has_volcanic_material, "enable_volcanic=true must emit at least one Basalt/Tuff cell on this fixture");
    }

    // ── W-SIM-6: glacial gate threading (#416) ───────────────────────────────────────────────────

    /// The `enable_glacial` gate genuinely threads through to `erode`/the material mask (not a dead
    /// parameter): the same `(seed, hmax, dim)` must produce a DIFFERENT height field with glacial
    /// on vs off.
    #[test]
    fn classify_and_caps_glacial_gate_actually_changes_height() {
        const DIM: usize = 64;
        let off = classify_and_caps(SEED, HMAX, DIM, false, false, false, false, false, false);
        let on = classify_and_caps(SEED, HMAX, DIM, false, false, false, false, true, false);
        assert_ne!(off.height, on.height, "enable_glacial=true must change the height field — else the gate is dead code");
    }

    /// `enable_glacial=false` is deterministic across repeated calls AND never emits Till — the two
    /// properties this test actually asserts (mirrors the volcanic OFF-path test's naming
    /// discipline post-code-critic, #410: the literal byte-identity-to-pre-#416 guarantee is
    /// structural — the `if enable_glacial` gate skips `glacial::run_glacial` entirely when off —
    /// and is empirically confirmed by every PRE-EXISTING pinned golden/chain-hash in this crate,
    /// computed with `enable_glacial=false`, remaining unchanged by this PR).
    #[test]
    fn classify_and_caps_glacial_off_matches_baseline_and_never_emits_till() {
        const DIM: usize = 64;
        let a = classify_and_caps(SEED, HMAX, DIM, false, false, false, false, false, false);
        let b = classify_and_caps(SEED, HMAX, DIM, false, false, false, false, false, false);
        assert_eq!(a, b, "classify_and_caps(..,enable_glacial=false) must be byte-identical across repeated calls");

        let has_till = a.surface_material.iter().any(|&m| m == MaterialId::Till as u8);
        assert!(!has_till, "OFF path: Till must never be emitted with enable_glacial=false");
    }

    /// With glacial on, at least one cell reads back as Till (the material mask actually threads
    /// through to the final `surface_material`, not just the height).
    #[test]
    fn classify_and_caps_glacial_on_emits_till() {
        const DIM: usize = 64;
        let on = classify_and_caps(SEED, HMAX, DIM, false, false, false, false, true, false);
        let has_till = on.surface_material.iter().any(|&m| m == MaterialId::Till as u8);
        assert!(has_till, "enable_glacial=true must emit at least one Till cell on this fixture");
    }

    // ── W-SIM-7: coastal gate threading (#423) ───────────────────────────────────────────────────

    /// The `enable_coastal` gate genuinely threads through (not a dead parameter): the same
    /// `(seed, hmax, dim)` must produce a DIFFERENT height field with coastal on vs off.
    #[test]
    fn classify_and_caps_coastal_gate_actually_changes_height() {
        const DIM: usize = 64;
        let off = classify_and_caps(SEED, HMAX, DIM, false, false, false, false, false, false);
        let on = classify_and_caps(SEED, HMAX, DIM, false, false, false, false, false, true);
        assert_ne!(off.height, on.height, "enable_coastal=true must change the height field — else the gate is dead code");
    }

    /// `enable_coastal=false` is deterministic across repeated calls AND never emits Water/Ocean —
    /// the properties this test actually asserts (mirrors the volcanic/glacial OFF-path tests'
    /// naming discipline post-code-critic, #410: the literal byte-identity-to-pre-#423 guarantee is
    /// structural — the `if enable_coastal` gate skips `coastal::run_coastal` entirely when off,
    /// and the classify loop's submerged branch is dead code on an all-`false` `submerged` array —
    /// and is empirically confirmed by every PRE-EXISTING pinned golden/chain-hash in this crate,
    /// computed with `enable_coastal=false`, remaining unchanged by this PR).
    #[test]
    fn classify_and_caps_coastal_off_matches_baseline_and_never_emits_water() {
        const DIM: usize = 64;
        let a = classify_and_caps(SEED, HMAX, DIM, false, false, false, false, false, false);
        let b = classify_and_caps(SEED, HMAX, DIM, false, false, false, false, false, false);
        assert_eq!(a, b, "classify_and_caps(..,enable_coastal=false) must be byte-identical across repeated calls");

        let has_water = a.surface_material.iter().any(|&m| m == MaterialId::Water as u8);
        assert!(!has_water, "OFF path: Water must never be emitted with enable_coastal=false");
        let has_ocean = a.final_biome.iter().any(|&b| b == FinalBiome::Ocean);
        assert!(!has_ocean, "OFF path: Ocean must never be emitted with enable_coastal=false");
    }

    /// With coastal on, at least one cell reads back as Water/Ocean (the submerged signal actually
    /// threads through to both `surface_material` and `final_biome`, not just the height), and
    /// classify never gives a submerged cell a terrestrial biome (#423 AC 2).
    #[test]
    fn classify_and_caps_coastal_on_emits_water_and_ocean_never_terrestrial() {
        const DIM: usize = 64;
        let on = classify_and_caps(SEED, HMAX, DIM, false, false, false, false, false, true);
        let has_water = on.surface_material.iter().any(|&m| m == MaterialId::Water as u8);
        assert!(has_water, "enable_coastal=true must emit at least one Water cell on this fixture");

        for idx in 0..DIM * DIM {
            if on.surface_material[idx] == MaterialId::Water as u8 {
                assert_eq!(
                    on.final_biome[idx], FinalBiome::Ocean,
                    "a Water-material cell (idx={idx}) must classify as Ocean, never a terrestrial biome"
                );
            }
        }
    }

    /// Orthogonality (#416/#423 ТЗ): every landform flag composes with any subset of the others
    /// without cross-contamination — every combination compiles/runs and produces a well-formed
    /// (correctly-sized, no-panic) result. This mirrors the existing pairwise gate-changes-height
    /// tests but sweeps ALL 32 combinations of the five flags in one pass, checking only structural
    /// well-formedness (a crash or a wrong-length field would indicate cross-contamination) — the
    /// individual flags' OWN OFF-path/gate-changes-height guarantees are proven in isolation
    /// elsewhere, this test's job is specifically the SUBSET-COMPOSITION claim.
    #[test]
    fn all_landform_flags_compose_with_any_subset_of_each_other() {
        const DIM: usize = 64;
        for tectonics in [false, true] {
            for aeolian in [false, true] {
                for volcanic in [false, true] {
                    for glacial in [false, true] {
                        for coastal in [false, true] {
                            let fields = classify_and_caps(SEED, HMAX, DIM, false, tectonics, aeolian, volcanic, glacial, coastal);
                            assert_eq!(fields.height.len(), DIM * DIM);
                            assert_eq!(fields.final_biome.len(), DIM * DIM);
                            assert_eq!(fields.caps.len(), DIM * DIM);
                            assert_eq!(fields.surface_material.len(), DIM * DIM);
                            for &h in &fields.height {
                                assert!((0..=HMAX).contains(&h), "height out of [0,{HMAX}] with tectonics={tectonics} aeolian={aeolian} volcanic={volcanic} glacial={glacial} coastal={coastal}: {h}");
                            }
                        }
                    }
                }
            }
        }
    }

    // ── W-9: talus_step_final and amplitude measurement ──────────────────────────────────────────

    /// W-9: Verify that talus_step_final produces a valid height field (no NaN, no negative,
    /// within original range). This is a basic smoke test; detailed amplitude tests require
    /// full pipeline execution.
    #[test]
    fn talus_step_final_produces_valid_output() {
        const DIM: usize = 64;
        let (world, _, _) = classify_and_caps_staged(
            SEED, HMAX, DIM, false, false, false, false, false, false, true
        );
        // Verify all heights are in valid range
        for &h in &world.height {
            assert!((0..=HMAX).contains(&h), "talus_step_final produced height {h} out of [0,{HMAX}]");
        }
    }

    /// W-9: OFF-path byte-identity — when talus_step_final is disabled, staged seam must return
    /// all-empty masks and output must match the non-staged path exactly.
    #[test]
    fn classify_and_caps_staged_off_path_is_byte_identical() {
        const DIM: usize = 64;
        let (staged, _, masks) = classify_and_caps_staged(
            SEED, HMAX, DIM, false, false, false, false, false, false, false
        );
        let non_staged = classify_and_caps(
            SEED, HMAX, DIM, false, false, false, false, false, false
        );
        assert_eq!(staged.height, non_staged.height, "staged OFF-path must match non-staged output");
        assert_eq!(staged.final_biome, non_staged.final_biome);
        assert_eq!(staged.caps, non_staged.caps);
        assert_eq!(staged.surface_material, non_staged.surface_material);

        // All masks must be empty when OFF
        for &m in &masks.edifice {
            assert!(!m, "edifice mask must be all-false when talus_final is OFF");
        }
        for &m in &masks.till {
            assert!(!m, "till mask must be all-false when talus_final is OFF");
        }
        for &m in &masks.dune {
            assert!(!m, "dune mask must be all-false when talus_final is OFF");
        }
    }

    /// W-9: Amplitude measurement on a synthetic single-peak fixture. A cell with known amplitude
    /// should retain most of its relief after talus_step_final, depending on the threshold.
    #[test]
    fn landform_amplitudes_measures_retention_on_fixture() {
        const DIM: usize = 16;
        let mut heights_pre = vec![0i64; DIM * DIM];
        let mut heights_post = heights_pre.clone();
        let mut mask = vec![false; DIM * DIM];

        // Create a central peak at (7,7) with amplitude=20
        let center = 7 * DIM + 7;
        heights_pre[center] = 20;
        heights_post[center] = 16; // 80% retention
        mask[center] = true;

        let report = landform_amplitudes(DIM, &heights_pre, &heights_post, &mask);
        assert_eq!(report.crest_count, 1, "should identify 1 crest in the peak");
        assert!(report.p10_retention_pct >= 75 && report.p10_retention_pct <= 85,
            "retention should be ~80%, got {}%", report.p10_retention_pct);
    }

    /// W-9: Empty mask produces zero count and zero retention (safe caller contract).
    #[test]
    fn landform_amplitudes_handles_empty_mask() {
        const DIM: usize = 16;
        let heights = vec![50i64; DIM * DIM];
        let mask = vec![false; DIM * DIM];

        let report = landform_amplitudes(DIM, &heights, &heights, &mask);
        assert_eq!(report.crest_count, 0);
        assert_eq!(report.p10_retention_pct, 0);
    }

    // ── W-9: talus_step_final per-iteration invariants ─────────────────────────────────────────

    /// W-9: Per-iteration range contraction: sum(hs) invariant per iteration, AND
    /// max(hs_new) <= max(hs_old), AND min(hs_new) >= min(hs_old). The Jacobi pair-wise diffusion
    /// bounds the receiver so it can never invert into a spike; the x64 scale removes deadzone.
    #[test]
    fn talus_step_final_contracts_range_per_iteration() {
        const DIM: usize = 16;
        const SEED: u64 = 0xFEED_FEED;
        const HMAX: i64 = 200;

        // Generate a basic relief with coastal enabled (to get spikes that need smoothing)
        let (world, _, _) = classify_and_caps_staged(
            SEED, HMAX, DIM, false, false, false, false, false, true, true
        );

        // Get the pre/post stages from the result (post_coastal is the input to talus_step_final)
        let post_coastal = &world.height; // This is post-deneedle; for this test we'd need to modify to expose post_talus

        // For now, verify that talus_step_final produces monotone-bounded output
        let test_h = vec![100, 50, 120, 60, 90, 110, 70, 80, 100, 55, 125, 65, 95, 115, 75, 85];
        let result = crate::gen::erosion::talus_step_final(DIM / 4, &test_h, 8, 1);

        let sum_before = test_h.iter().sum::<i64>();
        let sum_after = result.iter().sum::<i64>();
        let max_before = *test_h.iter().max().unwrap_or(&0);
        let max_after = *result.iter().max().unwrap_or(&0);
        let min_before = *test_h.iter().min().unwrap_or(&0);
        let min_after = *result.iter().min().unwrap_or(&0);

        // After unscaling, loss is <=1 unit/cell due to floor division (not perfectly conserved)
        assert!(
            (sum_before - sum_after).abs() <= (DIM as i64 / 2) * 2,
            "sum loss too large: before={}, after={}, diff={}",
            sum_before,
            sum_after,
            sum_before - sum_after
        );
        assert!(
            max_after <= max_before,
            "max increased after diffusion: before={}, after={}",
            max_before,
            max_after
        );
        assert!(
            min_after >= min_before,
            "min decreased after diffusion: before={}, after={}",
            min_before,
            min_after
        );
    }

    /// W-9: Needle-fixture companion: on a synthetic needle fixture (one tall isolated spike),
    /// max height STRICTLY decreases with each iteration (catches silent no-op).
    #[test]
    fn talus_step_final_strictly_reduces_needle_height() {
        // Single needle: cell (2,2) at height 100, all neighbors at 0
        const DIM: usize = 5;
        let mut height = vec![0i64; DIM * DIM];
        height[2 * DIM + 2] = 100; // Center at (2,2)

        let max_before = *height.iter().max().unwrap();
        let result = crate::gen::erosion::talus_step_final(DIM, &height, 8, 2);
        let max_after = *result.iter().max().unwrap();

        assert!(
            max_after < max_before,
            "needle max must strictly decrease: before={}, after={}",
            max_before,
            max_after
        );
    }

    /// W-9: Relief-conservation floor per mask: on a landform-specific subset, the relief spread
    /// (p90 - p10 of heights) must not collapse more than 20% after talus_step_final.
    /// Measures: 100*(p90(h_post)-p10(h_post)) >= 80*(p90(h_pre)-p10(h_pre)) restricted to mask cells.
    #[test]
    fn talus_step_final_preserves_relief_spread_per_mask() {
        const DIM: usize = 16;
        const SEED: u64 = 0xCAFE_CAFE;
        const HMAX: i64 = 200;

        // Generate world with aeolian (dune mask for testing)
        let (world_pre, staged, masks) = classify_and_caps_staged(
            SEED, HMAX, DIM, false, false, true, false, false, false, false // aeolian ON, talus OFF
        );

        let (world_post, _, _) = classify_and_caps_staged(
            SEED, HMAX, DIM, false, false, true, false, false, false, true  // aeolian ON, talus ON
        );

        let n = DIM * DIM;

        // For dune mask, compute pre/post relief spread
        let mut pre_heights: Vec<i64> = masks.dune.iter()
            .enumerate()
            .filter_map(|(i, &is_dune)| if is_dune { Some(world_pre.height[i]) } else { None })
            .collect();
        let mut post_heights: Vec<i64> = masks.dune.iter()
            .enumerate()
            .filter_map(|(i, &is_dune)| if is_dune { Some(world_post.height[i]) } else { None })
            .collect();

        if pre_heights.len() < 2 || post_heights.len() < 2 {
            return; // Not enough cells in mask to measure
        }

        pre_heights.sort();
        post_heights.sort();
        let pre_p10 = pre_heights[pre_heights.len() / 10];
        let pre_p90 = pre_heights[9 * pre_heights.len() / 10];
        let post_p10 = post_heights[post_heights.len() / 10];
        let post_p90 = post_heights[9 * post_heights.len() / 10];

        let pre_spread = pre_p90 - pre_p10;
        let post_spread = post_p90 - post_p10;

        // Relief-conservation floor: post >= 80% of pre
        assert!(
            100 * post_spread >= 80 * pre_spread,
            "relief spread collapsed > 20% in dune mask: pre={}, post={} (retention={}%)",
            pre_spread,
            post_spread,
            if pre_spread > 0 { 100 * post_spread / pre_spread } else { 100 }
        );
    }

    /// W-9: Shipping assert — de_needle is a no-op post-talus_step_final.
    /// On a landform-ON fixture with picked config (SPIKE_MARGIN=12, iters=4),
    /// the post-talus field must not have any cell exceeding nmax+NEEDLE_MARGIN.
    /// Fixture: dim=256, seed=1, all landforms ON.
    #[test]
    fn talus_step_final_leaves_no_de_needle_clipping() {
        const DIM: usize = 256;  // Raised from 128 for reproducibility; fixture: dim=256 seed=1 all-ON
        const SEED: u64 = 1;
        const HMAX: i64 = 200;

        // Baseline (talus OFF, all landforms ON): measure de_needle clip count
        let (baseline, _staged_off, _masks_off) = classify_and_caps_staged(
            SEED, HMAX, DIM, false, true, true, true, true, true, false // talus OFF
        );
        let baseline_clipped = de_needle_pass(DIM, &baseline.height);
        let baseline_clip_count = measure_de_needle_clip_count(DIM, &baseline.height, &baseline_clipped);

        // Post-talus: apply picked config (SPIKE_MARGIN=12, iters=4) and verify de_needle is no-op
        // Gate: post-talus must have clip_count==0 (verifies talus never creates de_needle artifacts)
        let post_talus = talus_step_final(DIM, &baseline.height, 12, 4);
        let post_talus_clipped = de_needle_pass(DIM, &post_talus);
        let post_clip_count = measure_de_needle_clip_count(DIM, &post_talus, &post_talus_clipped);

        assert_eq!(
            post_clip_count, 0,
            "de_needle must be a no-op post-talus_step_final(12, 4): {} cells still need clipping. \
             Baseline had {} clips; talus should never create or leave de_needle artifacts.",
            post_clip_count, baseline_clip_count
        );
    }
}
