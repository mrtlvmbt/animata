//! E-4b-i: proves the ontogenesis chain is genuinely LIVE in `phase2_config` — a DIRECT per-entity
//! comparison against a test-only "twin" config (the same `phase2_config`, with `morphogen`/`grn`
//! cleared to `None`), not the conserved-field golden hash (critic F2/F10). The conserved golden
//! only moves TRANSITIVELY through field sinks and could stay silent even if the chain leaked or
//! went dead — this test is the authoritative liveness proof; the golden's job (`golden_conserved.rs`)
//! is to LOCK determinism, not prove liveness.
//!
//! Runs on x86 (arch-independent: this compares two RELATIVE integer distributions within one arch,
//! no pinned constant).

use cli::{build_sim, phase2_config};
use sim_core::SimConfig;

const SEED: u64 = 0xA11A_2A11;
const TICKS: u64 = 400;
const N_LAYERS: usize = 2;

/// The test-only twin: identical `phase2_config`, specs cleared to `None` — NOT a second production
/// config (critic F2: "the same config with the specs cleared").
fn specs_none_twin(seed: u64) -> SimConfig {
    let mut cfg = phase2_config(seed);
    cfg.econ.morphogen = None;
    cfg.econ.grn = None;
    cfg
}

/// The chain-is-live proof: the `uptake_layer` distribution among live entities DIFFERS between
/// `phase2_config` (chain runs, `cell_type` drives `uptake_layer`) and its specs-`None` twin (E-1
/// trivial projection, `uptake_layer` stays the raw genome value — founder default 0, only children
/// with `uptake_layer` mutations differ). If the chain were dead or not wired to the consumer, the
/// two histograms would be statistically indistinguishable (both driven by the same genome mutation
/// random walk from the same seed).
#[test]
fn phase2_uptake_layer_distribution_differs_from_specs_none_twin() {
    let mut phase2 = build_sim(phase2_config(SEED));
    let mut twin = build_sim(specs_none_twin(SEED));

    for _ in 0..TICKS {
        phase2.step();
        twin.step();
    }

    let hist_phase2 = phase2.uptake_layer_histogram(N_LAYERS);
    let hist_twin = twin.uptake_layer_histogram(N_LAYERS);

    assert_ne!(
        hist_phase2, hist_twin,
        "chain-is-live proof failed: uptake_layer histogram identical between phase2_config \
         (chain enabled) and its specs-None twin — the ontogenesis chain is not reaching the \
         consumer. phase2={hist_phase2:?} twin={hist_twin:?}"
    );
}

/// Sanity companion: the specs-`None` twin behaves exactly like the E-1 trivial projection — its
/// founders (genome.uptake_layer == 0) start entirely in layer 0.
#[test]
fn specs_none_twin_founders_start_at_layer_zero() {
    let mut twin = build_sim(specs_none_twin(SEED));
    let hist = twin.uptake_layer_histogram(N_LAYERS);
    assert!(hist[0] > 0, "founders must start at uptake_layer=0 (E-1 trivial projection)");
    assert_eq!(hist[1], 0, "no layer-1 agents before any mutation has occurred");
}

/// `phase2_config` runs a bounded, non-degenerate trajectory (no extinction/explosion) over the
/// golden horizon — a precondition for the golden pin, checked independently of the liveness proof.
#[test]
fn phase2_config_population_is_bounded() {
    let mut sim = build_sim(phase2_config(SEED));
    let mut min = u64::MAX;
    let mut max = 0u64;
    for _ in 0..TICKS {
        sim.step();
        let p = sim.population();
        min = min.min(p);
        max = max.max(p);
    }
    assert!(min > 0, "phase2_config population went extinct");
    assert!(max < 100_000, "phase2_config population exploded ({max})");
}

/// Two-runs-same-seed on `phase2_config` (critic F6): proves the whole ECS pipeline is
/// deterministic under Phase-2 load. NOT, by itself, proof the `cell_type` consumer is
/// order-insensitive — that rests on it being a pure read (E-1 pattern) rather than a
/// natural-order-dependent write, which is what `decode`'s pure-function design guarantees.
#[test]
fn phase2_config_two_run_same_seed() {
    let mut a = build_sim(phase2_config(SEED));
    let mut b = build_sim(phase2_config(SEED));
    for t in 0..TICKS {
        a.step();
        b.step();
        assert_eq!(
            a.conserved_field_hash(),
            b.conserved_field_hash(),
            "phase2_config replay diverged at tick {t}"
        );
    }
}
