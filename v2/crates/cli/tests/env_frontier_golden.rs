//! ENV-0a'-a1: spatial monopolization golden + R15 energy conservation test.
//! The env_frontier_config introduces bonded pre-emption in resource ration, a determinism-critical
//! mechanic that reorders who takes resource in contested cells (WHO takes) but never creates/destroys
//! total energy (R15 exact conservation per tick).
//! This test verifies:
//! 1. Golden checksum (arm64 release only) at 384 ticks, seed=1.
//! 2. R15 energy conservation: within a retention-ON run, every tick has residual = 0
//!    (field energy + agent energy + dissipated = initial + produced), proving the priority-ration
//!    mechanic redistributes grants exactly without leaking energy.

use cli::{env_frontier_config, run, build_sim};
use sim_core::Sim;

/// Single folded trajectory checksum over the 384-tick run (arm64 release golden).
/// Uses FNV-1a style fold: any single-tick drift changes the fold, validating the full trajectory.
const ENV_FRONTIER_GOLDEN: u64 = 17981074702083343180; // pinned from CI golden-arm64 job pass 1

/// Golden drift (R19): arm64 release only. Skipped in debug (float-fusing differs).
#[test]
fn v2_golden_env_frontier_drift() {
    if cfg!(debug_assertions) {
        return; // golden pinned for release; debug float-fusing differs (run via the arm64 release job)
    }
    let hashes = run(env_frontier_config(1), 384);
    let fold = hashes.iter().fold(0xcbf29ce484222325u64, |acc, &h| (acc ^ h).wrapping_mul(0x100000001b3));
    assert_eq!(
        fold, ENV_FRONTIER_GOLDEN,
        "env_frontier golden drift at ticks 0..384 (left=run fold, right=ENV_FRONTIER_GOLDEN)"
    );
}

/// R15 energy conservation: priority-ration mechanic conserves energy exactly, tick by tick.
/// Within a retention-ON run, every tick must have residual = 0: the total energy removed from
/// the field is exactly credited to entities or dissipated (ledger). This proves the bonded
/// pre-emption mechanic redistributes GRANTS (who takes) without creating/destroying energy.
///
/// Note: Σ grants ≤ r_cell by construction (bonded take min(demand, remaining), remaining
/// starts at r_cell), so the ration never over-grants and conserves exactly.
#[test]
fn env_frontier_sigma_conservation() {
    const TEST_SEED: u64 = 1;
    const TEST_TICKS: u64 = 500;

    let cfg = env_frontier_config(TEST_SEED);
    let mut sim = build_sim(cfg);

    for tick in 0..TEST_TICKS {
        sim.step();
        // R15: energy conservation residual must be 0 after every tick when retention is active.
        // Priority-ration mechanic: every unit removed from the field is credited to an entity
        // or dissipated, so residual is exactly 0. This proves the mechanic conserves energy.
        let residual = sim.conservation_residual();
        assert_eq!(
            residual, 0,
            "energy conservation violated at seed={} tick={}: residual={}",
            TEST_SEED, tick, residual
        );
    }
}
