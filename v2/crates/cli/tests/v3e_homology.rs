//! V-3-e population-level teeth (#246): `Telemetry::genome_diversity` (sim-core `stages.rs`
//! `stage_observe`), the mean-consecutive-`genome_distance` observable over the live population's
//! GRN genomes (entity-id order). Golden-NEUTRAL: read-only, never folded into `state_hash` — the
//! byte-identity of the 6 pinned goldens (`golden.rs` + `golden_conserved.rs`, all un-re-pinned by
//! this change) is the direct proof of tooth 8; this file covers the remaining population-level
//! teeth (6/7/9) that need a running `Sim`, not just the pure `homology::genome_distance` fn
//! (covered by its own inline unit tests in `sim_core::homology`).

use cli::{build_sim, cprime_config, default_config, dprime_config, l3_config, phase2_config, DEFAULT_THREADS};
use sim_core::SimConfig;

const SEED: u64 = 0xA11A_2A11;

/// Tooth 6: a population of identical genomes (all founders, tick 1 — before any birth/mutation
/// could have fired) has `genome_diversity == 0`.
#[test]
fn v3e_diversity_zero_monoculture() {
    let mut sim = build_sim(phase2_config(SEED));
    sim.step();
    let tel = sim.telemetry();
    assert!(tel.population >= 2, "need >=2 founders to exercise the pairwise metric, got {}", tel.population);
    assert_eq!(
        tel.genome_diversity, 0,
        "an all-founder population (identical grn_spec content) must have genome_diversity == 0"
    );
}

/// Tooth 7: a population with variation (point-mutated GRN specs after many ticks of births) has
/// `genome_diversity > 0`.
#[test]
fn v3e_diversity_positive_mixed() {
    let mut sim = build_sim(phase2_config(SEED));
    for _ in 0..500 {
        sim.step();
    }
    let tel = sim.telemetry();
    assert!(tel.population >= 2, "need >=2 live genomes to exercise the pairwise metric, got {}", tel.population);
    assert!(
        tel.genome_diversity > 0,
        "a population with 500 ticks of point-mutation/reproduction must show genome_diversity > 0, got {}",
        tel.genome_diversity
    );
}

/// The five non-phase2 production configs never seed a `grn_spec` (`EconParams.grn` stays `None`
/// there), so `genome_diversity` must stay exactly 0 at every tick — the `Some`-filter in
/// `stage_observe` never finds a valid genome to compare. Complements the golden byte-identity
/// check (tooth 8) with a direct assertion on the new field itself.
#[test]
fn v3e_non_phase2_configs_diversity_always_zero() {
    let configs: [(&str, SimConfig); 4] = [
        ("default", default_config(SEED)),
        ("cprime", cprime_config(SEED)),
        ("dprime", dprime_config(SEED)),
        ("l3", l3_config(SEED)),
    ];
    for (name, cfg) in configs {
        let mut sim = build_sim(cfg);
        for t in 0..200 {
            sim.step();
            assert_eq!(
                sim.telemetry().genome_diversity, 0,
                "config '{name}' has no grn_spec — genome_diversity must stay 0 at tick {t}"
            );
        }
    }
}

/// Tooth 9: deterministic replay — same seed replayed twice yields the identical
/// `genome_diversity` trace (pure function of entity-id-ordered genome content, no RNG of its own).
#[test]
fn v3e_diversity_deterministic_replay() {
    let ticks = 300;
    let trace_of = |threads: usize| -> Vec<i64> {
        let cfg = SimConfig { sim_threads: threads, ..phase2_config(SEED) };
        let mut sim = build_sim(cfg);
        (0..ticks)
            .map(|_| {
                sim.step();
                sim.telemetry().genome_diversity
            })
            .collect()
    };

    let run1 = trace_of(DEFAULT_THREADS);
    let run2 = trace_of(DEFAULT_THREADS);
    assert_eq!(run1, run2, "same seed/thread-count must replay to the identical genome_diversity trace");

    // 1-vs-N thread-count independence: stage_observe sorts by entity bits before computing the
    // consecutive-pair mean, so the result must not depend on how many threads ran `decide`.
    let run_1_thread = trace_of(1);
    assert_eq!(
        run1, run_1_thread,
        "genome_diversity trace must be thread-count independent (1 vs {DEFAULT_THREADS} threads)"
    );
}
