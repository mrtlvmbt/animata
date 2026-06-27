//! Arch-INDEPENDENT gates — integer invariants that hold on every arch and profile, so they run on
//! BOTH CI jobs (outside the `v2_golden_*` namespace). The energy-conservation assertion fires inside
//! `run()` every tick, so simply running these exercises R15 always-on in release (F8).

use cli::{build_sim, default_config, run};
use telemetry::compute;

const TICKS: u64 = 384;

/// R13: the conserved (fixed-point) and signal (f32) classes are BOTH correct in the SAME tick — the
/// conserved residual stays exactly 0 while a finite pheromone field accumulates (>0) and decays.
#[test]
fn v2_both_field_classes_correct_together() {
    let mut sim = build_sim(default_config(0xA11A_2A11));
    for _ in 0..TICKS {
        sim.step();
        assert_eq!(sim.conservation_residual(), 0, "conserved leaked at tick {}", sim.tick());
        assert!(sim.signal_finite(), "signal NaN/Inf at tick {}", sim.tick());
    }
    assert!(sim.signal_total() > 0.0, "a pheromone trail (signal field) must exist alongside resource");
}

/// (a) two-run-same-seed at a FIXED sim-thread count: run-to-run determinism within an arch+profile
/// (catches a forgotten natural-order reduction / random hasher). Integer-and-within-arch-float-
/// deterministic ⇒ both runs match regardless of arch.
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

/// Ф0 emergence gate (M1/F4): the Price equation covariance cov(trait, offspring) is non-zero for
/// at least one trait after a fixed-seed run — directional selection IS operating. This gate fails
/// CI if a change silently kills selection pressure (e.g. a frozen reflex that never divides).
/// Uses f64 arithmetic from the telemetry crate; the assertion is `!= 0.0` (not an exact value),
/// so it is robust to arch-specific float rounding on both CI jobs.
#[test]
fn v2_phi0_selection_is_active() {
    let mut sim = build_sim(default_config(0xA11A_2A11));
    for _ in 0..TICKS {
        sim.step();
    }
    let rep = compute(sim.telemetry().samples.as_slice());
    assert!(rep.population > 0, "population went extinct — selection gate can't be checked");
    let any_nonzero = rep.price_cov.iter().any(|&c| c != 0.0);
    assert!(
        any_nonzero,
        "all Price covariances are zero — selection is not operating: {:?}",
        rep.price_cov
    );
}
