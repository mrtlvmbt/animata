//! C′-1 conservation and determinism teeth on cprime_config (R14 + R15).
//! Arch-independent integer invariants — run on BOTH CI jobs (x86 + arm64).
//! Outside the `v2_golden_*` namespace.

use cli::{build_sim, cprime_config, run_conserved_hashes};
use sim_core::SimConfig;

const SEED: u64 = 0xC0_DE_5EED;
const TICKS: u64 = 512;
const N_THREADS: usize = 4;

/// R15: energy residual = 0 every tick on cprime_config — the death→detritus redirect is
/// exactly conservative (no eu created or destroyed; truncation remainder goes to ledger.lost).
#[test]
fn cprime_r15_conservation_exact() {
    if cfg!(debug_assertions) {
        return;
    }
    let mut sim = build_sim(cprime_config(SEED));
    for _ in 0..TICKS {
        sim.step();
        assert_eq!(
            sim.conservation_residual(),
            0,
            "energy not conserved at tick {} on cprime_config (death→detritus path leaked)",
            sim.tick()
        );
    }
}

/// R14: 1-vs-N conserved-field hash identical on cprime_config — the detritus-layer deposit
/// goes through the same canonical integer-associative scatter as every other conserved layer.
#[test]
fn cprime_r14_thread_count_independent() {
    if cfg!(debug_assertions) {
        return;
    }
    let one = run_conserved_hashes(SimConfig { sim_threads: 1, ..cprime_config(SEED) }, TICKS);
    let many = run_conserved_hashes(SimConfig { sim_threads: N_THREADS, ..cprime_config(SEED) }, TICKS);
    for t in 0..TICKS as usize {
        assert_eq!(
            one[t], many[t],
            "cprime conserved hash differs 1-vs-{N_THREADS} at tick {t} \
             (R14 broken on detritus death-redirect path)"
        );
    }
}
