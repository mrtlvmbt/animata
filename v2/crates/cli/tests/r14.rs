//! The R14 gate WITH TEETH (F1) — the conserved layer is thread-count-independent.
//!
//! Both tests run on BOTH arches (integer/arch-independent: a RELATIVE 1-vs-N comparison, no pinned
//! constant), driving a REAL sim thread pool with N>1 (not the test runner). They are OUTSIDE the
//! `v2_golden_*` namespace.

use cli::{config_with, l3_config, run_conserved_hashes};
use sim_core::{MergeStrategy, SimConfig};

const SEED: u64 = 0xA11A_2A11;
const TICKS: u64 = 160;
const N: usize = 4;

/// R14 GREEN: the `Canonical` (integer-associative) merge gives a bit-identical CONSERVED-field hash
/// on 1 thread and on N threads, every tick.
#[test]
fn v2_conserved_field_is_thread_count_independent() {
    let one = run_conserved_hashes(config_with(SEED, 1, MergeStrategy::Canonical), TICKS);
    let many = run_conserved_hashes(config_with(SEED, N, MergeStrategy::Canonical), TICKS);
    for t in 0..TICKS as usize {
        assert_eq!(one[t], many[t], "conserved hash differs 1-vs-{N} at tick {t} (R14 broken)");
    }
}

/// The gate can go RED: the injected `NonAssociative` strategy folds the N per-thread partials with a
/// count-sensitive combine, so 1-vs-N diverges. Without this, R14 GREEN is correct-by-construction
/// decoration that catches zero regressions.
#[test]
fn v2_r14_gate_has_teeth_negative() {
    let one = run_conserved_hashes(config_with(SEED, 1, MergeStrategy::NonAssociative), TICKS);
    let many = run_conserved_hashes(config_with(SEED, N, MergeStrategy::NonAssociative), TICKS);
    assert_ne!(one, many, "the non-associative merge MUST make 1-vs-{N} diverge — the gate is toothless");
}

/// R14 at L=3: the integer-associative canonical merge is thread-count-independent across ALL three
/// conserved layers. Runs on both arches (relative integer comparison, no pinned constant).
#[test]
fn v2_conserved_l3_thread_count_independent() {
    let one = run_conserved_hashes(SimConfig { sim_threads: 1, ..l3_config(SEED) }, TICKS);
    let many = run_conserved_hashes(SimConfig { sim_threads: N, ..l3_config(SEED) }, TICKS);
    for t in 0..TICKS as usize {
        assert_eq!(one[t], many[t], "L=3 conserved hash differs 1-vs-{N} at tick {t} (R14 broken)");
    }
}
