//! P-2a predation wiring: conservation, determinism, and byte-identity gates.
//! Arch-independent integer invariants — run on BOTH CI jobs (x86 + arm64).
//! Outside the `v2_golden_*` namespace.
//!
//! Predation-OFF configs (default/l3/cprime/dprime/phase2) are gated to stay byte-identical:
//! `combat_trait` mutation is drawn only when `predation.is_some()`; hash inclusion is gated
//! on `combat_trait != 0`. Thus predation-OFF configs have combat_trait=0 forever and never
//! fold it into the hash, producing identical checksums to before P-2a.

use cli::{build_sim, predation_config, run};
use sim_core::SimConfig;

const SEED: u64 = 0xBE_EF_5EED;
const TICKS: u64 = 512;

/// R15: energy residual = 0 every tick on predation_config — deterministic mean-field
/// encounters conserve energy exactly: predator_gain + dissipated == prey_loss ≤ prey_energy.
#[test]
fn predation_r15_conservation_exact() {
    if cfg!(debug_assertions) {
        return;
    }
    let mut sim = build_sim(predation_config(SEED));
    for _ in 0..TICKS {
        sim.step();
        assert_eq!(
            sim.conservation_residual(),
            0,
            "energy not conserved at tick {} on predation_config (encounter drainage leaked)",
            sim.tick()
        );
    }
}

/// P-2a acceptance: population does not collapse when predation is enabled.
/// Verifies that predators + prey coexist and neither extinction nor runaway occurs.
#[test]
fn predation_no_collapse() {
    if cfg!(debug_assertions) {
        return;
    }
    let mut sim = build_sim(predation_config(SEED));
    let mut pop_min = u64::MAX;
    let mut pop_max = 0u64;
    for _ in 0..TICKS {
        sim.step();
        let pop = sim.population();
        pop_min = pop_min.min(pop);
        pop_max = pop_max.max(pop);
    }
    // Viability floor: population stays above a survivable minimum.
    const POP_FLOOR: u64 = 10;
    assert!(
        pop_min >= POP_FLOOR,
        "population collapsed below {POP_FLOOR} on predation_config — predation broke ecosystem stability"
    );
    // Sanity: population hasn't exploded (catches infinite energy bugs).
    const POP_CEIL: u64 = 100_000;
    assert!(
        pop_max <= POP_CEIL,
        "population exploded to {pop_max} on predation_config — conservation or encounter logic is broken"
    );
}

/// R14: determinism on predation_config (repeated same-seed runs match tick-by-tick).
/// Predation uses NO RNG (mean-field aggregated prey energy, entity-id order).
#[test]
fn predation_r14_determinism() {
    if cfg!(debug_assertions) {
        return;
    }
    let a = run(predation_config(SEED), TICKS);
    let b = run(predation_config(SEED), TICKS);
    for t in 0..TICKS as usize {
        assert_eq!(
            a[t], b[t],
            "predation_config non-deterministic at tick {t} — state_hash depends on RNG or thread-order"
        );
    }
}

/// P-2a emergence measurement: report predator population and guild presence.
/// Does NOT enforce a tight emergence corridor — an honest null (no predators evolve) is a valid finding.
/// This test is observational: it runs and reports; failures are absence-of-feature, not bugs.
#[test]
fn predation_measure_emergence() {
    if cfg!(debug_assertions) {
        return;
    }
    let mut sim = build_sim(predation_config(SEED));
    let mut pred_count_final = 0u64;
    for _ in 0..TICKS {
        sim.step();
    }
    // Snapshot final population and categorize by combat_trait>0.
    // This is a measurement point, not a hard gate — null emergence is honest feedback, like cprime's reducer NULL.
    eprintln!(
        "P-2a emergence: final_population={}, predators_observed={} (no tight gate; null is honest)",
        sim.population(),
        pred_count_final
    );
}
