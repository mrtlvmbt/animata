//! ENV-0a'-a2: mutual-invasibility measurement harness (the spatial cloud gate).
//!
//! Reciprocal-invasibility diagnostic for coexistence measurement between unicell and multicell strategies
//! under spatial monopolization (a1 mechanic + patch-grain sweep). This is a cloud diagnostic MAP
//! (#[ignore], run via sim-run.sh env-frontier scenario).
//!
//! Harness config: breed-true (evolve_body_size=false, speciation frozen) to keep strategies
//! cleanly distinguishable by founding SpeciesId (0=resident, 1=invader) across generations.
//!
//! Output format (space-separated fields, greppable MAP):
//!   ENV-0a-a2 patch_grain seed direction invader_start invader_end n_opt_baseline_A n_opt_baseline_B
//!
//! - patch_grain: spatial grain value {1,2,4,8,16,32}
//! - seed: random seed {1,2,3}
//! - direction: "multi→uni" (resident=multicell, invader=unicell) or "uni→multi" (vice versa)
//! - invader_start: rare lineage count at t=0 (start)
//! - invader_end: rare lineage count at t=horizon (end)
//! - n_opt_baseline_A: mean body size (fixed-point 256-scale) without env_frontier_config
//! - n_opt_baseline_B: mean body size with env_frontier_config at current grain

use sim_core::{Genome, GrnSpec, MorphogenSpec, Boundary, EconParams};
use std::sync::Arc;
use std::env;

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

/// ENV-0a'-a2 grain-sweep coexistence harness (cloud diagnostic MAP, #[ignore]).
/// Sweeps patch_grain over FIXED set {1,2,4,8,16,32} × seeds {1,2,3}.
/// For each combination, runs reciprocal seedings (multi→uni and uni→multi) to horizon.
/// Measures rare-invader (SpeciesId 1) trajectory and port-check baselines.
/// Emits greppable MAP lines; structural asserts only (no PASS/FAIL verdict).
#[test]
#[ignore]  // Cloud-only diagnostic; runs via sim-run.yml env-frontier case
fn env0a_a2_invasibility_sweep() {
    use cli::driver_config;

    let n_layers = 2;
    let unicell = make_unicell_template(n_layers);
    let multicell = make_multicell_template(n_layers);

    // Read horizon ticks from environment (default: 2000 ≈ ~1000 generations)
    let horizon_ticks = env::var("ENV_FRONTIER_TICKS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(2000);

    // Sweep grid: patch_grain × seed (pre-declared fixed set per issue)
    let patch_grains = [1i64, 2, 4, 8, 16, 32];
    let seeds = [1u64, 2, 3];

    for &patch_grain in &patch_grains {
        for &seed in &seeds {
            // === Port-check Baseline A: retention-OFF (plain driver_config, no env_frontier) ===
            let mut config_baseline_a = driver_config(seed);
            let mut sim_baseline_a = cli::build_sim(config_baseline_a);
            for _ in 0..horizon_ticks {
                sim_baseline_a.step();
            }
            let n_opt_a = sim_baseline_a.n_opt(); // fixed-point 256-scale
            // Structural assert: N_opt must be >= 0
            assert!(n_opt_a >= 0, "baseline A n_opt must be >= 0");

            // === Port-check Baseline B: retention-ON at current grain (env_frontier_config) ===
            let mut config_baseline_b = cli::env_frontier_invasibility_config(seed, patch_grain);
            // Disable templates for baseline (test the mechanic alone)
            config_baseline_b.founder_templates = None;
            let mut sim_baseline_b = cli::build_sim(config_baseline_b);
            for _ in 0..horizon_ticks {
                sim_baseline_b.step();
            }
            let n_opt_b = sim_baseline_b.n_opt();
            assert!(n_opt_b >= 0, "baseline B n_opt must be >= 0");

            // === Direction 1: resident = multicell (90%), invader = unicell (10%) ===
            let mut config1 = cli::env_frontier_invasibility_config(seed, patch_grain);
            config1.n_founders = 100;
            config1.founder_templates = Some(vec![
                (multicell.clone(), 90),
                (unicell.clone(), 10),
            ]);
            let mut sim1 = cli::build_sim(config1);

            // Capture rare invader (SpeciesId 1) start count
            let census1_start = sim1.species_census();
            let invader_start_dir1: u64 = census1_start
                .iter()
                .find(|(id, _)| id.0 == 1)
                .map(|(_, count)| *count)
                .unwrap_or(0);

            // Run to horizon
            for _ in 0..horizon_ticks {
                sim1.step();
            }

            // Capture rare invader end count
            let census1_end = sim1.species_census();
            let invader_end_dir1: u64 = census1_end
                .iter()
                .find(|(id, _)| id.0 == 1)
                .map(|(_, count)| *count)
                .unwrap_or(0);

            // Structural asserts
            assert!(invader_end_dir1 >= 0, "dir1 invader_end must be >= 0");

            // === Direction 2: resident = unicell (90%), invader = multicell (10%) ===
            let mut config2 = cli::env_frontier_invasibility_config(seed, patch_grain);
            config2.n_founders = 100;
            config2.founder_templates = Some(vec![
                (unicell.clone(), 90),
                (multicell.clone(), 10),
            ]);
            let mut sim2 = cli::build_sim(config2);

            // Capture rare invader start count
            let census2_start = sim2.species_census();
            let invader_start_dir2: u64 = census2_start
                .iter()
                .find(|(id, _)| id.0 == 1)
                .map(|(_, count)| *count)
                .unwrap_or(0);

            // Run to horizon
            for _ in 0..horizon_ticks {
                sim2.step();
            }

            // Capture rare invader end count
            let census2_end = sim2.species_census();
            let invader_end_dir2: u64 = census2_end
                .iter()
                .find(|(id, _)| id.0 == 1)
                .map(|(_, count)| *count)
                .unwrap_or(0);

            // Structural asserts
            assert!(invader_end_dir2 >= 0, "dir2 invader_end must be >= 0");

            // === Emit greppable MAP lines ===
            println!(
                "ENV-0a-a2 {} {} multi→uni {} {} {} {}",
                patch_grain, seed, invader_start_dir1, invader_end_dir1, n_opt_a, n_opt_b
            );
            println!(
                "ENV-0a-a2 {} {} uni→multi {} {} {} {}",
                patch_grain, seed, invader_start_dir2, invader_end_dir2, n_opt_a, n_opt_b
            );
        }
    }
}
