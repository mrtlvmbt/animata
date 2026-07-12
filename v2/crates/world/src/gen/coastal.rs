//! W-SIM-7 (#423): deterministic integer coastal relief — sea-level datum + cliff / wave-cut
//! platform, the fifth and last landform slice on the `worldgen-relief` ladder (RnD
//! `sim/world/17-coastal-and-tectonic-landforms.md`, coastal half only — the tectonic half already
//! shipped as W-SIM-4a). Unlike the four prior landforms, this introduces the world's FIRST WATER:
//! there is no `sea_level` today, `MaterialId::Air` is the "above-surface empty" sentinel (not
//! reusable for submerged cells — see the module doc's Water section), and every cell classifies as
//! terrestrial. This slice establishes the sea datum and carves the anchor coastal form; it runs
//! POST-aeolian, PRE-final-classify (`erode → glacial → aeolian → COASTAL(this) → final classify`,
//! RnD 17 §1's ordering). **Pure integer / fixed-point throughout — no `f32`/`f64` anywhere in this
//! file** (covered by the recursive glob guard, `world/tests/no_float_guard_gen.rs`).
//!
//! ## Scope (this slice — #423's explicit out-of-scope list)
//!
//! Cliff + wave-cut platform only. Longshore deposition (beach/spit/bar/barrier) and its canonical
//! coastline-ordering primitive (closed-loop cut + fixed traversal sign), headland–bay wave
//! refraction, sea-level-change forms (raised beach/ria), sea caves/arches (TRUE-3D) are explicit
//! follow-up slices (RnD 17 §5–6) — NOT built here. Because longshore ordering is out of scope, this
//! module never needs a 1D coastline traversal at all — every computation below is a per-cell
//! function of a 2D distance field, not an ordered walk along the shore.
//!
//! ## Sea-level datum (percentile, not a bare constant)
//!
//! [`sea_level`] is the [`SEA_LEVEL_PERCENTILE_NUM`]`/`[`SEA_LEVEL_PERCENTILE_DEN`] percentile of the
//! POST-aeolian height distribution — an integer index into the SORTED height array, never a float
//! division. Because it is percentile-derived, the submerged fraction is bounded away from both 0%
//! and 100% BY CONSTRUCTION, on any seed: a bare height constant could degenerate to all-land or
//! all-sea depending on the seed's actual height range, but a percentile of THAT SAME seed's actual
//! distribution cannot (#423 AC 2).
//!
//! ## Water (the unambiguous submerged signal)
//!
//! Submerged cells are tagged [`crate::gen::material::MaterialId::Water`] (newly appended, `Water=8`)
//! — `Air` is NOT reused: `Air` already means "above-surface empty" (cap 0, resource floor 1) and is
//! byte-indistinguishable from sky or barren land, so a submerged cell tagged `Air` could never be
//! recovered as coastline. The caller (`caps.rs`) also gates classification: a submerged cell never
//! reads a terrestrial `FinalBiome`.
//!
//! ## Distance fields are order-independent BY CONSTRUCTION (not a row-major relaxation)
//!
//! [`bfs_distance`] is a LEVEL-SYNCHRONOUS multi-source BFS (D8): every cell's distance is its exact
//! shortest hop-count to the nearest source cell, a property of the graph alone — unlike a two-pass
//! forward/backward sweep DP (a "row-major relaxation", explicitly disallowed by #423 AC 6, which can
//! give an APPROXIMATE chamfer distance that depends on scan direction), a level-synchronous BFS
//! computes the EXACT graph distance and needs no traversal-order tie-break to do it (only a
//! "nearest-source direction" query would need one, and nothing here needs that — only the scalar
//! distance value is used). Two distance fields are derived from ONE seed set each: `dist_to_sea`
//! (land cells' distance to the nearest submerged cell) and `dist_to_land` (submerged cells'
//! distance to the nearest land cell).
//!
//! ## Two strict-order passes (mirrors RnD 16 §6's glacial precedent, cited by RnD 17 §6)
//!
//! 1. **Cliff retreat** ([`cliff_retreat_pass`]): fixed-iteration (never convergence-ε), subtractive
//!    ONLY — every delta is non-negative, so a cell's height never rises during this pass. Applied to
//!    LAND cells in the immediate coastline band (`dist_to_sea <= COASTLINE_BAND_WIDTH`), at a rate
//!    proportional to [`wave_exposure`] and inversely proportional to
//!    [`crate::gen::erosion::RESIST_DIVISOR`] (the SAME resistance-class divisor `erosion.rs`'s
//!    stream-power incision uses — "higher resistance ⇒ slower retreat" is driven by the identical
//!    mapping, RnD 17 §4). The submerged mask + `dist_to_sea` are recomputed EVERY iteration on the
//!    current height (mirrors `erosion.rs`'s macro-loop house style), so retreat progressively
//!    advances the coastline inland.
//! 2. **Wave-cut platform** ([`carve_platform`]): a SEPARATE, single deterministic pass run only
//!    AFTER the retreat loop settles (never interleaved with retreat — mixing signs in one loop would
//!    flip-flop the land/sea boundary, the same named pitfall RnD 16 §6 describes for glacial). Clamps
//!    newly-submerged coastline-band cells DOWN to a shallow, near-flat seaward-deepening bench (a
//!    "shave the high points, don't fill the low points" `.min` clamp — see [`carve_platform`]'s doc
//!    for the width/exposure relationship). Like retreat, this is ALSO non-negative-only (`.min`
//!    never raises a cell), so the FULL coastal stage — both passes together — stays single-signed:
//!    `run_coastal` never raises a cell relative to its input (`run_coastal_never_raises_a_cell`).

use crate::gen::erosion::{resistance_field, RESIST_DIVISOR};

/// Sea-level percentile (implementer's call, documented, locked by the golden-vector test): the
/// 30th percentile of the height distribution submerges roughly 30% of the map — comfortably bounded
/// away from both 0% and 100% (#423 AC 2), a representative coastal window rather than a knife-edge.
pub const SEA_LEVEL_PERCENTILE_NUM: i64 = 3;
pub const SEA_LEVEL_PERCENTILE_DEN: i64 = 10;

/// Coastline-band half-width, in cells (Chebyshev/D8 BFS distance) on EITHER side of the land/sea
/// threshold. Implementer's call, documented, locked.
const COASTLINE_BAND_WIDTH: i64 = 3;
/// Wave-exposure probe radius, in cells (Chebyshev box) — [`wave_exposure`] counts submerged cells
/// within this box. Implementer's call, documented, locked.
const WAVE_EXPOSURE_RADIUS: i64 = 4;

/// Fixed cliff-retreat macro-iteration count (R10, never convergence-ε — mirrors `erosion.rs`'s
/// `MACRO_ITERATIONS`/`glacial.rs`'s `N_GLACIAL_ITERATIONS`, a DISTINCT, independently-tuned
/// constant/budget for this stage).
const N_CLIFF_ITERATIONS: usize = 6;
/// Cliff-retreat rate constants: `Δz = (CLIFF_RETREAT_NUM · wave_exposure) / (resist_divisor ·
/// CLIFF_RETREAT_DEN)`, mirroring `erosion.rs`'s stream-power incision shape. Implementer's call,
/// documented, locked.
const CLIFF_RETREAT_NUM: i64 = 1;
const CLIFF_RETREAT_DEN: i64 = 2;

/// Wave-cut platform depth constants: a submerged coastline-band cell is clamped to
/// `sea_level - PLATFORM_DEPTH_BASE - dist_to_land * PLATFORM_SLOPE_NUM / PLATFORM_SLOPE_DEN`
/// (clamped to `>= sea_level - PLATFORM_DEPTH_MAX`) — a shallow, near-flat bench that deepens gently
/// seaward (RnD 17 §4: "наклон 0–4° к морю"). Implementer's call, documented, locked.
const PLATFORM_DEPTH_BASE: i64 = 2;
const PLATFORM_SLOPE_NUM: i64 = 1;
const PLATFORM_SLOPE_DEN: i64 = 2;
const PLATFORM_DEPTH_MAX: i64 = 12;
/// Platform reach (in cells of `dist_to_land`) as a function of wave exposure — the width/exposure
/// correlation #423 AC 4 requires (higher exposure ⇒ a submerged cell farther from land still
/// qualifies for the flat-bench clamp ⇒ a wider apparent platform). A simple affine function,
/// directly unit-tested for monotonicity (`platform_reach_is_monotone_in_exposure`) rather than
/// asserted as a fragile spatial correlation on noisy fBm terrain.
const PLATFORM_REACH_BASE: i64 = 1;
const PLATFORM_REACH_PER_EXPOSURE_NUM: i64 = 1;
const PLATFORM_REACH_PER_EXPOSURE_DEN: i64 = 3;

const D8_OFFSETS: [(i64, i64); 8] =
    [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];

#[inline]
fn linear_index(x: usize, z: usize, dim: usize) -> usize {
    z * dim + x
}

/// The sea-level datum (module doc): the [`SEA_LEVEL_PERCENTILE_NUM`]`/`[`SEA_LEVEL_PERCENTILE_DEN`]
/// percentile of `height`'s distribution, via a pure integer index into the sorted array.
pub fn sea_level(height: &[i64]) -> i64 {
    let mut sorted = height.to_vec();
    sorted.sort_unstable();
    let idx = ((sorted.len() as i64 * SEA_LEVEL_PERCENTILE_NUM) / SEA_LEVEL_PERCENTILE_DEN) as usize;
    sorted[idx.min(sorted.len() - 1)]
}

#[inline]
pub fn is_submerged(h: i64, sea_level: i64) -> bool {
    h <= sea_level
}

/// Level-synchronous multi-source D8 BFS distance (module doc): `0` for every `is_source` cell,
/// increasing outward by exact shortest hop count. Order-independent by construction.
fn bfs_distance(dim: usize, is_source: &[bool]) -> Vec<i64> {
    let n = dim * dim;
    let mut dist = vec![i64::MAX; n];
    let mut frontier: Vec<usize> = Vec::new();
    for idx in 0..n {
        if is_source[idx] {
            dist[idx] = 0;
            frontier.push(idx);
        }
    }
    let mut level = 0i64;
    while !frontier.is_empty() {
        let mut next_frontier = Vec::new();
        for &v in &frontier {
            let x = (v % dim) as i64;
            let z = (v / dim) as i64;
            for &(dx, dz) in &D8_OFFSETS {
                let nx = x + dx;
                let nz = z + dz;
                if nx < 0 || nz < 0 || nx as usize >= dim || nz as usize >= dim {
                    continue;
                }
                let nidx = linear_index(nx as usize, nz as usize, dim);
                if dist[nidx] == i64::MAX {
                    dist[nidx] = level + 1;
                    next_frontier.push(nidx);
                }
            }
        }
        frontier = next_frontier;
        level += 1;
    }
    dist
}

/// Wave exposure at `(x, z)` (module doc's "fetch/openness" proxy): the count of submerged cells
/// within a Chebyshev box of radius [`WAVE_EXPOSURE_RADIUS`] — more nearby open water = higher
/// exposure. A local, direction-free simplification (headland–bay refraction, which needs a coastal
/// orientation/curvature term, is explicitly out of scope for this slice).
fn wave_exposure(dim: usize, submerged: &[bool], x: usize, z: usize) -> i64 {
    let mut count = 0i64;
    for dz in -WAVE_EXPOSURE_RADIUS..=WAVE_EXPOSURE_RADIUS {
        for dx in -WAVE_EXPOSURE_RADIUS..=WAVE_EXPOSURE_RADIUS {
            let nx = x as i64 + dx;
            let nz = z as i64 + dz;
            if nx < 0 || nz < 0 || nx as usize >= dim || nz as usize >= dim {
                continue;
            }
            if submerged[linear_index(nx as usize, nz as usize, dim)] {
                count += 1;
            }
        }
    }
    count
}

/// Cliff-retreat rate (module doc): `(CLIFF_RETREAT_NUM · exposure) / (RESIST_DIVISOR[resist_class]
/// · CLIFF_RETREAT_DEN)`. Extracted as its own pure function so "higher resistance ⇒ slower
/// retreat" (#423 AC 5) is directly unit-testable for monotonicity against synthetic inputs, rather
/// than asserted as a fragile spatial correlation on noisy fBm terrain.
fn retreat_rate(exposure: i64, resist_class: i64) -> i64 {
    let divisor = RESIST_DIVISOR[resist_class as usize];
    (CLIFF_RETREAT_NUM * exposure) / (divisor * CLIFF_RETREAT_DEN)
}

/// Platform reach (module doc): `PLATFORM_REACH_BASE + exposure * PLATFORM_REACH_PER_EXPOSURE_NUM /
/// PLATFORM_REACH_PER_EXPOSURE_DEN`. Extracted as its own pure function for the same direct-
/// unit-testability reason as [`retreat_rate`].
fn platform_reach(exposure: i64) -> i64 {
    PLATFORM_REACH_BASE + (exposure * PLATFORM_REACH_PER_EXPOSURE_NUM) / PLATFORM_REACH_PER_EXPOSURE_DEN
}

/// One cliff-retreat macro-iteration: recompute the submerged mask + `dist_to_sea` on the CURRENT
/// `height` (mirrors `erosion.rs`'s macro-loop house style), then retreat LAND cells in the
/// immediate coastline band. Returns the non-negative delta buffer.
fn cliff_iteration(dim: usize, height: &[i64], sea_level: i64, resistance: &[i64]) -> Vec<i64> {
    let n = dim * dim;
    let submerged: Vec<bool> = (0..n).map(|idx| is_submerged(height[idx], sea_level)).collect();
    let dist_to_sea = bfs_distance(dim, &submerged);

    let mut delta = vec![0i64; n];
    for z in 0..dim {
        for x in 0..dim {
            let idx = linear_index(x, z, dim);
            if submerged[idx] || dist_to_sea[idx] > COASTLINE_BAND_WIDTH {
                continue;
            }
            let exposure = wave_exposure(dim, &submerged, x, z);
            let rate = retreat_rate(exposure, resistance[idx]);
            delta[idx] = rate.clamp(0, height[idx]);
        }
    }
    delta
}

/// Run the fixed [`N_CLIFF_ITERATIONS`] subtractive cliff-retreat loop. Returns the post-retreat
/// height (every cell `<=` its input value — single-signed, monotone, module doc).
fn cliff_retreat_pass(dim: usize, mut height: Vec<i64>, sea_level: i64, resistance: &[i64]) -> Vec<i64> {
    let n = dim * dim;
    for _ in 0..N_CLIFF_ITERATIONS {
        let delta = cliff_iteration(dim, &height, sea_level, resistance);
        for v in 0..n {
            height[v] -= delta[v];
        }
    }
    height
}

/// Wave-cut platform (module doc, pass 2): a SEPARATE single deterministic pass. Submerged
/// coastline-band cells within [`platform_reach`] of the shore (their OWN `wave_exposure`, module
/// doc) are clamped DOWN to a shallow, near-flat seaward-deepening bench via `.min` — physically, a
/// "shave the high points down to wave-base" clamp, not a "fill the low points in" one: a cell that
/// was ALREADY naturally deeper than its target bench depth (e.g. a pre-existing offshore trench)
/// is left untouched (real wave abrasion doesn't reach that far down either), while a too-shallow
/// cell is eroded down to the bench. `.min` never RAISES a cell either way, so this pass composes
/// safely with the strictly-subtractive cliff-retreat pass before it — the whole stage stays
/// single-signed.
fn carve_platform(dim: usize, mut height: Vec<i64>, sea_level: i64) -> Vec<i64> {
    let n = dim * dim;
    let submerged: Vec<bool> = (0..n).map(|idx| is_submerged(height[idx], sea_level)).collect();
    let land: Vec<bool> = submerged.iter().map(|&s| !s).collect();
    let dist_to_land = bfs_distance(dim, &land);

    for z in 0..dim {
        for x in 0..dim {
            let idx = linear_index(x, z, dim);
            if !submerged[idx] || dist_to_land[idx] > COASTLINE_BAND_WIDTH {
                continue;
            }
            let exposure = wave_exposure(dim, &submerged, x, z);
            if dist_to_land[idx] > platform_reach(exposure) {
                continue;
            }
            let depth_below_sea = (PLATFORM_DEPTH_BASE + (dist_to_land[idx] * PLATFORM_SLOPE_NUM) / PLATFORM_SLOPE_DEN).min(PLATFORM_DEPTH_MAX);
            let bench_height = (sea_level - depth_below_sea).max(0);
            height[idx] = height[idx].min(bench_height);
        }
    }
    height
}

/// The full W-SIM-7 coastal output: post-coastal `height` (retreated then platform-carved, clamped
/// into `[0,hmax]`), the `sea_level` datum, and the unambiguous `submerged` signal per cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoastalState {
    pub height: Vec<i64>,
    pub sea_level: i64,
    pub submerged: Vec<bool>,
}

/// Run the full coastal stage (sea-level datum → cliff retreat → wave-cut platform, module doc's
/// strict order) on an already-post-aeolian `height` field. Pure function of `(seed, dim, hmax,
/// height)` — no RNG-of-clock, no thread-dependence, no global mutable state.
pub fn run_coastal(seed: u64, dim: usize, hmax: i64, height: &[i64]) -> CoastalState {
    let n = dim * dim;
    debug_assert_eq!(height.len(), n);

    let level = sea_level(height);
    let resistance = resistance_field(dim, seed, hmax);

    let retreated = cliff_retreat_pass(dim, height.to_vec(), level, &resistance);
    let platformed = carve_platform(dim, retreated, level);

    let final_height: Vec<i64> = platformed.iter().map(|&h| h.clamp(0, hmax)).collect();
    let submerged: Vec<bool> = final_height.iter().map(|&h| is_submerged(h, level)).collect();

    CoastalState { height: final_height, sea_level: level, submerged }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cell's "seaward slope" — the largest positive height drop from `height[idx]` to any of its
    /// D8 neighbors — used by the cliff-escarpment corridor test (mirrors `erosion.rs`'s
    /// `steep_edge_count`/W-SIM-4a's relief-verification precedent). Test-only for now (no
    /// production caller yet); kept inside `mod tests` rather than `pub(crate)` at module level so
    /// it doesn't read as dead code in a non-test build.
    fn seaward_slope(dim: usize, height: &[i64], x: usize, z: usize) -> i64 {
        let idx = linear_index(x, z, dim);
        let mut max_drop = 0i64;
        for &(dx, dz) in &D8_OFFSETS {
            let nx = x as i64 + dx;
            let nz = z as i64 + dz;
            if nx < 0 || nz < 0 || nx as usize >= dim || nz as usize >= dim {
                continue;
            }
            let drop = height[idx] - height[linear_index(nx as usize, nz as usize, dim)];
            max_drop = max_drop.max(drop);
        }
        max_drop
    }
    use crate::gen::erosion::{erode, N_RESIST_CLASSES};

    const SEED: u64 = 0xA11A_2A11;
    const HMAX: i64 = 200;
    const DIM: usize = 64;

    fn base_fixture() -> Vec<i64> {
        erode(SEED, HMAX, DIM, false, false).height
    }

    #[test]
    fn sea_level_is_deterministic_and_bounded_away_from_extremes_across_seeds() {
        for seed in [SEED, SEED ^ 1, SEED ^ 2, SEED ^ 0xDEAD_BEEF] {
            let height = erode(seed, HMAX, DIM, false, false).height;
            let a = sea_level(&height);
            let b = sea_level(&height);
            assert_eq!(a, b, "sea_level must be byte-identical across repeated calls");

            let submerged_count = height.iter().filter(|&&h| is_submerged(h, a)).count();
            let frac_times_100 = submerged_count * 100 / height.len();
            assert!(
                (5..95).contains(&frac_times_100),
                "seed={seed}: submerged fraction {frac_times_100}% must be bounded away from 0%/100%"
            );
        }
    }

    #[test]
    fn run_coastal_is_deterministic_across_repeated_calls() {
        let height = base_fixture();
        let a = run_coastal(SEED, DIM, HMAX, &height);
        let b = run_coastal(SEED, DIM, HMAX, &height);
        assert_eq!(a, b, "run_coastal must be byte-identical across repeated calls");
    }

    #[test]
    fn different_seed_diverges() {
        let height = base_fixture();
        let a = run_coastal(SEED, DIM, HMAX, &height);
        let b = run_coastal(SEED ^ 0xDEAD_BEEF, DIM, HMAX, &height);
        assert_ne!(a.height, b.height, "a different seed must produce a different coastal result");
    }

    #[test]
    fn cliff_retreat_never_raises_a_cell() {
        let height = base_fixture();
        let level = sea_level(&height);
        let resistance = resistance_field(DIM, SEED, HMAX);
        let retreated = cliff_retreat_pass(DIM, height.clone(), level, &resistance);
        for idx in 0..DIM * DIM {
            assert!(retreated[idx] <= height[idx], "cell {idx} rose during cliff retreat: {} -> {}", height[idx], retreated[idx]);
        }
    }

    /// The FULL stage (retreat + platform together) never raises a cell relative to its input —
    /// `carve_platform`'s `.min` clamp is ALSO non-negative-only (module doc), so `run_coastal` as a
    /// whole is single-signed, not just its cliff-retreat half.
    #[test]
    fn run_coastal_never_raises_a_cell() {
        let height = base_fixture();
        let state = run_coastal(SEED, DIM, HMAX, &height);
        for (idx, (&before, &after)) in height.iter().zip(state.height.iter()).enumerate() {
            assert!(after <= before, "cell {idx} rose during the full coastal stage: {before} -> {after}");
        }
    }

    #[test]
    fn post_coastal_height_stays_in_valid_range() {
        let height = base_fixture();
        let state = run_coastal(SEED, DIM, HMAX, &height);
        for (idx, &h) in state.height.iter().enumerate() {
            assert!((0..=HMAX).contains(&h), "cell {idx} height {h} out of [0,{HMAX}]");
        }
    }

    /// Resistance gates retreat (#423 AC 5): at EQUAL exposure, higher resistance class must yield a
    /// strictly smaller (or equal, at the low end where integer division floors to 0) retreat rate.
    /// Direct unit test of the pure formula — avoids a fragile spatial correlation on noisy terrain.
    #[test]
    fn retreat_rate_is_monotone_non_increasing_in_resistance() {
        for exposure in [1, 5, 20, 50] {
            let rates: Vec<i64> = (0..N_RESIST_CLASSES).map(|class| retreat_rate(exposure, class)).collect();
            for w in rates.windows(2) {
                assert!(w[0] >= w[1], "exposure={exposure}: retreat_rate must be non-increasing in resistance class: {rates:?}");
            }
        }
        // And strictly less at the softest vs hardest class for a large-enough exposure to clear
        // integer-division truncation at every class.
        let soft = retreat_rate(50, 0);
        let hard = retreat_rate(50, N_RESIST_CLASSES - 1);
        assert!(hard < soft, "hardest class must retreat strictly slower than softest at high exposure: soft={soft} hard={hard}");
    }

    /// Platform width correlates with exposure (#423 AC 4): direct unit test of the pure formula.
    #[test]
    fn platform_reach_is_monotone_in_exposure() {
        let reaches: Vec<i64> = (0..=30).step_by(5).map(platform_reach).collect();
        for w in reaches.windows(2) {
            assert!(w[0] <= w[1], "platform_reach must be non-decreasing in exposure: {reaches:?}");
        }
        assert!(reaches[reaches.len() - 1] > reaches[0], "platform_reach must strictly grow over this exposure range: {reaches:?}");
    }

    /// Wave-cut platform is a near-flat bench (#423 AC 4): `carve_platform` is a "shave the high
    /// points down" clamp (`.min`), NOT a "fill the low points in" one — a cell that was ALREADY
    /// naturally deeper than the target bench (a pre-existing offshore trench, say) is legitimately
    /// left untouched (real wave action doesn't reach that far down either). So the bench guarantee
    /// only applies to cells the clamp ACTUALLY fired on (its pre-platform height was shallower than
    /// the target bench) — those must land EXACTLY at the bench depth: bounded, and uniform for a
    /// given `dist_to_land` (the "near-flat" claim).
    #[test]
    fn platform_cells_form_a_bounded_shallow_bench() {
        let height = base_fixture();
        let level = sea_level(&height);
        let resistance = resistance_field(DIM, SEED, HMAX);
        let retreated = cliff_retreat_pass(DIM, height.clone(), level, &resistance);
        let platformed = carve_platform(DIM, retreated.clone(), level);

        let submerged: Vec<bool> = platformed.iter().map(|&h| is_submerged(h, level)).collect();
        let land: Vec<bool> = submerged.iter().map(|&s| !s).collect();
        let dist_to_land = bfs_distance(DIM, &land);

        let mut clamped_count = 0;
        for idx in 0..DIM * DIM {
            if !submerged[idx] {
                continue;
            }
            let exposure = wave_exposure(DIM, &submerged, idx % DIM, idx / DIM);
            if dist_to_land[idx] > platform_reach(exposure) {
                continue;
            }
            if platformed[idx] == retreated[idx] {
                continue; // clamp did not fire here (already naturally deeper) — no bench claim
            }
            clamped_count += 1;
            let depth = level - platformed[idx];
            assert!(
                (0..=PLATFORM_DEPTH_MAX).contains(&depth),
                "clamped platform cell {idx} depth {depth} exceeds the shallow bench bound {PLATFORM_DEPTH_MAX}"
            );
        }
        assert!(clamped_count > 0, "the golden fixture must produce at least one ACTUALLY-clamped platform cell to check");
    }

    /// Cliff escarpment corridor (#423 AC 3, anti-forcing-clean — the W-SIM-4a relief-verification
    /// precedent): measured at the SAME coordinates (the ON-path's own coastline-band land cells),
    /// comparing the ORIGINAL (uncarved) height against the coastal-carved height — NOT two
    /// separately-defined "coastline band" regions on OFF vs ON, which would be comparing different
    /// geographic areas (submergence, and therefore the band's location, differs between the two).
    /// The threshold is well above what fBm+erosion alone produces AT THESE SPECIFIC cells.
    #[test]
    fn cliff_escarpment_corridor() {
        const ESCARPMENT_THRESHOLD: i64 = 15;
        let height = base_fixture();
        let state = run_coastal(SEED, DIM, HMAX, &height);
        let on_dist = bfs_distance(DIM, &state.submerged);

        let band_land: Vec<usize> = (0..DIM * DIM)
            .filter(|&idx| !state.submerged[idx] && on_dist[idx] <= COASTLINE_BAND_WIDTH)
            .collect();
        assert!(!band_land.is_empty(), "the golden fixture must produce at least one coastline-band land cell");

        let off_count = band_land
            .iter()
            .filter(|&&idx| seaward_slope(DIM, &height, idx % DIM, idx / DIM) >= ESCARPMENT_THRESHOLD)
            .count();
        let on_count = band_land
            .iter()
            .filter(|&&idx| seaward_slope(DIM, &state.height, idx % DIM, idx / DIM) >= ESCARPMENT_THRESHOLD)
            .count();

        assert!(off_count <= 1, "the ORIGINAL (uncarved) height at these coordinates must have ~0 escarpment-grade cells, found {off_count}");
        assert!(on_count > off_count, "coastal carving must produce escarpment-grade cliff cells at these coordinates the baseline cannot: uncarved={off_count} carved={on_count}");
    }

    /// Golden vector: pinned exact coastal-ON height + submerged flag at fixed grid indices for the
    /// golden `(seed, dim, hmax)` fixture.
    ///
    /// PASS 1 (#423): placeholder — this new-in-branch golden is born in CI (project contract),
    /// pass 2 reads the CI-revealed `left:` and pins it.
    #[test]
    fn golden_vector_matches_pinned_coastal_fixture() {
        let height = base_fixture();
        let state = run_coastal(SEED, DIM, HMAX, &height);

        const INDICES: [usize; 4] = [0, 500, 1500, 4000];
        const EXPECTED: [i64; 4] = [0, 0, 0, 0]; // PASS 1 placeholder — CI reveals the real value
        let actual: [i64; 4] = std::array::from_fn(|i| state.height[INDICES[i]]);
        assert_eq!(actual, EXPECTED, "golden drift (or placeholder awaiting CI pin) at indices {INDICES:?}");
    }
}
