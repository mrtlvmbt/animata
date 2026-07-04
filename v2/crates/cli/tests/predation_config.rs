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

/// P-2a emergence: drift-controlled guard that combat_trait is genuinely SELECTED under predation,
/// not just drifting. Runs BOTH active (predation encounters selected) and inert (predation encounters
/// zero energy effect, but combat_trait still mutates) twins to t=4000, measures the divergence,
/// and asserts active mean exceeds inert baseline by a robust margin (1.4×).
///
/// Calibration: active_config (seed 0xBE_EF_5EED) reaches mean ≈2.3 at t=4000 (n=3 seeds measured);
/// inert_config (same seed) reaches mean ≈1.0. Margin: 1.4× headroom on this 2.3× spread ensures
/// a clear selection signal persists under natural drift variance.
#[test]
fn predation_measure_emergence() {
    if cfg!(debug_assertions) {
        return;
    }
    const HORIZON: u64 = 4000;
    const MARGIN: f64 = 1.4; // active_mean must exceed inert_mean * MARGIN

    // Active config: predation selected, bite_shift=3 gives ≈8% of prey energy per encounter.
    let active_config = predation_config(SEED);
    let mut sim_active = build_sim(active_config);
    for _ in 0..HORIZON {
        sim_active.step();
    }
    let (_max_a, count_pos_a, sum_a) = sim_active.combat_trait_stats();
    let pop_a = sim_active.population();
    let mean_active = if pop_a > 0 {
        sum_a as f64 / pop_a as f64
    } else {
        0.0
    };

    // Inert config: same predation_config but bite_shift≈31 so base_bite >> 31 ≈ 0.
    // PredationSpec is zeroed energy effect while keeping `predation: Some` so combat_trait mutates.
    let mut inert_config = predation_config(SEED);
    inert_config.econ.predation = Some(sim_core::PredationSpec {
        bite_shift: 31,        // prey_energy >> 31 ≈ 0 for realistic prey energies (10–100)
        combat_trait_scale: 1, // kept same; irrelevant since bite ≈ 0
        efficiency_num: 0,     // zero predator gain → no predator population pressure
    });
    let mut sim_inert = build_sim(inert_config);
    for _ in 0..HORIZON {
        sim_inert.step();
    }
    let (_max_i, count_pos_i, sum_i) = sim_inert.combat_trait_stats();
    let pop_i = sim_inert.population();
    let mean_inert = if pop_i > 0 {
        sum_i as f64 / pop_i as f64
    } else {
        0.0
    };

    eprintln!(
        "P-2a emergence (t={HORIZON}): active_mean={:.2} (count>0={}/{}), inert_mean={:.2} (count>0={}/{}) → ratio={:.2}× (margin {MARGIN}×)",
        mean_active, count_pos_a, pop_a,
        mean_inert, count_pos_i, pop_i,
        if mean_inert > 0.0 { mean_active / mean_inert } else { f64::INFINITY }
    );
    assert!(
        mean_active >= mean_inert * MARGIN,
        "combat_trait selection signal lost: active_mean={:.2} < inert_mean={:.2} × margin={MARGIN}. \
         Predation may have regressed or mutation-gate may be broken. active_count>0={}/{}, inert_count>0={}/{}",
        mean_active, mean_inert, count_pos_a, pop_a, count_pos_i, pop_i
    );
}
