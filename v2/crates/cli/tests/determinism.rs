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

/// B-3 / F7 calibration corridor: population must remain in the measured equilibrium band at
/// TICKS=384 (early-growth phase, before the cross-feeding bloom). Catches Km drift or an economy
/// regression that kills growth.
///
/// Bounds from x86 CI measurement (seed=0xa11a2a11, run #28319581988, B-3, t≤400):
///   N̄_max = 122, N̄_min = 40 (founders).  FLOOR=40, CEIL=160 (N̄_max × 1.31).
/// At t≤384, N≈80–122 on a 64×64=4096-cell grid → average 0.02–0.03 agents/cell → deficit
/// events are extremely rare → B-3 proportional rationing barely fires; dynamics ≈ B-1/B-2.
///
/// Km regime verified: field_total/2/4096 ≈ 73 at t=350 → R̄≈73 ≈ km=74 (Monod linear regime,
/// R̄/km≈0.99). If km drifts to ≪ R̄, uptake saturates and the corridor would widen; if km ≫ R̄,
/// agents starve and min_pop collapses below FLOOR. Either failure signals a km recalibration.
///
/// Bounds are arch-independent: integer-economy-dominated growth in this early window.
#[test]
fn v2_population_corridor_b3() {
    // C-slice (issue #167): d0=0.001/tick can kill 1-3 founders before first division (tick≈3-5).
    // Measured: min_pop=39 at tick=5 (seed=0xA11A_2A11, arm64+x86). FLOOR lowered 40→30 to bracket
    // d0 attrition without masking actual near-extinction. max_pop probe confirms ≤160 still holds
    // (d0 thins population but recycle raises substrate; net effect on early max_pop is negligible).
    const FLOOR: u64 = 30;  // 40 founders − expected ≤10 d0 kills before first division
    const CEIL: u64  = 160; // N̄_max(≤160) still valid: recycle raises plateau but not above B-3 CEIL
    let mut sim = build_sim(default_config(0xA11A_2A11));
    let mut min_pop = u64::MAX;
    let mut max_pop = 0u64;
    for _ in 0..TICKS {
        sim.step();
        let p = sim.population();
        min_pop = min_pop.min(p);
        max_pop = max_pop.max(p);
    }
    assert!(
        min_pop >= FLOOR,
        "population fell below {FLOOR} (near-extinction) at t≤{TICKS} — km may have drifted out of calibrated regime"
    );
    assert!(
        max_pop <= CEIL,
        "population reached {max_pop} (>{CEIL}) before t≤{TICKS} — B-3 economy has unexpected early bloom"
    );
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
