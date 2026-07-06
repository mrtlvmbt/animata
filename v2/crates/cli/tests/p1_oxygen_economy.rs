//! P1-2a arch-INDEPENDENT sim-level gates for the O₂ respiratory economy (`oxygen_config`,
//! enable_oxygen=true). Integer-deterministic + conservation invariants — run on BOTH CI jobs
//! (outside the `v2_golden_*` namespace). Complements the pure-function unit tests in
//! `sim-core/src/stages.rs` (choose_respiratory_pathway, R31/R34 mechanism) by exercising the FULL
//! stage pipeline over a run, so respiration (mutated respiratory_pathway>0 lineages) and the
//! COMPOSED metabolic efficiency actually fire live.
//!
//! P1-2b (hypoxia self-shading): tests for `phase2_oxygen_config` (multicellular + O₂ diffusion cost).
//! R33: two-run determinism. R15: energy conservation under hypoxia. Viability: no extinction/explosion.

use cli::{build_sim, oxygen_config, phase2_oxygen_config, run};

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

// ── P1-2b (hypoxia self-shading) sim-level tests ────────────────────────────────────────────────

const TICKS_PHASE2: u64 = 384;

/// R33: run-to-run determinism of phase2_oxygen_config (multicellular bodies + O₂ hypoxia).
/// Hypoxia uses CBRT_LUT (integer-only) and reads field @t once per entity — any floating-point
/// regression or field-race would show up here (both runs with same seed must match exactly).
#[test]
fn v2_phase2_oxygen_two_run_same_seed() {
    let a = run(phase2_oxygen_config(0xB22B_3B22), TICKS_PHASE2);
    let b = run(phase2_oxygen_config(0xB22B_3B22), TICKS_PHASE2);
    for t in 0..TICKS_PHASE2 as usize {
        assert_eq!(a[t], b[t], "phase2-oxygen-config run-to-run non-determinism at tick {t}");
    }
}

/// R15 UNDER HYPOXIA: energy conserved exactly with phase2_oxygen_config (morphogen + O₂ diffusion cost).
/// Hypoxia reduces `gained` through the `kept_x1000 = (1000 - hypoxia) / 1000` factor:
/// `gained = got × combined_eff / 256 × kept / 1000; lost = got - gained`.
/// Per-entity: got == gained + lost (conserved); aggregated: Σ(gained + lost) == Σ(got).
/// The hypoxia-penalised income still conserves because it's integer arithmetic over a ratio.
#[test]
fn v2_phase2_oxygen_energy_conserved_exactly() {
    let mut sim = build_sim(phase2_oxygen_config(0xB22B_3B22));
    for _ in 0..TICKS_PHASE2 {
        sim.step();
        assert_eq!(
            sim.conservation_residual(),
            0,
            "energy leaked under hypoxia at tick {}",
            sim.tick()
        );
    }
}

/// Phase2-oxygen viability check: multicellular bodies evolve, population stays bounded
/// (doesn't go extinct or explode due to hypoxia-income penalty or morphogen cost).
/// Hypoxia penalises N>1, so selection may favour smaller bodies; non-extinct check
/// ensures the penalty is not fatal.
#[test]
fn v2_phase2_oxygen_population_bounded() {
    let mut sim = build_sim(phase2_oxygen_config(0xB22B_3B22));
    let mut min = u64::MAX;
    let mut max = 0u64;
    for _ in 0..TICKS_PHASE2 {
        sim.step();
        let p = sim.population();
        min = min.min(p);
        max = max.max(p);
    }
    assert!(min > 0, "phase2-oxygen-config population went extinct");
    assert!(max < 100_000, "phase2-oxygen-config population exploded ({max})");
}

/// ★ Mechanic-is-EXERCISED guard (critic F6, anti-NULL-bypass): hypoxia self-shading only fires for
/// bodies with N>1 (`compute_hypoxia_factor_x1000` returns 0 at `body_cell_count<=1`). If
/// `phase2_oxygen_config` founders were unicellular, hypoxia would be DEAD CODE and the R33/R15/
/// viability tests above would pass spuriously (hypoxia≡0 no-op). This test proves multicellular
/// bodies (N>1) actually exist in the run, so the diffusion cost is genuinely on the economy.
#[test]
fn v2_phase2_oxygen_has_multicellular_bodies() {
    let mut sim = build_sim(phase2_oxygen_config(0xB22B_3B22));
    let mut ever_multicellular = 0u64;
    let mut max_body = 0i64;
    for _ in 0..TICKS_PHASE2 {
        sim.step();
        let (max_size, n_multi) = sim.body_size_stats();
        max_body = max_body.max(max_size);
        ever_multicellular = ever_multicellular.max(n_multi);
    }
    assert!(
        max_body > 1 && ever_multicellular > 0,
        "phase2_oxygen_config produced NO multicellular (N>1) bodies (max_body={max_body}, \
         multi_count={ever_multicellular}) → hypoxia self-shading is dead code (NULL-bypass)"
    );
}
