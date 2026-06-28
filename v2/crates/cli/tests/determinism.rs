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
/// at least one trait in at least one tick within the window [TICKS..TICKS+CHECK_WINDOW] —
/// directional selection IS operating over the run. This gate fails CI if a change silently kills
/// selection pressure (e.g. a frozen reflex that never divides).
///
/// Window-based (not fixed-tick): B-3 proportional rationing can shift the inter-wave reproduction
/// gap — a single-tick snapshot may hit the gap. Checking over ~64 ticks detects any reproduction
/// burst in the equilibrium phase and is insensitive to the exact gap location.
/// Uses f64 from telemetry; the assertion `!= 0.0` is robust to arch float rounding on both jobs.
#[test]
fn v2_phi0_selection_is_active() {
    const CHECK_WINDOW: u64 = 64; // ≥ one full reproduction wave at equilibrium population
    let mut sim = build_sim(default_config(0xA11A_2A11));
    for _ in 0..TICKS {
        sim.step();
    }
    assert!(
        sim.telemetry().population > 0,
        "population went extinct at tick {} — selection gate can't be checked",
        sim.tick()
    );
    let mut any_nonzero = false;
    for _ in 0..CHECK_WINDOW {
        sim.step();
        let rep = compute(sim.telemetry().samples.as_slice());
        if rep.price_cov.iter().any(|&c| c != 0.0) {
            any_nonzero = true;
            break;
        }
    }
    assert!(
        any_nonzero,
        "all Price covariances were zero in ticks {}..{} — selection is not operating",
        TICKS, TICKS + CHECK_WINDOW
    );
}
