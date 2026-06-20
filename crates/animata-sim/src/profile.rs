//! Phase profiler — a reusable wall-clock instrument for `Sim::step`.
//!
//! This is the timing dual of the metric/pressure registries: where a [`crate::metrics`] pulls a
//! pure observation out of the sim STATE, the profiler measures how long each PHASE of a tick takes
//! in real time. It is deliberately NOT part of the determinism contract — it reads `Instant` (wall
//! clock), influences no sim value, and is never folded into `state_checksum` (it lives on `Sim`
//! beside `grid`/`registry` as non-state). So profiling on or off, the golden is identical.
//!
//! Design mirrors the project idioms: a `#[non_exhaustive]` [`Span`] enum + `ALL` (like
//! `TrophicNiche`) indexes a fixed array of sliding-window rings (like the fps readout) — no hashing
//! on the hot path. Per-tick costs ACCUMULATE (a sub-span such as [`Span::Develop`] fires once per
//! birth) and are flushed to the rings by [`Profiler::commit_tick`] at the end of the tick, so a
//! many-times span and a once-per-tick phase are handled uniformly.
//!
//! Adding a span = one enum variant + its `label`/`depth` arms + a couple of `record` calls. The
//! headless `--profile` table, the HUD perf panel and the dev-bridge JSON all iterate `report()`, so
//! a new span surfaces everywhere automatically.

use std::collections::VecDeque;
use std::time::Duration;

/// A timed region of one tick. Top-level phases of `Sim::step` plus the `GridRebuild` sub-span broken
/// out of snapshot. `Develop` is its own (parallel) phase run after apply. Order = display = index.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Span {
    Snapshot,
    GridRebuild,
    Decide,
    Predation,
    Apply,
    Develop,
    Compact,
}

impl Span {
    /// Every span, in display order. Iterate this to render one row/bar per span.
    pub const ALL: &'static [Span] = &[
        Span::Snapshot,
        Span::GridRebuild,
        Span::Decide,
        Span::Predation,
        Span::Apply,
        Span::Develop,
        Span::Compact,
    ];

    /// Lower-case label for tables / HUD rows.
    pub fn label(self) -> &'static str {
        match self {
            Span::Snapshot => "snapshot",
            Span::GridRebuild => "grid.rebuild",
            Span::Decide => "decide",
            Span::Predation => "predation",
            Span::Apply => "apply",
            Span::Develop => "develop",
            Span::Compact => "compact",
        }
    }

    /// Nesting depth for indented display: `0` = a top-level phase, `1` = a sub-span of the phase
    /// above it (`GridRebuild` under `Snapshot`). `Develop` is its own top-level (parallel) phase that
    /// runs after `Apply` — `Apply` no longer includes it.
    pub fn depth(self) -> u8 {
        match self {
            Span::GridRebuild => 1,
            _ => 0,
        }
    }
}

const N_SPANS: usize = Span::ALL.len();
/// Sliding-window length (ticks). Long enough to smooth jitter, short enough to track a phase that
/// grows as the population does.
const WINDOW: usize = 240;

/// A bounded ring of the last `WINDOW` per-tick durations (nanoseconds) for one span.
struct Window {
    ring: VecDeque<u64>,
}

impl Window {
    fn new() -> Self {
        Window { ring: VecDeque::with_capacity(WINDOW) }
    }

    fn push(&mut self, ns: u64) {
        if self.ring.len() == WINDOW {
            self.ring.pop_front();
        }
        self.ring.push_back(ns);
    }

    /// Mean over the window, in milliseconds (`0.0` if empty).
    fn mean_ms(&self) -> f32 {
        if self.ring.is_empty() {
            return 0.0;
        }
        let sum: u64 = self.ring.iter().sum();
        (sum as f64 / self.ring.len() as f64 / 1.0e6) as f32
    }

    /// Worst single tick in the window, in milliseconds (`0.0` if empty).
    fn max_ms(&self) -> f32 {
        (self.ring.iter().copied().max().unwrap_or(0) as f64 / 1.0e6) as f32
    }
}

/// Per-span sliding-window timing for `Sim::step`. Owned by `Sim` as non-state; mutated through
/// `&mut self` between phases. Cheap when disabled (record/commit early-return).
pub struct Profiler {
    enabled: bool,
    /// Nanoseconds accumulated for each span THIS tick (flushed by `commit_tick`).
    accum: [u64; N_SPANS],
    windows: [Window; N_SPANS],
}

impl Default for Profiler {
    fn default() -> Self {
        Profiler {
            enabled: true,
            accum: [0; N_SPANS],
            windows: std::array::from_fn(|_| Window::new()),
        }
    }
}

impl Profiler {
    /// Add a measured duration to the current tick's accumulator for `span`. A span may be recorded
    /// many times per tick (e.g. once per birth) — the totals sum until `commit_tick`.
    pub fn record(&mut self, span: Span, d: Duration) {
        if !self.enabled {
            return;
        }
        self.accum[span as usize] += d.as_nanos() as u64;
    }

    /// Flush this tick's accumulators into the sliding windows and reset them. Call once at the end
    /// of `Sim::step`.
    pub fn commit_tick(&mut self) {
        if !self.enabled {
            return;
        }
        for i in 0..N_SPANS {
            self.windows[i].push(self.accum[i]);
            self.accum[i] = 0;
        }
    }

    /// Toggle profiling. When off, `record`/`commit_tick` are no-ops (the windows freeze).
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
    }

    /// Per-span `(span, mean_ms, max_ms)` over the window, in `Span::ALL` order.
    pub fn report(&self) -> Vec<(Span, f32, f32)> {
        Span::ALL
            .iter()
            .map(|&s| {
                let w = &self.windows[s as usize];
                (s, w.mean_ms(), w.max_ms())
            })
            .collect()
    }
}
