//! Tests for the pressure framework (PR3).

use crate::env::EnvSample;
use crate::pressure::{PressureRegistry, TickCtx};
use crate::terrain::VoxelTerrain;
use super::{column_index, Sim};

#[test]
fn pressure_registry_evaluates_all_pressures() {
    let terrain = VoxelTerrain::new(1);
    let sim = Sim::new(42, &terrain);
    let registry = PressureRegistry::new();

    if sim.creatures.is_empty() {
        return;
    }

    let c = &sim.creatures[0];
    let col = column_index(c.pos);
    let mut env = EnvSample::new(col, 0, &terrain);

    let ctx = TickCtx {
        stratum_count: [0.0; 4],
        n_auto: 0,
        autotroph_shading: 1.0,
    };

    let effect = registry.eval_all(&mut env, &c.pheno, &c.genome, &ctx);

    // All pressures should produce finite values.
    // For now, just check that eval doesn't panic.
    assert!(effect.food_mult.is_finite());
    assert!(effect.energy_add.is_finite());
    assert!(effect.metab_mult.is_finite());
    assert!(effect.detection_bias.is_finite());
    assert!(effect.mortality_add.is_finite());
    assert!(effect.repro_mult.is_finite());
}

#[test]
fn climate_pressure_returns_identity_when_disabled() {
    // Placeholder: when pressures are disabled via config (PR4), verify identity effect.
    // For now, all pressures are always enabled.
    let registry = PressureRegistry::new();
    assert_eq!(registry.pressures.len(), 6, "Should have 6 default pressures");
}
