//! ENV-0a'-a2: mutual-invasibility measurement harness (the spatial cloud gate).
//!
//! Reciprocal-invasibility test for coexistence measurement between unicell and multicell strategies
//! under spatial monopolization (a1 mechanic + patch-grain sweep). This is a cloud diagnostic MAP
//! (#[ignore], run via sim-run.sh env-frontier scenario).
//!
//! Harness config: breed-true (evolve_body_size=false, speciation frozen) to keep strategies
//! cleanly distinguishable by founding SpeciesId (0=resident, 1=invader) across generations.

use sim_core::{Genome, GrnSpec, MorphogenSpec, Boundary, EconParams};
use std::sync::Arc;

/// Construct unicell template (g_dev=1 → 1 cell) with verified decode.
/// Panics loudly if decode fails or cell count ≠ 1.
fn make_unicell_template(n_layers: usize) -> Genome {
    // Use phase2 morphogen spec with g_dev=1 for unicell
    let mspec = MorphogenSpec {
        g_dev: 1,
        n_dev: 8,
        boundary: Boundary::Reflecting,
        diffuse_shift: 3,
        decay_num: 1,
        decay_shift: 4,
        seed_scale: 4096,
        stop_threshold: 0,
        apoptosis_threshold: None,
        germ_threshold: None,
        supply_source: None,
        adhesion_threshold: None,
    };

    // Phase2 GRN spec (standard bistable)
    let gspec = GrnSpec::new(
        2,
        vec![32, -32, -32, 32],
        vec![0, 0],
        vec![0, 0],
        3,
        12,
        0,
        0,
        vec![144, 112],
    );

    let g = Genome::founder(n_layers)
        .with_specs(Some(Arc::new(gspec)), Some(mspec));

    // Verify decode: must decode to 1 cell, no panics
    let econ = EconParams::default();
    let ph = g.decode(&econ).expect(
        "unicell template (g_dev=1) must decode to Some; if decode fails, genome is malformed"
    );
    let cell_count = ph.graph.module_cell_count.iter().map(|&c| c as i64).sum::<i64>();
    assert_eq!(
        cell_count, 1,
        "unicell template decode must yield exactly 1 cell, got {}",
        cell_count
    );

    g
}

/// Construct multicell template (g_dev=2 → 4 cells) with verified decode.
/// Panics loudly if decode fails or cell count ∉ [2,4].
fn make_multicell_template(n_layers: usize) -> Genome {
    // Use phase2 morphogen spec with g_dev=2 for multicell
    let mspec = MorphogenSpec {
        g_dev: 2,
        n_dev: 8,
        boundary: Boundary::Reflecting,
        diffuse_shift: 3,
        decay_num: 1,
        decay_shift: 4,
        seed_scale: 4096,
        stop_threshold: 0,
        apoptosis_threshold: None,
        germ_threshold: None,
        supply_source: None,
        adhesion_threshold: None,
    };

    // Phase2 GRN spec (standard bistable)
    let gspec = GrnSpec::new(
        2,
        vec![32, -32, -32, 32],
        vec![0, 0],
        vec![0, 0],
        3,
        12,
        0,
        0,
        vec![144, 112],
    );

    let g = Genome::founder(n_layers)
        .with_specs(Some(Arc::new(gspec)), Some(mspec));

    // Verify decode: must decode to [2,4] cells, no panics
    let econ = EconParams::default();
    let ph = g.decode(&econ).expect(
        "multicell template (g_dev=2) must decode to Some; if decode fails, genome is malformed"
    );
    let cell_count = ph.graph.module_cell_count.iter().map(|&c| c as i64).sum::<i64>();
    assert!(
        (2..=4).contains(&cell_count),
        "multicell template decode must yield cell count ∈ [2,4], got {}",
        cell_count
    );

    g
}

/// Sanity test: verify template construction doesn't panic and decodes correctly.
#[test]
fn env0a_a2_templates_construct_and_decode() {
    let n_layers = 2; // default
    let unicell = make_unicell_template(n_layers);
    let multicell = make_multicell_template(n_layers);

    // Verify they differ in g_dev
    assert_eq!(
        unicell.morphogen_spec.expect("unicell must have mspec").g_dev,
        1,
        "unicell g_dev must be 1"
    );
    assert_eq!(
        multicell.morphogen_spec.expect("multicell must have mspec").g_dev,
        2,
        "multicell g_dev must be 2"
    );
}

/// Reciprocal invasibility harness: for a fixed patch_grain, measure rare-invader growth
/// in both directions (resident = multicell, invader = unicell) and vice versa.
/// Structure: reciprocal seeding at ~90/10 split, fixed horizon generations, track SpeciesId 1
/// via species_census(). Structural asserts only (ran to horizon, counts ≥ 0), no PASS/FAIL verdict.
#[test]
#[ignore] // cloud-only diagnostic MAP
fn env0a_a2_reciprocal_invasibility_harness() {
    let n_layers = 2;
    let unicell = make_unicell_template(n_layers);
    let multicell = make_multicell_template(n_layers);

    let patch_grain = 4i64; // fixed for this smoke test; swept {1,2,4,8,16,32} in cloud
    let seed = 42u64; // fixed for this smoke test; ≥3 seeds in cloud

    // Direction 1: resident = multicell (90%), invader = unicell (10%)
    let mut config1 = cli::env_frontier_invasibility_config(seed, patch_grain);
    config1.n_founders = 100;
    // Multi-template seeding: 90 multicell (SpeciesId 0) + 10 unicell (SpeciesId 1)
    config1.founder_templates = Some(vec![
        (multicell.clone(), 90),
        (unicell.clone(), 10),
    ]);

    let mut sim1 = cli::build_sim(config1);

    // Run to horizon (e.g., 100 generations ≈ 2000 ticks at typical turnover)
    // For this structural test, just run a short simulation
    let horizon_ticks = 200;
    for _ in 0..horizon_ticks {
        sim1.step();
    }

    // Read species census: SpeciesId 1 is the rare invader (unicell)
    let census1 = sim1.species_census();
    let invader_end_dir1 = census1.iter().find(|(id, _)| id.0 == 1).map(|(_, count)| *count).unwrap_or(0);

    // Structural assert: population counts are non-negative (always true, but documents expectation)
    assert!(invader_end_dir1 >= 0, "direction 1: invader count must be ≥ 0");

    // Direction 2: resident = unicell (90%), invader = multicell (10%)
    let mut config2 = cli::env_frontier_invasibility_config(seed, patch_grain);
    config2.n_founders = 100;
    // Multi-template seeding: 90 unicell (SpeciesId 0) + 10 multicell (SpeciesId 1)
    config2.founder_templates = Some(vec![
        (unicell.clone(), 90),
        (multicell.clone(), 10),
    ]);

    let mut sim2 = cli::build_sim(config2);

    // Run to same horizon
    for _ in 0..horizon_ticks {
        sim2.step();
    }

    // Read species census: SpeciesId 1 is the rare invader (multicell)
    let census2 = sim2.species_census();
    let invader_end_dir2 = census2.iter().find(|(id, _)| id.0 == 1).map(|(_, count)| *count).unwrap_or(0);

    // Structural assert: population counts are non-negative
    assert!(invader_end_dir2 >= 0, "direction 2: invader count must be ≥ 0");

    // No PASS/FAIL verdict — this is diagnostic telemetry only.
    // PM reads both directions' trajectories to infer coexistence domain.
    eprintln!(
        "ENV-0a'-a2 invasibility (grain=?, seed=42): dir1(multi→uni) invader_end={}, dir2(uni→multi) invader_end={}",
        invader_end_dir1, invader_end_dir2
    );
}
