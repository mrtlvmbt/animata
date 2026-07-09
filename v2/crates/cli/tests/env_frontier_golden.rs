//! ENV-0a'-a1: spatial monopolization golden + Σ-conservation test.
//! The env_frontier_config introduces bonded pre-emption in resource ration, a determinism-critical
//! mechanic that reorders who takes resource in contested cells but conserves the total taken.
//! This test verifies:
//! 1. Golden checksum (arm64 release only) at 384 ticks, seed=1.
//! 2. Σ-conservation: at seed=1, tick=500, `conserved_total_all()` is identical between
//!    retention-OFF and retention-ON runs (the rule redistributes, never creates/destroys).

use cli::{env_frontier_config, run, build_sim};
use sim_core::Sim;

/// Placeholder golden (pass 1, locally computed). Real arm64 value from CI golden-arm64 job.
/// This will be replaced in pass 2 once CI runs and captures the actual value.
const ENV_FRONTIER_GOLDEN: [u64; 384] = [
    0x0000000000000001; 384 // Placeholder: CI pass 2 replaces with actual arm64 value
];

/// Golden drift (R19): arm64 release only. Skipped in debug (float-fusing differs).
#[test]
fn env_frontier_golden_drift() {
    if cfg!(debug_assertions) {
        return; // golden pinned for release; debug float-fusing differs (run via the arm64 release job)
    }
    let h = run(env_frontier_config(1), ENV_FRONTIER_GOLDEN.len() as u64);
    for t in 0..ENV_FRONTIER_GOLDEN.len() {
        assert_eq!(
            h[t], ENV_FRONTIER_GOLDEN[t],
            "env_frontier golden drift at tick {t} (left=run, right=ENV_FRONTIER_GOLDEN)"
        );
    }
}

/// Σ-conservation: bonded pre-emption reorders grants but preserves total uptake.
/// At seed=1, tick=500, the field `conserved_total_all()` after the eat stage must be
/// integer-identical between retention-OFF and retention-ON runs.
#[test]
fn env_frontier_sigma_conservation() {
    const TEST_SEED: u64 = 1;
    const TEST_TICK: u64 = 500;

    // Run with env_frontier_config ON (bonded pre-emption enabled).
    let cfg_on = env_frontier_config(TEST_SEED);
    let mut sim_on = build_sim(cfg_on);
    for _ in 0..TEST_TICK {
        sim_on.step();
    }
    let total_on = sim_on.conserved_field_total_all();

    // Run with env_frontier_config OFF (proportional ration, no bonded pre-emption).
    // Clone driver_config but explicitly set env_frontier_config to None.
    let mut cfg_off = cli::driver_config(TEST_SEED);
    cfg_off.econ.env_frontier_config = None;
    let mut sim_off = build_sim(cfg_off);
    for _ in 0..TEST_TICK {
        sim_off.step();
    }
    let total_off = sim_off.conserved_field_total_all();

    // Σ-conservation: both runs must have IDENTICAL total after 500 ticks.
    // The bonded pre-emption mechanism redistributes WHO takes, never the total taken.
    assert_eq!(
        total_on, total_off,
        "Σ-conservation violated at seed={} tick={}: retention-ON={}, retention-OFF={} (diff={})",
        TEST_SEED, TEST_TICK, total_on, total_off, (total_on as i64) - (total_off as i64)
    );
}
