//! Phase-2 **P-1**: the predation SUBSTRATE — a standalone, deterministic, INTEGER
//! encounter-resolution routine that resolves predator↔prey interactions to a conservation-exact
//! [`Outcome`]. **Prod-inert**: nothing here is called by `Genome::decode`, any spawn site, or any
//! stage (0 sites changed); it exists here proven by unit tests over a production [`PredationSpec`]
//! fixture, so P-2 (the WIRE) reuses the types without a rewrite, mirroring E-2's `MorphogenSpec`
//! lesson (F9).
//!
//! **Determinism and conservation (plan §3, R15).** The [`resolve_encounter`] function is pure:
//! same inputs → byte-identical [`Outcome`], no interior mutability, no RNG, no wall-clock, no float.
//! Integer arithmetic SATURATES (never wraps — a wrap silently aliases; a saturate stays bounded/
//! detectable, per `morphogen.rs`). The load-bearing invariant is **`predator_gain + dissipated ==
//! prey_loss`** (exact integer — energy is conserved, only moved and dissipated) AND **`prey_loss ≤
//! prey.energy`** (no energy from nothing, even at predator starvation). This is what makes the future
//! wire R15-safe by construction.
//!
//! **Stand-in combat trait.** The predator's combat strength is read from `&Genome` via the
//! documented STAND-IN trait `genome.size`, exactly as E-2 used `size` so the function genuinely
//! consumes `&Genome`; the real semantic genome→combat-trait mapping is P-2's job. Prey's energy and
//! the spec define how much is taken and what fraction is dissipated vs gained.
// Guard: no float arithmetic in the predation substrate (mirrors energy.rs/genome.rs/morphogen.rs).
// CI runs nextest, not clippy — the PR notes a clean local `cargo clippy -p sim-core` run.
#![deny(clippy::float_arithmetic)]

use crate::Genome;

/// Production configuration for predation encounters — integer constants defining bite scale, combat
/// trait influence, and metabolic efficiency. NOT `#[cfg(test)]`: P-1 instantiates this with a test
/// *value*; P-2 reuses the *type* unchanged when it wires predation into the stage loop (mirrors
/// E-2's `MorphogenSpec` pattern, F9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PredationSpec {
    /// Bite size base scaling: how much prey energy becomes available to bite, as a right-shift
    /// of prey energy. E.g. `bite_shift=2` means the base bite is `prey_energy >> 2` (1/4 of prey).
    /// Range: 0..=10 (0 ⇒ can bite entire prey; 10 ⇒ tiny bites). Integer semantics: CFL-safe
    /// (never produces NaN or division by zero; saturating arithmetic prevents overflow).
    pub bite_shift: u32,

    /// Combat trait influence scale: how much the predator's combat trait (genome.size) amplifies
    /// the base bite. Formulation: `bite = (base_bite * (256 + combat_trait_scale * trait)) >> 8`,
    /// where `trait = predator.size` clamped to `[0, 256]`. Allows both positive (larger predator →
    /// bigger bite) and zero (all bites equal regardless of predator size). Range: 0..=16 (scaling
    /// factor applied before the >>8 shift; realistic combat strength modulation).
    pub combat_trait_scale: i32,

    /// Metabolic efficiency of predation: fraction of taken energy the predator actually gains,
    /// the rest is dissipated. Formulation: `dissipated = (bite * (256 - efficiency_num)) >> 8`,
    /// `gain = bite - dissipated`. This mirrors the feeding inefficiency in the existing energy
    /// economy (R13/R15). Range: 1..=256 (1 = almost all dissipated, 256 = 100% efficient).
    pub efficiency_num: i32,

    /// D-1: gated per-prey size-refuge (Boraas mechanism) — large multicellular prey bodies are
    /// harder to capture. `None` (every shipped config): bite unchanged, byte-identical to P-2a.
    /// `Some(spec)`: the bite is scaled down by `spec` as a function of the PREY'S OWN body size
    /// (see [`SizeRefugeSpec`] / [`resolve_encounter`]). This is the wire only — no shipped config
    /// turns it on yet (D-2).
    pub size_refuge: Option<SizeRefugeSpec>,
}

/// D-1: per-prey size-refuge parameters (Boraas mechanism) — larger prey bodies get a
/// monotonically smaller bite. Q-format fixed-point, integer-only, no float (mirrors
/// `PredationSpec`'s `>>8` combat-trait Q-format).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SizeRefugeSpec {
    /// Fixed-point shift `S` for the refuge Q-format: `bite_eff = (bite << S) / ((1 << S) +
    /// refuge_k * prey_body_size)`. Larger `shift` gives finer-grained refuge scaling. Documented
    /// range: 0..=16 (mirrors `combat_trait_scale`'s Q8 magnitude); defensively capped at 32
    /// inside `resolve_encounter` so `bite << shift` cannot overflow `i64` even on a misconfigured
    /// spec (bite is bounded by `VALUE_MAX`).
    pub shift: u32,
    /// Refuge strength `k`: how strongly a unit of prey body size shrinks the bite. `k=0` ⇒ the
    /// refuge denominator is always `1 << shift` ⇒ bite unchanged regardless of body size (a
    /// `Some` spec with `k=0` is inert, distinct from `size_refuge=None`). Larger `k` ⇒ a given
    /// body size shrinks the bite more. Expected non-negative for the monotone-decreasing
    /// property (Boraas: bigger body → smaller bite) to hold.
    pub refuge_k: i32,
}

/// The outcome of a single predator↔prey encounter under a [`PredationSpec`]. All three fields are
/// non-negative and satisfy the conservation invariant: `predator_gain + dissipated == prey_loss`
/// (exact integer) and `prey_loss ≤ prey.energy` (no energy from nothing).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Energy the predator gains from this encounter (saturated, never negative or wraps).
    pub predator_gain: i64,
    /// Energy lost by the prey (clamped to prey.energy, never exceeds what the prey has).
    pub prey_loss: i64,
    /// Energy dissipated (metabolic cost of predation); combined with `predator_gain` nets to
    /// `prey_loss` via the conservation invariant.
    pub dissipated: i64,
}

/// Concentration ceiling for intermediate calculations (i64 accumulator). The overflow guard below
/// saturates per-step accumulators to this bound before narrowing — never a raw `as i32` truncation
/// (which would silently WRAP on out-of-range input; see E-2's `overflow_saturates_not_wraps` test).
/// This is a conservative upper bound on realistic predation energy transfers (no organism can be
/// larger than a few thousand energy units in Ф0 ecology; 1M safely bounds any accumulator).
const VALUE_MAX: i64 = 1_000_000;

/// Pure integer predation encounter resolution. Given a predator (energy + combat trait via
/// `&Genome`), a prey (energy + body size), and a `spec`, returns the conservation-exact
/// [`Outcome`].
///
/// **Signature:** reads predator's `combat_trait = predator.size` from `&Genome` (a documented
/// stand-in; P-2 will map the real semantic combat trait); reads prey's raw `i64` energy and its
/// `i64` body size (D-1: `Σ Phenotype.graph.module_cell_count`, clamped `≥1` by the caller — a
/// non-positive `prey_body_size` here is clamped again defensively). All inputs are small, cheap
/// to read. The function is pure: no RNG, no state, no clock. Same inputs → byte-identical
/// `Outcome`.
///
/// **Invariants (load-bearing for R15):**
/// - `outcome.predator_gain + outcome.dissipated == outcome.prey_loss` (exact integer conservation)
/// - `outcome.prey_loss ≤ prey_energy` (prey never loses more than it has)
/// - All three fields ≥ 0 (no negative energy transfers; saturating arithmetic prevents wraps)
///
/// **D-1 size-refuge (gated).** `spec.size_refuge = None` (every shipped config): `prey_body_size`
/// is unused, bite unchanged — byte-identical to P-2a. `Some(refuge)`: the refuge scales the
/// PRE-CLAMP bite (after combat-trait scaling, before the `.min(prey_energy)` death-cap), so the
/// conservation invariants above still hold ∀ inputs: `bite_eff = (bite << shift) / ((1 << shift)
/// + refuge_k * prey_body_size)` — larger `prey_body_size` ⇒ smaller `bite_eff` (monotone).
///
/// **Overflow behavior.** Intermediate accumulators (e.g., `bite * trait_factor`) are computed in
/// `i64` and clamped to [`VALUE_MAX`] before narrowing. The refuge division widens to `i128` (still
/// integer, still deterministic) so a large `shift` cannot overflow before narrowing back to `i64`.
/// Saturating semantics ensure the result is detectable (stays within bounds) rather than silently
/// aliasing to wrong values.
pub fn resolve_encounter(
    predator: &Genome,
    prey_energy: i64,
    prey_body_size: i64,
    spec: &PredationSpec,
) -> Outcome {
    // Clamp prey energy to valid range (should already be non-negative, but guard against it).
    let prey_energy = prey_energy.clamp(0, VALUE_MAX);

    // Compute base bite: prey_energy >> bite_shift. At bite_shift=0, base_bite ≈ prey_energy;
    // at bite_shift=10, tiny bites.
    let base_bite: i64 = prey_energy >> spec.bite_shift;

    // Apply combat trait influence: larger predator (larger genome.size) → bigger bite.
    // Formula: bite = (base_bite * (256 + combat_trait_scale * trait)) >> 8
    // where trait = predator.size ∈ [0, 256]. This is a saturating multiply-accumulate in i64
    // to prevent intermediate overflow (worst case: base_bite ≈ VALUE_MAX, trait_factor ≈ 256 + 16*256 = 4352 ⇒ wide ≈ 4.3B, ≪ i64::MAX).
    let trait_val = (predator.size as i64).clamp(0, 256);
    let trait_factor: i64 = 256i64 + (spec.combat_trait_scale as i64 * trait_val);
    let bite_wide: i64 = (base_bite * trait_factor) >> 8;
    let bite = bite_wide.clamp(0, VALUE_MAX);

    // D-1: gated per-prey size-refuge. `None` ⇒ bite unchanged (byte-identical to P-2a).
    // `Some` ⇒ scale the PRE-CLAMP bite by the prey's own body size (larger body → smaller bite).
    let bite = match spec.size_refuge {
        Some(refuge) => {
            // Defensive cap on `shift`: bite ≤ VALUE_MAX (1e6), so `shift ≤ 32` keeps the i128
            // numerator far below i128::MAX with headroom to spare — a misconfigured spec cannot
            // overflow this widened arithmetic.
            let shift = refuge.shift.min(32);
            let body = prey_body_size.max(1) as i128; // defensive re-clamp (caller already clamps ≥1)
            let k = refuge.refuge_k as i128;
            let numer: i128 = (bite as i128) << shift;
            let denom: i128 = ((1i128) << shift) + k * body;
            let denom = denom.max(1); // guard non-positive denominator (misconfigured negative k)
            (numer / denom).clamp(0, VALUE_MAX as i128) as i64
        }
        None => bite,
    };

    // Clamp bite to what prey has available (prey cannot lose more than it carries).
    let actual_bite = bite.min(prey_energy);

    // Compute energy dissipated: (actual_bite * (256 - efficiency_num)) >> 8.
    // efficiency_num=1 ⇒ 255/256 dissipated (almost all lost).
    // efficiency_num=256 ⇒ 0/256 dissipated (100% efficient, all to predator).
    let efficiency_clamped = (spec.efficiency_num as i64).clamp(1, 256);
    let dissipation_frac: i64 = 256 - efficiency_clamped;
    let dissipated_wide: i64 = (actual_bite * dissipation_frac) >> 8;
    let dissipated = dissipated_wide.clamp(0, actual_bite); // dissipated cannot exceed bite

    // Predator gain: what remains of the bite after dissipation.
    let predator_gain = (actual_bite - dissipated).max(0);

    // Assemble outcome: prey_loss = actual_bite (what the prey gave up).
    Outcome {
        predator_gain,
        prey_loss: actual_bite,
        dissipated,
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::Genome;

    // ── Production fixture ───────────────────────────────────────────────────────────────────────

    /// P-1 fixture — a realistic predation spec for Ф2 ecology. Balances bite scale with
    /// predator size influence and metabolic efficiency. These values are what P-2 would plausibly
    /// wire into production.
    fn prod_spec() -> PredationSpec {
        PredationSpec {
            bite_shift: 3,             // base bite ≈ prey_energy / 8
            combat_trait_scale: 1,     // moderate trait influence
            efficiency_num: 160,       // ~62% efficiency (160/256)
            size_refuge: None,         // D-1: gated off — byte-identical to pre-D-1
        }
    }

    /// D-1 fixture — `prod_spec()` with the size-refuge gate turned on, for the refuge-specific
    /// teeth below. `shift=8, refuge_k=1` — moderate refuge strength.
    fn prod_spec_with_refuge(shift: u32, refuge_k: i32) -> PredationSpec {
        let mut spec = prod_spec();
        spec.size_refuge = Some(SizeRefugeSpec { shift, refuge_k });
        spec
    }

    fn predator_genome(size: i32) -> Genome {
        let mut g = Genome::founder(1);
        g.size = size.clamp(1, 32);
        g
    }

    fn prey_energy(eu: i64) -> i64 {
        eu.max(0)
    }

    // ── Conservation invariant: the load-bearing tooth (R15) ────────────────────────────────────

    #[test]
    fn conservation_invariant_holds_across_grid() {
        // Test the conservation invariant across the FULL input grid as specified in ТЗ.
        // Predator sizes 1..32 (all reasonable combat strengths), prey energies 0..10000
        // (including edge cases and realistic Ф0 ecology values).
        let spec = prod_spec();

        // Full sweep: predator sizes span the documented range (1..32 inclusive).
        for predator_size in 1..=32 {
            // Prey energies: 0 (edge), then logarithmic sampling to cover 1..10000.
            // This ensures both dense coverage near edges and reasonable step size for large values.
            let prey_energies: Vec<i64> = (0..=100)
                .map(|i| {
                    if i == 0 { 0 }
                    else if i <= 10 { i as i64 * 100 }     // 100..1000 by 100
                    else if i <= 20 { (i - 10) as i64 * 1000 + 1000 }  // 2000..11000 by 1000 (clamp to 10000)
                    else { 10000 }
                })
                .filter(|&e| e <= 10000)
                .collect();

            for prey_eu in prey_energies {
                let pred = predator_genome(predator_size as i32);
                let outcome = resolve_encounter(&pred, prey_eu, 1, &spec);

                // Invariant 1: predator_gain + dissipated == prey_loss (exact integer, R15)
                assert_eq!(
                    outcome.predator_gain + outcome.dissipated,
                    outcome.prey_loss,
                    "conservation invariant violated for predator.size={}, prey_eu={}: gain={} + dissipated={} != loss={}",
                    predator_size, prey_eu, outcome.predator_gain, outcome.dissipated, outcome.prey_loss
                );

                // Invariant 2: prey_loss <= prey_energy (no energy from nothing)
                assert!(
                    outcome.prey_loss <= prey_eu,
                    "prey_loss > prey_energy for predator.size={}, prey_eu={}: loss={} > energy={}",
                    predator_size, prey_eu, outcome.prey_loss, prey_eu
                );

                // Invariant 3: all three fields >= 0
                assert!(outcome.predator_gain >= 0, "predator_gain < 0: {}", outcome.predator_gain);
                assert!(outcome.prey_loss >= 0, "prey_loss < 0: {}", outcome.prey_loss);
                assert!(outcome.dissipated >= 0, "dissipated < 0: {}", outcome.dissipated);
            }
        }
    }

    // ── Determinism: byte-identical across runs ──────────────────────────────────────────────────

    #[test]
    fn deterministic_across_repeated_calls() {
        let spec = prod_spec();
        let pred = predator_genome(16);
        let prey_eu = 1000i64;

        let out_a = resolve_encounter(&pred, prey_eu, 1, &spec);
        let out_b = resolve_encounter(&pred, prey_eu, 1, &spec);

        assert_eq!(
            out_a, out_b,
            "determinism broken: same inputs produced different outcomes"
        );
    }

    #[test]
    fn reproduces_bytewise_on_rerun() {
        // A second, independent re-run (fresh call stack / fresh allocations) must reproduce the
        // exact same bytes — the computation is deterministic across runs, not just aliases.
        let spec = prod_spec();
        let pred = predator_genome(16);
        let prey_eu = 1000i64;

        let out_a = resolve_encounter(&pred, prey_eu, 1, &spec);
        let bytes_a = (out_a.predator_gain, out_a.prey_loss, out_a.dissipated);

        let out_b = resolve_encounter(&pred, prey_eu, 1, &spec);
        let bytes_b = (out_b.predator_gain, out_b.prey_loss, out_b.dissipated);

        assert_eq!(
            bytes_a, bytes_b,
            "re-run must reproduce byte-for-byte: {:?} vs {:?}",
            bytes_a, bytes_b
        );
    }

    // ── Saturation: i64::MAX boundaries do not wrap or panic ───────────────────────────────────

    #[test]
    fn saturation_at_max_energy_no_wrap() {
        // Test saturation behavior at i64::MAX-adjacent values (adversarial ТЗ check).
        // Prove that no wrapping occurs, even when intermediate accumulators exceed i32/i64 bounds.
        let mut spec = prod_spec();
        spec.bite_shift = 0; // force very large bites to stress accumulator

        let mut pred = predator_genome(32);
        pred.size = 32; // max realistic size

        // Test progression: realistic value, then VALUE_MAX, then i64::MAX boundary.
        let test_prey_energies = vec![
            500_000i64,        // realistic Ф0 value
            VALUE_MAX,         // module's documented ceiling (1M)
            900_000_000i64,    // large value (well before i64::MAX)
            i64::MAX / 2,      // near i64::MAX/2 to test wide accumulator
        ];

        for prey_eu in test_prey_energies {
            let out = resolve_encounter(&pred, prey_eu, 1, &spec);

            // Conservation must hold even at saturation edges (R15).
            assert_eq!(
                out.predator_gain + out.dissipated,
                out.prey_loss,
                "conservation violated at saturation (prey_eu={}): gain={} + dissipated={} != loss={}",
                prey_eu, out.predator_gain, out.dissipated, out.prey_loss
            );

            // No panic, no wrap — results are bounded and sensible.
            assert!(out.predator_gain >= 0, "predator_gain<0 at prey_eu={}: {}", prey_eu, out.predator_gain);
            assert!(out.prey_loss >= 0 && out.prey_loss <= prey_eu, "prey_loss oob at prey_eu={}: {}", prey_eu, out.prey_loss);
            assert!(out.dissipated >= 0, "dissipated<0 at prey_eu={}: {}", prey_eu, out.dissipated);

            // Double-check that clamping to VALUE_MAX worked: output never exceeds prey energy.
            let total_energy = out.predator_gain + out.dissipated;
            assert!(total_energy <= prey_eu, "energy conservation bound violated at prey_eu={}: total={} > prey={}", prey_eu, total_energy, prey_eu);
        }
    }

    // ── Monotonicity: stronger predator → non-decreasing gain ──────────────────────────────────

    #[test]
    fn stronger_predator_trait_non_decreasing_gain() {
        let spec = prod_spec();
        let prey_eu = 1000i64;

        let predator_small = predator_genome(1);
        let predator_medium = predator_genome(16);
        let predator_large = predator_genome(32);

        let out_small = resolve_encounter(&predator_small, prey_eu, 1, &spec);
        let out_medium = resolve_encounter(&predator_medium, prey_eu, 1, &spec);
        let out_large = resolve_encounter(&predator_large, prey_eu, 1, &spec);

        // Stronger predators (larger size) should gain non-decreasing amounts
        // (all else equal, the gain should not decrease as predator strength increases).
        assert!(
            out_small.predator_gain <= out_medium.predator_gain,
            "monotonicity broken: small predator gain {} > medium {}",
            out_small.predator_gain,
            out_medium.predator_gain
        );
        assert!(
            out_medium.predator_gain <= out_large.predator_gain,
            "monotonicity broken: medium predator gain {} > large {}",
            out_medium.predator_gain,
            out_large.predator_gain
        );

        // This is the property that makes predation a selective gradient.
        eprintln!(
            "Predator size scale: small(1)→gain={} medium(16)→gain={} large(32)→gain={}",
            out_small.predator_gain, out_medium.predator_gain, out_large.predator_gain
        );
    }

    // ── Prey-death cap: bite clamped to prey energy, still conservation-exact ──────────────────

    #[test]
    fn prey_death_cap_exact_conservation() {
        let mut spec = prod_spec();
        spec.bite_shift = 0; // aggressive bites

        let pred = predator_genome(32);

        // Small prey: the bite will likely exceed available energy
        let small_prey = 10i64;

        let out = resolve_encounter(&pred, small_prey, 1, &spec);

        // The prey loses at most what it has
        assert!(out.prey_loss <= small_prey, "bite exceeded prey capacity: {} > {}", out.prey_loss, small_prey);

        // Even with the clamp, conservation must hold exactly
        assert_eq!(
            out.predator_gain + out.dissipated,
            out.prey_loss,
            "conservation broken at prey-death boundary: gain={} + dissipated={} != loss={}",
            out.predator_gain, out.dissipated, out.prey_loss
        );
    }

    // ── Efficiency variation: different specs produce plausible results ─────────────────────────

    #[test]
    fn efficiency_parameter_affects_gain_vs_dissipation() {
        let pred = predator_genome(16);
        let prey_eu = 1000i64;

        // Low efficiency: most is dissipated
        let mut spec_low = prod_spec();
        spec_low.efficiency_num = 50; // ~20% efficiency

        // High efficiency: most goes to predator
        let mut spec_high = prod_spec();
        spec_high.efficiency_num = 230; // ~90% efficiency

        let out_low = resolve_encounter(&pred, prey_eu, 1, &spec_low);
        let out_high = resolve_encounter(&pred, prey_eu, 1, &spec_high);

        // Both must conserve
        assert_eq!(
            out_low.predator_gain + out_low.dissipated,
            out_low.prey_loss,
            "low-efficiency outcome not conserved"
        );
        assert_eq!(
            out_high.predator_gain + out_high.dissipated,
            out_high.prey_loss,
            "high-efficiency outcome not conserved"
        );

        // High-efficiency spec must give predator more (or equal) for same prey
        assert!(
            out_high.predator_gain >= out_low.predator_gain,
            "high-efficiency spec gave less gain: {} < {}",
            out_high.predator_gain,
            out_low.predator_gain
        );
    }

    // ── Edge cases: zero energy, zero prey ────────────────────────────────────────────────────

    #[test]
    fn zero_prey_energy_yields_zero_outcome() {
        let spec = prod_spec();
        let pred = predator_genome(32);

        let out = resolve_encounter(&pred, 0, 1, &spec);

        assert_eq!(out.predator_gain, 0);
        assert_eq!(out.prey_loss, 0);
        assert_eq!(out.dissipated, 0);

        // Conservation still holds trivially
        assert_eq!(out.predator_gain + out.dissipated, out.prey_loss);
    }

    #[test]
    fn different_specs_vary_bite_appropriately() {
        let pred = predator_genome(16);
        let prey_eu = 1000i64;

        // Aggressive spec: smaller shift → bigger bites
        let mut spec_aggressive = prod_spec();
        spec_aggressive.bite_shift = 1; // base bite ≈ prey / 2

        // Conservative spec: larger shift → smaller bites
        let mut spec_conservative = prod_spec();
        spec_conservative.bite_shift = 5; // base bite ≈ prey / 32

        let out_agg = resolve_encounter(&pred, prey_eu, 1, &spec_aggressive);
        let out_cons = resolve_encounter(&pred, prey_eu, 1, &spec_conservative);

        // Aggressive spec should result in larger prey loss (or equal)
        assert!(
            out_agg.prey_loss >= out_cons.prey_loss,
            "aggressive spec gave smaller bite: {} < {}",
            out_agg.prey_loss,
            out_cons.prey_loss
        );

        // Both conserve
        assert_eq!(out_agg.predator_gain + out_agg.dissipated, out_agg.prey_loss);
        assert_eq!(out_cons.predator_gain + out_cons.dissipated, out_cons.prey_loss);
    }

    // ── D-1 (#268): per-prey size-refuge teeth ──────────────────────────────────────────────────

    /// `d1_conservation_R15`: re-prove the conservation invariant with the refuge GATE ON, across
    /// a grid of predator size × prey energy × prey body size — the refuge scales the pre-clamp
    /// bite only, so `gain+dissipated==loss ∧ loss≤prey_energy` must still hold exactly ∀ inputs.
    #[test]
    fn d1_conservation_r15() {
        let spec = prod_spec_with_refuge(8, 3);

        for predator_size in [1i32, 8, 16, 32] {
            for prey_eu in [0i64, 1, 100, 1_000, 10_000] {
                for body_size in [1i64, 2, 5, 32, 1_000] {
                    let pred = predator_genome(predator_size);
                    let outcome = resolve_encounter(&pred, prey_eu, body_size, &spec);

                    assert_eq!(
                        outcome.predator_gain + outcome.dissipated,
                        outcome.prey_loss,
                        "R15 broken (refuge on) at size={predator_size}, prey_eu={prey_eu}, body={body_size}: \
                         gain={} + dissipated={} != loss={}",
                        outcome.predator_gain, outcome.dissipated, outcome.prey_loss
                    );
                    assert!(
                        outcome.prey_loss <= prey_eu,
                        "prey_loss > prey_energy (refuge on) at size={predator_size}, prey_eu={prey_eu}, body={body_size}: {} > {}",
                        outcome.prey_loss, prey_eu
                    );
                    assert!(outcome.predator_gain >= 0 && outcome.prey_loss >= 0 && outcome.dissipated >= 0);
                }
            }
        }
    }

    /// `d1_refuge_monotone`: larger `prey_body_size` → strictly smaller (or equal, at saturation)
    /// `bite_eff`, i.e. `prey_loss` is non-increasing in body size, and strictly decreasing over
    /// the un-saturated range (Boraas: bigger body → harder to capture).
    #[test]
    fn d1_refuge_monotone() {
        let spec = prod_spec_with_refuge(8, 2);
        let pred = predator_genome(16);
        let prey_eu = 10_000i64; // large enough that the bite doesn't hit the prey-energy cap

        let bodies = [1i64, 2, 4, 8, 16, 32, 64, 128];
        let losses: Vec<i64> = bodies
            .iter()
            .map(|&b| resolve_encounter(&pred, prey_eu, b, &spec).prey_loss)
            .collect();

        for w in losses.windows(2) {
            assert!(
                w[0] > w[1],
                "refuge must be strictly monotone-decreasing in body size: losses={:?} (bodies={:?})",
                losses, bodies
            );
        }
    }

    /// `d1_none_inert`: `size_refuge=None` reproduces the exact pre-D-1 (P-2a) `Outcome` for a
    /// grid of inputs, regardless of what `prey_body_size` is passed — the gate makes the new
    /// parameter dead weight when off.
    #[test]
    fn d1_none_inert() {
        let spec = prod_spec(); // size_refuge: None

        for predator_size in [1i32, 16, 32] {
            for prey_eu in [0i64, 10, 1_000, 10_000] {
                let pred = predator_genome(predator_size);
                let baseline = resolve_encounter(&pred, prey_eu, 1, &spec);
                for body_size in [1i64, 2, 100, 1_000_000] {
                    let out = resolve_encounter(&pred, prey_eu, body_size, &spec);
                    assert_eq!(
                        out, baseline,
                        "size_refuge=None must ignore prey_body_size entirely: size={predator_size}, \
                         prey_eu={prey_eu}, body={body_size}"
                    );
                }
            }
        }
    }

    /// `d1_determinism`: with the refuge gate ON, same inputs still → byte-identical `Outcome`
    /// across repeated calls (no RNG, no hidden state — the refuge division is pure integer).
    #[test]
    fn d1_determinism() {
        let spec = prod_spec_with_refuge(8, 1);
        let pred = predator_genome(16);

        let out_a = resolve_encounter(&pred, 1000, 7, &spec);
        let out_b = resolve_encounter(&pred, 1000, 7, &spec);
        assert_eq!(out_a, out_b, "determinism broken with refuge on: {:?} vs {:?}", out_a, out_b);
    }

    /// `d1_empty_body_clamp`: an empty-`CellGraph` / unicell prey (body size 0 or negative from a
    /// misbehaving caller) is clamped to 1 inside `resolve_encounter` — no divide-by-zero, no
    /// panic, no anomalous (negative or overflowing) outcome.
    #[test]
    fn d1_empty_body_clamp() {
        let spec = prod_spec_with_refuge(8, 4);
        let pred = predator_genome(16);

        for body_size in [0i64, -1, -1000] {
            let out = resolve_encounter(&pred, 1000, body_size, &spec);
            // Must match the clamped (body=1) result exactly — no divide-by-zero/anomalous value.
            let clamped = resolve_encounter(&pred, 1000, 1, &spec);
            assert_eq!(
                out, clamped,
                "non-positive prey_body_size={body_size} must clamp to 1: got {:?}, expected {:?}",
                out, clamped
            );
            assert!(out.predator_gain >= 0 && out.prey_loss >= 0 && out.dissipated >= 0);
        }
    }
}
