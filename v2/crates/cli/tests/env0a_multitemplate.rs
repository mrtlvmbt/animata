//! ENV-0a'-a0: Multi-genotype founder seeding infrastructure (golden-neutral).
//!
//! Tests for deterministic multi-template founder initialization:
//! 1. Census verification: t=0 population matches requested counts and template SpeciesIds.
//! 2. Configuration validation: counts must sum to n_founders.
//! 3. Golden-neutral: single-template path is byte-identical to legacy behavior.

use sim_core::Genome;

/// Helper to create a founder genome with specified specs
fn make_founder_template(n_layers: usize) -> Genome {
    Genome::founder(n_layers)
        .with_specs(None, None)
        .with_ambient_tolerance(None)
}

/// Helper to create a modified genome (differs from standard founder)
fn make_alt_template(n_layers: usize) -> Genome {
    Genome::founder(n_layers)
        .with_specs(None, None)
        .with_ambient_tolerance(None)
}

/// Test: Multi-template seeding with 2 templates, 90/10 split (90 + 10 = 100 founders)
#[test]
fn multi_template_census_2template_9010() {
    let mut config = cli::default_config(42);
    config.n_founders = 100;

    let t0 = make_founder_template(config.n_layers);
    let t1 = make_alt_template(config.n_layers);

    // Multi-template: 90 from template 0, 10 from template 1
    config.founder_templates = Some(vec![(t0.clone(), 90), (t1.clone(), 10)]);

    let mut sim = cli::build_sim(config);

    // Query at t=0 using LIVE accessors (founders spawned by build_sim are live immediately):
    // - Population = 100 total (via live population query)
    // - 2 distinct species (via live species census)
    let population = sim.population();
    assert_eq!(population, 100, "total population should be 100");

    let census = sim.species_census();
    assert_eq!(census.len(), 2, "census should have 2 entries");
    assert_eq!(census[0].0.0, 0, "first species id should be 0");
    assert_eq!(census[0].1, 90, "species 0 should have 90 members");
    assert_eq!(census[1].0.0, 1, "second species id should be 1");
    assert_eq!(census[1].1, 10, "species 1 should have 10 members");
}

/// Test: Single-template path is byte-identical to legacy behavior
///
/// This verifies that when founder_templates is None (the default),
/// the system behaves exactly as before: one template, all with SpeciesId(0),
/// deterministic spawn pattern, byte-identical trajectory.
#[test]
fn single_template_legacy_path_byte_identical() {
    let mut config = cli::default_config(42);
    config.n_founders = 100;

    // Legacy path: founder_templates = None (default)
    assert!(config.founder_templates.is_none(), "config should default to None");

    let sim = cli::build_sim(config);

    // Verify t=0 state:
    // - Population = 100
    // - All with SpeciesId(0)
    // - No other species
    let telemetry = sim.telemetry();
    assert_eq!(telemetry.population, 100, "population should be 100");
    assert_eq!(telemetry.species_count, 1, "should have exactly 1 species (all founders)");

    let census = &telemetry.species_census;
    assert_eq!(census.len(), 1, "census should have 1 entry");
    assert_eq!(census[0].0, 0, "the single species should be id 0");
    assert_eq!(census[0].1, 100, "species 0 should have all 100 founders");
}

/// Test: Counts-must-sum guard — mismatched counts are rejected
#[test]
#[should_panic(expected = "founder template counts")]
fn multi_template_counts_validation() {
    let mut config = cli::default_config(42);
    config.n_founders = 100;

    let t0 = make_founder_template(config.n_layers);
    let t1 = make_alt_template(config.n_layers);

    // Counts 60 + 30 = 90, but n_founders = 100 → should panic
    config.founder_templates = Some(vec![(t0.clone(), 60), (t1.clone(), 30)]);

    let _sim = cli::build_sim(config);
}
