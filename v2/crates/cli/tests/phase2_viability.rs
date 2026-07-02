//! E-5b: the size-viability criterion (`genome.rs`'s `(Some, Some)` chain arm) makes `decode` return
//! a REAL, production-reachable `None` in `phase2_config` — a stillbirth, not a test injection. This
//! file proves the criterion actually fires in production (a DIRECT counter, not an inference from
//! population size — critic F8) and that it fires INSIDE the golden window, which is what makes the
//! `v2_golden_conserved_phase2` re-pin an intended, mechanism-driven move rather than noise.
//!
//! Runs on x86 (arch-independent: an integer counter and a relative population bound, no pinned
//! constant).

use cli::{build_sim, phase2_config};

const SEED: u64 = 0xA11A_2A11;

/// Calibration horizon for the recurring-stream probe: longer than the 384-tick golden window so the
/// "recurs, not a one-shot" property is observable. Calibrated once against `phase2_config(SEED)`
/// (deterministic — always reproduces the same counts): at the golden horizon (tick 384) the
/// criterion has fired exactly N=1 time; by tick 1200 it has fired N=5 times. Per the issue's
/// calibration rule (`stillbirths >= max(2, N/2)`, N measured at the 384-tick horizon): `max(2, 1/2)
/// = 2`. K=2 is comfortably below the tick-1200 observed rate (5), so it is a real margin, not a
/// tautological `>= 1`.
const TICKS: u64 = 1200;
const K: u64 = 2;
/// Last hashed golden tick (`GOLDEN_CONSERVED_PHASE2.len() - 1` in `golden_conserved.rs`) — the
/// first stillbirth must land strictly before this for the golden re-pin to genuinely capture the
/// mechanism (critic F3).
const GOLDEN_LAST_TICK: u64 = 383;

/// The gate-fires proof (critic F8): a DIRECT counter of criterion-triggered stillbirths on a CLEAN
/// `phase2_config` run (critic F7 — no `force_decode_none` in this run, so every count is
/// attributable to the real size-viability gate). Also the golden-binding proof (critic F3): the
/// first stillbirth must land inside the golden window, or the phase2 golden re-pin would not
/// actually capture the mechanism.
#[test]
fn phase2_stillbirths_recur_and_first_is_inside_golden_window() {
    let mut sim = build_sim(phase2_config(SEED));
    let mut first_tick: Option<u64> = None;
    let mut prev = 0u64;
    for t in 0..TICKS {
        sim.step();
        let cur = sim.stillbirth_count();
        if cur > prev && first_tick.is_none() {
            first_tick = Some(t);
        }
        prev = cur;
    }
    assert!(
        first_tick.is_some(),
        "no criterion-triggered stillbirth observed over {TICKS} ticks on a clean phase2_config run \
         (no force_decode_none) — the viability gate never fired in production"
    );
    assert!(
        first_tick.unwrap() < GOLDEN_LAST_TICK,
        "first stillbirth at tick {:?} must land strictly before the last hashed golden tick ({GOLDEN_LAST_TICK}) \
         — otherwise the phase2 golden re-pin would not capture the mechanism",
        first_tick
    );
    assert!(
        prev >= K,
        "stillbirths must RECUR (>= {K}), not be a one-shot event — got {prev} over {TICKS} ticks"
    );
}

/// The no-extinction proof (critic F4): the existing bounded-population checks
/// (`phase2_liveness.rs`'s `phase2_config_population_is_bounded`) already run `phase2_config` WITH
/// this criterion live (decode's `(Some, Some)` arm is unconditional — there is no separate
/// "criterion enabled" toggle). This test re-states the same bound over the longer calibration
/// horizon used above, so the recurring-stillbirth stream is proven non-extinction-inducing over the
/// SAME window the recurrence is measured on, not just the shorter 400-tick liveness window.
#[test]
fn phase2_stays_bounded_over_the_calibration_horizon_with_criterion_live() {
    let mut sim = build_sim(phase2_config(SEED));
    let mut min = u64::MAX;
    let mut max = 0u64;
    for _ in 0..TICKS {
        sim.step();
        let p = sim.population();
        min = min.min(p);
        max = max.max(p);
    }
    assert!(min > 0, "phase2_config went extinct under the live viability criterion (min population 0)");
    assert!(max < 100_000, "phase2_config population exploded ({max}) under the live viability criterion");
}
