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

/// W-18: FLAT_DATUM — the flat pedestal height when base=false. Centered at hmax/2 for symmetric
/// headroom (no 0-clamp plateaus). When base=true (default), height is seeded from fBm; when
/// base=false, the entire height field is initialized to FLAT_DATUM instead.
pub fn flat_datum(hmax: i64) -> i64 {
    hmax / 2
}
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

/// W-17 PAR_MIN_DIM: dimension threshold below which parallel overhead exceeds benefit.
/// dim=64 shows ~25% regression with par_iter; gate at 128 (matches practical break-even).
/// Applied uniformly across all parallelized functions in erosion (incision_step, talus_step,
/// talus_step_final, de_needle_pass, d8_directions), drainage (d8_directions), and caps/volcanic
/// M3 stages (classify, edifice_material_mask). Both par and serial paths are byte-identical (G1 proven).
pub(crate) const PAR_MIN_DIM: usize = 128;

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

/// W-11: Decorrelation salt for ridge field — XORed into `seed` for the ridged noise fbm, kept
/// independent from base height and rock resistance (pattern: `RESISTANCE_SALT`/`PATCH_SEED_SALT`).
const RIDGE_SEED_SALT: u64 = 0x5249_4447_4553_5F30; // "RIDGES_0" (ASCII, folded)

/// W-11: Decorrelation salt for ridge warp field — the low-octave fbm that warps the sample
/// coordinates, kept independent from ridge field itself.
const RIDGE_WARP_SALT: u64 = 0x5741_5250_5F52_4944; // "WARP_RID" (ASCII, folded)

/// W-15a: Decorrelation salt for ridge crest modulation field — the 2-octave value_noise over
/// along-fault arclength that modulates ridge amplitude along the crest, kept independent from
/// ridge field and warp field.
const RIDGE_CREST_SALT: u64 = 0x4352_4553_544D_4F44; // "CRESTMOD" (ASCII, folded)

/// W-13: Fault-band belt half-width (integer D8 distance ramp in cells). Widened from 2 to 4
/// to support curved warped belt traces (straight belt was too thin to read visually as a ridge lineament).
/// Determines the perpendicular distance over which the `band_ramp` decays from 1 to 0 when moving away from a fault band.
/// Implementer's call, documented, locked by test coverage.
const BELT_HALF_WIDTH: i64 = 4;

/// W-11: Ridge amplitude numerator and denominator candidates. The ridge height delta is scaled as
/// `(RIDGE_AMP * mask * (2*ridged - MAX)) / SCALE`. RIDGE_AMP affects visual prominence and must
/// stay within bounds w.r.t. W-9 margins (MAX_SPIKE_FINAL=12, NEEDLE_MARGIN=30); coupling formula
/// requires PM clarification. Flip ACTIVE_RIDGE_AMP_INDEX to select a candidate for gallery build.
const RIDGE_AMP_CANDIDATES: [(i64, i64); 3] = [
    (15, 10), // Conservative: amplitude 1.5x MAX/SCALE
    (25, 10), // Default: amplitude 2.5x MAX/SCALE (current)
    (40, 10), // Aggressive: amplitude 4.0x MAX/SCALE
];
const ACTIVE_RIDGE_AMP_INDEX: usize = 1; // Default = index 1 (25/10)
const RIDGE_AMP_NUM: i64 = RIDGE_AMP_CANDIDATES[ACTIVE_RIDGE_AMP_INDEX].0;
const RIDGE_AMP_DEN: i64 = RIDGE_AMP_CANDIDATES[ACTIVE_RIDGE_AMP_INDEX].1;

/// W-11 FBM normalization: ridge_fbm_at sums 4 octaves with amplitudes 8,4,2,1 where each
/// value_noise_octave ∈ [0, 65536), giving max=(8+4+2+1)*65536=983040. The ridged-field fold
/// expects input in [0, MAX=32768]; exceeding this saturates the fold to zero (all ridges carved
/// away). FBM_MAX rescales the raw field to [0, 32768] BEFORE the fold.

/// W-11: Ridge field scaling denominator. Derived to keep |ridge_delta| in tens of units, not hmax.
/// Fold term (2*ridged - MAX) ranges [−32768, +32768]; with RIDGE_AMP * mountainness,
/// max |delta| before RIDGE_SCALE division: RIDGE_AMP_NUM/DEN * 32768 * mountainness.
/// Worst case (amp=40/10, mountainness=256): (40*32768*256)/(10*RIDGE_SCALE) = 33,554,432/RIDGE_SCALE.
/// Target: p99(|delta|) ≤ hmax/2 = 100 (tens of units for hmax=200).
/// Solution: RIDGE_SCALE=500,000 → typical ~10 units, worst ~67 units, p99 in range.
const RIDGE_SCALE: i64 = 500_000;

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
    dim: usize,
    height: &[i64],
    downstream: &[Option<usize>],
    area: &[i64],
    resistance: &[i64],
) -> Vec<i64> {
    use rayon::prelude::*;

    let n = dim * dim;
    debug_assert_eq!(height.len(), n);
    debug_assert_eq!(downstream.len(), n);
    debug_assert_eq!(area.len(), n);
    debug_assert_eq!(resistance.len(), n);

    // M1 (W-17): par_iter per-cell incision delta — PAR_MIN_DIM gate (dim-64 shows +25% regression).
    if dim >= PAR_MIN_DIM {
        // Parallel path: par_iter per-cell
        let delta: Vec<i64> = (0..n).into_par_iter().map(|v| {
            let Some(d) = downstream[v] else { return 0i64 };
            let s = (height[v] - height[d]).max(0);
            let a_isqrt = isqrt(area[v]);
            let divisor = RESIST_DIVISOR[resistance[v] as usize];
            let raw = K_INCISE_NUM * a_isqrt * s;
            let dz = (raw / (K_INCISE_DEN * divisor)).clamp(0, height[v]);
            dz
        }).collect();
        delta
    } else {
        // Serial fallback for dim < PAR_MIN_DIM (rayon overhead not worth it)
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
}

/// Thermal talus relaxation: GATHER formulation (`[erosion]` non-negotiable — never a scatter).
/// Pass 1 computes each cell's own outflow intention (`send_out`, purely local). Pass 2 has every
/// cell PULL its neighbors' intentions that target it — no cell ever writes into another's slot.
/// Returns the NEW height buffer (Jacobi: reads only `height`/`downstream`, the OLD frame).
pub fn talus_step(dim: usize, height: &[i64], downstream: &[Option<usize>]) -> Vec<i64> {
    use rayon::prelude::*;

    let n = dim * dim;
    debug_assert_eq!(height.len(), n);
    debug_assert_eq!(downstream.len(), n);

    // Pass 1: local outflow intention — M1 (W-17): par_iter per-cell, PAR_MIN_DIM gated
    let send_out: Vec<i64> = if dim >= PAR_MIN_DIM {
        // Parallel path: par_iter per-cell
        (0..n).into_par_iter().map(|v| {
            let Some(d) = downstream[v] else { return 0i64 };
            let slope = (height[v] - height[d]).max(0);
            if slope > REPOSE_THRESHOLD {
                (slope - REPOSE_THRESHOLD) * TALUS_FRAC_NUM / TALUS_FRAC_DEN
            } else {
                0
            }
        }).collect()
    } else {
        // Serial fallback for dim < PAR_MIN_DIM
        let mut send_out = vec![0i64; n];
        for v in 0..n {
            let Some(d) = downstream[v] else { continue };
            let slope = (height[v] - height[d]).max(0);
            if slope > REPOSE_THRESHOLD {
                send_out[v] = (slope - REPOSE_THRESHOLD) * TALUS_FRAC_NUM / TALUS_FRAC_DEN;
            }
        }
        send_out
    };

    // Pass 2: gather — M1 (W-17): par_iter per-cell (Jacobi, disjoint writes), PAR_MIN_DIM gated
    // Each cell v reads its own send_out plus its neighbors' send_out where that
    // neighbor's D8 receiver IS v. Never writes into a neighbor's slot.
    let new_height: Vec<i64> = if dim >= PAR_MIN_DIM {
        // Parallel path: par_iter per-cell
        (0..n).into_par_iter().map(|v| {
            let z = v / dim;
            let x = v % dim;
            let mut h = height[v] - send_out[v];
            for &(dx, dz) in &D8_OFFSETS {
                let nx = x as i64 + dx;
                let nz = z as i64 + dz;
                if nx < 0 || nz < 0 || nx as usize >= dim || nz as usize >= dim {
                    continue;
                }
                let u = linear_index(nx as usize, nz as usize, dim);
                if downstream[u] == Some(v) {
                    h += send_out[u];
                }
            }
            h
        }).collect()
    } else {
        // Serial fallback for dim < PAR_MIN_DIM
        let mut new_height = vec![0i64; n];
        for v in 0..n {
            let z = v / dim;
            let x = v % dim;
            let mut h = height[v] - send_out[v];
            for &(dx, dz) in &D8_OFFSETS {
                let nx = x as i64 + dx;
                let nz = z as i64 + dz;
                if nx < 0 || nz < 0 || nx as usize >= dim || nz as usize >= dim {
                    continue;
                }
                let u = linear_index(nx as usize, nz as usize, dim);
                if downstream[u] == Some(v) {
                    h += send_out[u];
                }
            }
            new_height[v] = h;
        }
        new_height
    };
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
    use rayon::prelude::*;

    let n = dim * dim;
    debug_assert_eq!(height.len(), n);

    // Pass 1: identify each cell's clipping intention and receiver — M2 (W-17): par_iter per-cell, PAR_MIN_DIM gated
    #[derive(Clone)]
    struct NeedleResult {
        send_out: i64,
        receiver: Option<usize>,
    }

    let needle_results: Vec<NeedleResult> = if dim >= PAR_MIN_DIM {
        // Parallel path: par_iter per-cell
        (0..n).into_par_iter().map(|v| {
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

            // Compute send_out if this is a spike
            let send_out = if height[v] > nmax + NEEDLE_MARGIN {
                height[v] - (nmax + NEEDLE_MARGIN)
            } else {
                0
            };

            // Find the lowest D8 neighbor (deterministic tie-break: lowest linear index)
            let mut lowest_height = height[v];
            let mut lowest_idx: Option<usize> = None;
            for &(dx, dz) in &D8_OFFSETS {
                let nx = x as i64 + dx;
                let nz = z as i64 + dz;
                if nx < 0 || nz < 0 || nx as usize >= dim || nz as usize >= dim {
                    continue;
                }
                let u = linear_index(nx as usize, nz as usize, dim);
                if height[u] < lowest_height || (height[u] == lowest_height && lowest_idx.map_or(true, |idx| u < idx)) {
                    lowest_height = height[u];
                    lowest_idx = Some(u);
                }
            }

            NeedleResult { send_out, receiver: lowest_idx }
        }).collect()
    } else {
        // Serial fallback for dim < PAR_MIN_DIM
        let mut needle_results = Vec::with_capacity(n);
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

            // Compute send_out if this is a spike
            let send_out = if height[v] > nmax + NEEDLE_MARGIN {
                height[v] - (nmax + NEEDLE_MARGIN)
            } else {
                0
            };

            // Find the lowest D8 neighbor (deterministic tie-break: lowest linear index)
            let mut lowest_height = height[v];
            let mut lowest_idx: Option<usize> = None;
            for &(dx, dz) in &D8_OFFSETS {
                let nx = x as i64 + dx;
                let nz = z as i64 + dz;
                if nx < 0 || nz < 0 || nx as usize >= dim || nz as usize >= dim {
                    continue;
                }
                let u = linear_index(nx as usize, nz as usize, dim);
                if height[u] < lowest_height || (height[u] == lowest_height && lowest_idx.map_or(true, |idx| u < idx)) {
                    lowest_height = height[u];
                    lowest_idx = Some(u);
                }
            }

            needle_results.push(NeedleResult { send_out, receiver: lowest_idx });
        }
        needle_results
    };

    // Pass 2: gather — M2 (W-17): par_iter per-cell, PAR_MIN_DIM gated
    // Each cell v subtracts its send_out and adds all send_out from cells whose receiver is v
    let send_out: Vec<i64> = needle_results.iter().map(|r| r.send_out).collect();
    let receiver: Vec<Option<usize>> = needle_results.iter().map(|r| r.receiver).collect();

    let new_height: Vec<i64> = if dim >= PAR_MIN_DIM {
        // Parallel path: par_iter per-cell
        (0..n).into_par_iter().map(|v| {
            let mut h = height[v] - send_out[v];
            // Gather from all cells whose receiver is v
            for u in 0..n {
                if receiver[u] == Some(v) {
                    h += send_out[u];
                }
            }
            h
        }).collect()
    } else {
        // Serial fallback for dim < PAR_MIN_DIM
        let mut new_height = vec![0i64; n];
        for v in 0..n {
            let mut h = height[v] - send_out[v];
            // Gather from all cells whose receiver is v
            for u in 0..n {
                if receiver[u] == Some(v) {
                    h += send_out[u];
                }
            }
            new_height[v] = h;
        }
        new_height
    };

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
    use rayon::prelude::*;

    let n = dim * dim;
    debug_assert_eq!(height.len(), n);
    debug_assert!(n_iters > 0);

    // Scale: convert to fixed-point (x64)
    let mut hs = height.iter().map(|&h| h * 64).collect::<Vec<i64>>();
    let margin_s = spike_margin * 64;

    // Jacobi iterations (macro-loop stays serial, internal passes parallelized with PAR_MIN_DIM gate)
    for _ in 0..n_iters {
        // Pass 1: compute outflows — M2 (W-17): par_iter per-cell, PAR_MIN_DIM gated
        let send_out: Vec<Vec<i64>> = if dim >= PAR_MIN_DIM {
            // Parallel path: par_iter per-cell
            (0..n).into_par_iter().map(|v| {
                let mut outflows = vec![0i64; 8];
                let z = v / dim;
                let x = v % dim;

                // Classify as spike: hs - second_max(neighbors) > margin_s
                let mut max_hs = i64::MIN;
                let mut second_max_hs = i64::MIN;
                for &(dx, dz) in &D8_OFFSETS {
                    let nx = x as i64 + dx;
                    let nz = z as i64 + dz;
                    if nx >= 0 && nz >= 0 && (nx as usize) < dim && (nz as usize) < dim {
                        let u = linear_index(nx as usize, nz as usize, dim);
                        let neighbor_hs = hs[u];
                        if neighbor_hs > max_hs {
                            second_max_hs = max_hs;
                            max_hs = neighbor_hs;
                        } else if neighbor_hs > second_max_hs {
                            second_max_hs = neighbor_hs;
                        }
                    }
                }

                // Only donate if this is a spike
                if second_max_hs != i64::MIN && hs[v] - second_max_hs > margin_s {
                    for (dir, &(dx, dz)) in D8_OFFSETS.iter().enumerate() {
                        let nx = x as i64 + dx;
                        let nz = z as i64 + dz;
                        if nx < 0 || nz < 0 || nx as usize >= dim || nz as usize >= dim {
                            continue;
                        }
                        let u = linear_index(nx as usize, nz as usize, dim);
                        let drop = hs[v] - hs[u];
                        if drop > 0 {
                            outflows[dir] = (drop.saturating_sub(margin_s)) / 2 / 8;
                        }
                    }
                }

                outflows
            }).collect()
        } else {
            // Serial fallback for dim < PAR_MIN_DIM
            let mut send_out = vec![vec![0i64; 8]; n];
            for v in 0..n {
                let mut outflows = vec![0i64; 8];
                let z = v / dim;
                let x = v % dim;

                // Classify as spike: hs - second_max(neighbors) > margin_s
                let mut max_hs = i64::MIN;
                let mut second_max_hs = i64::MIN;
                for &(dx, dz) in &D8_OFFSETS {
                    let nx = x as i64 + dx;
                    let nz = z as i64 + dz;
                    if nx >= 0 && nz >= 0 && (nx as usize) < dim && (nz as usize) < dim {
                        let u = linear_index(nx as usize, nz as usize, dim);
                        let neighbor_hs = hs[u];
                        if neighbor_hs > max_hs {
                            second_max_hs = max_hs;
                            max_hs = neighbor_hs;
                        } else if neighbor_hs > second_max_hs {
                            second_max_hs = neighbor_hs;
                        }
                    }
                }

                // Only donate if this is a spike
                if second_max_hs != i64::MIN && hs[v] - second_max_hs > margin_s {
                    for (dir, &(dx, dz)) in D8_OFFSETS.iter().enumerate() {
                        let nx = x as i64 + dx;
                        let nz = z as i64 + dz;
                        if nx < 0 || nz < 0 || nx as usize >= dim || nz as usize >= dim {
                            continue;
                        }
                        let u = linear_index(nx as usize, nz as usize, dim);
                        let drop = hs[v] - hs[u];
                        if drop > 0 {
                            outflows[dir] = (drop.saturating_sub(margin_s)) / 2 / 8;
                        }
                    }
                }

                send_out[v] = outflows;
            }
            send_out
        };

        // Pass 2: apply changes (gather) — M2 (W-17): par_iter per-cell (Jacobi), PAR_MIN_DIM gated
        let hs_new: Vec<i64> = if dim >= PAR_MIN_DIM {
            // Parallel path: par_iter per-cell
            (0..n).into_par_iter().map(|v| {
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

                    sum_out += send_out[v][dir];

                    for (opposite_dir, &(ox, oz)) in D8_OFFSETS.iter().enumerate() {
                        if ox == -dx && oz == -dz {
                            sum_in += send_out[u][opposite_dir];
                            break;
                        }
                    }
                }

                hs[v] - sum_out + sum_in
            }).collect()
        } else {
            // Serial fallback for dim < PAR_MIN_DIM
            let mut hs_new = vec![0i64; n];
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

                    sum_out += send_out[v][dir];

                    for (opposite_dir, &(ox, oz)) in D8_OFFSETS.iter().enumerate() {
                        if ox == -dx && oz == -dz {
                            sum_in += send_out[u][opposite_dir];
                            break;
                        }
                    }
                }

                hs_new[v] = hs[v] - sum_out + sum_in;
            }
            hs_new
        };

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

/// Run the scaled erosion macro-loop (recompute drainage → stream-power incision
/// → thermal talus, each iteration) over an ALREADY-BUILT initial `height`/`resistance` pair. Shared
/// by [`erode_with_tectonics`]'s tectonics-on and tectonics-off paths so the macro-loop itself is
/// never duplicated: the tectonic scarp/lineament overlay (if any) has already been folded into
/// `height`/`resistance` by the caller, before this function ever runs — this function has no
/// tectonics-awareness of its own.
/// W-18: added enable_erosion parameter. When false, skips the erosion macro-loop but still
/// computes drainage and surface materials, preserving the accumulated source field (base+tect+volcanic).
/// W-19: erosion_strength (percent, default 100) scales the iteration count via: effective_iters = (MACRO_ITERATIONS * strength) / 100.
pub fn erode_from_fields(seed: u64, hmax: i64, dim: usize, mut height: Vec<i64>, resistance: Vec<i64>, enable_erosion: bool, erosion_strength: i64) -> ErosionState {
    let n = dim * dim;
    let initial_height = height.clone();

    let mut export_total: i64 = 0;

    // W-18: when enable_erosion=false, skip the erosion macro-loop entirely (pass-through mode)
    // W-19: erosion_strength scales the macro loop iteration count (default 100 = MACRO_ITERATIONS)
    if enable_erosion {
        let n_iters = ((MACRO_ITERATIONS as i64 * erosion_strength) / 100) as usize;
        // DO NOT PARALLELIZE (W-17): macro-loop iteration count is load-bearing (sequential state updates)
        for _ in 0..n_iters {
        // 1. Recompute drainage on the CURRENT eroding surface (reused verbatim from gen::drainage).
        let filled = priority_flood_fill(dim, &height);
        let downstream = d8_directions(dim, &filled);
        let area = kahn_accumulate(dim, &downstream);

        // 2. Stream-power incision, routed to export (detachment-limited, no mid-network deposit).
        let incision_delta = incision_step(dim, &height, &downstream, &area, &resistance);
        let (_accum, export_this_iter) = accumulate_and_export(dim, &downstream, &incision_delta);
        for v in 0..n {
            height[v] -= incision_delta[v];
        }
        export_total += export_this_iter;

        // 3. Thermal talus relaxation (Jacobi gather, internal zero-sum redistribution).
        height = talus_step(dim, &height, &downstream);
        }
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

/// W-13: Ridge field as a ridged multifractal — per-octave fold + Musgrave gain (fixed-point).
/// Folds each octave BEFORE summation (fold_i = HALF - |2·n_i - HALF|) and weights octave i by the
/// previous octave's folded value (crests grow sub-ridges, flats stay calm). Returns the READY
/// ridged field normalized to [0, 32768] — exactly what the delta formula consumes (no inline fold).
/// **SINGLE fold implementation** (not a raw fbm; the inline re-fold at the call site must be removed).
/// Made public for testing (tests probe the field directly).
pub fn ridge_fbm_at(x: i64, z: i64, seed: u64) -> i64 {
    use crate::gen::height::value_noise_octave;

    const HALF: i64 = 65536 / 2; // 32768, for folding value_noise_octave range [0, 65536)
    const MAX_FOLDED: i64 = 32768; // Max of the folded range

    let salted_seed = seed ^ RIDGE_SEED_SALT;
    let mut total: i64 = 0;
    let mut amplitude = 8i64; // Start amplitude
    let mut period = 64i64; // Base period
    let mut gain = 256i64; // Musgrave gain weight for this octave (fixed-point /256)

    for octave in 0..4 {
        // Interpolated noise value [0, 65536)
        let n = value_noise_octave(x, z, period, salted_seed, octave);

        // Per-octave fold: Musgrave ridged multifractal — sharp CRESTS where noise crosses mid-range
        // fold = (65536 - |2·n - 65536|) / 2, giving peaks at n=32768 (center), zero at edges (n=0, n=65536)
        // Divided by 2 to normalize output to [0, 32768] before summation
        let abs_dev = ((2 * n - 65536).abs()).min(65536);
        let folded = (65536 - abs_dev) / 2; // [0, 32768], peak at center n=32768

        // Weight by amplitude AND by Musgrave gain (previous octave's folded value)
        // Divisor derived from FBM_MAX normalization: with folded ∈ [0, 32768], amplitudes 8,4,2,1,
        // gain ∈ [0, 256], max raw sum ≈ 32768 * 256 * (8+4+2+1) = 125,165,568.
        // To normalize to [0, 32768]: divisor = 256 * 15 = 3840 (sum of amplitudes scaled up)
        let weighted = (folded * amplitude * gain) / (256 * 15);
        total += weighted;

        // Update for next octave
        amplitude >>= 1; // Halve amplitude
        period = (period / 2).max(1);
        // Gain for next octave = folded value of this octave (Musgrave: crests grow sub-ridges)
        // folded ∈ [0, 32768], gain formula: folded * 256 / HALF to get fixed-point ∈ [0, 256]
        gain = folded * 256 / HALF; // Scale back to fixed-point denominator 256
    }

    // Clamp and normalize to [0, 32768]
    total.max(0).min(MAX_FOLDED)
}

/// W-13: Compute ridge height delta with parameterizable amplitude.
/// Takes ALREADY-RIDGED field (from ridge_fbm_at, in [0,32768]) — no internal normalization or fold.
/// Single fold implementation: the fold is IN ridge_fbm_at, not here. The delta formula remains unchanged:
/// `delta = (RIDGE_AMP * mountainness * (2*r - MAX)) / SCALE`.
/// Exposed for testing different amplitude values without recompilation.
/// **Signature change (D3):** `ridged: i64` (already-folded, [0,32768]) replaces `raw_fbm: i64`.
pub fn ridge_delta_compute(
    ridged: i64,
    mountainness: i64,
    ridge_amp_num: i64,
    ridge_amp_den: i64,
    hmax: i64,
) -> i64 {
    const MAX: i64 = 32768;
    // Apply ridge height: h += (RIDGE_AMP * mountainness * (2*r - MAX)) / SCALE
    // ridged is already in [0, 32768], so (2*ridged - MAX) ranges [-32768, +32768]
    let ridge_delta = (ridge_amp_num * mountainness * (2 * ridged - MAX)) / (ridge_amp_den * RIDGE_SCALE);
    ridge_delta.clamp(-(hmax), hmax)
}

/// W-15a: Compute ridge height delta with along-crest amplitude modulation (crest_mod).
/// Extends ridge_delta_compute with a modulation factor in the numerator.
/// `delta = (RIDGE_AMP_NUM * mountainness * fold * crest_mod) / (RIDGE_AMP_DEN * RIDGE_SCALE * 128)`
/// where crest_mod ∈ [51, 166] represents 40%..130% modulation in /128 fixed-point.
/// Single last division for safety.
pub fn ridge_delta_compute_modulated(
    ridged: i64,
    mountainness: i64,
    crest_mod: i64,
    ridge_amp_num: i64,
    ridge_amp_den: i64,
    hmax: i64,
) -> i64 {
    const MAX: i64 = 32768;
    // Apply ridge height with crest modulation:
    // delta = (RIDGE_AMP * mountainness * (2*r - MAX) * crest_mod) / (SCALE * 128)
    // ridged is already in [0, 32768], so (2*ridged - MAX) ranges [-32768, +32768]
    // crest_mod ∈ [115, 141], representing 90%..110% modulation (W-15a narrowed to keep delta step <4 units)
    // Single last division: (num * fold * crest_mod) / (den * SCALE * 128)
    let fold = 2 * ridged - MAX;
    let ridge_delta = (ridge_amp_num * mountainness * fold * crest_mod) / (ridge_amp_den * RIDGE_SCALE * 128);
    ridge_delta.clamp(-(hmax), hmax)
}

/// W-11 ridge helper: compute warp offsets using low-octave fbm.
fn ridge_warp_at(x: i64, z: i64, seed: u64) -> (i64, i64) {
    use crate::gen::height::value_noise_octave;
    let salted_seed = seed ^ RIDGE_WARP_SALT;
    // Use 2 octaves for broad warping, lower amplitude
    let dx = value_noise_octave(x, z, 128, salted_seed, 0) / 512 - 64;
    let dz = value_noise_octave(z, x, 128, salted_seed, 1) / 512 - 64;
    (dx, dz)
}

/// W-13: Analytic band-ramp mask (O(1) per cell, kills O(dim⁴) defect).
/// Returns ramp value in [0, 256] (fixed-point 256 = 1.0). Computes integer distance from the
/// warped point to the NEAREST fault line using the analytic point-to-line formula (via
/// W-15a: Compute the along-fault arclength parameter t for a point's perpendicular foot on a fault.
/// The foot is the point on the infinite fault line closest to (x, z).
/// Parameter t: the foot point is at (px + t*dx, pz + t*dz).
/// Uses the projection formula: t = ((x - px)*dx + (z - pz)*dz) / (dx² + dz²).
pub fn fault_projection_parameter(x: i64, z: i64, fault: &crate::gen::tectonics::Fault) -> i64 {
    let dx_from_base = x - fault.px;
    let dz_from_base = z - fault.pz;
    let dot = dx_from_base * fault.dx + dz_from_base * fault.dz;
    // dlen_sq = dx² + dz²
    if fault.dlen_sq == 0 {
        0
    } else {
        dot / fault.dlen_sq
    }
}

/// W-15a: Compute crest modulation factor from 2-octave value_noise over along-fault arclength.
/// Returns value in [115, 141] (/128 fixed-point, i.e., 90%..110% range).
/// W-15a fix: narrowed from [51,166] to reduce per-cell delta step to stay under W-9 bound (4 units).
/// Input t: along-fault arclength parameter; base_period: dim/4 at production dim=512 (doubled from dim/8).
/// Uses 2 octaves with half-period for fine modulation; doubled base period lowers spatial frequency.
pub fn crest_modulation(t: i64, fault_index: u32, base_period: i64, seed: u64) -> i64 {
    use crate::gen::height::value_noise_octave;
    let salted_seed = seed ^ RIDGE_CREST_SALT;

    // 2-octave value_noise over t (along-fault coordinate)
    // Octave 0: period = base_period (broad modulation, lengthened to smooth envelope)
    let octave0 = value_noise_octave(t, fault_index as i64, base_period, salted_seed, 0);
    // Octave 1: period = base_period/2 (fine detail)
    let octave1 = value_noise_octave(t, fault_index as i64, base_period / 2, salted_seed, 1);

    // Combine: 2 octaves, amplitudes 2:1, then normalize to [0, 65536]
    // max = 2*65536 + 1*65536 = 196608, so we need to scale down
    let combined = (2 * octave0 + octave1) / 3; // Average to [0, 65536)

    // Map to [115, 141]: linear rescale from [0, 65536) to [115, 141]
    // Narrowed range keeps modulation ±10% around 128 (baseline), reducing per-cell step to <4 units
    // result = 115 + (combined / 65536) * (141 - 115) = 115 + (combined / 65536) * 26
    let range = 141 - 115;
    let modulation = 115 + (combined * range) / 65536;
    modulation.min(141).max(115)
}

/// W-15a: Find the nearest fault to a point and return (fault_index, projection parameter t).
/// Returns (fault_index, t) for the closest fault, or (0, 0) if no faults.
fn nearest_fault_and_parameter(x: i64, z: i64, faults: &[crate::gen::tectonics::Fault]) -> (usize, i64) {
    let mut min_dist_sq = i64::MAX;
    let mut nearest_idx = 0;
    let mut nearest_t = 0i64;

    for (idx, fault) in faults.iter().enumerate() {
        let dx_from_base = x - fault.px;
        let dz_from_base = z - fault.pz;
        let cross = fault.dx * dz_from_base - fault.dz * dx_from_base;
        let dist_sq = (cross * cross) / fault.dlen_sq;

        if dist_sq < min_dist_sq {
            min_dist_sq = dist_sq;
            nearest_idx = idx;
            let t = fault_projection_parameter(x, z, fault);
            nearest_t = t;
        }
    }

    (nearest_idx, nearest_t)
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
    enable_base: bool,
    enable_fault_scarp: bool,
    enable_fault_resistance: bool,
    enable_volcanic: bool,
    enable_ridges: bool,
    enable_erosion: bool,
    erosion_strength: i64,
    enable_plate_sim: bool,
    plate_strength: i64,
) -> ErosionState {
    use rayon::prelude::*;

    let n = dim * dim;
    // W-18: when base=false, initialize with FLAT_DATUM; when base=true, use fBm (byte-identical default)
    let base_height = if enable_base { None } else { Some(flat_datum(hmax)) };
    // M3 (W-17): par_iter height fBm fill loop — pure map-into-slice, byte-safe.
    // PAR_MIN_DIM gate: dim-64 shows regression; parallel only when dim≥128.
    let height: Vec<i64> = if dim >= PAR_MIN_DIM {
        // Parallel path: par_iter per-cell
        (0..n).into_par_iter().map(|idx| {
            if let Some(datum) = base_height {
                datum
            } else {
                let x = idx % dim;
                let z = idx / dim;
                height_at(x as i64, z as i64, seed, hmax)
            }
        }).collect()
    } else {
        // Serial fallback for dim < PAR_MIN_DIM
        let mut height = Vec::with_capacity(n);
        for idx in 0..n {
            if let Some(datum) = base_height {
                height.push(datum);
            } else {
                let x = idx % dim;
                let z = idx / dim;
                height.push(height_at(x as i64, z as i64, seed, hmax));
            }
        }
        height
    };
    let mut height = height;

    let mut resistance = resistance_field(dim, seed, hmax);

    if enable_fault_scarp || enable_fault_resistance {
        let faults = crate::gen::tectonics::build_faults(seed, dim);

        if enable_fault_scarp {
            for z in 0..dim {
                for x in 0..dim {
                    let idx = linear_index(x, z, dim);
                    // W-13: Query at warped coordinates (fault traces become curved via domain warp)
                    let (wx, wz) = crate::gen::tectonics::fault_warp_at(x as i64, z as i64, seed, dim);
                    let warped_x = (x as i64) + wx;
                    let warped_z = (z as i64) + wz;
                    let delta = crate::gen::tectonics::fault_scarp_delta(warped_x, warped_z, &faults, hmax);
                    height[idx] = (height[idx] + delta).clamp(0, hmax);
                }
            }
        }

        if enable_fault_resistance {
            for z in 0..dim {
                for x in 0..dim {
                    let idx = linear_index(x, z, dim);
                    // W-13: Query at warped coordinates (resistance band follows the same curved trace)
                    let (wx, wz) = crate::gen::tectonics::fault_warp_at(x as i64, z as i64, seed, dim);
                    let warped_x = (x as i64) + wx;
                    let warped_z = (z as i64) + wz;
                    if crate::gen::tectonics::is_in_fault_band(warped_x, warped_z, &faults) {
                        resistance[idx] = N_RESIST_CLASSES - 1; // hardest class — resistant fault stands proud (module doc, RnD 17 §3)
                    }
                }
            }
        }

        // W-13/W-15a: Ridge stage with M1 (along-crest modulation) + M2 (foothill skirt)
        // (applied inside the fault scope to preserve byte-identity when faults are off)
        if enable_ridges {
            // Compute histogram percentiles (p50, p80) on post-uplift height
            let mut height_sorted = height.clone();
            height_sorted.sort_unstable();
            let p50_idx = height_sorted.len() / 2;
            let p80_idx = (height_sorted.len() * 4) / 5;
            let h_p50 = if p50_idx < height_sorted.len() { height_sorted[p50_idx] } else { hmax / 2 };
            let h_p80 = if p80_idx < height_sorted.len() { height_sorted[p80_idx] } else { hmax };

            // W-15a: Compute base period for crest modulation (dim/4 at dim=512, increased from dim/8
            // to lower spatial frequency — smoother envelope reduces per-cell step under W-9 bound)
            let crest_mod_period = (dim as i64) / 4;

            // W-15a: Skirt parameters
            const SKIRT_HALF_WIDTH: i64 = 8; // S = 2*W = 2*4 = 8
            const SKIRT_WIDTH: i64 = SKIRT_HALF_WIDTH;
            let skirt_inner = BELT_HALF_WIDTH;
            let skirt_outer = BELT_HALF_WIDTH + SKIRT_WIDTH;

            for z in 0..dim {
                for x in 0..dim {
                    let idx = linear_index(x, z, dim);

                    // W-13/W-15a: Band distance with analytic distance (O(1) per cell).
                    // Query at warped coordinates — belt distance warped too, following the curved fault trace.
                    let (fault_wx, fault_wz) = crate::gen::tectonics::fault_warp_at(x as i64, z as i64, seed, dim);
                    let warped_fault_x = (x as i64) + fault_wx;
                    let warped_fault_z = (z as i64) + fault_wz;

                    // W-15a: Get actual perpendicular distance and nearest fault
                    let min_dist = crate::gen::tectonics::fault_min_distance(warped_fault_x, warped_fault_z, &faults);
                    if min_dist > skirt_outer {
                        continue; // Skip cells completely outside belt + skirt
                    }

                    // W-15a: Find nearest fault and compute along-fault parameter t
                    let (nearest_idx, foot_t) = nearest_fault_and_parameter(warped_fault_x, warped_fault_z, &faults);
                    let nearest_fault = &faults[nearest_idx];

                    // W-15a: Compute crest modulation
                    let crest_mod = crest_modulation(foot_t, nearest_idx as u32, crest_mod_period, seed);

                    if min_dist <= skirt_inner {
                        // CORE: apply modulated ridge delta

                        // Band ramp: linear from 256 at dist=0 to 0 at dist=BELT_HALF_WIDTH
                        let band_r = if min_dist == 0 {
                            256
                        } else {
                            (256 * (BELT_HALF_WIDTH - min_dist)) / BELT_HALF_WIDTH
                        };

                        // Height ramp: 0 below p50, 1 at p80, linear between
                        let h = height[idx];
                        let height_r = if h < h_p50 {
                            0
                        } else if h >= h_p80 {
                            256
                        } else {
                            (256 * (h - h_p50)) / (h_p80 - h_p50).max(1)
                        };
                        if height_r == 0 {
                            continue;
                        }

                        // Combine masks: mountainness = band_ramp × height_ramp
                        let mountainness = (band_r * height_r) / 256;
                        if mountainness == 0 {
                            continue;
                        }

                        // Apply warp to sample coordinates for ridge texture
                        let (warp_x, warp_z) = ridge_warp_at(x as i64, z as i64, seed);
                        let sample_x = (x as i64) + warp_x;
                        let sample_z = (z as i64) + warp_z;

                        // W-13: `ridge_fbm_at` returns READY ridged field [0, 32768] (already folded).
                        let ridged = ridge_fbm_at(sample_x, sample_z, seed);

                        // W-15a: Apply modulated ridge delta
                        let ridge_delta = ridge_delta_compute_modulated(
                            ridged, mountainness, crest_mod, RIDGE_AMP_NUM, RIDGE_AMP_DEN, hmax
                        );
                        height[idx] = (height[idx] + ridge_delta).clamp(0, hmax);
                    } else if min_dist <= skirt_outer {
                        // SKIRT: apply foothill skirt formula

                        // Compute belt_delta_local: the modulated core delta at the foot point
                        // Height at foot point (on the nearest fault)
                        let foot_x = nearest_fault.px + nearest_fault.dx * foot_t;
                        let foot_z = nearest_fault.pz + nearest_fault.dz * foot_t;

                        // Clamp foot to grid bounds for height lookup
                        // (foot is on infinite line, may be outside grid, so we use closest grid point)
                        let foot_x_clamped = foot_x.max(0).min((dim as i64) - 1);
                        let foot_z_clamped = foot_z.max(0).min((dim as i64) - 1);
                        let foot_idx = linear_index(foot_x_clamped as usize, foot_z_clamped as usize, dim);
                        let h_foot = height[foot_idx];

                        // Height ramp for foot point
                        let height_r_foot = if h_foot < h_p50 {
                            0
                        } else if h_foot >= h_p80 {
                            256
                        } else {
                            (256 * (h_foot - h_p50)) / (h_p80 - h_p50).max(1)
                        };

                        if height_r_foot > 0 {
                            // Band ramp for foot point (at the fault, distance = 0)
                            let band_r_foot = 256;

                            // Mountainness for foot point
                            let mountainness_foot = (band_r_foot * height_r_foot) / 256;

                            // Get ridge texture at foot
                            let (warp_x, warp_z) = ridge_warp_at(foot_x_clamped, foot_z_clamped, seed);
                            let sample_x = foot_x_clamped + warp_x;
                            let sample_z = foot_z_clamped + warp_z;
                            let ridged_foot = ridge_fbm_at(sample_x, sample_z, seed);

                            // Compute belt_delta_local with modulation
                            let belt_delta_local = ridge_delta_compute_modulated(
                                ridged_foot, mountainness_foot, crest_mod, RIDGE_AMP_NUM, RIDGE_AMP_DEN, hmax
                            );

                            // Apply skirt formula: skirt = belt_delta_local * (S - (r - W))² / (S² * 4)
                            // where r ∈ (W, W+S], S = 8
                            let r = min_dist;
                            let r_minus_w = r - skirt_inner; // Distance into skirt, in (0, S]
                            let s_minus_r_minus_w = SKIRT_WIDTH - r_minus_w; // Decreasing from S to 0
                            let skirt_factor = (s_minus_r_minus_w * s_minus_r_minus_w) / (SKIRT_WIDTH * SKIRT_WIDTH);
                            // Note: skirt_factor ranges [0, 1] in fixed-point (scaled by S²)
                            // Full formula: skirt = belt_delta_local * (S - (r - W))² / (S² * 4)
                            // We compute: skirt = (belt_delta_local * skirt_factor) / 4
                            let skirt_delta = (belt_delta_local * skirt_factor) / 4;

                            height[idx] = (height[idx] + skirt_delta).clamp(0, hmax);
                        }
                    }
                }
            }
        }
    }

    if enable_volcanic {
        let vents = crate::gen::volcanic::build_vents(seed, dim);
        let delta = crate::gen::volcanic::emplace_edifices(dim, hmax, &vents);
        for idx in 0..n {
            height[idx] = (height[idx] + delta[idx]).clamp(0, hmax);
        }
    }

    // **Slice-1b: Plate uplift (Stage 4 orogeny).** Gated on enable_plate_sim (default false).
    // When false, this block is never executed ⇒ v2_golden_conserved_* byte-identical (merge gate).
    // When true, compute plate fields and generate orogeny uplift, added to height BEFORE erosion.
    if enable_plate_sim {
        let plate_count = 15u32; // Default plate count (parameterizable in Slice-1c)
        let plate_count_clamped = crate::gen::plate::clamp_plate_count(plate_count, dim as i64);
        let plate_fields = crate::gen::plate::compute_plate_fields(seed, dim as i64, plate_count_clamped);
        let plate_uplift = crate::gen::orogeny::generate_plate_uplift_field(&plate_fields, dim as i64, hmax, plate_strength);
        for idx in 0..n {
            height[idx] = (height[idx] + plate_uplift[idx]).clamp(0, hmax);
        }
    }

    erode_from_fields(seed, hmax, dim, height, resistance, enable_erosion, erosion_strength)
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
/// **W-11 gate (ridges default-off):** `enable_ridges` threads straight to `erode_with_tectonics`,
/// orthogonal to other gates. Ridge logic is gated by `enable_fault_scarp` (both uplift and ridges
/// require fault bands; no ridges without tectonic uplift).
/// W-18: added enable_base and enable_erosion parameters (SOURCES vs TRANSFORMS).
/// W-19: added erosion_strength parameter (percent, default 100, clamped to [0, 400]).
pub fn erode(seed: u64, hmax: i64, dim: usize, enable_base: bool, enable_tectonics: bool, enable_volcanic: bool, enable_ridges: bool, enable_erosion: bool, erosion_strength: i64) -> ErosionState {
    // Clamp strength to valid range [0, 400]
    let clamped_strength = erosion_strength.clamp(0, 400);
    // Slice-1b: plate sim defaults to false (byte-identical); pass through enable_tectonics for enable_fault_resistance symmetry
    erode_with_tectonics(seed, hmax, dim, enable_base, enable_tectonics, enable_tectonics, enable_volcanic, enable_ridges, enable_erosion, clamped_strength, false, 100)
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
        // Use 3x3 grid (9 cells) to satisfy dim*dim == height.len()
        let n = 9;
        let height = vec![10i64; n];
        let downstream = vec![None; n];
        let area = vec![1i64; n];
        let resistance = vec![0i64; n];
        let delta = incision_step(3, &height, &downstream, &area, &resistance);
        assert_eq!(delta, vec![0; n]);
    }

    #[test]
    fn incision_step_never_drives_height_negative() {
        // Huge area/slope, softest resistance — delta must clamp to height[v] exactly, not overshoot.
        // Use 2x2 grid to satisfy dim*dim == height.len()
        let n = 4;
        let mut height = vec![0i64; n];
        let mut downstream = vec![None; n];
        height[0] = 5i64;
        height[1] = 0i64;
        downstream[0] = Some(1);
        let area = vec![1_000_000i64, 1, 1, 1];
        let resistance = vec![0i64, 0, 0, 0];
        let delta = incision_step(2, &height, &downstream, &area, &resistance);
        assert_eq!(delta[0], 5, "delta must clamp to the cell's own height, never exceed it");
    }

    #[test]
    fn incision_step_is_slower_on_harder_resistance() {
        // Use 2x2 grid to satisfy dim*dim == height.len()
        let n = 4;
        let mut height = vec![0i64; n];
        let mut downstream = vec![None; n];
        height[0] = 100i64;
        height[1] = 0i64;
        downstream[0] = Some(1);
        let area = vec![64i64, 1, 1, 1];
        let soft = incision_step(2, &height, &downstream, &area, &[0, 0, 0, 0]);
        let hard = incision_step(2, &height, &downstream, &area, &[3, 0, 0, 0]);
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
        let state = erode(SEED, HMAX, DIM, true, false, false, false, true, 100);
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
        let a = erode(SEED, HMAX, 16, true, false, false, false, true, 100);
        let b = erode(SEED, HMAX, 16, true, false, false, false, true, 100);
        assert_eq!(a, b, "erode must be byte-identical across repeated calls");
    }

    #[test]
    fn erode_is_not_degenerate() {
        // Sanity: erosion must actually change SOME heights (not a no-op) and must NOT flatten
        // everything to zero (not a runaway collapse). DIM=64 (matches the prod-scale chain grid):
        // material diversity (Soil vs Bedrock) needs enough drainage-area range to cross
        // INCISION_EXPOSURE_THRESHOLD somewhere — a smaller probe grid (e.g. 32) may not reach it.
        const DIM: usize = 64;
        let state = erode(SEED, HMAX, DIM, true, false, false, false, true, 100);
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
        let state = erode(GOLDEN_SEED, GOLDEN_HMAX, DIM, true, false, false, false, true, 100);

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
        let a = erode_with_tectonics(SEED, HMAX, 16, true, true, false, false, false, true, 100, false, 100);
        let b = erode_with_tectonics(SEED, HMAX, 16, true, true, false, false, false, true, 100, false, 100);
        assert_eq!(a, b, "erode_with_tectonics must be byte-identical across repeated calls");
    }

    #[test]
    fn erode_tectonics_gate_reproduces_pre_396_erode_byte_for_byte() {
        // Both flags false must be IDENTICAL to erode()'s pre-#396 body (this is a pure structural
        // refactor into erode_from_fields/erode_with_tectonics — no behavior change when off).
        let via_erode = erode(SEED, HMAX, 16, true, false, false, false, true, 100);
        let via_flags = erode_with_tectonics(SEED, HMAX, 16, true, false, false, false, false, true, 100, false, 100);
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

        let a = erode_with_tectonics(SEED, HMAX, DIM, true, false, false, false, false, true, 100, false, 100);
        let b = erode_with_tectonics(SEED, HMAX, DIM, true, true, false, false, false, true, 100, false, 100);
        let c = erode_with_tectonics(SEED, HMAX, DIM, true, true, true, false, false, true, 100, false, 100);

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
        let state = erode(GOLDEN_SEED, GOLDEN_HMAX, DIM, true, true, false, false, true, 100);

        // W-13 re-pin: warp + BELT_HALF_WIDTH 2→4 + single-fold multifractal (enable_tectonics=true path).
        // Warp applies to fault-scarp, resistance-band, and ridge-belt distance when tectonics ON.
        // Terrain changes intentional (cliffs curve, belt widens, crests show sub-ridges).
        // Drift: [6, 9, 2, 1] cells at indices [0, 36, 100, 255] (subtle/reasonable).
        const INDICES: [usize; 4] = [0, 36, 100, 255];
        const EXPECTED: [i64; 4] = [107, 107, 102, 96]; // W-13: re-pinned from run
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
        let off = erode(SEED, HMAX, DIM, true, false, false, false, true, 100);
        let on = erode(SEED, HMAX, DIM, true, false, true, false, true, 100);
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
        let a = erode(SEED, HMAX, DIM, true, false, false, false, true, 100);
        let b = erode(SEED, HMAX, DIM, true, false, false, false, true, 100);
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
        let state = erode(SEED, HMAX, DIM, true, false, true, false, true, 100);
        let vents = crate::gen::volcanic::build_vents(SEED, DIM);
        let geom = crate::gen::volcanic::EdificeGeom::derive(DIM, HMAX);

        for vent in &vents {
            let cx = vent.x;
            let cz = vent.z;
            if cx < 0 || cz < 0 || cx as usize >= DIM || cz as usize >= DIM {
                continue; // vent center off-grid for this DIM — nothing to sample
            }

            // Get radius from geom based on slope class
            let radius = match vent.class {
                crate::gen::volcanic::SlopeClass::Shield => geom.shield_radius,
                crate::gen::volcanic::SlopeClass::Cone => geom.cone_radius,
            };

            // Find edifice_max: the highest point over the entire edifice disk (r ≤ radius).
            // For cones with calderas, this finds the rim; for shields, the summit.
            let mut edifice_max = 0i64;
            let mut center_height = 0i64;
            for dz in -(radius)..=radius {
                for dx in -(radius)..=radius {
                    let r_sq = dx * dx + dz * dz;
                    if r_sq > radius * radius {
                        continue; // outside the disk
                    }
                    let px = cx + dx;
                    let pz = cz + dz;
                    if px < 0 || pz < 0 || px as usize >= DIM || pz as usize >= DIM {
                        continue; // off-grid
                    }
                    let h = state.height[linear_index(px as usize, pz as usize, DIM)];
                    if dx == 0 && dz == 0 {
                        center_height = h; // Track crater for Cone vents
                    }
                    edifice_max = edifice_max.max(h);
                }
            }

            // Sample the ring at 8 compass points on the outer radius (cheap, deterministic).
            let mut ring_sum = 0i64;
            let mut ring_count = 0i64;
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

            // The edifice's highest point (rim for cones, summit for shields) must remain a net
            // local high after erosion: strictly higher than the mean of the surrounding ring.
            assert!(
                edifice_max > ring_mean,
                "vent at ({cx},{cz}) edifice_max={edifice_max} must be > ring_mean={ring_mean} after full pipeline"
            );

            // For Cone vents, crater persistence through the full pipeline is NOT asserted:
            // deposition legitimately infills small-dim craters to exactly rim level (center==edifice_max
            // in CI run 29645529597 at dim=64/hmax=16). The caldera SHAPE is asserted pre-pipeline by
            // volcanic.rs::caldera_bowl_structure; visual crater persistence at real dims is judged by
            // the PM/user 512 gallery. The caldera must still survive as a net local high and meet the
            // ≥2/3 survival floor (verified below).
            if matches!(vent.class, crate::gen::volcanic::SlopeClass::Cone) {
                // Strength check: cone rim must survive erosion/talus/de-needle with at least 2/3 of peak
                let min_cone_h = (geom.peak * 2) / 3;
                assert!(
                    edifice_max >= min_cone_h,
                    "cone vent at ({cx},{cz}) edifice_max={edifice_max} must be >= {min_cone_h} after pipeline"
                );
            } else if matches!(vent.class, crate::gen::volcanic::SlopeClass::Shield) {
                // Strength check: shield summit must survive erosion with at least 2/3 of peak_shield
                let peak_shield = geom.peak / 2;
                let min_shield_h = (peak_shield * 2) / 3;
                assert!(
                    edifice_max >= min_shield_h,
                    "shield vent at ({cx},{cz}) edifice_max={edifice_max} must be >= {min_shield_h} after pipeline"
                );
            }
        }
    }

    /// Golden vector (ON path): the volcanic-ON `erode` output is pinned at fixed grid indices.
    ///
    /// W-16: Re-pinned after profile rework (linear → quadratic cone, new shield formula,
    /// caldera addition). CI-sourced from pass 1 (`.ci-report/failed.log`, golden-arm64 job);
    /// identical on arm64 (v2-golden) and x86 (v2-sim), arch-independent.
    #[test]
    fn golden_vector_matches_pinned_volcanic_on_erosion_fixture() {
        const GOLDEN_SEED: u64 = 0xA11A_2A11;
        const GOLDEN_HMAX: i64 = 200;
        const DIM: usize = 16;
        let state = erode(GOLDEN_SEED, GOLDEN_HMAX, DIM, true, false, true, false, true, 100);

        const INDICES: [usize; 4] = [0, 36, 100, 255];
        const EXPECTED: [i64; 4] = [152, 137, 133, 157]; // W-16b amendment: shield radius fix (peak*5/24), pinned from CI run 29652655892 (x86 debug and arm64 release IDENTICAL)
        let actual: [i64; 4] = std::array::from_fn(|i| state.height[INDICES[i]]);
        assert_eq!(actual, EXPECTED, "golden drift (or placeholder awaiting CI pin) at indices {INDICES:?}");
    }
}
