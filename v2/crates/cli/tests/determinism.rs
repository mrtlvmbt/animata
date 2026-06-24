//! Arch-INDEPENDENT gates — integer invariants that hold on every arch and profile, so they run on
//! BOTH CI jobs (outside the `v2_golden_*` namespace). The energy-conservation assertion fires inside
//! `run()` every tick, so simply running these exercises R15 always-on in release (F8).

use cli::{build_sim, default_config, run};

const TICKS: u64 = 384;

/// (a) two-run-same-seed: run-to-run determinism within an arch+profile (catches a forgotten
/// natural-order reduction / random hasher). Integer-and-within-arch-float-deterministic ⇒ both runs
/// match regardless of arch.
#[test]
fn v2_two_run_same_seed() {
    let a = run(default_config(0xA11A_2A11), TICKS);
    let b = run(default_config(0xA11A_2A11), TICKS);
    for t in 0..TICKS as usize {
        assert_eq!(a[t], b[t], "run-to-run non-determinism at tick {t}");
    }
}

/// Energy conservation is EXACTLY 0 every tick (R15). `run()` asserts internally; here we also walk
/// the sim directly so the residual is checked from the public API.
#[test]
fn v2_energy_conserved_exactly() {
    let mut sim = build_sim(default_config(7));
    for _ in 0..TICKS {
        sim.step();
        assert_eq!(sim.conservation_residual(), 0, "energy not conserved at tick {}", sim.tick());
    }
}

/// Closed bookkeeping: the population neither goes extinct nor explodes (logistic bound). A coarse,
/// arch-independent guard on the demo's qualitative claim.
#[test]
fn v2_population_is_bounded() {
    let mut sim = build_sim(default_config(0xA11A_2A11));
    let mut min = u64::MAX;
    let mut max = 0u64;
    for _ in 0..TICKS {
        sim.step();
        let p = sim.population();
        min = min.min(p);
        max = max.max(p);
    }
    assert!(min > 0, "population went extinct");
    assert!(max < 100_000, "population exploded ({max})");
}

/// Different seed ⇒ different trajectory (sanity: the seed actually drives the run).
#[test]
fn v2_seed_changes_trajectory() {
    let a = run(default_config(1), 64);
    let b = run(default_config(2), 64);
    assert_ne!(a, b);
}
