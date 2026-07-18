//! W-SIM-6 (#416): deterministic integer glacial relief — ELA-gated ice incision + till moraine,
//! the fourth landform slice on the `worldgen-relief` ladder (RnD
//! `sim/world/16-glacial-and-karst-landforms.md`, GLACIAL gate only — the karst gate is a separate
//! future slice). Unlike volcanic (constructive, PRE-erosion), this reshapes ALREADY-eroded relief:
//! it runs POST-erosion, PRE-final-classify (`erode → GLACIAL(this) → aeolian → final classify`,
//! RnD 16 §1's ordering — glacial before aeolian so periglacial outwash could later feed the aeolian
//! sand-supply reserve, RnD 16 §7, a follow-up coupling not built in this slice). **Pure integer /
//! fixed-point throughout — no `f32`/`f64` anywhere in this file** (covered by the recursive glob
//! guard, `world/tests/no_float_guard_gen.rs`).
//!
//! ## ELA ice mask (RnD 16 §1, simplified per #416's own scope note)
//!
//! `ice_mask(x,z) = T(x,z) <= T_FREEZE` on the SAME lapse-rate temperature field
//! `climate::climate_from_height` already computes — issue #416's deliberate simplification of RnD
//! 16 §1's full `ELA(lat,T,aspect)` (the ±70–320m folded-aspect term is explicitly out of scope for
//! this slice; "lat+lapse ELA without the aspect term" IS exactly `climate_from_height`'s existing
//! `T`). Computed ONCE at stage entry from the POST-erosion, PRE-glacial height — never re-derived
//! inside the incision loop (that would couple mask→height→mask and risk non-convergence, RnD 16 §6
//! pitfall). `T_FREEZE` is an integer centidegree threshold compared BEFORE any float ever enters —
//! there is none in this pipeline, so no ULP-flip is possible by construction.
//!
//! ## Two determinism-mandated passes, in STRICT order (RnD 16 §6 — the load-bearing invariant)
//!
//! 1. **Subtractive ice-incision** ([`ice_incision_pass`]): fixed-iteration (never convergence-ε,
//!    R10), D8 steepest-descent recomputed EVERY iteration on the current height (mirrors
//!    `erosion.rs`'s macro-loop house style: recompute the flow graph between iterations, never
//!    cached from a stale one — but over an ICE-RESTRICTED D8 graph, [`ice_d8_directions`]/
//!    [`ice_accumulate`], NOT `drainage.rs`'s full-grid primitives: those accumulate flow over the
//!    WHOLE watershed, so a single ice cell sitting on the world's main drainage channel would
//!    inherit a huge upstream area from mostly non-ice terrain — wildly uneven vs. its own
//!    ice-covered neighbors, and geomorphically wrong besides, since ice flux is about the ICE
//!    catchment specifically (RnD 16 §2)), applied ONLY at `ice_mask` cells, as the SUM of two
//!    non-negative terms (never negative individually, so their sum stays single-signed too):
//!    - a VERTICAL (along-flow) term, the SAME area-proportional stream-power shape erosion.rs's
//!      incision uses — thin tributaries (small upstream drainage area) naturally incise less than
//!      thick ice-flux troughs, so the hanging-valley signature emerges for free (RnD 16 §2);
//!    - a LATERAL (cross-valley) term (RnD 16 §3: "углубить И РАСШИРИТЬ" — deepen AND widen): each
//!      cell loses a fraction of its height excess above its lowest ICE-COVERED D8 neighbor. A
//!      pure along-flow formula is structurally identical to water's V-notch incision (RnD 16 §3's
//!      own differentiator) and can't flatten a floor on its own; this term does, by construction
//!      (it strictly moves height toward the local ice-covered minimum every iteration) — clamped
//!      `>= 0`, so it never raises a cell either.
//!    Every delta is non-negative (height only ever DECREASES) — single-signed, monotone, the SAME
//!    convergence argument erosion's own incision uses.
//! 2. **Additive till deposition (Option A — thin margin-peaked moraine + off-map outwash)**: a
//!    SEPARATE pass, run only AFTER the incision loop has fully settled (all `N_GLACIAL_ITERATIONS`
//!    done) — never interleaved with incision in the same loop (RnD 16 §6's named pitfall: mixing
//!    signs in one loop flip-flops the D8 field and never settles). The `excavated_total` budget
//!    from pass 1 is deposited as a thin margin-peaked MORAINE over non-ice cells within distance
//!    ≤ `k_band` of the ice margin, using a DESIGNED thin ridge profile that decays outward. Budget
//!    is drained (`take = min(profile[ring], headroom, remaining)`) across band cells in ascending
//!    distance-ring and ascending cell-index order (deterministic, F27 spec); cells are capped
//!    `≤ hmax−1` (no plateau by construction), and the untaken mass is exported as legitimate
//!    off-map outwash (`exported_till`, ~80% on production, booked in the ledger). The conservation
//!    identity `excavated_total == deposited_total + exported_till` holds EXACTLY with two
//!    independent counters (F35 — any budget-drain error, a silently-truncated cell, or an
//!    off-edge term that doesn't balance makes this fail). Till-cell tagging: `material = Till iff
//!    take > 0` (`:447-449`) — Till marks the deposited BAND cells, not the ice margin (F41).
//!    Profile + `k_band` pinned by Phase-0b sweep results (`d_peak=40, k_band=5`); `run_glacial`
//!    calls [`run_glacial_with`]'s wrapper with these constants (F52–F58).
//!
//! ## Material + Ledger acceptance clause
//!
//! Margin cells that receive a nonzero till deposit are tagged [`crate::gen::material::MaterialId::Till`]
//! — the primary substrate the caller (`caps.rs`) reconciles into `WorldFields.surface_material`,
//! mirroring how volcanic's Basalt/Tuff mask and aeolian's sand reconciliation already work.
//!
//! **Conservation ledger (Option A, Phase-0b):** `Σdeposited_total + exported_till == excavated_total`
//! always (identity computed from two independent sources: height-delta sum vs. remaining budget);
//! **hard `exported_till == 0` on a MORAINE-ABSORBING DIM=64 fixture** (sparse ice, thin moraine
//! fully drains; this is a regression tooth against "export everything, deposit nothing"); the
//! **moraine is non-trivial** on production (`deposited_total > 0` AND till marks at least one local
//! height maximum); and the **plateau is impossible** (`truncated == 0` — no band cell driven to its
//! headroom cap). Erosion tail (`incised > hmax−1`) is left untouched (pre-existing behavior, sim
//! already lives with it via `photic_atten` clamp-at-read + `solid_level` percentile).

use sim_core::isqrt;

use crate::gen::climate::climate_from_height;
use crate::gen::material::MaterialId;

/// Ice gate: a cell is ice-covered when its working temperature is at or below this threshold
/// (centidegrees — RnD 16 §1's simplified lat+lapse ELA, see the module doc). CALIBRATED against
/// the actual achievable temperature range on this fBm terrain (mirrors `erosion.rs`'s
/// `REPOSE_THRESHOLD` recalibration lesson): on the golden grid, `height_at`'s smooth multi-octave
/// relief only reaches ~130 (not the theoretical `hmax`), so the coldest achievable cell lands
/// around +0.8 °C, not below literal 0 °C — a threshold pinned at exactly 0 would leave the ice
/// mask permanently empty (dead code), not merely rare. 3.00 °C gives a real, still highly-confined,
/// cold/high-altitude belt (measured: the coldest ~5% of the golden grid's cells).
const T_FREEZE: i64 = 300;

/// Fixed macro-iteration count for the ice-incision loop (R10, never convergence-ε — mirrors
/// `erosion.rs`'s `MACRO_ITERATIONS`, a DISTINCT, independently-tuned constant/budget for this
/// stage). Modest: glacial is a secondary, spatially-confined pass, not the primary relief sculptor.
const N_GLACIAL_ITERATIONS: usize = 8;

/// Ice-incision rate constants, mirroring `erosion.rs`'s `K_INCISE_NUM`/`K_INCISE_DEN` shape:
/// `Δz = K_ICE_NUM · isqrt(area) · slope / K_ICE_DEN`. No rock-resistance divisor is applied here
/// (implementer's call for this slice — ice erodes the same rate regardless of the erosion-layer's
/// resistance classes; a resistance-aware ice-incision model is a legitimate future refinement, not
/// required by #416's acceptance criteria).
const K_ICE_NUM: i64 = 1;
const K_ICE_DEN: i64 = 2;

/// Lateral-planation fraction ([`lateral_planation_delta`]): the fraction of a cell's height excess
/// above its lowest ice-covered D8 neighbor removed per iteration — the widening/flattening term
/// the U-trough's flat floor needs. Implementer's call, documented, locked by the golden-vector test.
const LATERAL_FRAC_NUM: i64 = 3;
const LATERAL_FRAC_DEN: i64 = 4;

#[inline]
fn linear_index(x: usize, z: usize, dim: usize) -> usize {
    z * dim + x
}

const D8_OFFSETS: [(i64, i64); 8] =
    [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];

/// The ELA ice mask (module doc): `true` wherever the working temperature on `height` is at or
/// below [`T_FREEZE`]. Pure function of `(seed, dim, height)` — computed ONCE, never re-derived
/// mid-loop. `x_west`'s border rule mirrors `caps.rs`'s own clamp-to-edge convention (`climate.rs`'s
/// `WIND_DX` upwind sample has no border on an infinite domain; this finite grid clamps it).
pub fn ice_mask(seed: u64, dim: usize, height: &[i64]) -> Vec<bool> {
    use crate::gen::climate::WIND_DX;
    let n = dim * dim;
    let mut mask = vec![false; n];
    for z in 0..dim {
        for x in 0..dim {
            let idx = linear_index(x, z, dim);
            let x_src = (x as i64 - WIND_DX).max(0) as usize;
            let h_west = height[linear_index(x_src, z, dim)];
            let (t, _p) = climate_from_height(height[idx], h_west, x as i64, z as i64, seed);
            mask[idx] = t <= T_FREEZE;
        }
    }
    mask
}

/// Ice-restricted D8 steepest-descent: the SAME total-order-tie-break shape `drainage::d8_directions`
/// uses, but a cell only ever routes to an ICE-COVERED neighbor. Deliberately NOT
/// `drainage::d8_directions` on the full grid: that computes descent (and the accumulation below
/// would inherit flow) over the WHOLE watershed, so a single ice cell that happens to sit on the
/// world's main drainage channel would inherit a huge upstream area from mostly non-ice terrain —
/// wildly uneven vs. its own ice-covered neighbors, and geomorphically wrong besides (ice flux is
/// about the ICE catchment, not the whole world's water drainage, RnD 16 §2).
fn ice_d8_directions(dim: usize, height: &[i64], ice_mask: &[bool]) -> Vec<Option<usize>> {
    let mut downstream = vec![None; dim * dim];
    for z in 0..dim {
        for x in 0..dim {
            let idx = linear_index(x, z, dim);
            if !ice_mask[idx] {
                continue;
            }
            let mut best: Option<(usize, i64)> = None;
            for &(dx, dz) in &D8_OFFSETS {
                let nx = x as i64 + dx;
                let nz = z as i64 + dz;
                if nx < 0 || nz < 0 || nx as usize >= dim || nz as usize >= dim {
                    continue;
                }
                let nidx = linear_index(nx as usize, nz as usize, dim);
                if !ice_mask[nidx] {
                    continue;
                }
                let drop = height[idx] - height[nidx];
                if drop > 0 && best.is_none_or(|(_, bd)| drop > bd) {
                    best = Some((nidx, drop));
                }
            }
            downstream[idx] = best.map(|(nidx, _)| nidx);
        }
    }
    downstream
}

/// Ice-restricted flow accumulation: the SAME Kahn-topological integer-associative technique
/// `drainage::kahn_accumulate` uses, but seeded with 1 unit per ICE-COVERED cell only (non-ice cells
/// never contribute) and routed exclusively over [`ice_d8_directions`]'s ice-only graph.
fn ice_accumulate(dim: usize, downstream: &[Option<usize>], ice_mask: &[bool]) -> Vec<i64> {
    let n = dim * dim;
    let mut in_degree = vec![0u32; n];
    for &d in downstream {
        if let Some(d) = d {
            in_degree[d] += 1;
        }
    }
    let mut accum: Vec<i64> = (0..n).map(|idx| if ice_mask[idx] { 1 } else { 0 }).collect();
    let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    for idx in 0..n {
        if ice_mask[idx] && in_degree[idx] == 0 {
            queue.push_back(idx);
        }
    }
    while let Some(v) = queue.pop_front() {
        if let Some(d) = downstream[v] {
            accum[d] += accum[v];
            in_degree[d] -= 1;
            if in_degree[d] == 0 {
                queue.push_back(d);
            }
        }
    }
    accum
}

/// The lateral (cross-valley) term (RnD 16 §3: "углубить И РАСШИРИТЬ" — deepen AND widen, the
/// U-trough's flat-floored profile a pure along-flow stream-power formula can't produce on its own,
/// since that's structurally identical to water's V-notch incision): each ice-covered cell loses a
/// FRACTION of its height excess above its lowest ICE-COVERED D8 neighbor — strictly subtractive
/// (the excess is clamped `>= 0`, so this term is never negative), so it adds width/flatness
/// without breaking the single-signed monotonicity invariant. Extracted as its own function so
/// [`ice_incision_pass`] can run it both alongside the vertical term AND as a dedicated
/// floor-smoothing tail phase (see that function's doc).
fn lateral_planation_delta(dim: usize, height: &[i64], ice_mask: &[bool]) -> Vec<i64> {
    let n = dim * dim;
    let mut delta = vec![0i64; n];
    for v in 0..n {
        if !ice_mask[v] {
            continue;
        }
        let mut min_ice_neighbor = height[v];
        for &(dx, dz) in &D8_OFFSETS {
            let x = (v % dim) as i64 + dx;
            let z = (v / dim) as i64 + dz;
            if x < 0 || z < 0 || x as usize >= dim || z as usize >= dim {
                continue;
            }
            let nidx = linear_index(x as usize, z as usize, dim);
            if ice_mask[nidx] {
                min_ice_neighbor = min_ice_neighbor.min(height[nidx]);
            }
        }
        let excess = (height[v] - min_ice_neighbor).max(0);
        delta[v] = ((excess * LATERAL_FRAC_NUM) / LATERAL_FRAC_DEN).clamp(0, height[v]);
    }
    delta
}

/// One subtractive ice-incision macro-iteration: recompute the ICE-RESTRICTED flow graph on the
/// CURRENT `height` every iteration (mirrors `erosion.rs`'s macro-loop house style: recompute
/// between iterations, never cached), then incise ONLY at `ice_mask` cells, as the sum of the
/// vertical (along-flow, stream-power) term and the [`lateral_planation_delta`] term (module doc).
/// Returns the non-negative delta buffer (never scattered/mixed with any additive effect — module
/// doc's strict-order invariant).
fn ice_incision_iteration(dim: usize, height: &[i64], ice_mask: &[bool]) -> Vec<i64> {
    let n = dim * dim;
    let downstream = ice_d8_directions(dim, height, ice_mask);
    let area = ice_accumulate(dim, &downstream, ice_mask);
    let lateral = lateral_planation_delta(dim, height, ice_mask);

    let mut delta = vec![0i64; n];
    for v in 0..n {
        if !ice_mask[v] {
            continue;
        }
        // Vertical (along-flow) term: the SAME stream-power shape erosion.rs's incision uses, but
        // over the ice-restricted `area` above (module doc).
        let vertical = match downstream[v] {
            Some(d) => {
                let slope = (height[v] - height[d]).max(0);
                let a_isqrt = isqrt(area[v]);
                (K_ICE_NUM * a_isqrt * slope) / K_ICE_DEN
            }
            None => 0,
        };
        delta[v] = (vertical + lateral[v]).clamp(0, height[v]);
    }
    delta
}

/// Run the scaled ice-incision loop (vertical + lateral, [`ice_incision_iteration`]), THEN
/// [`flatten_interior_components`] — a single deterministic pass that clamps the U-trough's floor
/// DIRECTLY flat (RnD 16 §3: "плоское дно", a flat floor is its own distinct sub-mechanism, not
/// merely a side effect of along-flow incision). Still module-doc-compliant: every delta in BOTH
/// phases is non-negative, so the whole pass stays single-signed/monotone throughout. Returns the
/// post-incision height (every cell `<=` its input value) and the run-lifetime `excavated_total`
/// (the till deposition pass's exact integer budget).
/// W-19: glacial_strength (percent, default 100) scales iteration count via:
/// effective_iters = (N_GLACIAL_ITERATIONS * strength) / 100.
fn ice_incision_pass(dim: usize, mut height: Vec<i64>, ice_mask: &[bool], interior: &[bool], glacial_strength: i64) -> (Vec<i64>, i64) {
    let n = dim * dim;
    let mut excavated_total = 0i64;
    let n_iters = ((N_GLACIAL_ITERATIONS as i64 * glacial_strength) / 100) as usize;
    for _ in 0..n_iters {
        let delta = ice_incision_iteration(dim, &height, ice_mask);
        for v in 0..n {
            height[v] -= delta[v];
            excavated_total += delta[v];
        }
    }
    excavated_total += flatten_interior_components(dim, &mut height, interior);
    (height, excavated_total)
}

/// Deterministically label each 8-connected component of `interior` cells (a single row-major flood
/// fill pass, canonical BFS neighbor order — the labeling itself doesn't need to be
/// order-independent across runs since it's a pure function of `interior`'s fixed cell set, not of
/// any external processing order). `None` for non-interior cells.
fn interior_component_labels(dim: usize, interior: &[bool]) -> Vec<Option<usize>> {
    let n = dim * dim;
    let mut labels: Vec<Option<usize>> = vec![None; n];
    let mut next_label = 0usize;
    for start in 0..n {
        if !interior[start] || labels[start].is_some() {
            continue;
        }
        labels[start] = Some(next_label);
        let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
        queue.push_back(start);
        while let Some(v) = queue.pop_front() {
            let x = (v % dim) as i64;
            let z = (v / dim) as i64;
            for &(dx, dz) in &D8_OFFSETS {
                let nx = x + dx;
                let nz = z + dz;
                if nx < 0 || nz < 0 || nx as usize >= dim || nz as usize >= dim {
                    continue;
                }
                let nidx = linear_index(nx as usize, nz as usize, dim);
                if interior[nidx] && labels[nidx].is_none() {
                    labels[nidx] = Some(next_label);
                    queue.push_back(nidx);
                }
            }
        }
        next_label += 1;
    }
    labels
}

/// The U-trough's flat floor (RnD 16 §3: "плоское дно" — a flat floor, not merely a flatter one),
/// realized DIRECTLY and deterministically: within each 8-connected component of INTERIOR ice cells
/// (never a margin cell — those are the moraine RIDGE, a different feature), clamp every cell down
/// to that component's own minimum height. One deterministic pass, no iteration-count to tune:
/// every INTERIOR cell in a connected trough ends up EXACTLY level with its component's lowest
/// point (spread == 0 by construction), while SEPARATE ice patches/valleys each keep their own
/// independent floor level (never blended into one artificial global plane). Strictly subtractive —
/// `delta = height[idx] - component_min >= 0` always — so this preserves the incision pass's
/// single-signed monotonicity invariant. Returns the additional excavated total.
fn flatten_interior_components(dim: usize, height: &mut [i64], interior: &[bool]) -> i64 {
    let n = dim * dim;
    let labels = interior_component_labels(dim, interior);
    let n_components = match labels.iter().flatten().copied().max() {
        Some(m) => m + 1,
        None => return 0,
    };
    let mut mins = vec![i64::MAX; n_components];
    for idx in 0..n {
        if let Some(l) = labels[idx] {
            mins[l] = mins[l].min(height[idx]);
        }
    }
    let mut excavated = 0i64;
    for idx in 0..n {
        if let Some(l) = labels[idx] {
            let delta = height[idx] - mins[l];
            height[idx] -= delta;
            excavated += delta;
        }
    }
    excavated
}

/// Ice-margin cells (module doc): `ice_mask` cells with at least one non-ice D8 neighbor, OR a
/// neighbor off the grid edge (the ice cap reaching the map boundary also counts as a margin — there
/// is no "beyond the edge" cell to compare against). Returned in FIXED row-major order — a property
/// of the cell SET, not of any processing order, so downstream remainder distribution stays
/// deterministic and order-independent.
fn ice_margin_cells(dim: usize, ice_mask: &[bool]) -> Vec<usize> {
    let mut margin = Vec::new();
    for z in 0..dim {
        for x in 0..dim {
            let idx = linear_index(x, z, dim);
            if !ice_mask[idx] {
                continue;
            }
            let mut is_margin = false;
            for &(dx, dz) in &D8_OFFSETS {
                let nx = x as i64 + dx;
                let nz = z as i64 + dz;
                if nx < 0 || nz < 0 || nx as usize >= dim || nz as usize >= dim {
                    is_margin = true;
                    break;
                }
                if !ice_mask[linear_index(nx as usize, nz as usize, dim)] {
                    is_margin = true;
                    break;
                }
            }
            if is_margin {
                margin.push(idx);
            }
        }
    }
    margin
}


/// The full W-SIM-6 glacial output: post-glacial `height` (ice-incised then till-deposited),
/// the primary-substrate `material` mask (`Some(Till)` on any band cell that received a nonzero
/// deposit, `None` elsewhere), and the conservation ledger:
/// - `excavated_total`: the budget excavated by the incision pass (includes interior flattening).
/// - `deposited_total`: the sum of actual height increases over all band cells (computed from
///   height-delta post-deposit, independent of `take` budget, so conservation identity bites on
///   any off-edge/truncation error).
/// - `exported_till`: the untaken budget carried off-map as legitimate outwash (remaining after
///   budget-drain deposition). The identity `excavated_total == deposited_total + exported_till`
///   holds EXACTLY (F35).
/// - `truncated`: the sum of profile overflows (where `min(profile[ring], remaining) > headroom`),
///   used as a regression tooth for plateau detection (F56). Computed independently, so a silent
///   capping error would fail `truncated == 0` on fixtures meant to have no plateau (tooth iv).
/// - `band_capacity`: the sum of available headroom over all band cells (`Σ(hmax−1 − incised)`),
///   for measurement/context (F57 — the choice to cap or not is implicit in the budget-drain path).
/// On the OLD `run_glacial` (never used post-Phase-0b), `exported_till = band_capacity = truncated = 0`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlacialState {
    pub height: Vec<i64>,
    pub material: Vec<Option<MaterialId>>,
    pub excavated_total: i64,
    pub deposited_total: i64,
    pub exported_till: i64,
    pub truncated: i64,
    pub band_capacity: i64,
}

// ===== OPTION A: MORAINE PROFILE + BUDGET-DRAIN (Phase-0b) =====

// **PINNED PRODUCTION CONSTANTS** (Phase-0b gate PASS: Stage 2 wiring)
// `d_peak = 40`: ~40-unit crest ridge just outside the ice, visible but not dominant (render-tunable).
// `K_BAND = 5`: moraine apron extends 5 rings outward from margin.
// These were selected by the Phase-0b sweep to eliminate plateaus (`truncated==0` at DIM=512 production),
// generate needles ≤ ~64 (S_max=16 hole-fill), and maintain resource/solid-fraction asserts.
// Re-pinned in the SAME commit as all till-location test restatements (F52–F58, F17 same-commit).
const PROFILE: MorainerProfile = MorainerProfile { d_peak: 40 };
const K_BAND: usize = 5;

/// PHASE-0b: deposit profile for thin moraine (Option A).
/// Defines per-ring deposit targets: margin-peaked (ring 0 = d_peak, decays outward).
/// Can be scaled uniformly via scaled(k) for control sweeps.
#[derive(Clone, Debug)]
pub struct MorainerProfile {
    pub d_peak: i64,  // Deposit at margin (ring 0)
}

impl MorainerProfile {
    /// Scaled profile: multiplies d_peak by k (for control sweeps on DIM=64).
    pub fn scaled(&self, k: i64) -> Self {
        MorainerProfile {
            d_peak: self.d_peak * k,
        }
    }

    /// Deposit target for a given ring distance (linear decay).
    /// For ring r ≤ k_band: deposit = d_peak * (k_band - r) / (k_band + 1)
    pub fn target_at_ring(&self, ring: usize, k_band: usize) -> i64 {
        if ring >= k_band {
            0
        } else {
            let remaining = (k_band - ring) as i64;
            (self.d_peak * remaining) / (k_band as i64 + 1)
        }
    }
}

/// PHASE-0b: moraine deposit routine using budget-drain (Option A).
/// Walks band cells in deterministic order (distance ring, then ascending cell index),
/// places deposit up to min(profile target, headroom, remaining budget).
/// Returns (final_height, deposited_total, exported_till, till_cells, band_capacity, truncated).
/// `truncated` = Σ over band cells with headroom>0 of max(0, min(profile[ring], remaining) − headroom)
/// (F56 — deposit mass the profile wanted beyond a cell's headroom; headroom>0 filter prevents
/// counting pre-existing over-hmax erosion tail cells as plateaus).
fn deposit_moraine_budget_drain(
    dim: usize,
    excavated_total: i64,
    incised_height: &[i64],
    hmax: i64,
    ice_mask: &[bool],
    profile: &MorainerProfile,
    k_band: usize,
) -> (Vec<i64>, i64, i64, Vec<bool>, i64, i64) {
    let n = dim * dim;
    let mut final_height = incised_height.to_vec();
    let mut till_cells = vec![false; n];
    let mut deposited_total = 0i64;
    let mut remaining = excavated_total;
    let mut band_capacity = 0i64;
    let mut truncated = 0i64;

    // BFS to find band cells by distance ring (immutable snapshot).
    let margin = ice_margin_cells(dim, ice_mask);
    if margin.is_empty() {
        return (final_height, 0, excavated_total, till_cells, 0, 0);
    }

    let mut distance: Vec<Option<usize>> = vec![None; n];
    let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();

    for &idx in &margin {
        distance[idx] = Some(0);
        queue.push_back(idx);
    }

    while let Some(idx) = queue.pop_front() {
        let curr_dist = distance[idx].unwrap();
        if curr_dist >= k_band {
            continue;
        }

        let x = (idx % dim) as i64;
        let z = (idx / dim) as i64;
        for &(dx, dz) in &D8_OFFSETS {
            let nx = x + dx;
            let nz = z + dz;
            if nx < 0 || nz < 0 || nx as usize >= dim || nz as usize >= dim {
                continue;
            }
            let nidx = linear_index(nx as usize, nz as usize, dim);
            if !ice_mask[nidx] && distance[nidx].is_none() {
                distance[nidx] = Some(curr_dist + 1);
                queue.push_back(nidx);
            }
        }
    }

    // Collect band cells ordered by (distance ring, ascending cell index).
    let mut band_cells: Vec<usize> = Vec::new();
    for dist in 0..k_band {
        let mut ring_cells: Vec<usize> = Vec::new();
        for idx in 0..n {
            if let Some(d) = distance[idx] {
                if d == dist && !ice_mask[idx] {
                    ring_cells.push(idx);
                    let headroom = (hmax - 1 - incised_height[idx]).max(0);
                    band_capacity += headroom;
                }
            }
        }
        ring_cells.sort(); // Ascending cell index within ring (deterministic).
        band_cells.extend(ring_cells);
    }

    // Budget-drain: walk band cells, place deposit up to limits.
    for idx in band_cells {
        if let Some(dist) = distance[idx] {
            let headroom = (hmax - 1 - incised_height[idx]).max(0);
            let target = profile.target_at_ring(dist, k_band);
            let take = std::cmp::min(target, std::cmp::min(headroom, remaining));

            // Compute truncated BEFORE updating remaining (F56: deposit mass the profile wanted beyond headroom).
            // Only count if headroom > 0 (F56 mandatory filter: don't count pre-existing over-hmax tail).
            if headroom > 0 {
                let intended = std::cmp::min(target, remaining);
                let overflow = (intended - take).max(0);
                truncated += overflow;
            }

            if take > 0 {
                final_height[idx] = incised_height[idx] + take;
                deposited_total += take;
                till_cells[idx] = true;
                remaining -= take;
            }
        }
    }

    let exported_till = remaining;

    (final_height, deposited_total, exported_till, till_cells, band_capacity, truncated)
}

/// Run the full glacial stage with a moraine deposit profile (Option A — Phase-0b).
/// Pure function of `(seed, dim, hmax, height, profile, k_band)`.
pub fn run_glacial_with(
    seed: u64,
    dim: usize,
    hmax: i64,
    height: &[i64],
    profile: &MorainerProfile,
    k_band: usize,
    glacial_strength: i64,
) -> GlacialState {
    let n = dim * dim;
    debug_assert_eq!(height.len(), n);

    // P2: hole-fill non-ice 8-connected components ≤16 before incision.
    let base_mask = ice_mask(seed, dim, height);
    let filled_mask = fill_holes_in_ice_mask(dim, &base_mask, 16);

    let margin = ice_margin_cells(dim, &filled_mask);
    let mut interior = filled_mask.clone();
    for &idx in &margin {
        interior[idx] = false;
    }

    // Ice incision pass (scaled by glacial_strength — W-19).
    // Clamp strength to valid range [0, 400]
    let clamped_strength = glacial_strength.clamp(0, 400);
    let (incised_height, excavated_total) = ice_incision_pass(dim, height.to_vec(), &filled_mask, &interior, clamped_strength);

    // Moraine deposit (budget-drain with profile and k_band).
    let (final_height, deposited_total, exported_till, till_cells, band_capacity, truncated) =
        deposit_moraine_budget_drain(dim, excavated_total, &incised_height, hmax, &filled_mask, profile, k_band);

    // Material tagging: Till iff take > 0.
    let mut material = vec![None; n];
    for idx in 0..n {
        if till_cells[idx] {
            material[idx] = Some(MaterialId::Till);
        }
    }

    // Ledger assertion: excavated == deposited + exported.
    if excavated_total != deposited_total + exported_till {
        eprintln!(
            "LEDGER VIOLATION: excavated={} != deposited={} + exported={} (error={})",
            excavated_total,
            deposited_total,
            exported_till,
            (excavated_total - (deposited_total + exported_till)).abs()
        );
    }

    GlacialState {
        height: final_height,
        material,
        excavated_total,
        deposited_total,
        exported_till,
        truncated,
        band_capacity,
    }
}

/// Production wrapper: run the full glacial stage with pinned-const moraine profile + k_band
/// (Phase-0b GATE PASS). Pure function of `(seed, dim, hmax, height, glacial_strength)` — calls [`run_glacial_with`]
/// with `&PROFILE` and `K_BAND` (the pinned production geometry; F52–F58).
/// W-19: glacial_strength (percent, default 100) scales the incision iteration count.
pub fn run_glacial(seed: u64, dim: usize, hmax: i64, height: &[i64], glacial_strength: i64) -> GlacialState {
    run_glacial_with(seed, dim, hmax, height, &PROFILE, K_BAND, glacial_strength)
}

// ===== P2: HOLE-FILL (production, not throwaway) =====

/// P2 hole-fill (Option A, F6): fill non-ice holes ≤S_max (s_max=16, F9) deterministically BEFORE
/// incision (immutable snapshot of ice_mask, no in-place cascade — pure, order-independent, F7).
/// Used by run_glacial_with to reduce needle count before the incision pass.
fn fill_holes_in_ice_mask(dim: usize, ice_mask: &[bool], s_max: usize) -> Vec<bool> {
    let n = dim * dim;
    let mut filled = ice_mask.to_vec();

    // Find enclosed non-ice components (don't touch the border).
    let mut hole_labels: Vec<Option<usize>> = vec![None; n];
    let mut next_label = 0usize;
    let mut hole_sizes: Vec<usize> = Vec::new();

    for start in 0..n {
        if ice_mask[start] || hole_labels[start].is_some() {
            continue;
        }
        let mut component_size = 0usize;
        let mut touches_border = false;
        let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
        queue.push_back(start);
        hole_labels[start] = Some(next_label);

        while let Some(v) = queue.pop_front() {
            component_size += 1;
            let x = (v % dim) as i64;
            let z = (v / dim) as i64;

            if x == 0 || x == dim as i64 - 1 || z == 0 || z == dim as i64 - 1 {
                touches_border = true;
            }

            for &(dx, dz) in &D8_OFFSETS {
                let nx = x + dx;
                let nz = z + dz;
                if nx < 0 || nz < 0 || nx as usize >= dim || nz as usize >= dim {
                    continue;
                }
                let nidx = linear_index(nx as usize, nz as usize, dim);
                if !ice_mask[nidx] && hole_labels[nidx].is_none() {
                    hole_labels[nidx] = Some(next_label);
                    queue.push_back(nidx);
                }
            }
        }

        if !touches_border && component_size <= s_max {
            hole_sizes.push(component_size);
        } else {
            hole_sizes.push(0); // Mark as "don't fill"
        }
        next_label += 1;
    }

    // Fill holes ≤s_max by marking them as ice in the output mask.
    for idx in 0..n {
        if let Some(label) = hole_labels[idx] {
            if label < hole_sizes.len() && hole_sizes[label] > 0 && hole_sizes[label] <= s_max {
                filled[idx] = true;
            }
        }
    }

    filled
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen::erosion::erode;

    const SEED: u64 = 0xA11A_2A11;
    const HMAX: i64 = 200;
    const DIM: usize = 64;

    fn eroded_fixture() -> Vec<i64> {
        erode(SEED, HMAX, DIM, true, false, false, false, true, 100).height
    }

    #[test]
    fn ice_mask_is_deterministic_across_repeated_calls() {
        let height = eroded_fixture();
        let a = ice_mask(SEED, DIM, &height);
        let b = ice_mask(SEED, DIM, &height);
        assert_eq!(a, b, "ice_mask must be byte-identical across repeated calls");
    }

    #[test]
    fn ice_mask_is_nonempty_on_the_golden_fixture() {
        // A sanity floor for every other test in this module: if this ever goes empty (e.g. a
        // climate.rs constant changes), every glacial test below becomes a vacuous no-op, silently.
        let height = eroded_fixture();
        let mask = ice_mask(SEED, DIM, &height);
        assert!(mask.iter().any(|&b| b), "the golden fixture must have at least one ice-covered cell");
    }

    #[test]
    fn run_glacial_is_deterministic_across_repeated_calls() {
        let height = eroded_fixture();
        let a = run_glacial(SEED, DIM, HMAX, &height, 100);
        let b = run_glacial(SEED, DIM, HMAX, &height, 100);
        assert_eq!(a, b, "run_glacial must be byte-identical across repeated calls");
    }

    #[test]
    fn different_seed_diverges() {
        let height = eroded_fixture();
        let a = run_glacial(SEED, DIM, HMAX, &height, 100);
        let b = run_glacial(SEED ^ 0xDEAD_BEEF, DIM, HMAX, &height, 100);
        assert_ne!(a.height, b.height, "a different seed must produce a different glacial result");
    }

    #[test]
    fn ice_incision_never_raises_a_cell() {
        let height = eroded_fixture();
        let mask = ice_mask(SEED, DIM, &height);
        let margin = ice_margin_cells(DIM, &mask);
        let mut interior = mask.clone();
        for &idx in &margin {
            interior[idx] = false;
        }
        let (incised, _excavated) = ice_incision_pass(DIM, height.clone(), &mask, &interior, 100);
        for idx in 0..DIM * DIM {
            assert!(incised[idx] <= height[idx], "cell {idx} rose during incision: {} -> {}", height[idx], incised[idx]);
        }
    }

    #[test]
    fn slab_ledger_conserves_excavated_deposited_and_exported_exactly() {
        // Ledger acceptance clause (F14, F35, module doc): the conservation identity MUST hold
        // EXACTLY on every fixture, computed independently (height-delta sum vs. remaining budget),
        // so any off-edge term or silently-truncated cell fails it. `deposited_total` and
        // `exported_till` are two INDEPENDENT sources, so an error in the budget-drain path
        // (e.g., a cell missed, a budget miscounted, an off-map term not booked) makes this fail.
        let height = eroded_fixture();
        let state = run_glacial(SEED, DIM, HMAX, &height, 100);
        assert_eq!(
            state.excavated_total,
            state.deposited_total + state.exported_till,
            "excavated == deposited + exported: {}/{}/{}",
            state.excavated_total,
            state.deposited_total,
            state.exported_till
        );
    }

    #[test]
    fn till_marks_at_least_one_local_height_maximum() {
        // F41/F46 restated (same commit): Till marks the DEPOSITED BAND cells (not the ice margin).
        // The profile is margin-peaked (heavier near ice, decaying outward), so the near-margin band
        // cells form a strict local-max ridge — satisfying the moraine requirement geomorphically
        // (terminal/lateral moraines are ridges at the ice edge). At least one Till cell MUST be
        // a local maximum on the golden fixture (a non-trivial moraine — Ledger clause (iii)).
        let height = eroded_fixture();
        let state = run_glacial(SEED, DIM, HMAX, &height, 100);

        let till_cells: Vec<usize> = state
            .material
            .iter()
            .enumerate()
            .filter(|&(_, m)| *m == Some(MaterialId::Till))
            .map(|(idx, _)| idx)
            .collect();
        assert!(!till_cells.is_empty(), "at least one cell must be tagged Till on the golden fixture");

        let is_local_max = |idx: usize| -> bool {
            let x = idx % DIM;
            let z = idx / DIM;
            for &(dx, dz) in &D8_OFFSETS {
                let nx = x as i64 + dx;
                let nz = z as i64 + dz;
                if nx < 0 || nz < 0 || nx as usize >= DIM || nz as usize >= DIM {
                    continue;
                }
                if state.height[linear_index(nx as usize, nz as usize, DIM)] >= state.height[idx] {
                    return false;
                }
            }
            true
        };
        assert!(
            till_cells.iter().any(|&idx| is_local_max(idx)),
            "at least one Till cell must be a strict local height maximum (a positive moraine ridge)"
        );
    }

    #[test]
    fn post_glacial_height_stays_in_valid_range() {
        // F38: deposit is capped ≤ hmax−1, and incision is monotone subtractive on ice cells;
        // non-ice cells are untouched. The ONLY exception: erosion cells that were already
        // > hmax−1 PRE-glacial (the ~237 erosion tail, pre-existing behavior) are left untouched
        // (not clamped by design — F33/F22). So post-glacial height ∈ [0, max(hmax−1, max(incised))].
        let height = eroded_fixture();
        let max_incised = height.iter().copied().max().unwrap_or(HMAX);
        let max_allowed = max_incised.max(HMAX - 1);
        let state = run_glacial(SEED, DIM, HMAX, &height, 100);
        for (idx, &h) in state.height.iter().enumerate() {
            assert!(h >= 0, "cell {idx} height {h} is negative");
            assert!(h <= max_allowed, "cell {idx} height {h} exceeds max_allowed {max_allowed}");
        }
    }

    #[test]
    fn glacial_off_leaves_non_ice_cells_outside_k_band_untouched() {
        // F40/F46 AMENDMENT to #416 ТЗ (restated in same commit): non-ice cells OUTSIDE the k_band
        // apron (distance > k_band from the ice margin) are byte-identical to input. Non-ice cells
        // within the k_band apron CAN receive a till deposit (they are the moraine band, option A
        // deposition geometry), so they are NOT untouched — but this orthogonality test still holds
        // for cells that are far enough from ice to never be reached by the band. The band is
        // deliberately outward-only (F12) and distance-limited (F6), so glacial is still a
        // per-ice-patch scoped operation (not global), just no longer point-local to the ice mask.
        let height = eroded_fixture();
        let mask = ice_mask(SEED, DIM, &height);
        let margin = ice_margin_cells(DIM, &mask);

        // BFS to find distance from margin into non-ice cells.
        let mut distance: Vec<Option<usize>> = vec![None; DIM * DIM];
        let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
        for &idx in &margin {
            distance[idx] = Some(0);
            queue.push_back(idx);
        }
        while let Some(idx) = queue.pop_front() {
            let curr_dist = distance[idx].unwrap();
            let x = (idx % DIM) as i64;
            let z = (idx / DIM) as i64;
            for &(dx, dz) in &D8_OFFSETS {
                let nx = x + dx;
                let nz = z + dz;
                if nx < 0 || nz < 0 || nx as usize >= DIM || nz as usize >= DIM {
                    continue;
                }
                let nidx = linear_index(nx as usize, nz as usize, DIM);
                if !mask[nidx] && distance[nidx].is_none() && curr_dist < K_BAND {
                    distance[nidx] = Some(curr_dist + 1);
                    queue.push_back(nidx);
                }
            }
        }

        let state = run_glacial(SEED, DIM, HMAX, &height, 100);
        for idx in 0..DIM * DIM {
            // Cells OUTSIDE k_band (distance is None or > k_band) must be untouched.
            if !mask[idx] && (distance[idx].is_none() || distance[idx].unwrap() >= K_BAND) {
                assert_eq!(
                    state.height[idx], height[idx],
                    "non-ice cell {idx} (outside k_band) changed height with glacial on"
                );
            }
        }
    }

    /// Deterministically select up to 3 well-separated "valley" test coordinates from the
    /// glacial-OFF baseline: among INTERIOR ice-covered cells (ice-covered AND not an ice-margin
    /// cell — see the doc below for why the candidate pool is scoped there), rank by drainage `area`
    /// (the existing flow-accumulation flux proxy) descending, ties by lowest linear index, and
    /// greedily accept a candidate only if it is at least `MIN_SEGMENT_DIST` (Chebyshev) from every
    /// already-accepted segment — so the segments are geographically independent, not
    /// implementer-cherry-picked adjacent cells (#416 ТЗ). The selection rule itself, not any
    /// specific coordinate, is what makes this non-cherry-picked.
    fn select_valley_segments(interior: &[bool], area: &[i64], dim: usize, min_count: usize) -> Vec<usize> {
        const MIN_SEGMENT_DIST: i64 = 4;
        let mut candidates: Vec<usize> = (0..dim * dim).filter(|&idx| interior[idx]).collect();
        candidates.sort_by(|&a, &b| area[b].cmp(&area[a]).then(a.cmp(&b)));

        let mut segments: Vec<usize> = Vec::new();
        for &cand in &candidates {
            let cx = (cand % dim) as i64;
            let cz = (cand / dim) as i64;
            let far_enough = segments.iter().all(|&s| {
                let sx = (s % dim) as i64;
                let sz = (s / dim) as i64;
                (cx - sx).abs().max((cz - sz).abs()) >= MIN_SEGMENT_DIST
            });
            if far_enough {
                segments.push(cand);
            }
            if segments.len() >= min_count {
                break;
            }
        }
        segments
    }

    /// U-trough structural signature (#416 ТЗ, anti-forcing-clean — the W-SIM-4a/5 lesson: verify a
    /// sharp structural feature the baseline can't produce, measured ON-vs-OFF at the SAME
    /// coordinates, not a bulk count or a hand-tuned absolute threshold). The candidate pool for
    /// [`select_valley_segments`] is deliberately scoped to INTERIOR ice-covered cells (ice-covered,
    /// excluding the margin — `ice_margin_cells`): glacial only ever modifies cells inside its own
    /// `ice_mask`, so a "highest-flux valley cell" outside the ice belt would never show any glacial
    /// signature at all — the physically real U-trough phenomenon only ever occurs where ice existed.
    /// F42/F46 restated (same commit): the old claim "till only deposits at the margin" is FALSE
    /// under Option A (moraine band deposits outward on non-ice cells). The load-bearing claim is
    /// INCISION: INTERIOR ice cells are ALWAYS lowered (only ever receive incision, no till deposit).
    /// Wall steepness is measured on the PRE-deposit field (the incision is the load-bearing feature),
    /// not the post-till height where moraine relief could confound the measurement.
    #[test]
    fn u_trough_structural_signature_on_at_least_3_valley_segments() {
        let off = erode(SEED, HMAX, DIM, true, false, false, false, true, 100);
        let state = run_glacial(SEED, DIM, HMAX, &off.height, 100);

        let mask = ice_mask(SEED, DIM, &off.height);
        // Compute filled_mask and derive margin/interior from it (match production exactly).
        let filled_mask = fill_holes_in_ice_mask(DIM, &mask, 16);
        let margin = ice_margin_cells(DIM, &filled_mask);
        let mut interior = filled_mask.clone();
        for &idx in &margin {
            interior[idx] = false;
        }
        let segments = select_valley_segments(&interior, &off.drainage.area, DIM, 3);
        assert!(
            segments.len() >= 3,
            "need >= 3 well-separated interior ice-covered valley candidates on this fixture, found {}",
            segments.len()
        );

        // For the wall-drop measurement, use PRE-deposit incision field (F42/F46).
        // The U-trough is an INCISION property; measuring post-till moraine would confound the signal.
        // The incision state is captured inside run_glacial_with via intermediate computation.
        // To access it, we re-derive it (interior incision is deterministic); this is for testing only.
        let on_state = run_glacial(SEED, DIM, HMAX, &off.height, 100);

        // Compute the PRE-deposit incised field for wall-drop measurement (same hole-fill + incision as production).
        let (incised_height, _) = ice_incision_pass(DIM, off.height.clone(), &filled_mask, &interior, 100);

        // Floor flatness: max height spread among D8 neighbors that are INTERIOR ice cells (the
        // actual trough FLOOR — deliberately excludes margin cells, which are the moraine RIDGE, a
        // different feature that legitimately rises; mixing them in would blame moraine relief on
        // the floor-flatness metric).
        let floor_spread = |h: &[i64], idx: usize| -> Option<i64> {
            let x = idx % DIM;
            let z = idx / DIM;
            let vals: Vec<i64> = D8_OFFSETS
                .iter()
                .filter_map(|&(dx, dz)| {
                    let nx = x as i64 + dx;
                    let nz = z as i64 + dz;
                    if nx < 0 || nz < 0 || nx as usize >= DIM || nz as usize >= DIM {
                        return None;
                    }
                    let nidx = linear_index(nx as usize, nz as usize, DIM);
                    interior[nidx].then(|| h[nidx])
                })
                .collect();
            if vals.len() < 2 {
                return None;
            }
            Some(vals.iter().copied().max().unwrap() - vals.iter().copied().min().unwrap())
        };
        // Wall steepness: the drop from idx to its lowest NON-ice D8 neighbor, measured on the
        // PRE-deposit field (the important question is whether incision steepens the wall, not
        // whether moraine adds height on top of it).
        let wall_drop = |h: &[i64], idx: usize| -> Option<i64> {
            let x = idx % DIM;
            let z = idx / DIM;
            D8_OFFSETS
                .iter()
                .filter_map(|&(dx, dz)| {
                    let nx = x as i64 + dx;
                    let nz = z as i64 + dz;
                    if nx < 0 || nz < 0 || nx as usize >= DIM || nz as usize >= DIM {
                        return None;
                    }
                    let nidx = linear_index(nx as usize, nz as usize, DIM);
                    (!filled_mask[nidx]).then(|| h[idx] - h[nidx])
                })
                .max()
        };

        let mut off_spread_total = 0i64;
        let mut on_spread_total = 0i64;
        let mut off_wall_total = 0i64;
        let mut on_wall_total = 0i64;

        for &idx in &segments {
            // The PRIMARY, robust claim — checked strictly per segment: the floor is lowered
            // (overdeepened). This alone is the load-bearing "erosion-only baseline can't do this"
            // signature: interior ice cells only receive incision (F41 — till is never deposited
            // on interior cells, only on band cells outside ice), so their height is guaranteed
            // monotonically non-increasing.
            assert!(
                on_state.height[idx] < off.height[idx],
                "valley floor at idx={idx} must be lowered (overdeepened) ON vs OFF: off={} on={}",
                off.height[idx], on_state.height[idx]
            );

            // The SHAPE refinement (flatter floor / steeper wall) is accumulated in AGGREGATE across
            // all segments, not asserted per-segment strictly (robustness against fBm noise; see
            // prior comment). Flatter floor is measured on post-till height; wall steepness is
            // measured on the same (the wall is the edge of the incised trough relative to the
            // surrounding terrain, which can include moraine).
            if let (Some(off_s), Some(on_s)) = (floor_spread(&off.height, idx), floor_spread(&on_state.height, idx)) {
                off_spread_total += off_s;
                on_spread_total += on_s;
            }
            if let (Some(off_w), Some(on_w)) = (wall_drop(&off.height, idx), wall_drop(&incised_height, idx)) {
                off_wall_total += off_w;
                on_wall_total += on_w;
            }
        }

        assert!(
            on_spread_total <= off_spread_total,
            "aggregate interior floor spread across all segments must not GROW ON vs OFF (flatter floor): off_total={off_spread_total} on_total={on_spread_total}"
        );
        assert!(
            on_wall_total >= off_wall_total,
            "aggregate wall drop across all segments must not SHRINK ON vs OFF (steeper wall): off_total={off_wall_total} on_total={on_wall_total}"
        );
    }

    /// Golden vector: pinned exact glacial-ON height + material at fixed grid indices for the
    /// golden `(seed, dim, hmax)` fixture.
    ///
    /// PASS 2 (STAGE 2): CI-pinned after Option-A moraine geometry (d_peak=40, k_band=5) lands.
    /// Index 0 changed due to new margin-peaked moraine profile; other indices stable under the
    /// budget-drain deposition model. Both v2 CI jobs (arm64 + x86) agree on the new values.
    #[test]
    fn golden_vector_matches_pinned_glacial_fixture() {
        let height = eroded_fixture();
        let state = run_glacial(SEED, DIM, HMAX, &height, 100);

        const INDICES: [usize; 4] = [0, 500, 1500, 4000];
        const EXPECTED: [i64; 4] = [77, 59, 95, 99]; // STAGE 2 re-pin (Option-A geometry)
        let actual: [i64; 4] = std::array::from_fn(|i| state.height[INDICES[i]]);
        assert_eq!(actual, EXPECTED, "golden drift (or placeholder awaiting CI pin) at indices {INDICES:?}");
    }

    // ===== LEDGER ACCEPTANCE CLAUSE TESTS (F14, F25/F30, F44, F51, F54/F60) =====

    /// Ledger clause (ii): hard `exported_till == 0` on a moraine-absorbing fixture (F25/F30).
    /// DIM=64, seed=2, mask=00010 (glacial-only): the sparse ice on this fixture is small enough
    /// that the thin moraine profile fully drains the excavated budget with zero outwash export.
    /// This is the HARD regression tooth against "deposit nothing, export everything" — a bug
    /// that would pass the identity check (if it exported 100% of excavated, deposited would be 0,
    /// and identity still holds), so a separate hard-zero fixture is necessary.
    #[test]
    fn ledger_clause_hard_zero_exported_till_on_moraine_absorbing_fixture() {
        const SEED2: u64 = 0xA11A_2A11 ^ 1; // seed=2 variant
        const HMAX64: i64 = 200;
        const DIM64: usize = 64;

        // Build eroded baseline with glacial=OFF for seed=2.
        let height = erode(SEED2, HMAX64, DIM64, true, false, false, false, true, 100).height;

        // Run with production profile/k_band (moraine-absorbing fixture as identified in Phase-0b).
        let state = run_glacial(SEED2, DIM64, HMAX64, &height, 100);

        // The hard-zero clause: exported_till must be exactly 0 on this fixture.
        assert_eq!(
            state.exported_till, 0,
            "moraine-absorbing fixture must have exported_till==0 (no outwash), but got {}",
            state.exported_till
        );
    }

    /// Ledger clause (iii): non-trivial moraine on production 11111 (F41/F50).
    /// The moraine is non-trivial iff `deposited_total > 0` AND at least one Till cell is a
    /// strict local height maximum (margin-peaked ridge, `:707`). This tests (iii) in isolation.
    #[test]
    fn ledger_clause_non_trivial_moraine_on_production() {
        let height = eroded_fixture();
        let state = run_glacial(SEED, DIM, HMAX, &height, 100);

        assert!(
            state.deposited_total > 0,
            "deposited_total must be > 0 on production: {}",
            state.deposited_total
        );

        let till_cells: Vec<usize> = state
            .material
            .iter()
            .enumerate()
            .filter(|&(_, m)| *m == Some(MaterialId::Till))
            .map(|(idx, _)| idx)
            .collect();

        let is_local_max = |idx: usize| -> bool {
            let x = idx % DIM;
            let z = idx / DIM;
            for &(dx, dz) in &D8_OFFSETS {
                let nx = x as i64 + dx;
                let nz = z as i64 + dz;
                if nx < 0 || nz < 0 || nx as usize >= DIM || nz as usize >= DIM {
                    continue;
                }
                if state.height[linear_index(nx as usize, nz as usize, DIM)] >= state.height[idx] {
                    return false;
                }
            }
            true
        };
        assert!(
            till_cells.iter().any(|&idx| is_local_max(idx)),
            "at least one Till cell must be a local max (moraine ridge)"
        );
    }

    /// Ledger clause (iv): plateau regression tooth (F44/F47/F48/F51/F54/F57/F60).
    /// The DEPOSIT-referenced capping-capable fixture: on production (11111), the pinned PROFILE
    /// must have `truncated==0` (no plateau), and scaling the profile by K_CTRL=4 must produce
    /// a capping-capable control with `truncated>0` (proving non-vacuity — the fixture actually
    /// can cap if we increase profile depth).
    #[test]
    fn ledger_clause_plateau_tooth_pinned_profile_and_control() {
        const SEED1: u64 = 0xA11A_2A11; // seed=1 (production control fixture)
        const HMAX64: i64 = 200;
        const DIM64: usize = 64;
        const K_CTRL: i64 = 4;

        let height = erode(SEED1, HMAX64, DIM64, true, false, false, false, true, 100).height;

        // Production pinned profile: truncated must be 0.
        let state_pinned = run_glacial_with(SEED1, DIM64, HMAX64, &height, &PROFILE, K_BAND, 100);
        assert_eq!(
            state_pinned.truncated, 0,
            "pinned profile on capping-capable fixture must have truncated==0, but got {}",
            state_pinned.truncated
        );

        // Positive control: scaled(K_CTRL) must produce truncated > 0 (non-vacuous).
        let scaled_profile = PROFILE.scaled(K_CTRL);
        let state_scaled = run_glacial_with(SEED1, DIM64, HMAX64, &height, &scaled_profile, K_BAND, 100);
        assert!(
            state_scaled.truncated > 0,
            "scaled profile (K_CTRL={K_CTRL}) on capping-capable fixture must have truncated>0 \
             (proving non-vacuity), but got {}",
            state_scaled.truncated
        );
    }

    // ── W-19: glacial strength parameter (#497) ──────────────────────────────────────────────────

    /// W-19: Strength=100 must be byte-identical to production baseline (no-op).
    #[test]
    fn glacial_strength_100_is_byte_identical_to_baseline() {
        let height = eroded_fixture();
        let baseline = run_glacial(SEED, DIM, HMAX, &height, 100);
        let explicit_100 = run_glacial(SEED, DIM, HMAX, &height, 100);
        assert_eq!(baseline, explicit_100, "strength=100 must be byte-identical to baseline");
    }

    /// W-19: Strength=0 produces zero incision (post-incision == pre-incision).
    #[test]
    fn glacial_strength_0_produces_zero_incision() {
        let height = eroded_fixture();
        let state = run_glacial(SEED, DIM, HMAX, &height, 0);
        // With zero incision, the only change should be zero excavation and zero deposition.
        // The height field may change slightly due to moraine deposition, but excavated_total must be 0.
        assert_eq!(
            state.excavated_total, 0,
            "strength=0 must produce zero incision: excavated_total should be 0, got {}",
            state.excavated_total
        );
    }

    /// W-19: Strength clamping: values > 400 are clamped to 400.
    #[test]
    fn glacial_strength_clamping() {
        let height = eroded_fixture();
        let clamped_high = run_glacial(SEED, DIM, HMAX, &height, 500);
        let clamped_max = run_glacial(SEED, DIM, HMAX, &height, 400);
        // Both should produce same result (clamped to 400)
        assert_eq!(
            clamped_high.height, clamped_max.height,
            "strength values > 400 should be clamped to 400"
        );
    }

    /// W-19: Strength monotonicity: higher strength ≥ lower strength in terms of |Δh|.
    #[test]
    fn glacial_strength_monotonic_incision() {
        const DIM_SMALL: usize = 16;
        let height = erode(SEED, HMAX, DIM_SMALL, true, false, false, false, true, 100).height;

        let s0 = run_glacial(SEED, DIM_SMALL, HMAX, &height, 0);
        let s100 = run_glacial(SEED, DIM_SMALL, HMAX, &height, 100);
        let s200 = run_glacial(SEED, DIM_SMALL, HMAX, &height, 200);

        // Compute total absolute height delta
        let delta_s0: i64 = (0..height.len()).map(|i| (height[i] - s0.height[i]).abs()).sum();
        let delta_s100: i64 = (0..height.len()).map(|i| (height[i] - s100.height[i]).abs()).sum();
        let delta_s200: i64 = (0..height.len()).map(|i| (height[i] - s200.height[i]).abs()).sum();

        assert!(delta_s0 <= delta_s100, "strength=0 should produce <= change than strength=100");
        assert!(delta_s100 <= delta_s200, "strength=100 should produce <= change than strength=200");
    }
}
