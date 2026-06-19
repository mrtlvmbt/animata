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

    /// Fold a real frame `dt` (seconds) into a count of whole sub-steps to run this frame —
    /// but DO NOT advance: the caller loops `for _ in 0..n { clock.advance(1); sim_step(..) }`
    /// so the sim runs exactly one fixed tick per sub-step (each sees its own `tick()`), and
    /// `advance` stays a pure counter. Paused ⇒ zero. The accumulator carries the sub-tick
    /// remainder so the average rate tracks `time_scale` exactly; a frame owing more than
    /// [`MAX_SUBSTEPS`] is capped and its backlog dropped (a lag spike resets, not snowballs).
    ///
    /// This is the **interactive** (wall-clock-driven) path → best-effort, NOT for seed replay.
    /// Headless replay drives `advance(1)` + `sim_step` a FIXED number of times instead.
    pub fn substeps(&mut self, dt: f32) -> u64 {
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
        n
    }
}

#[cfg(test)]
#[path = "clock_tests.rs"]
mod tests;
