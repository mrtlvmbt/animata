//! World clock — the simulation's monotonic time base.
//!
//! The sim must run at a FIXED time-step, decoupled from the render frame rate, and it must
//! be replayable. Both come from counting whole sub-steps in a `u64` `tick`: that integer IS
//! the canonical time (`sim_time = tick * TICK_LEN`), so there is no float drift in the time
//! reference — a long session stays exact.
//!
//! Two entry points, deliberately separate:
//! - [`WorldClock::advance`] runs exactly `n` sub-steps and reads no wall-clock. This is the
//!   **deterministic** path — headless tests and the future sim's batch runs call it, so a
//!   replay is the real code path, not a property of the test harness.
//! - [`WorldClock::frame`] is the interactive wrapper: it folds a real frame `dt` into whole
//!   sub-steps through an accumulator, clamped to [`MAX_SUBSTEPS`] so a lag spike can't
//!   spiral. Because the clamp drops backlog, the interactive path is only *best-effort*
//!   deterministic (a different frame pacing can run a different sub-step count) — by design.
//!
//! S2 scope: just the clock. The per-sub-step sim body is empty here (only the counter
//! advances); the simulation of creatures hooks into `advance` in a later program.

use crate::config::{DAY_LEN, MAX_SUBSTEPS, TICK_LEN};

pub struct WorldClock {
    /// Whole sub-steps elapsed = the canonical time. `u64` at `TICK_LEN = 0.1 s` overflows
    /// after ~58 billion years of sim-time — never a concern.
    tick: u64,
    /// Real-time carry (sim-seconds) not yet spent on a whole sub-step.
    accum: f32,
    /// Sim-seconds per real second. `1.0` = real time; raise to fast-forward the sim.
    pub time_scale: f32,
    /// When paused, `frame` runs no sub-steps (the deterministic `advance` is unaffected).
    pub paused: bool,
}

impl Default for WorldClock {
    fn default() -> Self {
        Self::new()
    }
}

impl WorldClock {
    pub fn new() -> Self {
        WorldClock { tick: 0, accum: 0.0, time_scale: 1.0, paused: false }
    }

    /// Sub-steps elapsed (the canonical integer time).
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Elapsed sim-time in seconds. Derived from the integer `tick`, so it carries no drift.
    pub fn sim_time(&self) -> f64 {
        self.tick as f64 * TICK_LEN as f64
    }

    /// Fraction through the current in-world day, in `[0, 1)`. (No day/night visual yet — this
    /// just exposes the phase for a later, deferred phase to bind to.)
    pub fn day_frac(&self) -> f32 {
        (self.sim_time() / DAY_LEN as f64).fract() as f32
    }

    /// Advance exactly `n` sub-steps and return `n`. The deterministic time path: it never
    /// reads wall-clock, so a given `n` sequence always produces the same `tick`. The future
    /// sim's per-step update will be driven from here.
    pub fn advance(&mut self, n: u64) -> u64 {
        self.tick += n;
        // (S2: the per-sub-step body is empty — only the counter moves. The life-sim plugs
        // its fixed-step update in here.)
        n
    }

    /// Fold a real frame `dt` (seconds) into whole sub-steps and run them, returning how many
    /// ran. Paused ⇒ zero. The accumulator carries the sub-tick remainder so the average rate
    /// tracks `time_scale` exactly; a frame that would owe more than [`MAX_SUBSTEPS`] is capped
    /// and its backlog dropped (a lag spike resets rather than snowballing).
    pub fn frame(&mut self, dt: f32) -> u64 {
        if self.paused {
            return 0;
        }
        self.accum += dt * self.time_scale;
        let want = (self.accum / TICK_LEN).floor().max(0.0) as u64;
        let n = want.min(MAX_SUBSTEPS);
        self.accum -= n as f32 * TICK_LEN;
        if want > MAX_SUBSTEPS {
            // Lag spike (or a huge time_scale): we are further behind than one frame may
            // catch up. Drop the backlog so we don't run the cap every frame for a while.
            self.accum = 0.0;
        }
        self.advance(n)
    }
}

#[cfg(test)]
mod tests {
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

    /// The interactive wrapper accumulates a steady `dt` into whole sub-steps at the right
    /// average rate (here: dt = half a tick at scale 1 ⇒ one sub-step every two frames).
    #[test]
    fn frame_accumulates_at_the_right_rate() {
        let mut c = WorldClock::new();
        let dt = TICK_LEN / 2.0; // two frames per sub-step
        let mut ran = 0u64;
        for _ in 0..20 {
            ran += c.frame(dt);
        }
        assert_eq!(ran, c.tick());
        assert_eq!(c.tick(), 10, "20 half-tick frames should yield 10 sub-steps");
    }

    /// Spiral-of-death guard: a single giant `dt` never runs more than `MAX_SUBSTEPS`, and the
    /// backlog is dropped (the next normal frame doesn't keep firing the cap).
    #[test]
    fn frame_caps_a_lag_spike() {
        let mut c = WorldClock::new();
        let n = c.frame(1000.0 * TICK_LEN); // would owe 1000 sub-steps
        assert_eq!(n, MAX_SUBSTEPS, "lag spike not clamped to MAX_SUBSTEPS");
        // Backlog dropped: a tiny following frame runs ~no sub-steps, not a flood.
        let n2 = c.frame(TICK_LEN / 2.0);
        assert!(n2 <= 1, "backlog was not dropped after the spike: ran {n2}");
    }

    /// Pause freezes the interactive clock but not the canonical `advance`.
    #[test]
    fn pause_freezes_frame_only() {
        let mut c = WorldClock::new();
        c.paused = true;
        assert_eq!(c.frame(10.0 * TICK_LEN), 0);
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
}
