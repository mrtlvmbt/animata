//! P4/SL-1: settling-selection mechanic golden folded checksum (two-pass gate).
//!
//! **Purpose**: pin the expected state trajectory for settling_config (new opt-in testbed)
//! as a SINGLE folded checksum (u64) protecting all 512 ticks of drift.
//!
//! **Structure**: fold 512 per-tick state hashes into one u64 using fnv_mix (FNV hash fold).
//! Any tick changing → the fold changes. CI's single `right:` value in the assertion failure
//! is the complete whole-trajectory checksum (CI-pinnable under no-local-sim constraint).
//!
//! **Arch**: settling_config runs on phase2 substrate (O₂ + morphogen) with integer determinism.
//! Arm64 only (per-arch baseline; CI job `golden-arm64` only).
//!
//! **Re-pin** (single-writer, PM): Read the single `right:` value from `.ci-report/failed.log`,
//! and substitute the const below. Never re-pin to silence drift.
//!
//! **Two-pass gate**: Pass 1 fails with the real folded checksum; PM pins it (pass 2 green).

use cli::{settling_config, run_conserved_hashes};
use sim_core::fnv_mix;

// P4/SL-1: settling-golden folded checksum — fold of 512 per-tick state hashes.
// Pinned value from arm64 CI job (14155481137023211160).
// This single u64 protects the entire 512-tick trajectory against any drift.
const SETTLING_GOLDEN_CHECKSUM: u64 = 14155481137023211160;

/// P4/SL-1: settling-golden pin — folded checksum of 512-tick trajectory for settling_config.
/// Arm64 only (per-arch baseline). Excluded from x86 jobs automatically via the `v2_golden`
/// name prefix.
#[test]
fn v2_golden_settling() {
    if cfg!(debug_assertions) {
        return;
    }

    let hashes = run_conserved_hashes(settling_config(0xA11A_2A11), 512);

    // Fold 512 per-tick hashes into a single u64 using FNV mixing.
    // Any change in any tick → the fold changes (full trajectory protection).
    let mut folded = sim_core::FNV_OFFSET;
    for (tick, &h) in hashes.iter().enumerate() {
        folded = fnv_mix(folded, h);
        // Include tick index in fold to catch reordering anomalies.
        folded = fnv_mix(folded, tick as u64);
    }

    assert_eq!(
        SETTLING_GOLDEN_CHECKSUM, folded,
        "P4/SL-1 settling-golden checksum mismatch (512-tick fold): expected {} got {}",
        SETTLING_GOLDEN_CHECKSUM, folded
    );
}
