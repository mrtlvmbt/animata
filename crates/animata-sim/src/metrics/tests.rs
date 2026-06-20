use super::*;
use crate::sim::{state_checksum, Sim};
use crate::terrain::VoxelTerrain;

fn stepped(seed: u64, ticks: u64) -> (Sim, VoxelTerrain) {
    let mut t = VoxelTerrain::new(1);
    let mut s = Sim::new(seed, &t);
    for tick in 0..ticks {
        s.step(&mut t, tick);
    }
    (s, t)
}

/// The registry samples on cadence, the series stays bounded, and CSV has a header + a row each.
#[test]
fn registry_samples_on_cadence_and_bounds_the_series() {
    let mut reg = MetricRegistry::new(10, 4); // sample every 10 ticks, keep 4
    let mut t = VoxelTerrain::new(1);
    let mut s = Sim::new(1, &t);
    for tick in 0..100 {
        s.step(&mut t, tick);
        reg.maybe_sample(&SimView { sim: &s, terrain: &t, tick });
    }
    // 100 ticks / cadence 10 = 10 samples, capped at 4.
    assert_eq!(reg.len(), 4, "ring buffer not bounded to cap");
    let csv = reg.to_csv();
    assert!(csv.starts_with("tick,population,"), "csv header wrong: {:?}", &csv[..40.min(csv.len())]);
    assert_eq!(csv.lines().count(), 1 + 4, "expected header + 4 rows");
}

/// The checksum metric matches `state_checksum` exactly (it IS the determinism lock as a series).
#[test]
fn checksum_metric_equals_state_checksum() {
    let (s, t) = stepped(42, 50);
    let mut reg = MetricRegistry::new(1, 64);
    reg.maybe_sample(&SimView { sim: &s, terrain: &t, tick: 50 });
    assert_eq!(reg.latest("checksum"), Some(MetricValue::Checksum(state_checksum(&s, &t))));
}

/// Scalar metrics mirror the `Sim` kernels they wrap (the registry is a faithful observer layer).
#[test]
fn scalar_metrics_mirror_sim_kernels() {
    let (s, t) = stepped(2, 80);
    let mut reg = MetricRegistry::new(1, 8);
    reg.maybe_sample(&SimView { sim: &s, terrain: &t, tick: 80 });
    assert_eq!(reg.latest("population"), Some(MetricValue::Scalar(s.population() as f64)));
    assert_eq!(reg.latest("species"), Some(MetricValue::Scalar(s.species_count() as f64)));
    assert_eq!(reg.latest("kills"), Some(MetricValue::Scalar(s.kills as f64)));
}

/// Extensibility surface: the default set exposes its membership by id.
#[test]
fn default_registry_lists_metrics() {
    let ids: Vec<_> = MetricRegistry::default().ids().collect();
    for want in ["population", "species", "thermal_correlation", "checksum"] {
        assert!(ids.contains(&want), "missing metric {want}");
    }
}
