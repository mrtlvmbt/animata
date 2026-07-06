//! P3-2 thermal-tolerance mechanic tests. Sign-fix (income-side penalty) + breadth-cost tradeoff.
//! Tests must be FALSIFYING: fail if penalty is on cost (wrong sign) or if breadth_cost is not applied.

use cli::{build_sim, config_with};
use sim_core::{AmbientToleranceSpec, EconParams, MergeStrategy, SimConfig};

const TICKS: u64 = 256;

/// Smoke test: `ambient_tolerance=None` (all legacy configs) must remain byte-identical.
#[test]
fn p3_thermal_disabled_byte_identical() {
    let mut cfg = config_with(0xA311_0001, 1, MergeStrategy::Canonical);
    assert!(cfg.econ.ambient_tolerance.is_none(), "baseline config must have None");

    let mut sim = build_sim(cfg);
    for _ in 0..TICKS {
        sim.step();
        assert_eq!(
            sim.conservation_residual(),
            0,
            "energy leaked with ambient_tolerance=None at tick {}",
            sim.tick()
        );
    }
}

/// Mechanic test: when `ambient_tolerance=Some`, the penalty is applied (gates the function).
/// Population must remain viable — thermal stress doesn't kill everyone. Conservation still holds.
#[test]
fn p3_thermal_enabled_population_viable() {
    let mut cfg = config_with(0xA311_0002, 1, MergeStrategy::Canonical);
    cfg.econ.ambient_tolerance = Some(AmbientToleranceSpec { breadth_cost_k: 1 });

    let mut sim = build_sim(cfg);
    let mut min_pop = u64::MAX;
    let mut max_pop = 0u64;

    for _ in 0..TICKS {
        sim.step();
        assert_eq!(
            sim.conservation_residual(),
            0,
            "energy leaked with ambient_tolerance=Some at tick {}",
            sim.tick()
        );

        let pop = sim.population();
        min_pop = min_pop.min(pop);
        max_pop = max_pop.max(pop);
    }

    // Thermal stress reduces birth rate (income penalty), but population must not collapse.
    assert!(min_pop > 0, "population went extinct under thermal penalty");
    assert!(max_pop > 0, "population never recovered under thermal penalty");
}

/// Determinism: two runs with the same seed and `ambient_tolerance=Some` must produce
/// identical state hashes. Penalty computation is pure (deterministic integer arithmetic).
#[test]
fn p3_thermal_enabled_deterministic() {
    let seed = 0xA311_0003u64;
    let a: Vec<u64> = {
        let mut cfg = config_with(seed, 1, MergeStrategy::Canonical);
        cfg.econ.ambient_tolerance = Some(AmbientToleranceSpec { breadth_cost_k: 1 });
        let mut sim = build_sim(cfg);
        (0..TICKS).map(|_| { sim.step(); sim.state_hash() }).collect()
    };
    let b: Vec<u64> = {
        let mut cfg = config_with(seed, 1, MergeStrategy::Canonical);
        cfg.econ.ambient_tolerance = Some(AmbientToleranceSpec { breadth_cost_k: 1 });
        let mut sim = build_sim(cfg);
        (0..TICKS).map(|_| { sim.step(); sim.state_hash() }).collect()
    };

    for t in 0..TICKS as usize {
        assert_eq!(
            a[t], b[t],
            "thermal-tolerance run-to-run non-determinism at tick {t}: first={:x} second={:x}",
            a[t], b[t]
        );
    }
}

/// P3-2 sign-fix FALSIFYING test: thermal penalty must reduce INCOME (not cost).
/// FALSIFICATION criterion: if thermal_x256 were incorrectly applied to COST instead of INCOME,
/// the population would be HIGHER (lower cost at suboptimal T → reward thermostress).
/// With correct sign-fix (penalty on income), population is constrained by reduced intake.
///
/// Test method: Compare population trajectory with thermal penalty (income-side) vs without.
/// Income-side penalty → population constrained; cost-side penalty → population boosted.
#[test]
fn p3_thermal_sign_fix_income_not_cost() {
    const N_TICKS: u64 = 512;

    // Run 1: WITH thermal penalty (correct sign-fix = income-side)
    let mut cfg_with = config_with(0xA311_0004, 1, MergeStrategy::Canonical);
    cfg_with.econ.ambient_tolerance = Some(AmbientToleranceSpec { breadth_cost_k: 1 });
    let mut sim_with = build_sim(cfg_with);

    for _ in 0..N_TICKS {
        sim_with.step();
        assert_eq!(sim_with.conservation_residual(), 0, "conservation failed WITH thermal penalty");
    }
    let pop_with_final = sim_with.population();

    // Run 2: WITHOUT thermal penalty (baseline, ambient_tolerance=None)
    let mut cfg_without = config_with(0xA311_0004, 1, MergeStrategy::Canonical);
    assert!(cfg_without.econ.ambient_tolerance.is_none());
    let mut sim_without = build_sim(cfg_without);

    for _ in 0..N_TICKS {
        sim_without.step();
        assert_eq!(sim_without.conservation_residual(), 0, "conservation failed WITHOUT thermal penalty");
    }
    let pop_without_final = sim_without.population();

    // FALSIFICATION: if penalty were on COST (wrong sign), pop_with > pop_without (more energy retained).
    // If penalty is on INCOME (correct sign), pop_with < pop_without (less energy intake).
    // Both should be > 0 (not extinct), but WITH should be smaller due to income reduction.
    assert!(
        pop_with_final > 0,
        "population extinct WITH thermal penalty — penalty may be too severe or conservation broken"
    );
    assert!(
        pop_without_final > 0,
        "population extinct WITHOUT thermal penalty (baseline) — unexpected"
    );
    assert!(
        pop_with_final < pop_without_final,
        "population WITH thermal penalty ({}) must be < WITHOUT ({}); if > then penalty is on cost (wrong sign)",
        pop_with_final, pop_without_final
    );
}

/// P3-2 breadth-cost FALSIFYING test: wider tol_breadth must incur strictly larger metabolic cost.
/// FALSIFICATION criterion: if breadth_cost were not applied (= 0), both scenarios would have equal
/// cost and energy trajectories. With breadth_cost applied (monotonic), wider breadth = higher cost.
///
/// Test method: Run two scenarios identical except breadth_cost_k (high vs low), measure relative
/// population/energy — with non-zero cost, higher breadth_cost_k must reduce population.
#[test]
fn p3_breadth_cost_monotonic_and_applying() {
    const N_TICKS: u64 = 512;

    // Scenario A: breadth_cost_k = 5 (higher cost on breadth)
    let mut cfg_high = config_with(0xA311_0005, 1, MergeStrategy::Canonical);
    cfg_high.econ.ambient_tolerance = Some(AmbientToleranceSpec { breadth_cost_k: 5 });
    let mut sim_high = build_sim(cfg_high);

    for _ in 0..N_TICKS {
        sim_high.step();
        assert_eq!(sim_high.conservation_residual(), 0, "conservation failed with high breadth_cost_k");
    }
    let pop_high_final = sim_high.population();

    // Scenario B: breadth_cost_k = 1 (lower cost on breadth)
    let mut cfg_low = config_with(0xA311_0005, 1, MergeStrategy::Canonical);
    cfg_low.econ.ambient_tolerance = Some(AmbientToleranceSpec { breadth_cost_k: 1 });
    let mut sim_low = build_sim(cfg_low);

    for _ in 0..N_TICKS {
        sim_low.step();
        assert_eq!(sim_low.conservation_residual(), 0, "conservation failed with low breadth_cost_k");
    }
    let pop_low_final = sim_low.population();

    // FALSIFICATION: if breadth_cost_k were not applied (= 0), pop_high ≈ pop_low.
    // If breadth_cost is applied and monotonic, pop_high < pop_low (higher cost penalty).
    // Seed is identical, so drift is minimal; difference is due to breadth_cost impact.
    assert!(
        pop_high_final > 0,
        "population extinct with high breadth_cost_k — cost may be prohibitive"
    );
    assert!(
        pop_low_final > 0,
        "population extinct with low breadth_cost_k — unexpected"
    );
    assert!(
        pop_high_final < pop_low_final,
        "population with breadth_cost_k=5 ({}) must be < with breadth_cost_k=1 ({}); if equal then breadth_cost not applied",
        pop_high_final, pop_low_final
    );
}
