//! W-SIM-3a (#403): deterministic integer aeolian dunes — Werner slab-CA, the second landform slice
//! on the `worldgen-relief` ladder (RnD `sim/world/13-aeolian-landforms.md`, sibling of erosion
//! [10]). Runs POST-erosion, before the final classify: `erode → AEOLIAN → final climate/material/
//! biome` (RnD 13 §1). **Pure integer / fixed-point throughout — no `f32`/`f64` anywhere in this
//! file** (covered by the recursive glob guard, `world/tests/no_float_guard_gen.rs`).
//!
//! ## Sand supply (this slice's scope)
//!
//! RnD 13 §6 lists deflation of arid basins, erosion fines, volcanic ash, and glacial outwash as
//! sand SOURCES — none of those upstream producers exist yet (future landform slices). This slice
//! bootstraps from a WORKING ARIDITY ESTIMATE (RnD 13 §1's own chicken-egg resolution — aridity
//! depends on climate, climate depends on height, height is what this pass changes, so it reads a
//! PRE-aeolian estimate rather than the not-yet-final climate): the caller
//! (`caps::classify_and_caps`) seeds [`INITIAL_SAND_DEPTH`] slabs at every cell whose PRE-aeolian
//! working precipitation (on the post-erosion, pre-aeolian height) falls below
//! `caps::ARID_P_THRESHOLD`; everywhere else starts bare. This is deliberately PRECIPITATION, not
//! the `Desert` zonal biome (RnD 13 §1 itself specifies "атмосферный P_base", not a biome
//! classification) — this climate model's temperature never exceeds ~16°C on any realistic grid, so
//! `Desert`'s T_ref=25°C reference point is UNREACHABLE via real climate output, making a
//! biome-based gate permanently dead code rather than merely rare.
//!
//! ## The Werner slab-CA (RnD 13 §3), Jacobi-simultaneous reformulation
//!
//! Five rules per macro-iteration, over the OLD frame (double-buffer, R10):
//!
//! 1. **Pickup**: a cell emits exactly 1 slab if it holds sand AND is NOT in wind shadow.
//!    Werner's original CA picks a random cell per sub-step; this Jacobi reformulation has EVERY
//!    eligible cell emit simultaneously against the old frame (RnD 13 §4's own honesty note: this
//!    is deterministic-by-construction, not a probabilistic gate — Werner assigns no probability to
//!    pickup itself, only to deposition, so no RNG roll is needed here).
//! 2. **Transport**: the slab jumps a fixed [`HOP_LENGTH`] cells downwind (+X, the map's uniform
//!    prevailing wind — `climate.rs::WIND_DX`'s direction).
//! 3. **Probabilistic deposition**: on bare rock `p_bare` = [`P_BARE_NUM`]/[`P_BARE_DEN`] (≈0.4), on
//!    sand `p_sand` = [`P_SAND_NUM`]/[`P_SAND_DEN`] (≈0.6) — `p_sand > p_bare` so grains bounce off
//!    bare rock and stick to sand, nucleating and growing dunes. In shadow, deposition is certain
//!    (`p=1`, no roll needed). Undeposited slabs hop again, up to [`K_MAX_HOPS`] (RnD 13 §4's
//!    `exported-at-edge` bound).
//! 4. **Wind shadow**: a leeward geometric mask, a single +X sweep per row with an integer-fraction
//!    decaying ceiling ([`SHADOW_DROP_NUM`]/[`SHADOW_DROP_DEN`], ≈tan 14°, close to the ≈15° target
//!    — an exact tan(15°) has no small integer ratio, so this is the nearest clean fraction,
//!    documented rather than silently approximated) — no float, no trig.
//! 5. **Avalanche (angle of repose)**: [`avalanche_pass`], a fixed-iteration Jacobi gather over the
//!    SAND COLUMN ONLY (never rock), moving material to each cell's local steepest-descent D8
//!    neighbor when the total-height (rock+sand) drop exceeds [`AVALANCHE_REPOSE_THRESHOLD`] — the
//!    same CLASS of mechanism as `erosion.rs`'s `talus_step`, but with a sand-specific threshold
//!    (distinct from the rock talus angle) and its own local receiver (not erosion's flood-filled
//!    drainage `downstream` — avalanche is about surface geometry, not river routing).
//!
//! ## Determinism (RnD 13 §4 — the load-bearing novelty vs RNG-free erosion)
//!
//! The CA's only stochastic draw is the PER-HOP deposition roll — realized as a stateless
//! counter-based keyed hash: `seed_fold(seed, [SALT_AEOLIAN_DEPOSIT, morton(source), iteration,
//! hop])`. `seed_fold`'s iterated-splitmix64 fold over an explicit, distinct-per-role parts tuple
//! IS this codebase's counter-based-keyed-RNG primitive (the same technique `genome.rs` uses for
//! every one of its RNG draws) — the RnD's "not splitmix" pitfall warns against a BARE single-value
//! XOR-fold (`morton⊕iter`, which collides `(morton=5,iter=3)` against `(morton=3,iter=5)`), not
//! against `seed_fold`'s distinct-parts chain, which folds `morton`, `iteration`, and `hop` as
//! SEPARATE parts in sequence — no such collision is possible. This is a stateless function of
//! `(seed, source-cell, iteration, hop-index)` alone: traversal order is irrelevant by construction.
//!
//! **Scatter-add is permitted (#403 ТЗ, explicit):** a slab's destination is a pure function of its
//! source + deposit-roll chain, so each source resolves to EXACTLY ONE destination (or the
//! `exported_at_edge` ledger bucket) — accumulating multiple sources' contributions into a
//! destination cell is plain INTEGER ADDITION, commutative/associative regardless of processing
//! order or thread count. This module runs serially (matching every sibling `gen/` stage — none of
//! `erosion.rs`/`tectonics.rs`/`caps.rs` are parallelized), so no thread-count test is needed (the
//! ТЗ's thread-count clause is conditional: "if the pass is parallel"); the repeat-run determinism
//! test below is the applicable guarantee. If a future perf pass parallelizes this with `rayon`,
//! correctness is preserved by the same commutative-accumulation argument, unchanged.
//!
//! **Honesty note (RnD 13 §4, mirrors the erosion honesty notes):** a Jacobi-parallel Werner CA is
//! an APPROXIMATION of the characteristic morphology of the serial CA, not a bit-faithful
//! reproduction of its trajectory (every eligible cell picks up against the old frame at once,
//! weakening the serial CA's global "one grain moves at a time" sand competition). The goal here is
//! the emergent dune PATTERN (barchan/transverse ridges), not serial-trajectory identity — and a
//! FIXED iteration count (never convergence-ε, R10) means dunes come out characteristic, not fully
//! mature (accepted, matches erosion's own "not fully relaxed" honesty note).
//!
//! ## Slab-ledger conservation (R14)
//!
//! Integer buckets `sand_depth` (current, per-cell) + `exported_at_edge` (cumulative, run-lifetime)
//! conserve EXACTLY against the initial seeded total: `Σsand_depth + exported_at_edge ==
//! Σinitial_sand_depth` after every iteration (mirrors erosion's `Σheight + export == initial
//! Σheight` sediment ledger). The avalanche pass is a SEPARATE, purely internal redistribution of
//! `sand_depth` (zero-sum, like erosion's talus) — it never touches the ledger buckets.
//!
//! ## 2.5D, sand tracked separately from rock (load-bearing)
//!
//! `sand_depth` is a distinct per-cell quantity from `rock_height` (the post-erosion height this
//! module receives as input, never mutated). Passable height = `rock_height + sand_depth` (2.5D,
//! `height(x,z)`, no 3D overhangs — R16). The avalanche pass moves ONLY `sand_depth`, floored at 0
//! (never negative, never touches `rock_height`) — conflating the two would apply the sand repose
//! angle to rock (wrong physics) and break the slab ledger (which counts sand slabs, not total
//! height).

use sim_core::{morton2, seed_fold};

/// Fixed macro-iteration count for the whole aeolian pass (R10, never convergence-ε — same
/// discipline as `erosion.rs`'s `MACRO_ITERATIONS`, a DISTINCT constant/budget since this is a
/// different, independently-tuned stage). Kept modest: the pass is meant to produce CHARACTERISTIC
/// dune structure on a cheap localized substep, not run to maturity (RnD 13 §4).
pub const N_AEOLIAN_ITERATIONS: usize = 6;

/// Downwind jump length per hop, in cells (RnD 13 §3: "L≈1–5"). Implementer's call, documented,
/// locked by the golden-vector test.
const HOP_LENGTH: i64 = 3;
/// Maximum hops a single picked-up slab may attempt before giving up unresolved (RnD 13 §4's
/// `exported-at-edge` bound — bounds the multi-hop chain to a fixed cost, never unbounded).
const K_MAX_HOPS: i64 = 5;

/// Deposition probability on bare (non-sand) substrate ≈0.4 (RnD 13 §3: `p_ns≈0.4`).
const P_BARE_NUM: u64 = 2;
const P_BARE_DEN: u64 = 5;
/// Deposition probability on existing sand ≈0.6 (RnD 13 §3: `p_s≈0.6`, `p_s > p_ns` — the asymmetry
/// that nucleates and grows dunes: grains bounce off bare rock, stick to sand).
const P_SAND_NUM: u64 = 3;
const P_SAND_DEN: u64 = 5;

/// Wind-shadow ceiling decay per downwind cell, as an integer fraction (RnD 13 §3: shadow angle
/// ≈15°; `1/4` ≈ tan 14°, the nearest clean small-integer ratio — documented approximation, not a
/// silent one). Implementer's call, locked by the golden-vector test.
const SHADOW_DROP_NUM: i64 = 1;
const SHADOW_DROP_DEN: i64 = 4;

/// Avalanche (angle-of-repose) threshold: a total-height (rock+sand) drop to a cell's steepest D8
/// neighbor exceeding this many height units triggers a sand slide. Calibrated LOW (mirrors
/// `erosion.rs`'s `REPOSE_THRESHOLD=0` recalibration lesson): slabs deposit in single-unit
/// increments on this grid's scale, so a large threshold would leave avalanche permanently inert.
const AVALANCHE_REPOSE_THRESHOLD: i64 = 1;
/// Fraction of the excess (above threshold) a cell sends per avalanche sub-iteration (mirrors
/// `erosion.rs`'s `TALUS_FRAC_NUM`/`_DEN` pattern — a fraction, not the whole excess at once, so
/// avalanche approaches repose over several sub-iterations rather than fully flattening in one).
const AVALANCHE_FRAC_NUM: i64 = 1;
const AVALANCHE_FRAC_DEN: i64 = 2;
/// Fixed avalanche sub-iterations per macro-iteration (R10 — never convergence-ε). Dunes settle
/// toward, but not fully to, repose (accepted, RnD 13 §4 honesty note).
const AVALANCHE_SUBITERS: usize = 2;

/// Initial sand slabs seeded at every Desert-derived-Sand cell (see the module doc's sand-supply
/// note). Implementer's call: enough to give the CA real material to move without dominating the
/// iteration budget.
pub const INITIAL_SAND_DEPTH: i64 = 3;

/// Decorrelation salt for the deposit-roll counter-based hash ("AEOLDEP0", ASCII-folded — mirrors
/// `erosion.rs`'s `RESISTANCE_SALT` / `tectonics.rs`'s `FAULT_SEED_SALT` convention).
const SALT_AEOLIAN_DEPOSIT: u64 = 0x4145_4F4C_4445_5030;

const D8_OFFSETS: [(i64, i64); 8] =
    [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];

#[inline]
fn linear_index(x: usize, z: usize, dim: usize) -> usize {
    z * dim + x
}

/// The stateless counter-based deposit roll — see the module doc's Determinism section. `hop` and
/// `iteration` are distinct parts (never folded together), so different hops/iterations of the SAME
/// source cell never collide.
#[inline]
fn deposit_roll(seed: u64, source_x: i64, source_z: i64, iteration: usize, hop: i64) -> u64 {
    let m = morton2(source_x as u32, source_z as u32) as u64;
    seed_fold(seed, &[SALT_AEOLIAN_DEPOSIT, m, iteration as u64, hop as u64])
}

/// Leeward wind-shadow mask (RnD 13 §3 rule 4): a single +X sweep per row tracking a decaying
/// "shadow ceiling" — any cell strictly below the current ceiling is in shadow. The ceiling rises to
/// meet any taller cell it passes (a new obstacle re-casts the shadow) and decays by
/// [`SHADOW_DROP_NUM`]/[`SHADOW_DROP_DEN`] per cell traveled otherwise. Integer-only, O(dim) per
/// row, no trig/float.
fn wind_shadow_mask(dim: usize, total_height: &[i64]) -> Vec<bool> {
    let n = dim * dim;
    let mut mask = vec![false; n];
    for z in 0..dim {
        let mut shadow_h = i64::MIN;
        let mut carry = 0i64;
        for x in 0..dim {
            let idx = linear_index(x, z, dim);
            let h = total_height[idx];
            mask[idx] = h < shadow_h;
            if h > shadow_h {
                shadow_h = h;
                carry = 0;
            }
            carry += SHADOW_DROP_NUM;
            if carry >= SHADOW_DROP_DEN {
                shadow_h -= carry / SHADOW_DROP_DEN;
                carry %= SHADOW_DROP_DEN;
            }
        }
    }
    mask
}

/// One macro-iteration of pickup→transport→deposit (RnD 13 §3 rules 1–4), Jacobi (reads only the
/// OLD `sand_depth_old` frame). Returns the scatter-add delta buffer (apply via `sand_depth[i] +=
/// delta[i]`, see the module doc's scatter-add-is-permitted note) and this iteration's
/// `exported_at_edge` count (slabs that fell off the grid edge or exceeded `K_MAX_HOPS`
/// unresolved).
fn transport_iteration(
    dim: usize,
    seed: u64,
    iteration: usize,
    rock_height: &[i64],
    sand_depth_old: &[i64],
) -> (Vec<i64>, i64) {
    let n = dim * dim;
    let total_height: Vec<i64> = (0..n).map(|i| rock_height[i] + sand_depth_old[i]).collect();
    let shadow = wind_shadow_mask(dim, &total_height);

    let mut delta = vec![0i64; n];
    let mut exported_at_edge = 0i64;

    for z in 0..dim {
        for x in 0..dim {
            let idx = linear_index(x, z, dim);
            if sand_depth_old[idx] <= 0 || shadow[idx] {
                continue; // no sand to pick up, or self-shadowed (rule 1)
            }
            delta[idx] -= 1; // deterministic pickup (module doc: no RNG roll needed here)

            let mut landed = false;
            for hop in 1..=K_MAX_HOPS {
                let land_x = x as i64 + hop * HOP_LENGTH;
                if land_x as usize >= dim {
                    exported_at_edge += 1;
                    landed = true;
                    break;
                }
                let land_idx = linear_index(land_x as usize, z, dim);
                if shadow[land_idx] {
                    delta[land_idx] += 1; // certain deposition in shadow (rule 3)
                    landed = true;
                    break;
                }
                let has_sand = sand_depth_old[land_idx] > 0;
                let (p_num, p_den) =
                    if has_sand { (P_SAND_NUM, P_SAND_DEN) } else { (P_BARE_NUM, P_BARE_DEN) };
                let roll = deposit_roll(seed, x as i64, z as i64, iteration, hop);
                if roll % p_den < p_num {
                    delta[land_idx] += 1;
                    landed = true;
                    break;
                }
            }
            if !landed {
                exported_at_edge += 1; // exceeded K_MAX_HOPS unresolved (RnD 13 §4)
            }
        }
    }

    (delta, exported_at_edge)
}

/// Avalanche / angle-of-repose pass (RnD 13 §3 rule 5): moves ONLY `sand_depth`, never `rock_height`
/// (load-bearing — see the module doc). A fixed [`AVALANCHE_SUBITERS`] Jacobi gather per call: each
/// sub-iteration recomputes every cell's local steepest-descent D8 receiver from the CURRENT total
/// height, then sends a fraction of the repose-exceeding excess there (clamped to the cell's own
/// available sand — never sends more than it has, never goes negative).
fn avalanche_pass(dim: usize, rock_height: &[i64], sand_depth: &mut [i64]) {
    let n = dim * dim;
    for _ in 0..AVALANCHE_SUBITERS {
        let total: Vec<i64> = (0..n).map(|i| rock_height[i] + sand_depth[i]).collect();

        // Local steepest-descent D8 receiver per cell (surface geometry, NOT erosion's flood-filled
        // drainage `downstream` — see the module doc).
        let mut receiver: Vec<Option<usize>> = vec![None; n];
        for z in 0..dim {
            for x in 0..dim {
                let idx = linear_index(x, z, dim);
                let mut best: Option<(usize, i64)> = None;
                for &(dx, dz) in &D8_OFFSETS {
                    let nx = x as i64 + dx;
                    let nz = z as i64 + dz;
                    if nx < 0 || nz < 0 || nx as usize >= dim || nz as usize >= dim {
                        continue;
                    }
                    let nidx = linear_index(nx as usize, nz as usize, dim);
                    let drop = total[idx] - total[nidx];
                    if drop > 0 && best.is_none_or(|(_, bd)| drop > bd) {
                        best = Some((nidx, drop));
                    }
                }
                receiver[idx] = best.map(|(nidx, _)| nidx);
            }
        }

        let mut send_out = vec![0i64; n];
        for idx in 0..n {
            if let Some(ridx) = receiver[idx] {
                let drop = total[idx] - total[ridx];
                if drop > AVALANCHE_REPOSE_THRESHOLD {
                    let amt = (drop - AVALANCHE_REPOSE_THRESHOLD) * AVALANCHE_FRAC_NUM / AVALANCHE_FRAC_DEN;
                    send_out[idx] = amt.min(sand_depth[idx]).max(0);
                }
            }
        }

        // Scatter-add the outflow into each cell's receiver (integer, commutative — module doc).
        for idx in 0..n {
            sand_depth[idx] -= send_out[idx];
        }
        for idx in 0..n {
            if let Some(ridx) = receiver[idx] {
                sand_depth[ridx] += send_out[idx];
            }
        }
    }
}

/// The full W-SIM-3a aeolian output: post-aeolian passable `height` (`rock_height + sand_depth`,
/// 2.5D), the `sand_depth` layer itself (the primary substrate signal — see caps.rs's reconciliation
/// note), and the cumulative `exported_at_edge` slab-ledger bucket (conservation test: `Σsand_depth +
/// exported_at_edge == initial_sand_total`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AeolianState {
    pub height: Vec<i64>,
    pub sand_depth: Vec<i64>,
    pub exported_at_edge: i64,
    pub initial_sand_total: i64,
}

/// Run the fixed [`N_AEOLIAN_ITERATIONS`] Werner slab-CA macro-loop (transport+deposit → avalanche,
/// each iteration) over an already-built `rock_height` (the post-erosion heightmap, never mutated)
/// and an explicit `initial_sand_depth` seed (see the module doc's sand-supply note — the caller,
/// `caps::classify_and_caps`, builds this from the erosion-baseline Desert→Sand material). Pure
/// function of its inputs — no RNG-of-clock, no thread/order-dependence.
pub fn run_aeolian(seed: u64, dim: usize, rock_height: &[i64], initial_sand_depth: Vec<i64>) -> AeolianState {
    let n = dim * dim;
    debug_assert_eq!(rock_height.len(), n);
    debug_assert_eq!(initial_sand_depth.len(), n);

    let initial_sand_total: i64 = initial_sand_depth.iter().sum();
    let mut sand_depth = initial_sand_depth;
    let mut exported_at_edge = 0i64;

    for iteration in 0..N_AEOLIAN_ITERATIONS {
        let (delta, exp) = transport_iteration(dim, seed, iteration, rock_height, &sand_depth);
        for i in 0..n {
            // `.max(0)` is a defensive floor, not a load-bearing clamp: by construction the only
            // negative contribution to delta[i] is that cell's OWN pickup (-1), gated on
            // sand_depth_old[i] > 0, so sand_depth_old[i] - 1 >= 0 always.
            sand_depth[i] = (sand_depth[i] + delta[i]).max(0);
        }
        exported_at_edge += exp;

        avalanche_pass(dim, rock_height, &mut sand_depth);
    }

    let height: Vec<i64> = (0..n).map(|i| rock_height[i] + sand_depth[i]).collect();
    AeolianState { height, sand_depth, exported_at_edge, initial_sand_total }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: u64 = 0xA11A_2A11;
    const DIM: usize = 64;

    /// A synthetic fixture with a real sand supply band (cells 0..8 in x) and otherwise FLAT bare
    /// rock — enough for the CA to have real material to move without depending on the full erosion
    /// pipeline (keeps this module's tests self-contained, mirrors `erosion.rs`'s own test style).
    /// Flat (not rippled) rock is deliberate: the dune-morphology corridor test below measures
    /// windward/leeward ASYMMETRY, which a rippled baseline would confound with its own structure.
    fn seeded_fixture() -> (Vec<i64>, Vec<i64>) {
        let n = DIM * DIM;
        let rock_height = vec![50i64; n];
        let mut initial_sand = vec![0i64; n];
        for z in 0..DIM {
            for x in 0..8 {
                initial_sand[linear_index(x, z, DIM)] = INITIAL_SAND_DEPTH;
            }
        }
        (rock_height, initial_sand)
    }

    #[test]
    fn run_aeolian_is_deterministic_across_repeated_calls() {
        let (rock, sand) = seeded_fixture();
        let a = run_aeolian(SEED, DIM, &rock, sand.clone());
        let b = run_aeolian(SEED, DIM, &rock, sand);
        assert_eq!(a, b, "run_aeolian must be byte-identical across repeated calls");
    }

    #[test]
    fn different_seed_diverges() {
        let (rock, sand) = seeded_fixture();
        let a = run_aeolian(SEED, DIM, &rock, sand.clone());
        let b = run_aeolian(SEED ^ 0xDEAD_BEEF, DIM, &rock, sand);
        assert_ne!(a.sand_depth, b.sand_depth, "a different seed must produce a different sand distribution");
    }

    /// Slab-ledger conservation (R14): the total sand mass is exactly conserved between the current
    /// grid and the cumulative exported-at-edge bucket — no slab is ever created or silently lost.
    #[test]
    fn slab_ledger_conserves_total_sand() {
        let (rock, sand) = seeded_fixture();
        let state = run_aeolian(SEED, DIM, &rock, sand);
        let final_total: i64 = state.sand_depth.iter().sum();
        assert_eq!(
            final_total + state.exported_at_edge,
            state.initial_sand_total,
            "Σsand_depth + exported_at_edge must equal the initial seeded total exactly"
        );
    }

    /// The CA must actually move material (a broken wiring bug — e.g. shadow always true, or the
    /// deposit roll always failing — would leave sand_depth exactly at its initial seed forever).
    #[test]
    fn aeolian_pass_actually_redistributes_sand() {
        let (rock, sand) = seeded_fixture();
        let state = run_aeolian(SEED, DIM, &rock, sand.clone());
        assert_ne!(state.sand_depth, sand, "sand_depth must change from its initial seed over N_AEOLIAN_ITERATIONS");
    }

    #[test]
    fn wind_shadow_mask_is_deterministic_and_flags_leeward_of_a_ridge() {
        // A single modest ridge at x=10 (3 units above the flat baseline) on an otherwise flat grid
        // (every row identical); downwind cells close behind it must be shadowed, cells far
        // downwind (beyond the shadow's ~12-cell decay at 1 unit per 4 cells, SHADOW_DROP_NUM/DEN)
        // must not be. A tall ridge would need a proportionally huge grid to see the decay complete
        // — kept modest so this fits a small test fixture.
        let dim = 32usize;
        let mut height = vec![10i64; dim * dim];
        for z in 0..dim {
            height[linear_index(10, z, dim)] = 13;
        }
        let a = wind_shadow_mask(dim, &height);
        let b = wind_shadow_mask(dim, &height);
        assert_eq!(a, b, "wind_shadow_mask must be byte-identical across repeated calls");
        assert!(a[linear_index(11, 0, dim)], "the cell immediately leeward of a tall ridge must be in shadow");
        assert!(!a[linear_index(dim - 1, 0, dim)], "far enough downwind, the shadow must have decayed away");
    }

    /// Count of "dune crest" cells: a cell whose leeward drop strictly exceeds its windward rise —
    /// the gentle-windward / steep-leeward slip-face signature (RnD 13 §3 rule 5). A RELATIVE
    /// comparison (leeward > windward), not an absolute repose-degree threshold — this sidesteps
    /// needing a CI-revealed under-relaxed slope constant (RnD 13 §4's own honesty note: the fixed
    /// iteration count under-relaxes the CA, so slip faces sit below the theoretical 34°) while
    /// still isolating genuine emergent asymmetry from a flat/isotropic baseline.
    fn asymmetric_profile_count(dim: usize, height: &[i64]) -> usize {
        let mut count = 0;
        for z in 0..dim {
            for x in 1..dim - 1 {
                let idx = linear_index(x, z, dim);
                let windward = height[idx] - height[linear_index(x - 1, z, dim)];
                let leeward = height[idx] - height[linear_index(x + 1, z, dim)];
                if leeward > 0 && leeward > windward {
                    count += 1;
                }
            }
        }
        count
    }

    /// Dune-morphology corridor (#403 ТЗ, anti-forcing-clean — the W-SIM-4a scarp-crank lesson: this
    /// verifies emergent STRUCTURE the OFF baseline cannot produce, no constant is tuned to move a
    /// number): with aeolian ON, the seeded sand supply must self-organize into MORE asymmetric
    /// (gentle-windward / steep-leeward) profiles than the flat bare-rock baseline, which has none
    /// by construction (flat rock ⇒ every windward/leeward pair is exactly 0, never `leeward > 0`).
    #[test]
    fn dune_asymmetric_profile_corridor() {
        let (rock, sand) = seeded_fixture();
        let off_count = asymmetric_profile_count(DIM, &rock);
        assert_eq!(off_count, 0, "the flat bare-rock baseline must have ZERO asymmetric profiles by construction");

        let state = run_aeolian(SEED, DIM, &rock, sand);
        let on_count = asymmetric_profile_count(DIM, &state.height);
        assert!(
            on_count > off_count,
            "aeolian ON must produce dune crests with steeper leeward than windward slopes — \
             a structure the flat baseline cannot: OFF={off_count} ON={on_count}"
        );
    }

    /// Golden vector: pinned exact aeolian-ON `sand_depth` at fixed grid indices for the golden
    /// `(seed, dim)` fixture, over the SAME synthetic sand-seeded fixture the other tests in this
    /// module use (self-contained — does not depend on the full erosion/classify pipeline).
    /// Indices 0/100/500/2000 land outside the dune zone (mostly-zero, still catches determinism
    /// drift); index 10 lands squarely in the migrated dune band, so the lock also covers real
    /// nonzero dune output, not only empty cells.
    ///
    /// Re-pinned for #403 pass 2: CI-sourced — `left:` from both x86 debug (`v2 sim` job) and
    /// arm64 release (`v2 golden` job), run #29183595801, commit 15de74c; both arches agree
    /// (integer + counter-based keyed hash, arch-stable).
    #[test]
    fn golden_vector_matches_pinned_aeolian_fixture() {
        let (rock, sand) = seeded_fixture();
        let state = run_aeolian(SEED, DIM, &rock, sand);

        const INDICES: [usize; 5] = [0, 100, 500, 2000, 10];
        const EXPECTED_SAND: [i64; 5] = [0, 0, 0, 1, 4];
        let actual_sand: [i64; 5] = std::array::from_fn(|i| state.sand_depth[INDICES[i]]);
        assert_eq!(actual_sand, EXPECTED_SAND, "golden drift (or placeholder awaiting CI pin) at indices {INDICES:?}");
    }
}
