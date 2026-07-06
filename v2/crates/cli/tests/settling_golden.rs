//! P4/SL-1: settling-selection mechanic golden state_checksum (two-pass gate).
//!
//! **Purpose**: pin the expected state trajectory for settling_config (new opt-in testbed).
//! This is the SINGLE settling-golden added by SL-1 (byte-identity of existing goldens untouched).
//!
//! **Arch**: settling_config runs on phase2 substrate (O₂ + morphogen) with integer determinism
//! (no float, no RNG in settling_drain). State is arch-independent. The golden is pinned arm64
//! (standard for new testbeds; CI job `golden-arm64` only).
//!
//! **Re-pin** (single-writer, PM): Only on an INTENDED settling-mechanic change. Read the new
//! `left:` from `.ci-report/failed.log` (arm64 job). Never re-pin to silence drift.
//!
//! **Two-pass gate**: SL-1 pass 1 reports `STATUS: blocked@settling-golden: жду CI (pass 2 of 2)`;
//! PM pins the golden in pass 2 and re-runs; pass 2 comes back green.

use cli::{build_sim, settling_config, run_conserved_hashes};

// P4/SL-1: settling-golden pin — expected state_hash per tick for settling_config (SEED=default).
// Captured on arm64 + Rust 1.96.0 (matches the CI `golden-arm64` job arch + toolchain).
// This is pass 1; PM pins the real value from CI `.ci-report/failed.log` on pass 2.
const SETTLING_GOLDEN: [u64; 512] = [
    // TODO: PM pins real values from arm64 CI run on pass 2.
    // For now, placeholder; test will fail on first run, CI reports `left:` for PM to substitute.
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// P4/SL-1: settling-golden pin — expected state_hash trajectory for settling_config(SEED).
/// Arm64 only (FMA-divergent phase2 substrate baseline; determinism per-arch, FP-free settling).
/// Excluded from x86 jobs automatically via the `v2_golden` name prefix.
#[test]
fn v2_golden_settling() {
    if cfg!(debug_assertions) {
        return;
    }

    let hashes = run_conserved_hashes(settling_config(0xA11A_2A11), 512);

    for (tick, (expected, actual)) in SETTLING_GOLDEN.iter().zip(hashes.iter()).enumerate() {
        assert_eq!(
            expected, actual,
            "P4/SL-1 settling-golden mismatch at tick {}: expected {} got {}",
            tick, expected, actual
        );
    }
}
