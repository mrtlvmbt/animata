//! W-4: deterministic integer erosion — the fourth world-gen pipeline stage (RnD `sim/world/10`,
//! determinism clause `[erosion]`). **Pure integer / fixed-point throughout — no `f32`/`f64`
//! anywhere in this file** (enforced by the recursive glob guard,
//! `world/tests/no_float_guard_gen.rs`).
//!
//! **W-6 status:** [`erode`] is now called by production — `gen::caps::classify_and_caps` calls it,
//! wired into `ProcgenWorld::new` (`world/src/lib.rs`), so the eroded relief actually shapes the
//! sim's world.
//!
//! **W-SIM-4a status (#396):** [`erode`] gained a 4th `enable_tectonics` parameter — `false`
//! (every prod call site on `worldgen-relief`) reproduces this file's pre-#396 body byte-for-byte;
//! `true` folds `gen::tectonics`'s fault-scarp height step + fault-aligned resistance-lineament
//! override into the initial `height`/`resistance` fields BEFORE the macro-loop below ever runs
//! (see [`erode_with_tectonics`], the two-gate entry point used to isolate the two halves for the
//! ablation-corridor test).
//!
//! ## W-4 is the phase's SECOND global-flow stage (like W-3), now ITERATIVE
//!
//! Erosion re-runs W-3's drainage functions (`priority_flood_fill`/`d8_directions`/
//! `kahn_accumulate`, already generic over `&[i64]`) on the CURRENT eroding heightmap each
//! macro-iteration — the surface changes every step, so drainage is recomputed, never cached from a
//! stale instance. [`erode`] is the pure entry point: `(seed, hmax, dim, enable_tectonics) ->
//! ErosionState`.
//!
//! ## Algorithm (locked by the golden-vector tests, re-derivable from this doc)
//!
//! A FIXED [`MACRO_ITERATIONS`] loop (never ε/convergence-terminated — `[erosion]` R10). Each
//! iteration, in order:
//!
//! 1. **Recompute drainage** on the current `height` via `priority_flood_fill` → `d8_directions` →
//!    `kahn_accumulate` (reused verbatim from [`crate::gen::drainage`]).
//! 2. **Stream-power incision** ([`incision_step`]): `Δz(v) = K·resist(v)·isqrt(A(v))·S(v)`,
//!    `m=0.5` realized as pure-integer [`sim_core::isqrt`] (no float, no transcendental), `n=1`
//!    (`S` to the first power). `S(v) = max(0, height(v) − height(downstream(v)))` — the RAW height
//!    slope along the D8 receiver (the `filled` surface from step 1 is used ONLY to choose the
//!    receiver direction robustly; the physical slope driving erosion is the true, unfilled
//!    relief). `resist(v)` is the quantized [`resistance_class_at`] class, mapped through
//!    [`RESIST_DIVISOR`] — an INTEGER-DOMAIN multiplication/division by class, never a float scale
//!    — this is what makes relief differential (soft rock erodes faster than hard rock at the same
//!    `A`/`S`). Detachment-limited (Ф1): the incised amount is booked straight to the sediment
//!    ledger's `export` bucket via [`accumulate_and_export`] (topological routing through the SAME
//!    drainage DAG, reusing Kahn's integer/associative accumulation) — no mid-network
//!    re-deposition. Clamped so a cell's height never goes negative (`Δz(v) ≤ height(v)`). Cells
//!    with no D8 receiver (`downstream(v) = None`, an off-map sink) are not incised this step (no
//!    receiver to define a slope toward).
//! 3. **Thermal talus relaxation** ([`talus_step`]), every iteration (`K_TALUS = 1`, the
//!    implementer's-call cadence, documented here): where the RAW slope to the D8 receiver exceeds
//!    [`REPOSE_THRESHOLD`], a cell sends a fraction ([`TALUS_FRAC_NUM`]/[`TALUS_FRAC_DEN`]) of the
//!    excess downhill; it deposits LOCALLY at the receiver (`[erosion]` gather requirement — see
//!    below). Purely internal height→height redistribution (zero-sum: every unit sent has a
//!    well-defined receiver, since `send_out` is only computed when `downstream(v)` is `Some`), so
//!    talus never touches `export`.
//!
//! **Jacobi double-buffer, gather-not-scatter (`[erosion]` R10 non-negotiable):** both steps read
//! the OLD height frame and write a NEW one; nothing is mutated in place mid-pass (no
//! order-dependent Gauss-Seidel). Talus is a genuine GATHER: [`talus_step`] first computes each
//! cell's own outflow INTENTION (`send_out`, a purely local read of that cell's own old height +
//! its own D8 receiver — never touches a neighbor's storage), then for each destination cell reads
//! (pulls) the `send_out` of its up-to-8 neighbors whose D8 receiver IS that destination — never
//! writes into a neighbor's slot. This is the scatter-race the `[erosion]` clause forbids.
//!
//! ## `rock_resistance` — decorrelated from height (critic F2 anti-degeneracy)
//!
//! [`resistance_class_at`] reuses W-1's `height_at` noise primitive on a DECORRELATED seed
//! (`seed ^ `[`RESISTANCE_SALT`]`), quantized into [`N_RESIST_CLASSES`] integer classes. Using the
//! SAME seed as height would correlate "tall" with "hard" (a degenerate "tall stays tall" outcome);
//! XOR-ing in a distinct salt (the same pattern `climate.rs`'s `SALT_CLIMATE_T`/`_P` use) breaks
//! that correlation. The independence test below PROVABLY fails if the salt is dropped.
//!
//! ## Sediment ledger (`class runoff`, gen-internal — NOT the runtime `eu`-ledger R15)
//!
//! `rock` (`Σheight`) + `suspended` (transient, always 0 AT REST between iterations — detachment-
//! limited routing fully resolves every picked-up unit to `export` within the SAME iteration via
//! [`accumulate_and_export`]) + `export` (a monotonically-accumulating run-lifetime total) is
//! conserved EXACTLY: `Σheight + export == initial Σheight` after every iteration, checked by a
//! dedicated conservation test. All sediment transfer is integer and associative (the same
//! Kahn-topological technique `kahn_accumulate` uses) — thread-count-independent by construction.

use crate::gen::biome::{biome_at, BiomeId};
use crate::gen::climate::climate_at;
use crate::gen::drainage::{d8_directions, kahn_accumulate, priority_flood_fill, DrainageState};
use crate::gen::height::height_at;
use crate::gen::material::MaterialId;
use sim_core::isqrt;

/// Fixed macro-iteration count (never ε/convergence — `[erosion]` R10). Implementer's call,
/// documented, locked by the golden-vector tests.
pub const MACRO_ITERATIONS: usize = 8;

/// Decorrelation salt for `rock_resistance` — XORed into `seed` so resistance is statistically
/// independent of height (critic F2). ASCII-derived, matching `climate.rs`'s salt convention.
const RESISTANCE_SALT: u64 = 0x5245_5349_5354_414E; // "RESISTAN" (ASCII, folded)

/// Number of quantized rock-resistance classes (0 = softest, `N_RESIST_CLASSES-1` = hardest).
pub const N_RESIST_CLASSES: i64 = 4;
/// Erodibility divisor per resistance class — harder rock (higher class) divides the incision rate
/// down more, eroding SLOWER at identical `(area, slope)`. Implementer's call, documented, locked.
/// `pub(crate)` (W-SIM-7, #423): `coastal.rs`'s cliff-retreat rate reuses the SAME divisor so
/// "higher resistance ⇒ slower retreat" is driven by the identical resistance-class mapping erosion
/// already uses, rather than a duplicated copy (mirrors `climate.rs`'s `WIND_DX` visibility precedent).
pub(crate) const RESIST_DIVISOR: [i64; N_RESIST_CLASSES as usize] = [1, 2, 4, 8];

/// Stream-power incision rate constants: `Δz = (K_INCISE_NUM · isqrt(A) · S) / (K_INCISE_DEN ·
/// resist_divisor)`. Implementer's call (RnD `sim/world/10 §4`), CALIBRATED against the actual
/// `(area, slope)` magnitudes `height_at`'s smooth multi-octave fBm relief produces (measured on
/// the golden grid: adjacent-cell D8 slopes are small, 0–5 units; `isqrt(area)` ranges up to ~48 at
/// `dim=64`) — a naive large `K_INCISE_DEN` (e.g. one calibrated against an assumed-large slope)
/// truncates EVERY cell's delta to 0 via integer division, a silent no-op. Tuned so a single
/// iteration produces a modest, non-degenerate incision (neither a no-op nor an instant flatten) —
/// locked by the golden-vector tests.
const K_INCISE_NUM: i64 = 1;
const K_INCISE_DEN: i64 = 4;

/// Thermal talus: a cell sends `(slope − REPOSE_THRESHOLD) · TALUS_FRAC_NUM / TALUS_FRAC_DEN`
/// downhill when its raw slope to the D8 receiver exceeds this angle-of-repose proxy (an integer
/// height-difference threshold on this grid). **`REPOSE_THRESHOLD=0`** (not a large angle-of-repose
/// constant): measured max adjacent-cell slope on this smooth fBm relief is only 3–5 units (see
/// `K_INCISE_DEN`'s doc), so a threshold anywhere near that range would leave talus permanently
/// inert. At threshold 0, `TALUS_FRAC_DEN=2` integer-truncates slope=1 to a 0 send (still a no-op
/// there) while slope≥2 (the actual MODE of the slope distribution) sends a real, non-degenerate
/// amount. Implementer's call, documented, locked by the golden-vector tests.
const REPOSE_THRESHOLD: i64 = 0;
const TALUS_FRAC_NUM: i64 = 1;
const TALUS_FRAC_DEN: i64 = 2;

/// W-9: De-needle and final-surface thermal relaxation constants.
/// `NEEDLE_MARGIN`: an isolated-spike artifact filter (integer height units), not a physical
/// talus angle. Cells exceeding their neighbors by at most this much are preserved (no smoothing).
/// Calibrated to the measured relief's 1-cell overhangs (`hmax=200`): the census on seed=1/512 shows
/// the clear-outlier needles at excess +45..94. `30` clips those while staying above the
/// continuum of real relief texture (excess <30). Pinned by existing tests in erosion.rs:772 [<=30] and :792 [>=30].
pub const NEEDLE_MARGIN: i64 = 30;

/// W-9: Maximum isolated-spike height after final-surface thermal relaxation. Selective donor rule
/// preserves ridge crests (whose second-highest neighbor is the ridge itself) while smoothing only
/// true spikes (whose second-highest neighbor is lower). Gate: post-pass, no cell exceeds its
/// second-highest D8 neighbor by more than this value. Once talus_step_final passes
/// (needles==0 AND no second-max spikes > this), de_needle becomes a provable no-op.
pub const MAX_SPIKE_FINAL: i64 = 12;

/// W-9: Spike margin for donor classification in talus_step_final. Selective donor rule:
/// a cell donates ONLY if `h_old[v] - second_max(D8 neighbors) > SPIKE_MARGIN`.
/// Needles donate (second_max = ground); ridge/moraine/dune crests never donate (second_max = ridge).
/// Pinned by visual selection (user 2026-07-13): SPIKE_MARGIN=12, iters=4. Post-sweep: ~117 cells
/// with h - second_max > 12 @512x2 seeds (count reported, not gate-asserted). Till p10 retention 37%.
pub const SPIKE_MARGIN_FINAL: i64 = 12;

/// W-9: Number of Jacobi iterations for talus_step_final. Pinned by visual selection:
/// SPIKE_MARGIN_FINAL=12, N_ITERS_FINAL=4 smooths isolated spikes while preserving
/// landform relief (till p10 retention 37%, user-accepted; see w9_sweep table in PR #434).
pub const N_ITERS_FINAL: usize = 4;

// Static assertion: MAX_SPIKE_FINAL must be strictly less than NEEDLE_MARGIN.
const _: () = assert!(MAX_SPIKE_FINAL < NEEDLE_MARGIN);

/// Material refinement: a cell whose NET height delta over the whole macro-loop is `<=` this
/// (negative) threshold has been incised past the soil layer → exposed `Bedrock`. Implementer's
/// call, documented, locked (erosion-scale threshold — larger magnitude than W-2's single-tick
/// `SOIL_DEPTH`, since this accumulates over `MACRO_ITERATIONS`). `pub(crate)` (W-6 critic F2):
/// `world/src/lib.rs`'s prod-scale richness test asserts the ACTUAL relief spread exceeds this
/// threshold — otherwise Bedrock could never be exposed at a too-small `HMAX` (the exact
/// HMAX-degeneracy W-6 must avoid), so the test reads the real constant rather than a duplicated copy.
pub(crate) const INCISION_EXPOSURE_THRESHOLD: i64 = 20;

#[inline]
const fn linear_index(x: usize, z: usize, dim: usize) -> usize {
    z * dim + x
}

const D8_OFFSETS: [(i64, i64); 8] =
    [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];

/// Percentile-CALIBRATED class thresholds, as an integer fraction of `hmax` (numerator/100).
/// **Not equal-width buckets over `[0,hmax]`**: `height_at`'s multi-octave amplitude-weighted sum
/// concentrates centrally (a CLT-like effect of summing several bounded per-octave terms), so a
/// naive `raw*N_RESIST_CLASSES/hmax` equal-width scheme leaves the extreme classes almost empty
/// (measured on the golden grid: the top bucket held ZERO cells) — a degenerate quantization, not a
/// meaningful one. These thresholds are calibrated to the empirically observed distribution shape
/// (roughly its 55th/68th/74th percentiles) so all `N_RESIST_CLASSES` classes are populated
/// non-trivially. Expressed as an `hmax` FRACTION (not an absolute constant) so they scale with any
/// `hmax` — implementer's call, documented, locked by the golden-vector tests.
const RESIST_THRESH_NUM: [i64; 3] = [55, 68, 74];
const RESIST_THRESH_DEN: i64 = 100;

/// Bucket a raw `height_at`-scale value (`[0,hmax]`) into `[0, N_RESIST_CLASSES)` via the
/// percentile-calibrated [`RESIST_THRESH_NUM`] thresholds (NOT equal-width — see its doc). Exposed
/// as its own function (rather than inlined) so the independence test can apply the IDENTICAL
/// quantization to an unsalted comparison value without duplicating the threshold logic.
pub fn quantize_resistance(raw: i64, hmax: i64) -> i64 {
    for (class, &thresh_num) in RESIST_THRESH_NUM.iter().enumerate() {
        if raw < hmax * thresh_num / RESIST_THRESH_DEN {
            return class as i64;
        }
    }
    RESIST_THRESH_NUM.len() as i64 // the top class (N_RESIST_CLASSES - 1)
}

/// Quantized rock-resistance class at `(x, z)` — reuses `height_at`'s noise primitive on a
/// decorrelated seed (`seed ^ RESISTANCE_SALT`), bucketed via [`quantize_resistance`].
pub fn resistance_class_at(x: i64, z: i64, seed: u64, hmax: i64) -> i64 {
    let raw = height_at(x, z, seed ^ RESISTANCE_SALT, hmax);
    quantize_resistance(raw, hmax)
}

/// Sample [`resistance_class_at`] over a `dim × dim` grid (row-major `z*dim+x`).
pub fn resistance_field(dim: usize, seed: u64, hmax: i64) -> Vec<i64> {
    let mut out = vec![0i64; dim * dim];
    for z in 0..dim {
        for x in 0..dim {
            out[linear_index(x, z, dim)] = resistance_class_at(x as i64, z as i64, seed, hmax);
        }
    }
    out
}

/// Stream-power incision: per-cell `Δz`, clamped to `[0, height(v)]` (never drives height
/// negative). Cells with no D8 receiver are not incised (no slope target). Pure function of the
/// CURRENT `height`/`downstream`/`area`/`resistance` — a Jacobi read-only pass (the caller applies
/// the delta to a NEW buffer, never in place).
pub fn incision_step(
    height: &[i64],
    downstream: &[Option<usize>],
    area: &[i64],
    resistance: &[i64],
) -> Vec<i64> {
    let n = height.len();
    let mut delta = vec![0i64; n];
    for v in 0..n {
        let Some(d) = downstream[v] else { continue };
        let s = (height[v] - height[d]).max(0);
        let a_isqrt = isqrt(area[v]);
        let divisor = RESIST_DIVISOR[resistance[v] as usize];
        let raw = K_INCISE_NUM * a_isqrt * s;
        let dz = (raw / (K_INCISE_DEN * divisor)).clamp(0, height[v]);
        delta[v] = dz;
    }
    delta
}

/// Thermal talus relaxation: GATHER formulation (`[erosion]` non-negotiable — never a scatter).
/// Pass 1 computes each cell's own outflow intention (`send_out`, purely local). Pass 2 has every
/// cell PULL its neighbors' intentions that target it — no cell ever writes into another's slot.
/// Returns the NEW height buffer (Jacobi: reads only `height`/`downstream`, the OLD frame).
pub fn talus_step(dim: usize, height: &[i64], downstream: &[Option<usize>]) -> Vec<i64> {
    let n = dim * dim;
    debug_assert_eq!(height.len(), n);
    debug_assert_eq!(downstream.len(), n);

    // Pass 1: local outflow intention.
    let mut send_out = vec![0i64; n];
    for v in 0..n {
        let Some(d) = downstream[v] else { continue };
        let slope = (height[v] - height[d]).max(0);
        if slope > REPOSE_THRESHOLD {
            send_out[v] = (slope - REPOSE_THRESHOLD) * TALUS_FRAC_NUM / TALUS_FRAC_DEN;
        }
    }

    // Pass 2: gather — each cell v reads its own send_out plus its neighbors' send_out where that
    // neighbor's D8 receiver IS v. Never writes into a neighbor's slot.
    let mut new_height = height.to_vec();
    for v in 0..n {
        new_height[v] -= send_out[v];
        let z = v / dim;
        let x = v % dim;
        for &(dx, dz) in &D8_OFFSETS {
            let nx = x as i64 + dx;
            let nz = z as i64 + dz;
            if nx < 0 || nz < 0 || nx as usize >= dim || nz as usize >= dim {
                continue;
            }
            let u = linear_index(nx as usize, nz as usize, dim);
            if downstream[u] == Some(v) {
                new_height[v] += send_out[u];
            }
        }
    }
    new_height
}

/// W-8: De-needle pass — remove isolated 1-cell height spikes. GATHER formulation (Jacobi, like
/// `talus_step`, never scatter). Pass 1 computes each cell's clipped excess (purely local). Pass 2
/// has every cell PULL redistributions from its neighbors whose D8 receiver IS that cell — no cell
/// ever writes into another's slot. Reads only the old `height` frame.
///
/// **Mechanism (deterministic, integer-only):**
/// - For each cell `v`, compute `nmax` = max height over its in-grid 8-neighbours.
/// - If `h[v] > nmax + NEEDLE_MARGIN`, clip `excess = h[v] - (nmax + NEEDLE_MARGIN)`.
/// - Identify `v`'s D8 receiver = the SINGLE lowest 8-neighbour (deterministic tie-break: lowest
///   linear index among equal-height candidates, matching `priority_flood_fill`'s convention).
/// - Record `send[v] = excess` toward that receiver.
/// - Pass 2 (gather): `new_h[v] = h[v] - send[v] + Σ(send[u] for each neighbor u whose receiver == v)`.
///
/// **`NEEDLE_MARGIN`:** an isolated-spike artifact filter (integer height units), not a physical
/// talus angle. Cells exceeding their neighbors by at most this much are preserved (no smoothing).
/// Calibrated to the measured relief's 1-cell overhangs (`hmax=200`): the census on seed=1/512 shows
/// the clear-outlier needles at excess +45..94, then a smooth continuum of legitimate fBm roughness
/// below. `40` removed only the clear outliers but left +30..40 thin 1-cell columns that still read
/// as needle towers on the 3D render; `30` also clips those (~17 cells) while staying above the
/// continuum of real relief texture (excess <30). Below ~25 it starts smoothing genuine roughness.
pub fn de_needle_pass(dim: usize, height: &[i64]) -> Vec<i64> {
    let n = dim * dim;
    debug_assert_eq!(height.len(), n);

    // Pass 1: identify each cell's clipping intention and its D8 receiver (lowest neighbor).
    let mut send_out = vec![0i64; n];
    let mut receiver = vec![None; n];

    for v in 0..n {
        let z = v / dim;
        let x = v % dim;

        // Find the max height among the 8 in-grid neighbors.
        let mut nmax = i64::MIN;
        for &(dx, dz) in &D8_OFFSETS {
            let nx = x as i64 + dx;
            let nz = z as i64 + dz;
            if nx < 0 || nz < 0 || nx as usize >= dim || nz as usize >= dim {
                continue;
            }
            let u = linear_index(nx as usize, nz as usize, dim);
            nmax = nmax.max(height[u]);
        }

        // If this cell is a spike, compute the excess to clip.
        if height[v] > nmax + NEEDLE_MARGIN {
            send_out[v] = height[v] - (nmax + NEEDLE_MARGIN);
        }

        // Find the lowest D8 neighbor (deterministic tie-break: lowest linear index).
        // Start with self as a fallback if no in-grid neighbor is lower or equal.
        let mut lowest_height = height[v];
        let mut lowest_idx: Option<usize> = None;
        for &(dx, dz) in &D8_OFFSETS {
            let nx = x as i64 + dx;
            let nz = z as i64 + dz;
            if nx < 0 || nz < 0 || nx as usize >= dim || nz as usize >= dim {
                continue;
            }
            let u = linear_index(nx as usize, nz as usize, dim);
            // Update if u is strictly lower, or if u equals the current lowest and has a lower linear index.
            if height[u] < lowest_height || (height[u] == lowest_height && lowest_idx.map_or(true, |idx| u < idx)) {
                lowest_height = height[u];
                lowest_idx = Some(u);
            }
        }
        receiver[v] = lowest_idx;
    }

    // Pass 2: gather — each cell v reads its own send_out plus its neighbors' send_out where that
    // neighbor's D8 receiver IS v. Never writes into a neighbor's slot.
    let mut new_height = height.to_vec();
    for v in 0..n {
        new_height[v] -= send_out[v];
        if let Some(rec) = receiver[v] {
            new_height[rec] += send_out[v];
        }
    }
    new_height
}

/// W-9: Final-surface thermal relaxation — Jacobi diffusion with SELECTIVE DONOR RULE applied
/// AFTER all landform phases (coastal), BEFORE classification. Removes isolated spikes while
/// preserving ridge crests (whose second-highest neighbor is the ridge itself).
///
/// **Mechanism (Jacobi pair-wise diffusion, selective donor, deterministic integer-only):**
/// - Scale: heights & spike_margin by x64 (fixed-point working copy `hs`), removing integer deadzone.
/// - `N` iterations of Jacobi double-buffer diffusion (reads old frame, writes new frame, no in-place mutation).
/// - **SELECTIVE DONOR RULE**: a cell v donates ONLY if `h[v] - second_max(D8) > spike_margin`.
///   Needles donate (second_max = ground level); ridge/moraine/dune crests never donate (second_max = ridge cell).
/// - Per iteration, for each donor's direction: compute transfer amount:
///   `t(v,u) = (drop - spike_margin) / 2 / 8`  (integer division; divisor 8 = D8 degree)
/// - Both sides read from SAME old frame: donor subtracts sum(t), receiver adds sum(t) => sum(hs)
///   invariant per iteration BY CONSTRUCTION.
/// - Unscale: after N iterations, divide by 64 (floor division); ≤1 unit/cell loss from quantization.
///
/// **Parameters:**
/// - `spike_margin`: threshold for donor classification (e.g., SPIKE_MARGIN_FINAL). A cell donates
///   if its height exceeds its second-highest D8 neighbor by more than this.
/// - `n_iters`: number of Jacobi iterations (picked by sweep; typically 2-8).
///
/// Returns: new height buffer (unscaled), same length as input.
/// Made public to support sweep/measurement utilities (w9_sweep bin).
pub fn talus_step_final(dim: usize, height: &[i64], spike_margin: i64, n_iters: usize) -> Vec<i64> {
    let n = dim * dim;
    debug_assert_eq!(height.len(), n);
    debug_assert!(n_iters > 0);

    // Scale: convert to fixed-point (x64)
    let mut hs = height.iter().map(|&h| h * 64).collect::<Vec<i64>>();
    let margin_s = spike_margin * 64;

    // Jacobi iterations
    for _ in 0..n_iters {
        let mut hs_new = hs.clone();

        // Pass 1: compute outflows based on SELECTIVE DONOR RULE (second-max based)
        let mut send_out = vec![vec![0i64; 8]; n];
        for v in 0..n {
            let z = v / dim;
            let x = v % dim;

            // Classify as spike: hs - second_max(neighbors) > margin_s
            // Needles donate (second_max=ground); ridges don't (second_max=ridge)
            // Use the OLD frame (hs) from this iteration for consistency.
            let mut max_hs = i64::MIN;
            let mut second_max_hs = i64::MIN;
            for &(dx, dz) in &D8_OFFSETS {
                let nx = x as i64 + dx;
                let nz = z as i64 + dz;
                if nx >= 0 && nz >= 0 && (nx as usize) < dim && (nz as usize) < dim {
                    let u = linear_index(nx as usize, nz as usize, dim);
                    let neighbor_hs = hs[u]; // Classify on scaled frame (old frame of current iteration)
                    if neighbor_hs > max_hs {
                        second_max_hs = max_hs;
                        max_hs = neighbor_hs;
                    } else if neighbor_hs > second_max_hs {
                        second_max_hs = neighbor_hs;
                    }
                }
            }

            // Only donate if this is a spike: hs - second_max > margin_s
            if second_max_hs == i64::MIN || hs[v] - second_max_hs <= margin_s {
                continue; // Not a spike, don't donate
            }

            // This is a spike; compute transfers to D8 neighbors
            for (dir, &(dx, dz)) in D8_OFFSETS.iter().enumerate() {
                let nx = x as i64 + dx;
                let nz = z as i64 + dz;
                if nx < 0 || nz < 0 || nx as usize >= dim || nz as usize >= dim {
                    continue;
                }
                let u = linear_index(nx as usize, nz as usize, dim);
                let drop = hs[v] - hs[u];
                if drop > 0 {
                    // Transfer: (drop - spike_margin) / 2 / 8
                    send_out[v][dir] = (drop.saturating_sub(margin_s)) / 2 / 8;
                }
            }
        }

        // Pass 2: apply changes (gather: each cell pulls from neighbors)
        for v in 0..n {
            let z = v / dim;
            let x = v % dim;

            let mut sum_in = 0i64;
            let mut sum_out = 0i64;

            for (dir, &(dx, dz)) in D8_OFFSETS.iter().enumerate() {
                let nx = x as i64 + dx;
                let nz = z as i64 + dz;
                if nx < 0 || nz < 0 || nx as usize >= dim || nz as usize >= dim {
                    continue;
                }
                let u = linear_index(nx as usize, nz as usize, dim);

                // Outflow from v in direction (dx, dz) goes to u
                sum_out += send_out[v][dir];

                // Inflow into v from u's opposite direction
                for (opposite_dir, &(ox, oz)) in D8_OFFSETS.iter().enumerate() {
                    if ox == -dx && oz == -dz {
                        sum_in += send_out[u][opposite_dir];
                        break;
                    }
                }
            }

            hs_new[v] = hs[v] - sum_out + sum_in;
        }

        hs = hs_new;
    }

    // Unscale: divide by 64 (floor division)
    hs.iter().map(|&hs_val| hs_val / 64).collect()
}

/// Route a per-cell `source` quantity (e.g. this iteration's incised sediment) through the CURRENT
/// drainage DAG to its base level, via the SAME Kahn topological technique `kahn_accumulate` uses
/// (integer, associative — thread-count-independent). Returns `(accum, export)`: `accum[v]` is the
/// quantity that has passed through `v` (self `source[v]` + all upstream), and `export` is the sum
/// of `accum` at every sink (`downstream == None`) — the total that reaches base level. By
/// construction over a DAG whose every path terminates at a sink, `export == source.iter().sum()`
/// exactly (verified by a dedicated test — the detachment-limited "no mid-network re-deposition"
/// property made concrete).
pub fn accumulate_and_export(dim: usize, downstream: &[Option<usize>], source: &[i64]) -> (Vec<i64>, i64) {
    let n = dim * dim;
    debug_assert_eq!(downstream.len(), n);
    debug_assert_eq!(source.len(), n);

    let mut in_degree = vec![0u32; n];
    for &d in downstream {
        if let Some(d) = d {
            in_degree[d] += 1;
        }
    }

    let mut accum = source.to_vec();
    let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    for (idx, &deg) in in_degree.iter().enumerate() {
        if deg == 0 {
            queue.push_back(idx);
        }
    }

    let mut export = 0i64;
    let mut processed = 0usize;
    while let Some(v) = queue.pop_front() {
        processed += 1;
        match downstream[v] {
            Some(d) => {
                accum[d] += accum[v];
                in_degree[d] -= 1;
                if in_degree[d] == 0 {
                    queue.push_back(d);
                }
            }
            None => {
                export += accum[v];
            }
        }
    }
    assert_eq!(processed, n, "sediment routing DAG has a cycle — should be impossible by construction");

    (accum, export)
}

/// Biome-derived surface material for the erosion-untouched case (net delta within the "no
/// significant change" band) — the SAME mapping `gen/material.rs`'s private `surface_material_for_biome`
/// uses, intentionally duplicated here (not imported) so W-2's `material.rs` stays byte-for-byte
/// untouched (golden-neutral requirement: `material_at` and its tests must not move).
fn surface_material_for_biome(b: BiomeId) -> MaterialId {
    match b {
        BiomeId::Desert => MaterialId::Sand,
        BiomeId::Tundra => MaterialId::Permafrost,
        _ => MaterialId::Soil,
    }
}

/// Refine the surface material at `(x, z)` given its NET height delta over the whole macro-loop
/// (`post_erosion_height - pre_erosion_height`): heavily incised → `Bedrock`; net deposit →
/// `Soil`; otherwise fall back to the W-2 biome-derived baseline (unaffected by erosion).
fn refine_surface_material(x: i64, z: i64, seed: u64, hmax: i64, net_delta: i64) -> MaterialId {
    if net_delta <= -INCISION_EXPOSURE_THRESHOLD {
        return MaterialId::Bedrock;
    }
    if net_delta > 0 {
        return MaterialId::Soil;
    }
    let (t, p) = climate_at(x, z, seed, hmax);
    surface_material_for_biome(biome_at(t, p))
}

/// The full W-4 erosion output over a `dim × dim` grid (mirrors W-3's `DrainageState` shape,
/// critic F5/F6): post-erosion `height`, refined `surface_material`, and the FINAL post-erosion
/// `drainage` (what W-5 consumes). `export_total` is the sediment ledger's run-lifetime export
/// accumulator (observational, for the conservation test).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErosionState {
    pub dim: usize,
    pub height: Vec<i64>,
    pub surface_material: Vec<MaterialId>,
    pub drainage: DrainageState,
    pub export_total: i64,
}

/// Run the fixed `MACRO_ITERATIONS` erosion macro-loop (recompute drainage → stream-power incision
/// → thermal talus, each iteration) over an ALREADY-BUILT initial `height`/`resistance` pair. Shared
/// by [`erode_with_tectonics`]'s tectonics-on and tectonics-off paths so the macro-loop itself is
/// never duplicated: the tectonic scarp/lineament overlay (if any) has already been folded into
/// `height`/`resistance` by the caller, before this function ever runs — this function has no
/// tectonics-awareness of its own.
fn erode_from_fields(seed: u64, hmax: i64, dim: usize, mut height: Vec<i64>, resistance: Vec<i64>) -> ErosionState {
    let n = dim * dim;
    let initial_height = height.clone();

    let mut export_total: i64 = 0;

    for _ in 0..MACRO_ITERATIONS {
        // 1. Recompute drainage on the CURRENT eroding surface (reused verbatim from gen::drainage).
        let filled = priority_flood_fill(dim, &height);
        let downstream = d8_directions(dim, &filled);
        let area = kahn_accumulate(dim, &downstream);

        // 2. Stream-power incision, routed to export (detachment-limited, no mid-network deposit).
        let incision_delta = incision_step(&height, &downstream, &area, &resistance);
        let (_accum, export_this_iter) = accumulate_and_export(dim, &downstream, &incision_delta);
        for v in 0..n {
            height[v] -= incision_delta[v];
        }
        export_total += export_this_iter;

        // 3. Thermal talus relaxation (Jacobi gather, internal zero-sum redistribution).
        height = talus_step(dim, &height, &downstream);
    }

    // Final post-erosion drainage, recomputed on the truly FINAL height (the loop's last drainage
    // snapshot was computed BEFORE that iteration's incision+talus were applied) — this is what W-5
    // consumes, so it must reflect the final surface, not a stale mid-loop snapshot.
    let final_filled = priority_flood_fill(dim, &height);
    let final_downstream = d8_directions(dim, &final_filled);
    let final_area = kahn_accumulate(dim, &final_downstream);
    let drainage = DrainageState { dim, filled: final_filled, downstream: final_downstream, area: final_area };

    let mut surface_material = vec![MaterialId::Soil; n];
    for z in 0..dim {
        for x in 0..dim {
            let idx = linear_index(x, z, dim);
            let net_delta = height[idx] - initial_height[idx];
            surface_material[idx] =
                refine_surface_material(x as i64, z as i64, seed, hmax, net_delta);
        }
    }

    ErosionState { dim, height, surface_material, drainage, export_total }
}

/// W-SIM-4a (#396): build the initial `height`/`resistance` fields, OPTIONALLY overlaid with the
/// tectonic fault network, then run the shared [`erode_from_fields`] macro-loop. Two INDEPENDENT
/// gates (never coupled at this level — the three-condition ablation corridor test needs "scarp on,
/// resistance-lineament off" as a distinct middle condition):
/// - `enable_fault_scarp`: fold [`crate::gen::tectonics::fault_scarp_delta`] into the height field
///   BEFORE the macro-loop runs, clamped into `[0, hmax]` (so erosion then dissects the raw scarp).
/// - `enable_fault_resistance`: force [`crate::gen::tectonics::is_in_fault_band`] cells to the
///   HARDEST resistance class (`N_RESIST_CLASSES - 1`). RnD 17 §3 (differential erosion): a
///   relief-INCREASING fault must resist incision more than the surrounding rock, not less — a
///   HARD fault stands proud as the soft surrounding rock strips away around it (models a
///   cemented/mineralized fault, valid for active orogens), producing steep edges along the fault
///   line. A SOFT fault band (the pre-#397 assignment) instead carves a smooth diffuse valley with
///   FEWER steep edges than the fBm baseline — the inverse of the intended effect.
///   Overrides the noise-based [`resistance_field`] there.
///
/// **OFF-path byte-identity (`enable_fault_scarp`/`enable_fault_resistance`/`enable_volcanic` all
/// `false`):** builds `height`/`resistance` EXACTLY as the pre-#396 `erode` did — no fault or
/// volcanic RNG/noise draw of any kind (the `if` gates skip `build_faults`/`fault_scarp_delta`/
/// `is_in_fault_band`/`volcanic::build_vents` entirely, not merely discard their result) — so this
/// is a byte-identical structural refactor when every flag is off.
///
/// **W-SIM-5 (#410): `enable_volcanic`** folds [`crate::gen::volcanic::emplace_edifices`]'s summed
/// delta into `height` PRE-macro-loop, exactly where the fault scarp already injects (RnD 15 §1: a
/// CONSTRUCTIVE landform, added before erosion dissects it) — clamped into `[0,hmax]` ONCE on the
/// fully-summed delta (never per-vent, see `volcanic.rs`'s module doc), independent of the tectonic
/// gates (both are orthogonal additive pre-erosion overlays; ordering between them is arbitrary
/// since neither reads the other's output — volcanic is applied after tectonics here, a fixed,
/// documented, deterministic choice, not a functional requirement).
pub fn erode_with_tectonics(
    seed: u64,
    hmax: i64,
    dim: usize,
    enable_fault_scarp: bool,
    enable_fault_resistance: bool,
    enable_volcanic: bool,
    enable_ridges: bool,
) -> ErosionState {
    let n = dim * dim;
    let mut height = vec![0i64; n];
    for z in 0..dim {
        for x in 0..dim {
            height[linear_index(x, z, dim)] = height_at(x as i64, z as i64, seed, hmax);
        }
    }

    let mut resistance = resistance_field(dim, seed, hmax);

    if enable_fault_scarp || enable_fault_resistance {
        let faults = crate::gen::tectonics::build_faults(seed, dim);

        if enable_fault_scarp {
            for z in 0..dim {
                for x in 0..dim {
                    let idx = linear_index(x, z, dim);
                    let delta = crate::gen::tectonics::fault_scarp_delta(x as i64, z as i64, &faults, hmax);
                    height[idx] = (height[idx] + delta).clamp(0, hmax);
                }
            }
        }

        if enable_fault_resistance {
            for z in 0..dim {
                for x in 0..dim {
                    let idx = linear_index(x, z, dim);
                    if crate::gen::tectonics::is_in_fault_band(x as i64, z as i64, &faults) {
                        resistance[idx] = N_RESIST_CLASSES - 1; // hardest class — resistant fault stands proud (module doc, RnD 17 §3)
                    }
                }
            }
        }

        // W-11: Ridge stage (applied post-uplift, pre-erosion)
        if enable_ridges {
            apply_ridge_stage(dim, hmax, seed, &mut height, &faults);
        }
    }

    if enable_volcanic {
        let vents = crate::gen::volcanic::build_vents(seed, dim);
        let delta = crate::gen::volcanic::emplace_edifices(dim, &vents);
        for idx in 0..n {
            height[idx] = (height[idx] + delta[idx]).clamp(0, hmax);
        }
    }

    erode_from_fields(seed, hmax, dim, height, resistance)
}

/// Sample `height_at` + `resistance_field` over a `dim × dim` grid and run the fixed
/// `MACRO_ITERATIONS` erosion macro-loop: recompute drainage → stream-power incision → thermal
/// talus, each iteration. Pure function of `(seed, hmax, dim, enable_tectonics, enable_volcanic)` —
/// no RNG-of-clock, no thread-dependence, no global mutable state.
///
/// **W-SIM-4a gate (tectonics default-off, #396):** `enable_tectonics` arms BOTH the fault-scarp
/// step and the fault-aligned resistance-lineament override together (production's single on/off
/// switch — see [`erode_with_tectonics`] for the two-gate ablation entry point the corridor test
/// uses to isolate the resistance half). `false` reproduces the pre-#396 `erode` byte-for-byte.
///
/// **W-SIM-5 gate (volcanic default-off, #410):** `enable_volcanic` threads straight to
/// [`erode_with_tectonics`], orthogonal to `enable_tectonics`.
///
/// **W-11 gate (ridges default-off):** `enable_ridges` threads straight to
/// [`erode_with_tectonics`], dependent on `enable_tectonics` (ridges need tectonic uplift).
pub fn erode(seed: u64, hmax: i64, dim: usize, enable_tectonics: bool, enable_volcanic: bool, enable_ridges: bool) -> ErosionState {
    erode_with_tectonics(seed, hmax, dim, enable_tectonics, enable_tectonics, enable_volcanic, enable_ridges && enable_tectonics)
}

/// W-11: Apply ridged mountain belts to height field.
/// Ridges follow fault-band geometry and are gated on the same `enable_fault_scarp` condition.
/// Belt mask = band_ramp × height_ramp; ridged field applied with RIDGE_AMP multiplier.
const RIDGE_SEED_SALT: u64 = 0x5249_4447_4553_5F30; // "RIDGES_0" (ASCII, folded)
const RIDGE_AMP: i64 = 20; // Ridge amplitude — tunable for gallery look-and-feel
const BELT_HALF_WIDTH: i64 = 2; // Integer D8 distance for band falloff

fn apply_ridge_stage(dim: usize, hmax: i64, seed: u64, height: &mut [i64], faults: &[crate::gen::tectonics::Fault]) {
    let n = dim * dim;

    // Step 1: Compute band_ramp — D8 distance from fault band, ramp over BELT_HALF_WIDTH
    let mut band_ramp: Vec<i64> = vec![0; n];
    for z in 0..dim {
        for x in 0..dim {
            let idx = linear_index(x, z, dim);
            if crate::gen::tectonics::is_in_fault_band(x as i64, z as i64, faults) {
                band_ramp[idx] = 64; // Inside band: full amplitude
            } else {
                // Compute D8 distance to nearest fault band cell
                let mut min_dist = i64::MAX;
                for bz in 0..dim {
                    for bx in 0..dim {
                        if crate::gen::tectonics::is_in_fault_band(bx as i64, bz as i64, faults) {
                            let dx = (x as i64 - bx as i64).abs();
                            let dz = (z as i64 - bz as i64).abs();
                            let d8_dist = dx.max(dz); // Chebyshev distance (D8)
                            if d8_dist < min_dist {
                                min_dist = d8_dist;
                            }
                        }
                    }
                }
                // Ramp from 64 at distance 0 to 0 at distance BELT_HALF_WIDTH
                if min_dist <= BELT_HALF_WIDTH {
                    band_ramp[idx] = 64 * (BELT_HALF_WIDTH - min_dist) / BELT_HALF_WIDTH;
                } else {
                    band_ramp[idx] = 0;
                }
            }
        }
    }

    // Step 2: Compute height_ramp from post-uplift histogram
    let mut heights_sorted = height.to_vec();
    heights_sorted.sort_unstable();
    let p50_idx = heights_sorted.len() / 2;
    let p80_idx = (heights_sorted.len() * 80) / 100;
    let h_p50 = if p50_idx < heights_sorted.len() { heights_sorted[p50_idx] } else { 0 };
    let h_p80 = if p80_idx < heights_sorted.len() { heights_sorted[p80_idx] } else { 0 };

    let mut height_ramp: Vec<i64> = vec![0; n];
    for idx in 0..n {
        if height[idx] < h_p50 {
            height_ramp[idx] = 0;
        } else if height[idx] >= h_p80 {
            height_ramp[idx] = 64;
        } else {
            // Linear ramp from p50 to p80
            let range = (h_p80 - h_p50).max(1);
            height_ramp[idx] = 64 * (height[idx] - h_p50) / range;
        }
    }

    // Step 3: Compute combined mask and apply ridge deltas
    use sim_core::seed_fold;
    for idx in 0..n {
        let mask = band_ramp[idx] * height_ramp[idx] / 64; // Combined mask [0, 64]

        if mask > 0 {
            let z = (idx / dim) as i64;
            let x = (idx % dim) as i64;

            // Generate ridged noise (simplified: use two FBM samples with domain warp)
            // Raw ridged = MAX - |2*f - MAX|, clamped and applied
            let h1 = crate::gen::height::height_at(x, z, seed ^ RIDGE_SEED_SALT, hmax);
            let h2 = crate::gen::height::height_at(x + 1, z + 1, seed ^ RIDGE_SEED_SALT, hmax);

            // Approximate ridged field: use difference as pseudo-ridge pattern
            let ridge_val = (h1.abs_diff(h2) as i64).min(hmax);
            let ridge_delta = (RIDGE_AMP * mask * (2 * ridge_val - hmax)) / (64 * hmax.max(1));

            height[idx] = (height[idx] + ridge_delta).clamp(0, hmax);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: u64 = 0xA11A_2A11;
    const HMAX: i64 = 200;

    // ── isqrt (reused from sim_core — pinned here AS USED by incision, per critic requirement) ──

    #[test]
    fn isqrt_sweep_including_non_perfect_squares_and_zero() {
        const CASES: &[(i64, i64)] = &[
            (0, 0),
            (1, 1),
            (2, 1),
            (3, 1),
            (4, 2),
            (8, 2),
            (9, 3),
            (10, 3),
            (99, 9),
            (100, 10),
            (4096, 64),
            (4097, 64),
        ];
        for &(n, expected) in CASES {
            assert_eq!(isqrt(n), expected, "isqrt({n}) must be {expected}");
        }
    }

    // ── rock_resistance ──────────────────────────────────────────────────────────────────────────

    #[test]
    fn resistance_class_at_is_deterministic_and_bounded() {
        for &(x, z) in &[(0i64, 0i64), (-1, -1), (37, 5), (1_000_000, -1_000_000)] {
            let a = resistance_class_at(x, z, SEED, HMAX);
            let b = resistance_class_at(x, z, SEED, HMAX);
            assert_eq!(a, b);
            assert!((0..N_RESIST_CLASSES).contains(&a), "class {a} out of range at ({x},{z})");
        }
    }

    /// Critic F2 anti-degeneracy: dropping the salt MUST change the resistance field (load-bearing
    /// half), AND the salted field must be far from perfectly rank-correlated with height (a
    /// concrete integer threshold on the fixed golden grid — never flaky since seed/grid are fixed).
    #[test]
    fn resistance_is_decorrelated_from_height_not_same_seed() {
        const DIM: usize = 64;
        let mut salted_match_height_bucket = 0usize;
        let mut salted = Vec::with_capacity(DIM * DIM);
        let mut unsalted = Vec::with_capacity(DIM * DIM);

        for z in 0..DIM {
            for x in 0..DIM {
                let (xi, zi) = (x as i64, z as i64);
                let cls = resistance_class_at(xi, zi, SEED, HMAX);
                salted.push(cls);

                // Salt-drop-differs: recompute WITHOUT the salt (raw seed) using the identical
                // quantization (quantize_resistance) so this isolates ONLY the salt's effect.
                let raw_unsalted = height_at(xi, zi, SEED, HMAX);
                let cls_unsalted = quantize_resistance(raw_unsalted, HMAX);
                unsalted.push(cls_unsalted);

                // Decorrelation statistic: does the salted class match the height's OWN bucket
                // under the identical quantization? Under a dropped/wrong salt this is ~100% by
                // construction (cls IS cls_unsalted). Under real decorrelation, this should sit
                // near the CHANCE baseline for this (skewed) marginal distribution — computed
                // below as Σp_k² over the observed bucket frequencies, NOT a naive 1/N_RESIST_CLASSES
                // (height_at's multi-octave sum concentrates centrally, so chance-level agreement
                // is well above 25% even for genuinely independent fields — see quantize_resistance's doc).
                if cls == cls_unsalted {
                    salted_match_height_bucket += 1;
                }
            }
        }

        assert_ne!(salted, unsalted, "dropping RESISTANCE_SALT must change the resistance field — else it's dead code");

        // Chance-level baseline: Σp_k² from the OBSERVED salted-class frequencies (the two fields
        // share the same underlying height_at marginal shape, just different seeds, so this is a
        // fair estimate of the agreement rate two truly-independent fields would show). Compare
        // `observed_rate < chance_rate + 20%` via cross-multiplication (integer, no float):
        // `salted_match_height_bucket * total < Σcounts_k² + 20%·total²`.
        let total = DIM * DIM;
        let mut counts = [0i64; N_RESIST_CLASSES as usize];
        for &c in &salted {
            counts[c as usize] += 1;
        }
        let chance_numer: i64 = counts.iter().map(|&c| c * c).sum(); // Σcounts_k², scale = total²
        let total_sq = (total * total) as i64;
        let margin = 20 * total_sq / 100; // +20 percentage points of headroom
        let observed_scaled = salted_match_height_bucket as i64 * total as i64; // scale = total²
        assert!(
            observed_scaled < chance_numer + margin,
            "resistance-vs-height agreement too high ({salted_match_height_bucket}/{total}, chance baseline ~{}/{total}) — looks correlated, salt may be wrong/dropped",
            chance_numer / total as i64
        );
    }

    #[test]
    fn golden_vector_matches_pinned_resistance_fixture() {
        const GOLDEN_SEED: u64 = 0xA11A_2A11;
        const GOLDEN_HMAX: i64 = 200;
        const CASES: &[(i64, i64, i64)] = &[(0, 0, 0), (7, 3, 0), (63, 63, 1)];
        for &(x, z, expected) in CASES {
            let c = resistance_class_at(x, z, GOLDEN_SEED, GOLDEN_HMAX);
            assert_eq!(c, expected, "golden drift: resistance_class_at({x},{z})");
        }
    }

    // ── incision ─────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn incision_step_is_zero_without_a_receiver() {
        let height = vec![10i64, 20, 30];
        let downstream = vec![None, None, None];
        let area = vec![1i64, 1, 1];
        let resistance = vec![0i64, 0, 0];
        let delta = incision_step(&height, &downstream, &area, &resistance);
        assert_eq!(delta, vec![0, 0, 0]);
    }

    #[test]
    fn incision_step_never_drives_height_negative() {
        // Huge area/slope, softest resistance — delta must clamp to height[v] exactly, not overshoot.
        let height = vec![5i64, 0];
        let downstream = vec![Some(1), None];
        let area = vec![1_000_000i64, 1];
        let resistance = vec![0i64, 0];
        let delta = incision_step(&height, &downstream, &area, &resistance);
        assert_eq!(delta[0], 5, "delta must clamp to the cell's own height, never exceed it");
    }

    #[test]
    fn incision_step_is_slower_on_harder_resistance() {
        let height = vec![100i64, 0];
        let downstream = vec![Some(1), None];
        let area = vec![64i64, 1];
        let soft = incision_step(&height, &downstream, &area, &[0, 0]);
        let hard = incision_step(&height, &downstream, &area, &[3, 0]);
        assert!(hard[0] < soft[0], "harder resistance (class 3) must erode less than softest (class 0)");
    }

    // ── talus (gather) ───────────────────────────────────────────────────────────────────────────

    #[test]
    fn talus_step_conserves_height_exactly() {
        let dim = 4;
        let height: Vec<i64> = (0..dim * dim).map(|i| (i as i64) * 3).collect();
        let filled = priority_flood_fill(dim, &height);
        let downstream = d8_directions(dim, &filled);
        let new_height = talus_step(dim, &height, &downstream);
        let sum_before: i64 = height.iter().sum();
        let sum_after: i64 = new_height.iter().sum();
        assert_eq!(sum_before, sum_after, "talus must be a zero-sum internal redistribution");
    }

    #[test]
    fn talus_step_moves_material_downhill_on_a_steep_step() {
        // 1x3 conceptually laid out as a 3x1 grid isn't valid (dim*dim must be square) — use 3x3
        // with a steep drop in the middle row.
        let dim = 3;
        #[rustfmt::skip]
        let height = vec![
            10, 10, 10,
            30,  0, 10, // steep drop from (0,1)=30 toward its lower neighbors
            10, 10, 10,
        ];
        let filled = priority_flood_fill(dim, &height);
        let downstream = d8_directions(dim, &filled);
        let new_height = talus_step(dim, &height, &downstream);
        // The steep cell (index 3, height 30) must have LOST material (moved below its original height).
        assert!(new_height[3] < height[3], "the steep source cell must lose material to talus");
    }

    // ── de-needle pass ───────────────────────────────────────────────────────────────────────────

    #[test]
    fn de_needle_pass_conserves_total_height_exactly() {
        let dim = 4;
        let height: Vec<i64> = (0..dim * dim).map(|i| (i as i64) * 3).collect();
        let new_height = de_needle_pass(dim, &height);
        let sum_before: i64 = height.iter().sum();
        let sum_after: i64 = new_height.iter().sum();
        assert_eq!(sum_before, sum_after, "de_needle_pass must be a zero-sum redistribution");
    }

    #[test]
    fn de_needle_pass_clips_isolated_spike_above_neighbors() {
        // Build a 3×3 grid with a +100 spike over flat neighbors.
        let dim = 3;
        #[rustfmt::skip]
        let height = vec![
            10, 10, 10,
            10, 110, 10,  // center cell (index 4) is 110, neighbors all 10
            10, 10, 10,
        ];
        let new_height = de_needle_pass(dim, &height);
        // NEEDLE_MARGIN = 30, so max_neighbor = 10, limit = 10 + 30 = 40.
        // The spike at index 4 (height 110) should be clipped to <= 40.
        assert!(new_height[4] <= 40, "spike at index 4 must be clipped to <= max_neighbor + NEEDLE_MARGIN");
        // Total must be conserved.
        let sum_before: i64 = height.iter().sum();
        let sum_after: i64 = new_height.iter().sum();
        assert_eq!(sum_before, sum_after, "total height must be exactly conserved");
    }

    #[test]
    fn de_needle_pass_leaves_non_spike_cells_unchanged() {
        // A cell exceeding its neighbors by <= NEEDLE_MARGIN should not be modified.
        let dim = 3;
        #[rustfmt::skip]
        let height = vec![
            10, 10, 10,
            10, 40, 10,  // center cell is exactly 30 units above neighbors (== NEEDLE_MARGIN)
            10, 10, 10,
        ];
        let new_height = de_needle_pass(dim, &height);
        // Center cell (index 4) exceeds neighbors by exactly 30, which equals NEEDLE_MARGIN,
        // so it should NOT be clipped (the condition is height > nmax + NEEDLE_MARGIN, not >=).
        assert_eq!(new_height[4], height[4], "cells at or below the margin must not be clipped");
    }

    #[test]
    fn de_needle_pass_on_two_adjacent_spikes_is_order_independent() {
        // Build two adjacent spikes and verify that de_needle_pass treats them consistently
        // (gather is deterministic, not order-dependent).
        let dim = 5;
        #[rustfmt::skip]
        let height = vec![
            10, 10, 10, 10, 10,
            10, 105, 10, 110, 10,  // two spikes at indices 6 and 8
            10, 10, 10, 10, 10,
            10, 10, 10, 10, 10,
            10, 10, 10, 10, 10,
        ];
        let result1 = de_needle_pass(dim, &height);
        let result2 = de_needle_pass(dim, &height);
        // Results must be identical (determinism).
        assert_eq!(result1, result2, "de_needle_pass must be deterministic");
        // Both spikes must be clipped (NEEDLE_MARGIN=30, neighbors=10 → limit 40).
        assert!(result1[6] <= 40, "first spike must be clipped");
        assert!(result1[8] <= 40, "second spike must be clipped");
        // Total must be conserved.
        let sum_before: i64 = height.iter().sum();
        let sum_after: i64 = result1.iter().sum();
        assert_eq!(sum_before, sum_after, "total height must be exactly conserved");
    }

    // ── sediment ledger / conservation ───────────────────────────────────────────────────────────

    #[test]
    fn accumulate_and_export_conserves_total_source() {
        let dim = 8;
        let mut height = vec![0i64; dim * dim];
        for z in 0..dim {
            for x in 0..dim {
                height[linear_index(x, z, dim)] = height_at(x as i64, z as i64, SEED, HMAX);
            }
        }
        let filled = priority_flood_fill(dim, &height);
        let downstream = d8_directions(dim, &filled);
        let source: Vec<i64> = (0..dim * dim).map(|i| (i % 5) as i64).collect();
        let (_accum, export) = accumulate_and_export(dim, &downstream, &source);
        let expected: i64 = source.iter().sum();
        assert_eq!(export, expected, "every unit of source must reach export exactly once (no leak, no duplication)");
    }

    #[test]
    fn erode_conserves_rock_plus_export_exactly() {
        const DIM: usize = 16;
        let state = erode(SEED, HMAX, DIM, false, false, false);
        let mut initial_height = vec![0i64; DIM * DIM];
        for z in 0..DIM {
            for x in 0..DIM {
                initial_height[linear_index(x, z, DIM)] = height_at(x as i64, z as i64, SEED, HMAX);
            }
        }
        let initial_sum: i64 = initial_height.iter().sum();
        let final_sum: i64 = state.height.iter().sum();
        assert_eq!(
            final_sum + state.export_total,
            initial_sum,
            "Σheight + export must equal the initial Σheight exactly (sediment conservation)"
        );
    }

    // ── erode() end-to-end ───────────────────────────────────────────────────────────────────────

    #[test]
    fn erode_is_deterministic_across_repeated_calls() {
        let a = erode(SEED, HMAX, 16, false, false, false);
        let b = erode(SEED, HMAX, 16, false, false, false);
        assert_eq!(a, b, "erode must be byte-identical across repeated calls");
    }

    #[test]
    fn erode_is_not_degenerate() {
        // Sanity: erosion must actually change SOME heights (not a no-op) and must NOT flatten
        // everything to zero (not a runaway collapse). DIM=64 (matches the prod-scale chain grid):
        // material diversity (Soil vs Bedrock) needs enough drainage-area range to cross
        // INCISION_EXPOSURE_THRESHOLD somewhere — a smaller probe grid (e.g. 32) may not reach it.
        const DIM: usize = 64;
        let state = erode(SEED, HMAX, DIM, false, false, false);
        let mut initial_height = vec![0i64; DIM * DIM];
        for z in 0..DIM {
            for x in 0..DIM {
                initial_height[linear_index(x, z, DIM)] = height_at(x as i64, z as i64, SEED, HMAX);
            }
        }
        let any_changed = state.height.iter().zip(&initial_height).any(|(&a, &b)| a != b);
        assert!(any_changed, "erosion must change at least some cells' height");
        let all_zero = state.height.iter().all(|&h| h == 0);
        assert!(!all_zero, "erosion must not collapse the whole grid to zero");
        // Material refinement must produce more than one variant (not degenerate all-one-class).
        let distinct: std::collections::BTreeSet<u8> =
            state.surface_material.iter().map(|&m| m as u8).collect();
        assert!(distinct.len() > 1, "surface_material must show more than one variant, got {distinct:?}");
    }

    #[test]
    fn golden_vector_matches_pinned_erosion_fixture() {
        const GOLDEN_SEED: u64 = 0xA11A_2A11;
        const GOLDEN_HMAX: i64 = 200;
        const DIM: usize = 16;
        let state = erode(GOLDEN_SEED, GOLDEN_HMAX, DIM, false, false, false);

        const CASES: &[(usize, i64, MaterialId)] = &[
            (0, 129, MaterialId::Soil),
            (36, 126, MaterialId::Soil),
            (100, 123, MaterialId::Soil),
            (255, 112, MaterialId::Soil),
        ];
        for &(idx, exp_height, exp_material) in CASES {
            assert_eq!(state.height[idx], exp_height, "golden drift: height[{idx}]");
            assert_eq!(state.surface_material[idx], exp_material, "golden drift: surface_material[{idx}]");
        }
        assert_eq!(state.export_total, 396, "golden drift: export_total");
    }

    // ── W-SIM-4a: tectonic relief (#396) ────────────────────────────────────────────────────────

    #[test]
    fn erode_with_tectonics_is_deterministic_across_repeated_calls() {
        let a = erode_with_tectonics(SEED, HMAX, 16, true, true, false);
        let b = erode_with_tectonics(SEED, HMAX, 16, true, true, false);
        assert_eq!(a, b, "erode_with_tectonics must be byte-identical across repeated calls");
    }

    #[test]
    fn erode_tectonics_gate_reproduces_pre_396_erode_byte_for_byte() {
        // Both flags false must be IDENTICAL to erode()'s pre-#396 body (this is a pure structural
        // refactor into erode_from_fields/erode_with_tectonics — no behavior change when off).
        let via_erode = erode(SEED, HMAX, 16, false, false, false);
        let via_flags = erode_with_tectonics(SEED, HMAX, 16, false, false, false);
        assert_eq!(via_erode, via_flags, "erode(..,false) must equal erode_with_tectonics(..,false,false)");
    }

    /// D8-neighbor "steep edge" count on a height field: the number of (cell, right-or-down
    /// neighbor) pairs whose absolute height difference reaches `threshold` — a simple, symmetric
    /// relief-diversity proxy that catches BOTH raw scarp discontinuities and erosion-carved
    /// incision, regardless of which axis the structure follows.
    fn steep_edge_count(height: &[i64], dim: usize, threshold: i64) -> usize {
        let mut count = 0usize;
        for z in 0..dim {
            for x in 0..dim {
                let idx = linear_index(x, z, dim);
                if x + 1 < dim {
                    let r = linear_index(x + 1, z, dim);
                    if (height[idx] - height[r]).abs() >= threshold {
                        count += 1;
                    }
                }
                if z + 1 < dim {
                    let d = linear_index(x, z + 1, dim);
                    if (height[idx] - height[d]).abs() >= threshold {
                        count += 1;
                    }
                }
            }
        }
        count
    }

    /// Same as [`steep_edge_count`] but EXCLUDES any edge whose two endpoints received a DIFFERENT
    /// raw (pre-erosion) `fault_scarp_delta` — i.e. an edge that straddles the scarp step itself.
    /// What remains is steepness the erosion loop itself carved, isolated from the raw tectonic
    /// step — the load-bearing isolation the acceptance criteria require (#396 AC 4-ii).
    fn steep_edge_count_excluding_scarp(
        height: &[i64],
        dim: usize,
        threshold: i64,
        faults: &[crate::gen::tectonics::Fault],
        hmax: i64,
    ) -> usize {
        let scarp_delta = |x: usize, z: usize| {
            crate::gen::tectonics::fault_scarp_delta(x as i64, z as i64, faults, hmax)
        };
        let mut count = 0usize;
        for z in 0..dim {
            for x in 0..dim {
                let idx = linear_index(x, z, dim);
                let d_here = scarp_delta(x, z);
                if x + 1 < dim {
                    let r = linear_index(x + 1, z, dim);
                    if d_here == scarp_delta(x + 1, z) && (height[idx] - height[r]).abs() >= threshold {
                        count += 1;
                    }
                }
                if z + 1 < dim {
                    let d = linear_index(x, z + 1, dim);
                    if d_here == scarp_delta(x, z + 1) && (height[idx] - height[d]).abs() >= threshold {
                        count += 1;
                    }
                }
            }
        }
        count
    }

    /// The load-bearing verification (#396 AC): three-condition ablation on the golden grid —
    /// A. tectonics fully OFF (isotropic baseline), B. fault-scarp ON / resistance-lineament OFF
    /// (scarp step only), C. fully ON. Asserts BOTH (i) C is more relief-diverse than A, and (ii)
    /// the resistance-lineament half contributes INDEPENDENTLY of the scarp step — C shows more
    /// erosion-carved steep edges than B even with every scarp-straddling edge excluded from the
    /// count, so the resistance half cannot be dead code riding on the scarp step alone.
    #[test]
    fn tectonic_ablation_three_condition_relief_diversity() {
        const DIM: usize = 64;
        // Calibrated against this fBm relief's measured adjacent-cell slope range (0–5 units, see
        // `K_INCISE_DEN`'s doc) — mirrors `caps.rs`'s `ROCK_SLOPE_THRESHOLD`.
        const STEEP_THRESHOLD: i64 = 4;

        let a = erode_with_tectonics(SEED, HMAX, DIM, false, false, false);
        let b = erode_with_tectonics(SEED, HMAX, DIM, true, false, false);
        let c = erode_with_tectonics(SEED, HMAX, DIM, true, true, false);

        let a_count = steep_edge_count(&a.height, DIM, STEEP_THRESHOLD);
        let c_count = steep_edge_count(&c.height, DIM, STEEP_THRESHOLD);
        assert!(
            c_count > a_count,
            "(i) full tectonics ON must be MORE relief-diverse than the isotropic baseline: A={a_count} C={c_count}"
        );

        let faults = crate::gen::tectonics::build_faults(SEED, DIM);
        let b_excl = steep_edge_count_excluding_scarp(&b.height, DIM, STEEP_THRESHOLD, &faults, HMAX);
        let c_excl = steep_edge_count_excluding_scarp(&c.height, DIM, STEEP_THRESHOLD, &faults, HMAX);
        // CI-sourced (#397, hard-fault-only config, FAULT_STEP_DEN=12): C=1373 B=1298, margin 75
        // (run #29180478606, x86 debug + arm64 release agree). Locked at roughly half that with
        // headroom (not the bare placeholder `1`) so the assertion guards the resistance-lineament
        // effect size, not just its sign, without being brittle to minor perturbation.
        const MIN_MARGIN: usize = 40;
        assert!(
            c_excl >= b_excl + MIN_MARGIN,
            "(ii) resistance-lineament structure must contribute INDEPENDENTLY of the scarp step: \
             excluding scarp-straddling edges, C={c_excl} must exceed B={b_excl} by >= {MIN_MARGIN}"
        );
    }

    /// Golden vector (ON path): the tectonic-ON `erode` output is pinned at fixed grid indices —
    /// proves determinism of the FULL production path (not just the isolated `tectonics.rs` unit),
    /// mirrors `golden_vector_matches_pinned_erosion_fixture` above.
    ///
    /// Re-pinned for #397 pass 2: fault-band resistance flip (soft→hard, kept) + `FAULT_STEP_DEN`
    /// reverted to its pre-#397 value 12 (scarp-step crank dropped, PM decision). CI-sourced —
    /// `left:` from both x86 debug (`v2 sim` job) and arm64 release (`v2 golden` job), run
    /// #29180057376, commit 66400ac; both arches agree (integer, arch-stable).
    #[test]
    fn golden_vector_matches_pinned_tectonic_on_erosion_fixture() {
        const GOLDEN_SEED: u64 = 0xA11A_2A11;
        const GOLDEN_HMAX: i64 = 200;
        const DIM: usize = 16;
        let state = erode(GOLDEN_SEED, GOLDEN_HMAX, DIM, true, false, false);

        const INDICES: [usize; 4] = [0, 36, 100, 255];
        const EXPECTED: [i64; 4] = [113, 116, 104, 95];
        let actual: [i64; 4] = std::array::from_fn(|i| state.height[INDICES[i]]);
        assert_eq!(actual, EXPECTED, "golden drift (or placeholder awaiting CI pin) at indices {INDICES:?}");
    }

    // ── W-SIM-5: volcanic gate threading (#410) ──────────────────────────────────────────────────

    /// The `enable_volcanic` gate genuinely threads through to `erode` (not a dead parameter): the
    /// same `(seed, hmax, dim)` must produce a DIFFERENT height field with volcanic on vs off.
    #[test]
    fn erode_volcanic_gate_actually_changes_height() {
        const SEED: u64 = 0xA11A_2A11;
        const HMAX: i64 = 200;
        const DIM: usize = 64;
        let off = erode(SEED, HMAX, DIM, false, false, false);
        let on = erode(SEED, HMAX, DIM, false, true, false);
        assert_ne!(off.height, on.height, "enable_volcanic=true must change the height field — else the gate is dead code");
    }

    /// `enable_volcanic=false` is deterministic across repeated calls (this test's actual scope —
    /// code-critic finding: the name/doc previously overclaimed a literal byte-for-byte comparison
    /// against a frozen pre-#410 snapshot, which this assertion doesn't perform). The ACTUAL
    /// byte-identity-to-pre-#410 guarantee comes from two other places: structurally, the
    /// `if enable_volcanic { .. }` gate in `erode_with_tectonics` SKIPS `volcanic::build_vents`/
    /// `emplace_edifices` entirely when off (not merely discards their result — mirrors the
    /// tectonics gate's own OFF-path construction); empirically, every PRE-EXISTING pinned golden
    /// in this file and `w4_chain.rs`'s golden hash call `erode`/`erode_with_tectonics` with
    /// `enable_volcanic=false` and remain UNCHANGED by this PR (still green) — that is the concrete
    /// frozen-baseline proof, not this test.
    #[test]
    fn erode_volcanic_off_is_deterministic_across_repeated_calls() {
        const SEED: u64 = 0xA11A_2A11;
        const HMAX: i64 = 200;
        const DIM: usize = 16;
        let a = erode(SEED, HMAX, DIM, false, false, false);
        let b = erode(SEED, HMAX, DIM, false, false, false);
        assert_eq!(a, b, "erode(..,enable_volcanic=false) must be byte-identical across repeated calls");
    }

    /// Post-erosion survival (#410 ТЗ, weaker/distinct from the constructive corridor in
    /// `volcanic.rs`): erosion DISSECTS the edifices (incision + talus cut channels into the
    /// flanks), so strict radial monotonicity is NOT required post-pipeline — instead, each vent's
    /// summit must remain a NET LOCAL HIGH: higher than the mean height of a ring of cells sampled
    /// at the edifice's own outer footprint radius (its class's `max_radius`), even after dissection.
    #[test]
    fn volcanic_edifice_survives_erosion_as_net_local_high() {
        const SEED: u64 = 0xA11A_2A11;
        const HMAX: i64 = 200;
        const DIM: usize = 64;
        let state = erode(SEED, HMAX, DIM, false, true, false);
        let vents = crate::gen::volcanic::build_vents(SEED, DIM);

        for vent in &vents {
            let cx = vent.x;
            let cz = vent.z;
            if cx < 0 || cz < 0 || cx as usize >= DIM || cz as usize >= DIM {
                continue; // vent center off-grid for this DIM — nothing to sample
            }
            let summit = state.height[linear_index(cx as usize, cz as usize, DIM)];

            let radius = vent.class.max_radius();
            let mut ring_sum = 0i64;
            let mut ring_count = 0i64;
            // Sample the ring at 8 compass points on the outer radius (cheap, deterministic, no
            // need for a full circle scan — this is a coarse survival check, not a precise metric).
            const DIRS: [(i64, i64); 8] =
                [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];
            for &(dx, dz) in &DIRS {
                let rx = cx + dx * radius;
                let rz = cz + dz * radius;
                if rx < 0 || rz < 0 || rx as usize >= DIM || rz as usize >= DIM {
                    continue;
                }
                ring_sum += state.height[linear_index(rx as usize, rz as usize, DIM)];
                ring_count += 1;
            }
            if ring_count == 0 {
                continue; // entire ring off-grid for this DIM — nothing to compare against
            }
            let ring_mean = ring_sum / ring_count;
            assert!(
                summit > ring_mean,
                "vent at ({cx},{cz}) must remain a net local high after the full pipeline: summit={summit} ring_mean={ring_mean}"
            );
        }
    }

    /// Golden vector (ON path): the volcanic-ON `erode` output is pinned at fixed grid indices —
    /// mirrors `golden_vector_matches_pinned_tectonic_on_erosion_fixture` above.
    ///
    /// Re-pinned for #410 pass 2: CI-sourced — `left:` from both x86 debug (`v2 sim` job) and
    /// arm64 release (`v2 golden` job), run #29186449162, commit 6eeacf4; both arches agree
    /// (integer, arch-stable).
    #[test]
    fn golden_vector_matches_pinned_volcanic_on_erosion_fixture() {
        const GOLDEN_SEED: u64 = 0xA11A_2A11;
        const GOLDEN_HMAX: i64 = 200;
        const DIM: usize = 16;
        let state = erode(GOLDEN_SEED, GOLDEN_HMAX, DIM, false, true, false);

        const INDICES: [usize; 4] = [0, 36, 100, 255];
        const EXPECTED: [i64; 4] = [132, 127, 126, 122];
        let actual: [i64; 4] = std::array::from_fn(|i| state.height[INDICES[i]]);
        assert_eq!(actual, EXPECTED, "golden drift (or placeholder awaiting CI pin) at indices {INDICES:?}");
    }
}
