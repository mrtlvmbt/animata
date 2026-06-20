//! The default metric set — read-only observers wrapping the `Sim` statistics kernels plus the
//! full-state determinism checksum. Adding a metric is a one-liner here (or a new `Metric` impl).

use super::{Metric, MetricValue, SimView};
use crate::sim::state_checksum;

/// A scalar metric from a (non-capturing) read fn — the common case.
struct ScalarFn {
    id: &'static str,
    f: fn(&SimView) -> f64,
}

impl Metric for ScalarFn {
    fn id(&self) -> &'static str {
        self.id
    }
    fn sample(&self, v: &SimView) -> MetricValue {
        MetricValue::Scalar((self.f)(v))
    }
}

/// The determinism checksum as a metric: a time-series of the full-state hash, so divergence (e.g.
/// from a future rayon/tiling change) shows up as the moment the series breaks from a known run.
struct StateChecksum;

impl Metric for StateChecksum {
    fn id(&self) -> &'static str {
        "checksum"
    }
    fn sample(&self, v: &SimView) -> MetricValue {
        MetricValue::Checksum(state_checksum(v.sim, v.terrain))
    }
}

pub(super) fn default_metrics() -> Vec<Box<dyn Metric>> {
    fn s(id: &'static str, f: fn(&SimView) -> f64) -> Box<dyn Metric> {
        Box::new(ScalarFn { id, f })
    }
    vec![
        s("population", |v| v.sim.population() as f64),
        s("avg_energy", |v| v.sim.avg_energy() as f64),
        s("avg_biomass", |v| v.sim.avg_biomass() as f64),
        s("multicellular_frac", |v| v.sim.complexity_mix().0 as f64),
        s("complex_frac", |v| v.sim.complexity_mix().1 as f64),
        s("frac_carnivore", |v| v.sim.frac_carnivore() as f64),
        s("frac_autotroph", |v| v.sim.frac_autotroph() as f64),
        s("species", |v| v.sim.species_count() as f64),
        s("niches", |v| v.sim.niche_coverage(v.terrain) as f64),
        s("thermal_correlation", |v| v.sim.thermal_correlation(v.terrain) as f64),
        s("crypsis_correlation", |v| v.sim.crypsis_correlation(v.terrain) as f64),
        s("avg_nutrient", |v| v.sim.avg_nutrient(v.terrain, v.tick) as f64),
        s("stratum_underground", |v| v.sim.stratum_mix(v.terrain)[0] as f64),
        s("stratum_surface", |v| v.sim.stratum_mix(v.terrain)[1] as f64),
        s("stratum_air", |v| v.sim.stratum_mix(v.terrain)[2] as f64),
        s("stratum_water", |v| v.sim.stratum_mix(v.terrain)[3] as f64),
        s("births", |v| v.sim.births as f64),
        s("deaths", |v| v.sim.deaths as f64),
        s("kills", |v| v.sim.kills as f64),
        Box::new(StateChecksum),
    ]
}
