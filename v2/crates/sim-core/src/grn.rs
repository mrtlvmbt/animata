//! Phase-2 **E-3**: the GRN SUBSTRATE — a standalone, deterministic, INTEGER gene-regulatory-network
//! function that reads the E-2 [`Gradient`] and resolves it to a discrete [`CellType`]. **Prod-inert**:
//! nothing here is called by `Genome::decode` or any spawn site (0 sites changed); proven by unit
//! tests over a production `GrnSpec` fixture, so E-4 reuses the type without a rewrite (mirrors the
//! E-2/`MorphogenSpec` lesson).
//!
//! **What this proves and does not prove.** The regulatory matrix is a `GrnSpec` class constant here
//! (heritable/evolvable networks are #37, out of scope). E-3 proves the MECHANISM: (1) the integer
//! dynamics are genuinely multistable — ≥2 initial states at one fixed gradient settle into ≥2
//! distinct attractors (a real attractor network, not a positional threshold in disguise), and (2)
//! different gradient positions select different cell types from a fixed initial state. It does NOT
//! prove differentiation is selected or heritable — that needs a driver + #37 (plan §5).
//!
//! **σ semantics (critic F3).** Gene state here is a non-negative EXPRESSION LEVEL (`[0, EXPR_MAX]`),
//! NOT a signed concentration — so σ is a fresh committed LOGISTIC LUT ([`grn_lut`], own offline
//! generator `v2/tools/gen_grn_lut.py`), not a reuse of `brain::TANH_LUT` (which is signed `[-256,
//! 256]` and would silently recode repression as negative "mass").
//!
//! **Non-convergence (critic F2/F7 — the blocker).** An integer recurrent net is not guaranteed to
//! reach a fixed point; it can enter a limit cycle. `grn_resolve` runs a full-double-buffer step,
//! tracking every visited state (`Vec<Vec<i32>>` — no `HashMap`, no random hasher, exact `Vec`
//! equality). The moment a state REPEATS, the cycle it closes (a period-1 cycle is a genuine fixed
//! point — the same mechanism handles both cases uniformly) is resolved to its LEXICOGRAPHIC-MINIMUM
//! state — an exact-integer, phase-independent decision: replaying `N` or `N + period` steps of an
//! oscillating spec reaches the SAME resolved state, because the cycle (once closed) is windowed by
//! its own repeat, not by where the step counter happened to stop. If the step budget (`max_steps`,
//! documented ≤16, mirroring the morphogen's `n_dev`) is exhausted with no repeat detected, the state
//! at that step is returned as a last-resort deterministic fallback (documented, not phase-independent
//! for arbitrarily larger budgets — the shipped fixtures never hit this branch, proven by a test).
// Guard: no float arithmetic in the GRN path (mirrors energy.rs/genome.rs/morphogen.rs). CI runs
// nextest, not clippy — no_float_guard.rs's token scan is what's actually CI-gated for this module.
#![deny(clippy::float_arithmetic)]

use crate::grn_lut::{self, EXPR_MAX};
use crate::morphogen::Gradient;

/// Discrete cell-type descriptor (E-3's output, E-4's `Phenotype` cache target). A production type,
/// not test-only. `Mixed` is the exact-integer tie outcome (`state[0] == state[1]`) — deterministic,
/// never a float-threshold "close enough".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellType {
    A,
    B,
    Mixed,
}

/// V-3-a: Lineage gene-identifier type. Each gene in a GrnSpec carries a unique, injective,
/// lineage-derived identifier that supports present base genes (dense 0..n_genes-1) and future
/// duplicated genes (encoded as parent_gene_id + duplication-counter-derived index). This type
/// encodes the mapping: for a base gene at index `i`, the id is simply `i`; for future
/// V-3-b/c duplications, the id will be a compact (parent_id, dup_index) pair. In V-3-a,
/// all genomes are founders with no duplication, so gene_ids = vec![0, 1, ..., n_genes-1]
/// exactly (recomputable from n_genes, so NOT hashed separately).
pub type GeneId = u32;

/// Production GRN configuration — regulatory matrix, gradient sample position, step policy, and the
/// initial network state. NOT `#[cfg(test)]`: E-3 instantiates this with a test *value*; E-4 reuses
/// the *type* unchanged when it wires the GRN into `decode` (mirrors E-2's `MorphogenSpec` / F9).
///
/// **V-3-a:** Length is self-describing — `n_genes` is the ground truth that determines the
/// dimensions of all matrix and vector fields (weights, input_weights, bias, initial, gene_ids).
/// Decode reads the network by `n_genes` (no hardcoded matrix size); the length is data that
/// evolves via V-3-b/c operators (duplication/indel), not a schema version (which only changes
/// if the decode ALGORITHM changes, not if individual genomes grow or shrink).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrnSpec {
    /// Gene count (state dimension). Ф0 fixtures use 2 (the canonical bistable toggle-switch motif).
    /// V-3-a: all founders/existing genomes have fixed n_genes (no variable-length operators yet).
    /// Future V-3-b/c will allow n_genes to grow/shrink via duplication/indel.
    pub n_genes: usize,
    /// Regulatory matrix, row-major `n_genes × n_genes`: `weights[j*n_genes+k]` is gene `k`'s
    /// influence on gene `j`'s next state.
    pub weights: Vec<i32>,
    /// Per-gene weight on the single sampled `Gradient` concentration (the "positional" input).
    pub input_weights: Vec<i32>,
    /// Per-gene additive bias.
    pub bias: Vec<i32>,
    /// Integer rescale shift applied to the raw accumulator before the σ-LUT lookup (mirrors
    /// `brain::BRAIN_SHIFT`).
    pub shift: u32,
    /// Step cap `N` (mirrors the morphogen's `n_dev`; documented ≤16 for this slice).
    pub max_steps: u32,
    /// `Gradient` cell sampled as the positional drive.
    pub sample_x: usize,
    pub sample_z: usize,
    /// Initial network state (`n_genes` entries, each in `[0, EXPR_MAX]`). Varying THIS across two
    /// `GrnSpec` values (same matrix, same gradient) is how the multistability tooth drives the
    /// dynamics from "≥2 different initial network states".
    pub initial: Vec<i32>,
    /// **V-3-a:** Lineage-derived gene identifiers (one per gene). Parallel to the genes in the
    /// network (length = n_genes). For base/founder genes, gene_ids[i] = i (deterministic,
    /// recomputable from n_genes). For V-3-b/c duplications, gene_ids[i] will encode the
    /// (parent_gene_id, dup_index) pair injectively, so no two genes collide by construction.
    /// This field supports homology detection during alignment (via id matching) and future
    /// recombination/sexual reproduction (§4 / #41). In V-3-a, all genomes are founders, so
    /// gene_ids is deterministically 0..n_genes-1; not hashed separately (recomputable).
    pub gene_ids: Vec<GeneId>,
    /// **V-3-a:** Monotonic per-genome duplication-event counter (cold field). Incremented by
    /// the duplication operator (V-3-b) to generate unique dup_index values; used to encode
    /// derived gene ids as (parent_id, dup_index). In V-3-a, all founders/existing genomes
    /// have dup_counter = 0 (no duplications yet). Only folded into hash_contribution when
    /// dup_counter != 0 (gated pattern: preserves byte-identity for all V-3-a genomes).
    pub dup_counter: u32,
}

impl GrnSpec {
    /// Length-validated constructor (E-4b-i, critic F7): `grn_resolve`/`step`/`classify` index into
    /// `weights`/`input_weights`/`bias`/`initial` using `n_genes` as the stride — a mis-sized spec
    /// would index-panic mid-`decode()` at the FIRST birth in production, not at construction. This
    /// is the config-construction-boundary guard the E-4a `classify` generalization deferred to.
    ///
    /// Panics (loudly, at construction) if any length disagrees with `n_genes`, or if
    /// `n_genes < 2` (below that, `classify`'s A/B split is meaningless — see `grn.rs`'s
    /// `classify_is_panic_safe_below_two_genes` for the *runtime* fallback this constructor exists
    /// to make unreachable in practice).
    ///
    /// **V-3-a:** automatically synthesizes `gene_ids = 0..n_genes` (base genes) and `dup_counter = 0`
    /// (no duplications yet). These fields are infrastructure for V-3-b/c variable-length operators.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        n_genes: usize,
        weights: Vec<i32>,
        input_weights: Vec<i32>,
        bias: Vec<i32>,
        shift: u32,
        max_steps: u32,
        sample_x: usize,
        sample_z: usize,
        initial: Vec<i32>,
    ) -> Self {
        assert!(n_genes >= 2, "GrnSpec::new: n_genes must be >= 2 (got {n_genes})");
        assert_eq!(
            weights.len(), n_genes * n_genes,
            "GrnSpec::new: weights.len() ({}) must equal n_genes^2 ({})", weights.len(), n_genes * n_genes
        );
        assert_eq!(
            input_weights.len(), n_genes,
            "GrnSpec::new: input_weights.len() ({}) must equal n_genes ({n_genes})", input_weights.len()
        );
        assert_eq!(
            bias.len(), n_genes,
            "GrnSpec::new: bias.len() ({}) must equal n_genes ({n_genes})", bias.len()
        );
        assert_eq!(
            initial.len(), n_genes,
            "GrnSpec::new: initial.len() ({}) must equal n_genes ({n_genes})", initial.len()
        );
        // V-3-a: base genes = dense range 0..n_genes (injective by construction, recomputable).
        let gene_ids = (0..n_genes as GeneId).collect();
        GrnSpec {
            n_genes,
            weights,
            input_weights,
            bias,
            shift,
            max_steps,
            sample_x,
            sample_z,
            initial,
            gene_ids,
            dup_counter: 0, // V-3-a: no duplications yet (founder genomes).
        }
    }
}

/// Conservative `i64` accumulator bound for the overflow guard (mirrors `brain::ACC_BOUND`'s
/// derive-from-topology discipline): `n_genes` regulatory terms at `|weight| ≤ i32::MAX` times a
/// state bounded by `EXPR_MAX`, plus one input term at the same weight bound times a `Gradient`
/// concentration bounded by `i32::MAX`, plus a bias bounded by `i32::MAX`. Generous on purpose — the
/// shipped fixtures sit far inside it (their weights are small, single/double digits); the overflow
/// test deliberately picks weights near `i32::MAX` to exceed it on purpose.
fn acc_bound(n_genes: usize) -> i64 {
    n_genes as i64 * (i32::MAX as i64) * (EXPR_MAX as i64)
        + (i32::MAX as i64) * (i32::MAX as i64)
        + i32::MAX as i64
        + 1
}

/// σ via the committed integer LUT (own provenance, NOT `brain::activate` — see module docs).
/// `preact` is Q8.8; out-of-range CLAMPS (never wraps) to the table ends. Result ∈ `[0, EXPR_MAX]`.
#[inline]
pub fn sigma(preact: i64) -> i32 {
    let clamped = preact.clamp(grn_lut::PREACT_MIN, grn_lut::PREACT_MAX);
    let idx = ((clamped - grn_lut::PREACT_MIN) / grn_lut::LUT_BIN) as usize;
    grn_lut::SIGMA_LUT[idx] as i32
}

/// Exact-integer attractor → cell-type decision. `n_genes == 2` for the Ф0 fixture (argmax of the two
/// gene levels; an exact tie is `Mixed`). Never a float threshold.
/// Generalized (E-4a, critic F7): panic-safe for any `n_genes`, not just the `n_genes == 2` Ф0
/// fixture. `n_genes < 2` has no meaningful A/B split → `Mixed` (the same "no differentiation"
/// value as an exact tie). This is a construction-boundary safety net, not a per-tick guard: a
/// production `GrnSpec` should still be built with `n_genes >= 2` (the safety here is defense in
/// depth, not a substitute for constructing a sane spec).
/// M7-a: made public for CellGraph's per-grid-cell classification.
pub fn classify(state: &[i32]) -> CellType {
    match (state.first(), state.get(1)) {
        (Some(a), Some(b)) => match a.cmp(b) {
            std::cmp::Ordering::Greater => CellType::A,
            std::cmp::Ordering::Less => CellType::B,
            std::cmp::Ordering::Equal => CellType::Mixed,
        },
        _ => CellType::Mixed,
    }
}

/// One full-double-buffer GRN step: every gene reads the OLD state entirely and the new state is
/// computed from it (order-independent by construction, like the morphogen's stencil).
fn step(state: &[i32], drive: i64, spec: &GrnSpec) -> Vec<i32> {
    let n = spec.n_genes;
    let bound = acc_bound(n);
    let mut new_state = vec![0i32; n];
    for (j, out) in new_state.iter_mut().enumerate() {
        let mut acc: i64 = spec.bias[j] as i64 + spec.input_weights[j] as i64 * drive;
        let row = &spec.weights[j * n..(j + 1) * n];
        for (w, s) in row.iter().zip(state.iter()) {
            acc += *w as i64 * *s as i64;
        }
        // SATURATE (never wrap): clamp into the documented bound BEFORE the rescale/cast — the same
        // discipline as `morphogen::diffuse_decay_cell`.
        let clamped = acc.clamp(-bound, bound);
        *out = sigma(clamped >> spec.shift);
    }
    new_state
}

/// Run the GRN to a deterministically-resolved attractor and return `(resolved_state, cell_type,
/// steps_taken)`. See the module docs for the cycle-detection / non-convergence policy.
pub fn grn_resolve(gradient: &Gradient, spec: &GrnSpec) -> (Vec<i32>, CellType, u32) {
    let drive = gradient.at(spec.sample_x, spec.sample_z) as i64;
    let mut visited: Vec<Vec<i32>> = vec![spec.initial.clone()];
    let mut state = spec.initial.clone();

    for step_no in 1..=spec.max_steps {
        let new_state = step(&state, drive, spec);
        if let Some(first_idx) = visited.iter().position(|v| *v == new_state) {
            // Cycle closed (period-1 ⇒ a genuine fixed point; period>1 ⇒ a limit cycle) — resolve to
            // the lexicographic-minimum state in the cycle. Exact-integer, phase-independent: replaying
            // N or N+period steps closes the SAME cycle at the SAME `first_idx`, so the resolved
            // minimum is identical regardless of where the step budget happened to stop.
            let resolved = visited[first_idx..].iter().cloned().min().expect("cycle is non-empty");
            let ct = classify(&resolved);
            return (resolved, ct, step_no);
        }
        visited.push(new_state.clone());
        state = new_state;
    }
    // Fallback: budget exhausted with no detected repeat. Deterministic for THIS fixed `max_steps`
    // (documented as not phase-independent for arbitrarily larger budgets); the shipped fixtures are
    // proven (by test) to always close a cycle well within `max_steps`, so this branch is never hit
    // by production-shaped specs.
    let ct = classify(&state);
    (state, ct, spec.max_steps)
}

/// Pure integer GRN reaction: reads only `(gradient, spec)` — no `FieldStore`, no global sim state, no
/// RNG, no clock. All working state is local to the call; only the [`CellType`] survives.
pub fn grn(gradient: &Gradient, spec: &GrnSpec) -> CellType {
    grn_resolve(gradient, spec).1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::morphogen::{morphogen, Boundary, MorphogenSpec};
    use crate::Genome;
    use sha2::{Digest, Sha256};

    fn flat_gradient(g_dev: usize, value: i32) -> Gradient {
        Gradient { g_dev, cells: vec![value; g_dev * g_dev] }
    }

    /// E-4a critic F7: `classify` must not panic for `n_genes < 2` (a malformed/degenerate spec) —
    /// generalized to `Mixed` (no meaningful split) instead of hard-indexing `state[0]/state[1]`.
    #[test]
    fn classify_is_panic_safe_below_two_genes() {
        assert_eq!(classify(&[]), CellType::Mixed);
        assert_eq!(classify(&[42]), CellType::Mixed);
        // Sanity: n_genes >= 2 still classifies normally.
        assert_eq!(classify(&[10, 5]), CellType::A);
        assert_eq!(classify(&[5, 10]), CellType::B);
        assert_eq!(classify(&[7, 7]), CellType::Mixed);
    }

    // ── E-4b-i: validated GrnSpec::new (critic F7 — construction-boundary, not per-tick) ────────

    #[test]
    fn grn_spec_new_accepts_correctly_sized_spec() {
        let spec = GrnSpec::new(2, vec![1, 2, 3, 4], vec![5, 6], vec![7, 8], 3, 8, 0, 0, vec![9, 10]);
        assert_eq!(spec.n_genes, 2);
        assert_eq!(spec.weights, vec![1, 2, 3, 4]);
    }

    #[test]
    #[should_panic(expected = "n_genes must be >= 2")]
    fn grn_spec_new_rejects_n_genes_below_2() {
        GrnSpec::new(1, vec![1], vec![1], vec![1], 3, 8, 0, 0, vec![1]);
    }

    #[test]
    #[should_panic(expected = "weights.len()")]
    fn grn_spec_new_rejects_mis_sized_weights() {
        GrnSpec::new(2, vec![1, 2, 3], vec![5, 6], vec![7, 8], 3, 8, 0, 0, vec![9, 10]);
    }

    #[test]
    #[should_panic(expected = "input_weights.len()")]
    fn grn_spec_new_rejects_mis_sized_input_weights() {
        GrnSpec::new(2, vec![1, 2, 3, 4], vec![5], vec![7, 8], 3, 8, 0, 0, vec![9, 10]);
    }

    #[test]
    #[should_panic(expected = "bias.len()")]
    fn grn_spec_new_rejects_mis_sized_bias() {
        GrnSpec::new(2, vec![1, 2, 3, 4], vec![5, 6], vec![7], 3, 8, 0, 0, vec![9, 10]);
    }

    #[test]
    #[should_panic(expected = "initial.len()")]
    fn grn_spec_new_rejects_mis_sized_initial() {
        GrnSpec::new(2, vec![1, 2, 3, 4], vec![5, 6], vec![7, 8], 3, 8, 0, 0, vec![9]);
    }

    /// Symmetric bistable toggle-switch fixture (self-activation + mutual inhibition): from the two
    /// extreme corner initial states it settles into two DISTINCT fixed points. Values tuned (and
    /// verified by `bistable_fixture_actually_converges_within_budget`) so the accumulator saturates
    /// the σ-LUT well inside `max_steps`.
    fn bistable_spec(initial: Vec<i32>) -> GrnSpec {
        const SELF: i32 = 64;
        const CROSS: i32 = 64;
        GrnSpec {
            n_genes: 2,
            weights: vec![SELF, -CROSS, -CROSS, SELF],
            input_weights: vec![0, 0],
            bias: vec![0, 0],
            shift: 3,
            max_steps: 12,
            sample_x: 0,
            sample_z: 0,
            initial,
            gene_ids: vec![0, 1],
            dup_counter: 0,
        }
    }

    fn fixture_gradient() -> Gradient {
        flat_gradient(4, 0) // input_weights=0 in the bistable fixture ⇒ gradient value is irrelevant
    }

    // ── LUT provenance ───────────────────────────────────────────────────────────────────────────

    #[test]
    fn lut_checksum_matches_committed() {
        let mut hasher = Sha256::new();
        for v in grn_lut::SIGMA_LUT {
            hasher.update(v.to_le_bytes());
        }
        let digest = hasher.finalize();
        assert_eq!(format!("{digest:x}"), grn_lut::SIGMA_LUT_SHA256, "SIGMA_LUT drifted from its committed checksum");
    }

    #[test]
    fn lut_is_nonnegative_and_monotone_saturating() {
        assert_eq!(grn_lut::SIGMA_LUT[0], 0, "far-negative preact must floor to 0 (non-negative range)");
        assert_eq!(grn_lut::SIGMA_LUT[511], EXPR_MAX as i16, "far-positive preact must saturate to EXPR_MAX");
        assert!(grn_lut::SIGMA_LUT.iter().all(|&v| (0..=EXPR_MAX as i16).contains(&v)), "σ must stay in [0, EXPR_MAX]");
        assert!(grn_lut::SIGMA_LUT.windows(2).all(|w| w[0] <= w[1]), "σ must be monotone non-decreasing");
    }

    // ── determinism teeth (mirror E-2) ───────────────────────────────────────────────────────────

    #[test]
    fn grn_is_deterministic_across_repeated_calls() {
        let (grad, spec) = (fixture_gradient(), bistable_spec(vec![EXPR_MAX, 0]));
        let a = grn_resolve(&grad, &spec);
        let b = grn_resolve(&grad, &spec);
        assert_eq!(a, b, "same (gradient, spec) must produce byte-identical resolution");
    }

    #[test]
    fn grn_reproduces_bytewise_on_rerun() {
        let (grad, spec) = (fixture_gradient(), bistable_spec(vec![EXPR_MAX, 0]));
        let run_a: Vec<u8> = grn_resolve(&grad, &spec).0.iter().flat_map(|v| v.to_le_bytes()).collect();
        let run_b: Vec<u8> = grn_resolve(&grad, &spec).0.iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(run_a, run_b, "re-run must reproduce byte-for-byte");
    }

    #[test]
    fn bistable_fixture_actually_converges_within_budget() {
        // Proves the fallback (budget-exhausted) branch is never hit by this fixture — both corner
        // initial states close a cycle (here: a genuine fixed point) well inside max_steps=12.
        let grad = fixture_gradient();
        let (_s, _ct, steps_a) = grn_resolve(&grad, &bistable_spec(vec![EXPR_MAX, 0]));
        let (_s2, _ct2, steps_b) = grn_resolve(&grad, &bistable_spec(vec![0, EXPR_MAX]));
        assert!(steps_a < 12, "must converge to a fixed point before exhausting the step budget");
        assert!(steps_b < 12, "must converge to a fixed point before exhausting the step budget");
    }

    // ── genuine multistability (the non-tautological mechanism tooth, F1) ───────────────────────

    #[test]
    fn multistability_two_initial_states_settle_to_distinct_attractors() {
        let grad = fixture_gradient();
        let (_state_a, type_a, _) = grn_resolve(&grad, &bistable_spec(vec![EXPR_MAX, 0]));
        let (_state_b, type_b, _) = grn_resolve(&grad, &bistable_spec(vec![0, EXPR_MAX]));
        assert_ne!(type_a, type_b, "≥2 initial states at ONE fixed gradient must settle into ≥2 distinct attractors");
        assert_eq!(type_a, CellType::A);
        assert_eq!(type_b, CellType::B);
    }

    #[test]
    fn multistability_positional_readout_from_fixed_initial_state() {
        // A separate fixture where the gradient genuinely drives the dynamics (nonzero input_weights,
        // symmetric matrix — no self-reinforcement, so the SAME fixed initial state resolves purely by
        // which way the input tips it): two different gradient concentrations select different types.
        let spec = || GrnSpec {
            n_genes: 2,
            weights: vec![0, 0, 0, 0],
            input_weights: vec![32, -32],
            bias: vec![0, 0],
            shift: 3,
            max_steps: 4,
            sample_x: 0,
            sample_z: 0,
            initial: vec![EXPR_MAX / 2, EXPR_MAX / 2], // fixed, neutral start
            gene_ids: vec![0, 1],
            dup_counter: 0,
        };
        let hi = grn(&flat_gradient(2, sample_value_hi()), &spec());
        let lo = grn(&flat_gradient(2, sample_value_lo()), &spec());
        assert_ne!(hi, lo, "different gradient positions must select different cell types from the SAME initial state");
    }

    fn sample_value_hi() -> i32 {
        4096
    }
    fn sample_value_lo() -> i32 {
        -4096
    }

    // ── oscillation / non-convergence tooth (critic F2/F7 — the blocker) ────────────────────────

    /// Positive symmetric cross-coupling with a strong negative bias (no self term): each gene reads
    /// the OTHER gene's OLD value, so the pair SWAPS every step instead of settling — verified (below)
    /// to be an EXACT period-2 cycle from step 1: `[256,0] → [5,251] → [251,5] → [5,251] → …`. The
    /// negative bias is what breaks the naive drift-to-a-single-fixed-point failure mode: a `0` input
    /// alone would sit at the LUT midpoint (a soft, decaying signal); biasing it deep negative first
    /// forces genuine two-state alternation instead.
    fn oscillating_spec() -> GrnSpec {
        GrnSpec {
            n_genes: 2,
            weights: vec![0, 64, 64, 0],
            input_weights: vec![0, 0],
            bias: vec![-8192, -8192],
            shift: 3,
            max_steps: 16,
            sample_x: 0,
            sample_z: 0,
            initial: vec![EXPR_MAX, 0],
            gene_ids: vec![0, 1],
            dup_counter: 0,
        }
    }

    #[test]
    fn oscillating_spec_resolves_same_celltype_regardless_of_phase() {
        let spec = oscillating_spec();
        let grad = fixture_gradient();
        let (resolved_a, ct_a, steps_a) = grn_resolve(&grad, &spec);

        // Confirm this fixture actually cycles (period > 1): the resolved state must NOT equal the
        // immediately-preceding raw trajectory's single-step continuation (a period-1 "fixed point"
        // would make this tooth vacuous — assert the mechanism is exercised, not just re-checked).
        let mut probe_spec = spec.clone();
        probe_spec.max_steps = steps_a + 4; // run further past the first detected repeat
        let (resolved_b, ct_b, steps_b) = grn_resolve(&grad, &probe_spec);

        assert_eq!(resolved_a, resolved_b, "N vs a larger N past the same cycle must resolve to the SAME state");
        assert_eq!(ct_a, ct_b, "N vs a larger N past the same cycle must resolve to the SAME cell type");
        assert!(steps_b >= steps_a, "sanity: the longer run's detection step must not be earlier");
    }

    #[test]
    fn oscillating_fixture_genuinely_cycles_not_a_fixed_point() {
        // Direct proof the fixture used above is a REAL oscillator: step the raw dynamics twice from
        // the initial state and confirm state(1) != state(2) actually differ then re-agree at period 2
        // (a fixed point would have state(1) == state(2) immediately).
        let spec = oscillating_spec();
        let drive = fixture_gradient().at(0, 0) as i64;
        let s1 = step(&spec.initial, drive, &spec);
        let s2 = step(&s1, drive, &spec);
        let s3 = step(&s2, drive, &spec);
        assert_ne!(s1, s2, "the oscillating fixture must actually move (not already a fixed point)");
        assert_eq!(s1, s3, "the oscillating fixture must have period exactly 2 (state repeats at t+2)");
    }

    // ── overflow: saturate + still deterministic under saturation (critic F6) ───────────────────

    #[test]
    fn overflow_saturates_deterministically() {
        // Weights near i32::MAX force every accumulator far past the documented bound every step —
        // the clamp fires every single step, and the resolved cell-type must STILL be deterministic
        // across repeated calls (not merely "the raw accumulator saturates" in isolation).
        let spec = GrnSpec {
            n_genes: 2,
            weights: vec![i32::MAX, -i32::MAX, -i32::MAX, i32::MAX],
            input_weights: vec![i32::MAX, i32::MAX],
            bias: vec![i32::MAX, i32::MAX],
            shift: 1,
            max_steps: 10,
            sample_x: 0,
            sample_z: 0,
            initial: vec![EXPR_MAX, 0],
            gene_ids: vec![0, 1],
            dup_counter: 0,
        };
        let grad = flat_gradient(2, i32::MAX);
        let a = grn_resolve(&grad, &spec);
        let b = grn_resolve(&grad, &spec);
        assert_eq!(a, b, "saturating dynamics must still resolve a deterministic cell type");
        assert!(a.2 <= spec.max_steps, "must terminate within the step cap even while saturating every step");
    }

    #[test]
    fn overflow_bound_clamps_not_wraps_at_the_step_level() {
        // Direct proof at the `step` level: an adversarial (weight, state) pair forces the raw
        // pre-clamp sum far past the documented bound; the result must land exactly at the sigma of
        // the clamped boundary, not at some wrapped/aliased value.
        let spec = GrnSpec {
            n_genes: 1,
            weights: vec![i32::MAX],
            input_weights: vec![i32::MAX],
            bias: vec![i32::MAX],
            shift: 0,
            max_steps: 1,
            sample_x: 0,
            sample_z: 0,
            initial: vec![EXPR_MAX],
            gene_ids: vec![0],
            dup_counter: 0,
        };
        let out = step(&[EXPR_MAX], i32::MAX as i64, &spec);
        // The clamped accumulator is `acc_bound(1)` (positive ceiling) — sigma of that (deep positive)
        // must saturate to EXPR_MAX, never an aliased small/negative value from a wrapped cast.
        assert_eq!(out[0], EXPR_MAX, "must saturate to the sigma of the clamped ceiling, not wrap");
    }

    // ── gradient dependence control + golden vector ──────────────────────────────────────────────

    #[test]
    fn grn_ignores_gradient_when_input_weights_are_zero() {
        let spec = bistable_spec(vec![EXPR_MAX, 0]);
        assert_eq!(spec.input_weights, vec![0, 0]);
        let a = grn(&flat_gradient(4, 0), &spec);
        let b = grn(&flat_gradient(4, 999_999), &spec);
        assert_eq!(a, b, "with input_weights=0 the gradient must not affect the result");
    }

    #[test]
    fn grn_depends_on_gradient_when_input_weights_are_nonzero() {
        let spec = GrnSpec {
            n_genes: 2,
            weights: vec![0, 0, 0, 0],
            input_weights: vec![32, -32],
            bias: vec![0, 0],
            shift: 3,
            max_steps: 4,
            sample_x: 0,
            sample_z: 0,
            initial: vec![EXPR_MAX / 2, EXPR_MAX / 2],
            gene_ids: vec![0, 1],
            dup_counter: 0,
        };
        let a = grn(&flat_gradient(2, 4096), &spec);
        let b = grn(&flat_gradient(2, -4096), &spec);
        assert_ne!(a, b, "with nonzero input_weights the gradient must affect the result");
    }

    #[test]
    fn golden_vector_matches_pinned_resolution() {
        let (state, ct, steps) = grn_resolve(&fixture_gradient(), &bistable_spec(vec![EXPR_MAX, 0]));
        // Pinned on this implementation (integer, deterministic).
        assert_eq!(state, vec![256, 0]);
        assert_eq!(ct, CellType::A);
        assert_eq!(steps, 1);
    }

    // ── morphogen → GRN integration smoke (critic F5) ────────────────────────────────────────────

    #[test]
    fn morphogen_to_grn_chain_is_deterministic_and_reaches_distinct_types() {
        let morph_spec = MorphogenSpec {
            g_dev: 4,
            n_dev: 8,
            boundary: Boundary::Reflecting,
            diffuse_shift: 3,
            decay_num: 1,
            decay_shift: 4,
            seed_scale: 4096,
            stop_threshold: 0,
            apoptosis_threshold: None,
            germ_threshold: None,
            supply_source: None,
        };
        // Gene 0 reads the sampled concentration directly; gene 1 is a FIXED threshold reference (no
        // input coupling) tuned (verified empirically) to sit strictly between the near-source and
        // far-source sigma outputs — the morphogen's `Gradient` is a non-negative CONCENTRATION field
        // (unlike the signed synthetic fixtures above), so classification here compares MAGNITUDE
        // against a reference rather than sign against zero.
        let grn_spec_at = |x: usize, z: usize| GrnSpec {
            n_genes: 2,
            weights: vec![0, 0, 0, 0],
            input_weights: vec![1, 0],
            bias: vec![0, 2048],
            shift: 3,
            max_steps: 4,
            sample_x: x,
            sample_z: z,
            initial: vec![EXPR_MAX / 2, EXPR_MAX / 2],
            gene_ids: vec![0, 1],
            dup_counter: 0,
        };

        let gradient = morphogen(&Genome::founder(1), &morph_spec);

        // (a) terminates + (b) deterministic across repeated runs of the SAME chain.
        let a = grn_resolve(&gradient, &grn_spec_at(0, 0));
        let b = grn_resolve(&gradient, &grn_spec_at(0, 0));
        assert_eq!(a, b, "the morphogen→GRN chain must be deterministic across repeated runs");
        assert!(a.2 <= grn_spec_at(0, 0).max_steps, "the chain must terminate within the step cap");

        // (c) ≥2 distinct cell types across ≥2 different sample positions on the SAME gradient (the
        // morphogen's monotonic decay from (0,0) guarantees a concentration difference to read).
        let near = grn(&gradient, &grn_spec_at(0, 0));
        let far = grn(&gradient, &grn_spec_at(3, 3));
        assert_ne!(near, far, "sampling different positions on the SAME gradient must reach distinct cell types");
    }

    #[test]
    fn morphogen_to_grn_chain_never_touches_decode_or_production() {
        // Structural sanity: this test file only imports morphogen()/grn() — a production-path caller
        // would import Genome::decode and the ECS spawn machinery, neither of which appears here.
        let spec = MorphogenSpec {
            g_dev: 4,
            n_dev: 4,
            boundary: Boundary::Absorbing,
            diffuse_shift: 3,
            decay_num: 1,
            decay_shift: 4,
            seed_scale: 2048,
            stop_threshold: 1,
            apoptosis_threshold: None,
            germ_threshold: None,
            supply_source: None,
        };
        let gradient = morphogen(&Genome::founder(1), &spec);
        let _ = grn(&gradient, &bistable_spec(vec![EXPR_MAX, 0]));
    }

    // ── V-3-a: gene-id infrastructure (lineage, injectivity, determinism) ────────────────────────

    /// V-3-a: Base genes must have injective, dense ids in the range 0..n_genes-1.
    #[test]
    fn v3a_gene_ids_are_dense_base_range() {
        let spec = bistable_spec(vec![EXPR_MAX, 0]);
        assert_eq!(spec.n_genes, 2, "bistable fixture must have n_genes=2");
        assert_eq!(spec.gene_ids, vec![0, 1], "base genes must have dense ids 0..n_genes-1");
    }

    /// V-3-a: Gene-ids are injective (no duplicates). In V-3-a, all genomes are founders,
    /// so ids = 0..n_genes-1 exactly, which is injective by construction.
    #[test]
    fn v3a_gene_ids_are_injective_no_duplicates() {
        let spec = GrnSpec::new(
            4,
            vec![1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1],
            vec![0, 0, 0, 0],
            vec![0, 0, 0, 0],
            3,
            8,
            0,
            0,
            vec![128, 128, 128, 128],
        );
        assert_eq!(spec.gene_ids.len(), 4, "gene_ids must have length n_genes");
        let mut sorted = spec.gene_ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            spec.gene_ids.len(),
            "gene_ids must be unique (injective)"
        );
        assert_eq!(
            spec.gene_ids,
            vec![0, 1, 2, 3],
            "base genes must be exactly 0..n_genes-1 in order"
        );
    }

    /// V-3-a: dup_counter defaults to 0 for all founders/existing genomes. Incremented only by
    /// V-3-b duplication operator; V-3-a touches nothing.
    #[test]
    fn v3a_dup_counter_defaults_to_zero() {
        let spec1 = bistable_spec(vec![EXPR_MAX, 0]);
        assert_eq!(spec1.dup_counter, 0, "founder genome must have dup_counter=0 (no duplications yet)");

        let spec2 = GrnSpec::new(
            3,
            vec![0; 9],
            vec![0; 3],
            vec![0; 3],
            3,
            8,
            0,
            0,
            vec![100; 3],
        );
        assert_eq!(
            spec2.dup_counter, 0,
            "any newly-constructed spec must have dup_counter=0"
        );
    }

    /// V-3-a: Gene-ids are reproducible — the same n_genes value produces the same id sequence
    /// across multiple builds/runs (deterministic, not dependent on RNG or runtime state).
    #[test]
    fn v3a_gene_ids_are_reproducible_from_n_genes() {
        let spec_a = GrnSpec::new(
            5,
            vec![0; 25],
            vec![0; 5],
            vec![0; 5],
            3,
            8,
            0,
            0,
            vec![100; 5],
        );
        let spec_b = GrnSpec::new(
            5,
            vec![1; 25],
            vec![2; 5],
            vec![3; 5],
            4,
            16,
            1,
            2,
            vec![50; 5],
        );
        assert_eq!(
            spec_a.gene_ids, spec_b.gene_ids,
            "two specs with the same n_genes must have identical gene_ids (recomputable)"
        );
        assert_eq!(spec_a.gene_ids, vec![0, 1, 2, 3, 4], "gene_ids are determined solely by n_genes");
    }

    /// V-3-a: Future V-3-b duplication encoding is injective. This is a proof-of-concept:
    /// we demonstrate that a synthetic duplication scheme (parent_id, dup_index) can be encoded
    /// injectively. V-3-a uses it nowhere (no operator yet); V-3-b will use it.
    /// The encoding: base genes stay as 0..n_genes-1. A duplicated gene from parent p with
    /// dup_event_counter d encodes as ((p + 1) << 16) | d, which is injective:
    /// - Base genes occupy [0, n_genes), always ≤ 2^16 (well under 1 << 16)
    /// - Duplication ids occupy [1 << 16, ∞), starting where base genes cannot reach
    /// - Within each parent's dup_index space, ids are unique because parent is encoded in high bits
    #[test]
    fn v3a_future_duplication_encoding_is_injective() {
        // Base genes n_genes=2: ids are 0, 1
        let base_0: u32 = 0;
        let base_1: u32 = 1;

        // Synthetic future duplications (V-3-b will compute these; V-3-a only demonstrates injectivity):
        // parent=0, dup_event_counters 1..5 => ids ((0+1) << 16) | 1..5 = 65536+1..65536+5
        // parent=1, dup_event_counters 1..5 => ids ((1+1) << 16) | 1..5 = 131072+1..131072+5
        let dup_from_parent_0: Vec<u32> = (1..=5).map(|d| ((0 + 1) << 16) | d).collect();
        let dup_from_parent_1: Vec<u32> = (1..=5).map(|d| ((1 + 1) << 16) | d).collect();

        let mut all_ids = vec![base_0, base_1];
        all_ids.extend(&dup_from_parent_0);
        all_ids.extend(&dup_from_parent_1);

        let mut sorted = all_ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            all_ids.len(),
            "all synthetic ids must be unique (injective encoding)"
        );

        // Verify the ranges don't overlap:
        // Base: [0, 2)
        // Dup from parent 0: [65536+1, 65536+6)
        // Dup from parent 1: [131072+1, 131072+6)
        assert!(
            dup_from_parent_0
                .iter()
                .all(|id| *id >= (1 << 16) && *id < (2 << 16)),
            "dup_from_parent_0 ids must be in distinct high-bit range"
        );
        assert!(
            dup_from_parent_1
                .iter()
                .all(|id| *id >= (2 << 16) && *id < (3 << 16)),
            "dup_from_parent_1 ids must be in even higher range"
        );
    }
}
