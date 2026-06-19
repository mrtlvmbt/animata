use super::*;

/// `advance(n)` is the canonical path: exactly `n` ticks, and `sim_time` is the integer
/// tick times the step — no wall-clock, fully deterministic.
#[test]
fn advance_is_exact_and_driftless() {
    let mut c = WorldClock::new();
    assert_eq!(c.tick(), 0);
    assert_eq!(c.sim_time(), 0.0);
    c.advance(10);
    c.advance(5);
    assert_eq!(c.tick(), 15);
    assert!((c.sim_time() - 15.0 * TICK_LEN as f64).abs() < 1e-12);
}

/// The interactive scheduler accumulates a steady `dt` into whole sub-steps at the right
/// average rate (here: dt = half a tick at scale 1 ⇒ one sub-step every two frames). The
/// caller is what advances; `substeps` only schedules.
#[test]
fn substeps_accumulate_at_the_right_rate() {
    let mut c = WorldClock::new();
    let dt = TICK_LEN / 2.0; // two frames per sub-step
    let mut ran = 0u64;
    for _ in 0..20 {
        let n = c.substeps(dt);
        for _ in 0..n {
            c.advance(1);
        }
        ran += n;
    }
    assert_eq!(ran, c.tick());
    assert_eq!(c.tick(), 10, "20 half-tick frames should yield 10 sub-steps");
}

/// Spiral-of-death guard: a single giant `dt` never schedules more than `MAX_SUBSTEPS`, and
/// the backlog is dropped (the next normal frame doesn't keep firing the cap).
#[test]
fn substeps_cap_a_lag_spike() {
    let mut c = WorldClock::new();
    let n = c.substeps(1000.0 * TICK_LEN); // would owe 1000 sub-steps
    assert_eq!(n, MAX_SUBSTEPS, "lag spike not clamped to MAX_SUBSTEPS");
    let n2 = c.substeps(TICK_LEN / 2.0);
    assert!(n2 <= 1, "backlog was not dropped after the spike: scheduled {n2}");
}

/// Pause freezes the interactive scheduler but not the canonical `advance`.
#[test]
fn pause_freezes_substeps_only() {
    let mut c = WorldClock::new();
    c.paused = true;
    assert_eq!(c.substeps(10.0 * TICK_LEN), 0);
    assert_eq!(c.tick(), 0);
    c.advance(3); // deterministic path ignores pause
    assert_eq!(c.tick(), 3);
}

/// `day_frac` stays in `[0, 1)` and wraps across a day boundary.
#[test]
fn day_frac_in_unit_range() {
    let mut c = WorldClock::new();
    // Step to 1.5 days of sim-time.
    let ticks = (1.5 * DAY_LEN as f64 / TICK_LEN as f64).round() as u64;
    c.advance(ticks);
    let f = c.day_frac();
    assert!((0.0..1.0).contains(&f), "day_frac out of range: {f}");
    assert!((f - 0.5).abs() < 0.01, "expected mid-day (~0.5), got {f}");
}

/// Same `advance` sequence ⇒ identical state (the sim must replay).
#[test]
fn deterministic_replay() {
    let mut a = WorldClock::new();
    let mut b = WorldClock::new();
    for &n in &[3u64, 7, 1, 0, 12] {
        a.advance(n);
        b.advance(n);
    }
    assert_eq!(a.tick(), b.tick());
    assert_eq!(a.sim_time(), b.sim_time());
}
