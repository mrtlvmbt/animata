//! Metrics framework — the read-only dual of the selection-pressure registry.
//!
//! Where a pressure pushes selection INTO creatures, a metric pulls an OBSERVATION out. A metric is
//! a pure, read-only function of a [`SimView`] (the population + terrain at a tick) producing a
//! [`MetricValue`]. The [`MetricRegistry`] samples every metric on a cadence and keeps a bounded
//! time-series (ring buffer) per metric — the source for graphs, CSV reports, regression asserts,
//! and the determinism checksum series. **Adding a metric** = one [`Metric`] impl + a line in
//! [`MetricRegistry::default`]; nothing else changes.
//!
//! Metrics never mutate anything (the `SimView` is shared), so sampling can't perturb the
//! simulation or its determinism — the golden is unaffected by what is observed.

use std::collections::VecDeque;

use crate::sim::Sim;
use crate::terrain::VoxelTerrain;

mod builtin;

/// A read-only window onto the simulation at a tick — all a metric may read.
pub struct SimView<'a> {
    pub sim: &'a Sim,
    pub terrain: &'a VoxelTerrain,
    pub tick: u64,
}

/// One sampled observation. Scalars cover the population statistics; `Checksum` carries the
/// full-state hash (its own type because a `u64` state hash does not fit an `f64` exactly).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MetricValue {
    Scalar(f64),
    Checksum(u64),
}

impl MetricValue {
    /// CSV/plot cell rendering.
    pub fn to_cell(self) -> String {
        match self {
            MetricValue::Scalar(v) => format!("{v}"),
            MetricValue::Checksum(h) => format!("{h}"),
        }
    }
    /// The scalar value, or `None` for a checksum (for numeric consumers like graphs).
    pub fn as_scalar(self) -> Option<f64> {
        match self {
            MetricValue::Scalar(v) => Some(v),
            MetricValue::Checksum(_) => None,
        }
    }
}

/// A named, pure, read-only observation of the simulation.
pub trait Metric: Send + Sync {
    fn id(&self) -> &'static str;
    fn sample(&self, view: &SimView) -> MetricValue;
}

/// The active metrics plus a bounded time-series per metric (ring buffer). All metrics share one
/// sample cadence, so their series stay tick-aligned (one row per sampled tick).
pub struct MetricRegistry {
    metrics: Vec<Box<dyn Metric>>,
    cadence: u64,
    cap: usize,
    ticks: VecDeque<u64>,
    cols: Vec<VecDeque<MetricValue>>, // parallel to `metrics`
}

impl MetricRegistry {
    /// Build the default metric set, sampling every `cadence` ticks and keeping the last `cap`
    /// samples per metric.
    pub fn new(cadence: u64, cap: usize) -> Self {
        let metrics = builtin::default_metrics();
        let cols = (0..metrics.len()).map(|_| VecDeque::with_capacity(cap)).collect();
        MetricRegistry { metrics, cadence: cadence.max(1), cap: cap.max(1), ticks: VecDeque::new(), cols }
    }

    /// Sample every metric iff this tick is on the cadence, appending to each series (oldest dropped
    /// past `cap`). Call once per tick with the post-step view.
    pub fn maybe_sample(&mut self, view: &SimView) {
        if !view.tick.is_multiple_of(self.cadence) {
            return;
        }
        self.ticks.push_back(view.tick);
        for (i, m) in self.metrics.iter().enumerate() {
            self.cols[i].push_back(m.sample(view));
        }
        if self.ticks.len() > self.cap {
            self.ticks.pop_front();
            for c in &mut self.cols {
                c.pop_front();
            }
        }
    }

    /// Ids of the active metrics, in column order.
    pub fn ids(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.metrics.iter().map(|m| m.id())
    }

    /// The most recent value of a metric by id (e.g. for a HUD readout).
    pub fn latest(&self, id: &str) -> Option<MetricValue> {
        let idx = self.metrics.iter().position(|m| m.id() == id)?;
        self.cols[idx].back().copied()
    }

    /// The full time-series of a metric by id, as `(tick, value)` pairs.
    pub fn series(&self, id: &str) -> Option<Vec<(u64, MetricValue)>> {
        let idx = self.metrics.iter().position(|m| m.id() == id)?;
        Some(self.ticks.iter().copied().zip(self.cols[idx].iter().copied()).collect())
    }

    /// Number of samples currently held.
    pub fn len(&self) -> usize {
        self.ticks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ticks.is_empty()
    }

    /// Render the whole series as CSV: `tick,<metric ids…>` then one row per sampled tick.
    pub fn to_csv(&self) -> String {
        let mut out = String::from("tick");
        for m in &self.metrics {
            out.push(',');
            out.push_str(m.id());
        }
        out.push('\n');
        for (row, &t) in self.ticks.iter().enumerate() {
            out.push_str(&t.to_string());
            for c in &self.cols {
                out.push(',');
                out.push_str(&c[row].to_cell());
            }
            out.push('\n');
        }
        out
    }
}

impl Default for MetricRegistry {
    /// Sensible defaults: sample every 100 ticks, keep the last 4096 samples.
    fn default() -> Self {
        MetricRegistry::new(100, 4096)
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
