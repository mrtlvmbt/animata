//! B-3 proportional-rationing integration tests (issue #156). Arch-independent (no golden
//! constants, no float equality) — run on BOTH CI jobs.

use cli::{build_sim, default_config};
use sim_core::MergeStrategy;

const TICKS: u64 = 384;

/// `ration_conserves` (R15 / B-3): cross-layer excretion + proportional grant keeps the
/// conserved-field ledger residual exactly 0 every tick. The truncation remainder (R_cell mod Σ)
/// must stay in the field, not disappear.
#[test]
fn b3_ration_conserves() {
    let mut sim = build_sim(default_config(0xB3_C043));
    for _ in 0..TICKS {
        sim.step();
        assert_eq!(
            sim.conservation_residual(),
            0,
            "energy leaked at tick {} under proportional rationing",
            sim.tick()
        );
    }
}

/// `ration_order_independent` (B-3 / R10/R14): with proportional rationing the per-cell grants
/// depend only on Σ demand and R_cell — both Σ-associative — so a 1-thread vs 4-thread sim
/// produces IDENTICAL state hashes (R14 invariant holds after the algorithm change).
///
/// This is the B-3 analog of the existing R14 1-vs-N test (`r14.rs`) but focused specifically
/// on the proportionality property: if grants were still entity-order-dependent (as the old serial
/// take was), the two runs would diverge once the thread batching changes processing order.
/// Proportional rationing removes the ordering sensitivity.
#[test]
fn b3_ration_order_independent() {
    use cli::config_with;
    let cfg1 = config_with(0xB3_0110, 1, MergeStrategy::Canonical);
    let cfg4 = config_with(0xB3_0110, 4, MergeStrategy::Canonical);
    let hashes1: Vec<u64> = {
        let mut sim = build_sim(cfg1);
        (0..TICKS).map(|_| { sim.step(); sim.state_hash() }).collect()
    };
    let hashes4: Vec<u64> = {
        let mut sim = build_sim(cfg4);
        (0..TICKS).map(|_| { sim.step(); sim.state_hash() }).collect()
    };
    for t in 0..TICKS as usize {
        assert_eq!(
            hashes1[t], hashes4[t],
            "state diverged at tick {t}: 1-thread={:x} 4-thread={:x}",
            hashes1[t], hashes4[t]
        );
    }
}

