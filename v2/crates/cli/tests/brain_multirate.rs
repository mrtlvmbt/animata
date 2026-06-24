//! M3 acceptance gates — arch-INDEPENDENT (integer): they run on BOTH CI jobs (outside the
//! `v2_golden_*` namespace). Cover the spawn contract (D-Brain-2a), the global multi-rate phase
//! (D-Brain-4/4a), and bit-for-bit replay with the recurrent brain + mixed frequencies (R19/R20).

use cli::{build_sim, default_config, run};
use sim_core::EconParams;

const SEED: u64 = 0xA11A_2A11;
const TICKS: u64 = 384;

fn brain_period() -> u64 {
    EconParams::default().brain_period
}

fn out_is_neutral(out: &[i16]) -> bool {
    out.iter().all(|&v| v == 0)
}

/// Spawn contract (D-Brain-2a / D-Brain-2b): EVERY newborn, in the snapshot taken immediately after
/// its birth step, has ALL per-entity brain buffers zeroed — `BrainOutput == 0` and `BrainState`
/// (`h_old` AND `h_new`) `== 0`. Bevy reuses freed entity slots, so this is exactly the "death + slot
/// reuse → newborn starts h=0 AND Act neutral, never a corpse's command" guarantee. The brain stage
/// (2) runs before BirthDeath (7), so a newborn is never inferred on its birth tick → must be neutral.
#[test]
fn v2_newborn_brain_buffers_are_zeroed_on_spawn() {
    let mut sim = build_sim(default_config(SEED));
    let mut seen: std::collections::BTreeSet<u64> =
        sim.brain_snapshot().into_iter().map(|x| x.0).collect();
    let founders = seen.len();
    let mut newborns = 0u64;

    for _ in 0..TICKS {
        sim.step();
        for (bits, bo, bs) in sim.brain_snapshot() {
            if seen.insert(bits) {
                newborns += 1;
                assert!(out_is_neutral(&bo.out), "newborn {bits:#x} spawned with a non-neutral motor command (leak)");
                assert!(bs.h_old.iter().all(|&v| v == 0), "newborn {bits:#x} spawned with non-zero h_old (state leak)");
                assert!(bs.h_new.iter().all(|&v| v == 0), "newborn {bits:#x} spawned with non-zero h_new (state leak)");
            }
        }
    }
    assert!(newborns > 0, "no births happened — the spawn contract was never exercised (founders={founders})");
}

/// Multi-rate freeze (D-Brain-4a): a newborn born OFF the global Brain phase stays frozen (neutral
/// Act) until the next GLOBAL Brain tick (`tick % K == 0`), then starts inferring — and the delay is
/// reproduced deterministically. We track the first such off-phase newborn and watch it across ticks.
#[test]
fn v2_offphase_newborn_frozen_until_global_brain_tick() {
    let k = brain_period();
    let mut sim = build_sim(default_config(SEED));
    let mut seen: std::collections::BTreeSet<u64> =
        sim.brain_snapshot().into_iter().map(|x| x.0).collect();

    // Find the first newborn born on an off-phase tick.
    let mut tracked: Option<(u64, u64)> = None; // (entity bits, birth tick)
    for _ in 0..TICKS {
        let birth_tick = sim.tick(); // the phase the upcoming step uses
        sim.step();
        for (bits, _, _) in sim.brain_snapshot() {
            if seen.insert(bits) && !birth_tick.is_multiple_of(k) {
                tracked = Some((bits, birth_tick));
                break;
            }
        }
        if tracked.is_some() {
            break;
        }
    }
    let (bits, birth_tick) = tracked.expect("expected at least one off-phase birth in the window");
    let first_brain_tick = birth_tick.next_multiple_of(k);

    // From birth up to (but not including) its first global Brain tick, the newborn must stay neutral.
    while sim.tick() < first_brain_tick {
        if let Some((_, bo, _)) = sim.brain_snapshot().into_iter().find(|x| x.0 == bits) {
            assert!(out_is_neutral(&bo.out), "off-phase newborn {bits:#x} acted before its first global Brain tick {first_brain_tick} (tick {})", sim.tick());
        } else {
            return; // it died before its first Brain tick — vacuously fine.
        }
        sim.step();
    }
}

/// At least one creature produces a NON-neutral motor decision during the run — i.e. the brains
/// actually drive behaviour (the freeze assertions above aren't passing because output is always 0).
#[test]
fn v2_brains_produce_motor_output() {
    let mut sim = build_sim(default_config(SEED));
    let mut any_active = false;
    for _ in 0..TICKS {
        sim.step();
        if sim.brain_snapshot().iter().any(|(_, bo, _)| !out_is_neutral(&bo.out)) {
            any_active = true;
            break;
        }
    }
    assert!(any_active, "no creature ever produced a non-neutral brain output — Act would be inert");
}

/// R19/R20: bit-for-bit replay WITH the recurrent `BrainState` AND mixed frequencies (K brain, N
/// metabolism). Two runs of the full multi-rate trajectory must match per tick (the state hash now
/// folds `BrainState` + `BrainOutput`). Arch-independent as a relative two-run comparison.
#[test]
fn v2_multirate_recurrent_replays_identical() {
    let a = run(default_config(SEED), TICKS);
    let b = run(default_config(SEED), TICKS);
    assert_eq!(a, b, "multi-rate + recurrent-brain trajectory is not bit-for-bit reproducible");
}
