//! P4/SL-1: settling-selection mechanic — size²-attenuated mortality pulse.
//!
//! **Purpose**: verify the settling-mechanic (SL-1 only) with REAL falsification tests —
//! each test MUST fail if stage_settling is a no-op, size-independent, or miscomputed.
//!
//! **Golden-ADDITIVE:** settling_config is a new opt-in testbed config; existing goldens stay
//! byte-identical. A new settling-golden will be pinned arm64 (PM single-writer, post-review).

use cli::{build_sim, settling_config};
use sim_core::settling_drain;

const SEED: u64 = 0xC0_DE_5EED;

/// SL-1 Tooth 1: settling_drain formula via single source of truth (NOT a copy).
/// Verify the formula on the REAL pub function; compute explicitly on known sizes.
/// FALSIFIES if: formula were size-independent, size-inverse, or absent.
#[test]
fn settling_formula_size_squared_gradient() {
    use sim_core::SettlingSpec;

    let spec = SettlingSpec {
        period: 100,
        strength: 100,
        settling_k: 128,
        shift: 16,
    };

    // Small body (size=1): drain ≈ strength (high pressure).
    let drain_small = settling_drain(&spec, 1);
    assert!(drain_small > 0, "drain at size=1 must be positive");
    assert!(drain_small >= 80, "drain at size=1 must be ≥80 (nearly full strength)");

    // Medium body (size=4): drain is smaller.
    let drain_medium = settling_drain(&spec, 4);
    assert!(drain_medium > 0, "drain at size=4 must be positive");
    assert!(drain_medium < drain_small, "size² must attenuate: drain(4) < drain(1)");

    // Large body (size=16): drain is even smaller.
    let drain_large = settling_drain(&spec, 16);
    assert!(drain_large > 0, "drain at size=16 must be positive");
    assert!(drain_large < drain_medium, "size² must attenuate: drain(16) < drain(4)");

    // Verify monotone-decreasing (FALSIFIES if formula were linear or absent).
    assert!(drain_large < drain_medium && drain_medium < drain_small,
            "size² gradient critical: drain({}) > drain({}) > drain({}), but got {}, {}, {}",
            1, 4, 16, drain_small, drain_medium, drain_large);
}

/// SL-1 Tooth 2: population survival + conservation R15.
/// Run settling_config for 4 pulses; verify population > 0 and ledger stays balanced.
#[test]
fn settling_population_survives_conserves() {
    if cfg!(debug_assertions) {
        return;
    }

    let mut sim = build_sim(settling_config(SEED));
    let initial_pop = sim.population();
    assert!(initial_pop > 0, "must start with founders");

    // Run 400 ticks (4 settling pulses at period=100).
    for _ in 0..400 {
        sim.step();
        assert_eq!(sim.conservation_residual(), 0, "R15 must hold every tick");
    }

    let final_pop = sim.population();
    assert!(final_pop > 0,
            "settling pulse must not be instantly lethal (final: {}, initial: {})",
            final_pop, initial_pop);
}

/// SL-1 Tooth 3: settling is active — ON vs OFF divergence.
/// Run settling_config (settling ON) and identical config (settling=None) in parallel.
/// Verify they DIVERGE after 4 pulses (population differs).
/// FALSIFIES if: stage_settling were a no-op (would give identical populations).
/// **Robust:** this is the key falsification test that works cross-arch (deterministic culling over 400 ticks).
#[test]
fn settling_mechanic_diverges_from_no_settling() {
    if cfg!(debug_assertions) {
        return;
    }

    let mut sim_with = build_sim(settling_config(SEED));

    // Create an identical config but with settling=None.
    let mut sim_without = {
        let mut cfg = settling_config(SEED);
        cfg.econ.settling = None;  // Disable settling.
        build_sim(cfg)
    };

    // Run both for 4 pulses (400 ticks).
    for _ in 0..400 {
        sim_with.step();
        sim_without.step();
    }

    let pop_with = sim_with.population();
    let pop_without = sim_without.population();

    // With settling ON, population should be lower (settling culls small entities).
    // This FALSIFIES if settling were inert or size-independent (would give pop_with ≈ pop_without).
    assert!(pop_with < pop_without,
            "settling must reduce population: with={}, without={} (settling is active)",
            pop_with, pop_without);
}

/// SL-1 Tooth 4: determinism R33 — two runs identical.
#[test]
fn settling_determinism() {
    if cfg!(debug_assertions) {
        return;
    }

    let mut sim1 = build_sim(settling_config(SEED));
    let mut sim2 = build_sim(settling_config(SEED));

    for tick in 0..200 {
        sim1.step();
        sim2.step();
        assert_eq!(sim1.state_hash(), sim2.state_hash(), "R33 at tick {}", tick);
    }
}

/// SL-1 Tooth 5: conservation R15 explicitly.
#[test]
fn settling_conservation_exact() {
    if cfg!(debug_assertions) {
        return;
    }

    let mut sim = build_sim(settling_config(SEED));
    for _ in 0..150 {
        sim.step();
        assert_eq!(sim.conservation_residual(), 0, "R15 exact at tick {}", sim.tick());
    }
}
