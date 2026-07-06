//! P1-2a arch-INDEPENDENT sim-level gates for the O₂ respiratory economy (`oxygen_config`,
//! enable_oxygen=true). Integer-deterministic + conservation invariants — run on BOTH CI jobs
//! (outside the `v2_golden_*` namespace). Complements the pure-function unit tests in
//! `sim-core/src/stages.rs` (choose_respiratory_pathway, R31/R34 mechanism) by exercising the FULL
//! stage pipeline over a run, so respiration (mutated respiratory_pathway>0 lineages) and the
//! COMPOSED metabolic efficiency actually fire live.

use cli::{build_sim, oxygen_config, run};

const TICKS: u64 = 384;

/// R33: run-to-run determinism of the O₂ economy at a fixed thread count. Respiration reads the O₂
/// field @t (read-old) and applies an integer yield multiplier — a forgotten natural-order
/// reduction or a field-read race would surface as a mismatch here. Integer-and-within-arch
/// deterministic ⇒ both runs match regardless of arch.
#[test]
fn v2_oxygen_two_run_same_seed() {
    let a = run(oxygen_config(0xA11A_2A11), TICKS);
    let b = run(oxygen_config(0xA11A_2A11), TICKS);
    for t in 0..TICKS as usize {
        assert_eq!(a[t], b[t], "oxygen-economy run-to-run non-determinism at tick {t}");
    }
}

/// R15 UNDER RESPIRATION: energy stays EXACTLY conserved every tick with enable_oxygen=true. This is
/// the load-bearing guard on P1-2a's income path — the respiratory yield multiplier COMPOSES with
/// `metabolism_eff` on the FULL grant (eat all, dissipate the inefficiency), so per entity
/// `got == gained + lost` and the conserved residual must remain 0. A reduced-take or a phantom
/// `dissipated += shortfall` would leak here. O₂ is a non-energy layer (excluded from the ledger),
/// so it never enters the residual.
#[test]
fn v2_oxygen_energy_conserved_exactly() {
    let mut sim = build_sim(oxygen_config(0xA11A_2A11));
    for _ in 0..TICKS {
        sim.step();
        assert_eq!(
            sim.conservation_residual(),
            0,
            "energy leaked under respiration at tick {}",
            sim.tick()
        );
    }
}

/// The O₂ economy neither goes extinct nor explodes over the window — a coarse arch-independent
/// guard that respiration + aerobe-cost did not break viability (e.g. an inverted anoxia yield that
/// starves everyone, or a runaway free-energy bug). Isolation at the founder (respiratory_pathway=0
/// → None → combined_eff == metabolism_eff) means the baseline economy must remain viable.
#[test]
fn v2_oxygen_population_bounded() {
    let mut sim = build_sim(oxygen_config(0xA11A_2A11));
    let mut min = u64::MAX;
    let mut max = 0u64;
    for _ in 0..TICKS {
        sim.step();
        let p = sim.population();
        min = min.min(p);
        max = max.max(p);
    }
    assert!(min > 0, "oxygen-config population went extinct");
    assert!(max < 100_000, "oxygen-config population exploded ({max})");
}
