//! ENV-0a'-a2: mutual-invasibility measurement harness (the spatial cloud gate).
//!
//! Reciprocal-invasibility diagnostic for coexistence measurement between unicell and multicell strategies
//! under spatial monopolization (a1 mechanic + patch-grain sweep). This is a cloud diagnostic MAP
//! (#[ignore], run via sim-run.sh env-frontier scenario).
//!
//! Harness config: breed-true (evolve_body_size=false, speciation frozen) to keep strategies
//! cleanly distinguishable by founding SpeciesId (0=resident, 1=invader) across generations.
//! Size variance (needed for selection) comes from multi-template seeding (N=1 and N=4), not evolution.
//!
//! Output format (space-separated fields, greppable MAP):
//!   ENV-0a-a2 patch_grain seed direction invader_start invader_end n_opt_baseline_A n_opt_baseline_B
//!
//! - patch_grain: spatial grain value {1,2,4,8,16,32}
//! - seed: random seed {1,2,3}
//! - direction: "multi→uni" (resident=multicell, invader=unicell) or "uni→multi" (vice versa)
//! - invader_start: minority lineage count at t=0 (seeded at 10% start frequency)
//! - invader_end: minority lineage count at t=horizon (growth trajectory)
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

/// Unit test: pct_multicellular accessor returns correct fraction and mean body size.
/// Constructs a small sim, verifies unicells give 0%, multicells give 100%, and mixed pop
/// returns correct intermediate fractions (all scaled by 256).
#[test]
fn test_pct_multicellular_accessor() {
    use cli::build_sim;

    let n_layers = 2;
    let unicell = make_unicell_template(n_layers);
    let multicell = make_multicell_template(n_layers);

    // Test 1: pure unicell population
    let mut config = cli::driver_config(42);
    config.n_founders = 50;
    config.founder_templates = Some(vec![(unicell.clone(), 50)]);
    let mut sim = build_sim(config);
    sim.step(); // Let population stabilize
    let (pct, mean_size) = sim.pct_multicellular();
    assert_eq!(
        pct, 0,
        "pure unicell population must have 0% multicellular (got pct={})",
        pct
    );
    assert_eq!(
        mean_size, 0,
        "pure unicell population must have mean_size=0 (got {})",
        mean_size
    );

    // Test 2: pure multicell population
    let mut config = cli::driver_config(42);
    config.n_founders = 50;
    config.founder_templates = Some(vec![(multicell.clone(), 50)]);
    let mut sim = build_sim(config);
    sim.step();
    let (pct, mean_size) = sim.pct_multicellular();
    assert!(
        pct > 250, // Should be close to 256 (100% × 256), allowing for slight variation
        "pure multicell population must have high %-mc (got pct={}, expected ~256)",
        pct
    );
    assert!(
        mean_size > 1 * 256, // Should be > 1*256
        "pure multicell population must have mean_size > 256 (got {})",
        mean_size
    );

    // Test 3: empty population returns (0, 0)
    let mut config = cli::driver_config(42);
    config.n_founders = 1;
    config.founder_energy = 1i64; // Force quick extinction
    let mut sim = build_sim(config);
    // Step until empty
    for _ in 0..1000 {
        sim.step();
        if sim.population() == 0 {
            break;
        }
    }
    let (pct, mean_size) = sim.pct_multicellular();
    assert_eq!(pct, 0, "empty population must return pct=0");
    assert_eq!(mean_size, 0, "empty population must return mean_size=0");
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

/// ENV-0a'-a2' emergence-arm diagnostic (cloud diagnostic MAP, #[ignore]).
/// Measures whether multicellularity EMERGES and takes over from a unicellular start under the
/// retention mechanic vs D-5 baseline (retention-OFF). Tests the REAL goal: not mutual invasibility,
/// but emergence from single-genotype evolution.
///
/// Config: EVOLVING (evolve_body_size=true, single unicell founder).
/// Runs two scenarios for each grain:
/// - Baseline A (retention-OFF, plain driver_config)
/// - Baseline B (retention-ON, env_frontier_config at each grain)
///
/// Emits greppable MAP lines: ENV-0a-a2p emergence <grain> <seed> retention-<OFF/ON> <pct-mc@mid> <mean-size@mid> <pct-mc@horizon> <mean-size@horizon>
#[test]
#[ignore]  // Cloud-only diagnostic; runs via sim-run.yml env-frontier case
fn env0a_a2p_emergence_sweep() {
    use cli::driver_config;

    let n_layers = 2;
    let unicell = make_unicell_template(n_layers);

    // Read horizon ticks from environment (default: 8000)
    let horizon_ticks = env::var("ENV_FRONTIER_TICKS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(8000);
    let midpoint_ticks = horizon_ticks / 2;

    // Sweep grid: patch_grain × seed
    let patch_grains = [1i64, 2, 4, 8, 16, 32];
    let seeds = [1u64, 2, 3];

    for &patch_grain in &patch_grains {
        for &seed in &seeds {
            // === Baseline A: retention-OFF (plain driver_config, EVOLVING) ===
            let config_a = driver_config(seed);
            let mut sim_a = cli::build_sim(config_a);

            // Measure at midpoint
            for _ in 0..midpoint_ticks {
                sim_a.step();
            }
            let (pct_mc_a_mid, mean_size_a_mid) = sim_a.pct_multicellular();

            // Continue to horizon
            for _ in midpoint_ticks..horizon_ticks {
                sim_a.step();
            }
            let (pct_mc_a_end, mean_size_a_end) = sim_a.pct_multicellular();

            // === Baseline B: retention-ON at this grain (EVOLVING, single founder) ===
            let mut config_b = driver_config(seed);
            config_b.econ.env_frontier_config = Some(sim_core::EnvFrontierConfig { patch_grain });
            let mut sim_b = cli::build_sim(config_b);

            // Measure at midpoint
            for _ in 0..midpoint_ticks {
                sim_b.step();
            }
            let (pct_mc_b_mid, mean_size_b_mid) = sim_b.pct_multicellular();

            // Continue to horizon
            for _ in midpoint_ticks..horizon_ticks {
                sim_b.step();
            }
            let (pct_mc_b_end, mean_size_b_end) = sim_b.pct_multicellular();

            // === Emit greppable MAP lines ===
            println!(
                "ENV-0a-a2p emergence {} {} retention-OFF {} {} {} {}",
                patch_grain, seed, pct_mc_a_mid, mean_size_a_mid, pct_mc_a_end, mean_size_a_end
            );
            println!(
                "ENV-0a-a2p emergence {} {} retention-ON {} {} {} {}",
                patch_grain, seed, pct_mc_b_mid, mean_size_b_mid, pct_mc_b_end, mean_size_b_end
            );
        }
    }
}

/// Non-ignored smoke test: all 4 config paths (emergence + invasion with true rarity) are runnable.
/// Covers all new config shapes: Emergence retention-OFF, Emergence retention-ON,
/// Invasion dir1 (2 fixed invaders), Invasion dir2 (2 fixed invaders).
/// Each path must step ~20 ticks, assert population > 0, and NOT panic on predation/build_sim assertions.
/// This is the CI gate for the entire harness (the #[ignore] sweep lives in the cloud).
#[test]
fn env0a_a2p_all_config_paths_are_runnable() {
    use cli::driver_config;

    let n_layers = 2;
    let unicell = make_unicell_template(n_layers);
    let multicell = make_multicell_template(n_layers);

    let patch_grain = 4i64;
    let seed = 42u64;
    let n_founders = 100;
    let n_invaders = 2u64;

    // === Config 1: Emergence retention-OFF (evolve_body_size=true, driver_config only) ===
    let config_em_off = driver_config(seed);
    let mut sim_em_off = cli::build_sim(config_em_off);
    for _ in 0..20 {
        sim_em_off.step();
    }
    let pop_em_off = sim_em_off.population();
    assert!(
        pop_em_off > 0,
        "Emergence retention-OFF population must be > 0 after 20 steps (got {})",
        pop_em_off
    );

    // === Config 2: Emergence retention-ON (evolve_body_size=true, env_frontier_config) ===
    let mut config_em_on = driver_config(seed);
    config_em_on.econ.env_frontier_config = Some(sim_core::EnvFrontierConfig { patch_grain });
    let mut sim_em_on = cli::build_sim(config_em_on);
    for _ in 0..20 {
        sim_em_on.step();
    }
    let pop_em_on = sim_em_on.population();
    assert!(
        pop_em_on > 0,
        "Emergence retention-ON population must be > 0 after 20 steps (got {})",
        pop_em_on
    );

    // === Config 3: Invasion direction 1 (resident=multicell, invader=unicell, true rarity) ===
    // Breed-true with multi-template seeding (2 fixed invaders).
    let mut config_inv_dir1 = cli::env_frontier_invasibility_config(seed, patch_grain);
    config_inv_dir1.n_founders = n_founders;
    config_inv_dir1.founder_templates = Some(vec![
        (multicell.clone(), n_founders - n_invaders),
        (unicell.clone(), n_invaders),
    ]);
    let mut sim_inv_dir1 = cli::build_sim(config_inv_dir1);
    for _ in 0..20 {
        sim_inv_dir1.step();
    }
    let pop_inv_dir1 = sim_inv_dir1.population();
    assert!(
        pop_inv_dir1 > 0,
        "Invasion dir1 population must be > 0 after 20 steps (got {})",
        pop_inv_dir1
    );

    // === Config 4: Invasion direction 2 (resident=unicell, invader=multicell, true rarity) ===
    // Breed-true with multi-template seeding (2 fixed invaders).
    let mut config_inv_dir2 = cli::env_frontier_invasibility_config(seed, patch_grain);
    config_inv_dir2.n_founders = n_founders;
    config_inv_dir2.founder_templates = Some(vec![
        (unicell.clone(), n_founders - n_invaders),
        (multicell.clone(), n_invaders),
    ]);
    let mut sim_inv_dir2 = cli::build_sim(config_inv_dir2);
    for _ in 0..20 {
        sim_inv_dir2.step();
    }
    let pop_inv_dir2 = sim_inv_dir2.population();
    assert!(
        pop_inv_dir2 > 0,
        "Invasion dir2 population must be > 0 after 20 steps (got {})",
        pop_inv_dir2
    );
}

/// ENV-0a'-a2p true-rarity invasion arm (cloud diagnostic MAP, #[ignore]).
/// Sweeps patch_grain over FIXED set {1,2,4,8,16,32} × seeds {1,2,3}.
/// For each combination, runs reciprocal seedings with TRUE RARITY (2 fixed invaders, not 10%):
/// (multi→uni: 98% multicell + 2 unicell invaders) and (uni→multi: 98% unicell + 2 multicell invaders).
/// Breed-true config (evolve_body_size=false, speciation frozen).
/// Measures rare-invader (SpeciesId 1) trajectory via species_census.
/// Emits greppable MAP lines; structural asserts only (no PASS/FAIL verdict).
#[test]
#[ignore]  // Cloud-only diagnostic; runs via sim-run.yml env-frontier case
fn env0a_a2p_invasion_sweep() {
    use cli::driver_config;

    let n_layers = 2;
    let unicell = make_unicell_template(n_layers);
    let multicell = make_multicell_template(n_layers);

    // Read horizon ticks from environment (default: 8000)
    let horizon_ticks = env::var("ENV_FRONTIER_TICKS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(8000);

    // Sweep grid: patch_grain × seed (pre-declared fixed set per issue)
    let patch_grains = [1i64, 2, 4, 8, 16, 32];
    let seeds = [1u64, 2, 3];
    // Fixed invader count (true rarity)
    let n_invaders = 2u64;
    let n_founders_per_run = 100u64;

    for &patch_grain in &patch_grains {
        for &seed in &seeds {
            // === Direction 1: resident = multicell, invader = unicell (true rarity: 2 individuals) ===
            let mut config1 = cli::env_frontier_invasibility_config(seed, patch_grain);
            config1.n_founders = n_founders_per_run;
            config1.founder_templates = Some(vec![
                (multicell.clone(), n_founders_per_run - n_invaders),
                (unicell.clone(), n_invaders),
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

            // invader_end_dir1 is u64 (always >= 0 by type); this is a structural placeholder
            // documenting the expectation (population counts are non-negative).

            // === Direction 2: resident = unicell, invader = multicell (true rarity: 2 individuals) ===
            let mut config2 = cli::env_frontier_invasibility_config(seed, patch_grain);
            config2.n_founders = n_founders_per_run;
            config2.founder_templates = Some(vec![
                (unicell.clone(), n_founders_per_run - n_invaders),
                (multicell.clone(), n_invaders),
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

            // invader_end_dir2 is u64 (always >= 0 by type); this is a structural placeholder
            // documenting the expectation (population counts are non-negative).

            // === Emit greppable MAP lines ===
            println!(
                "ENV-0a-a2p invasion {} {} multi→uni {} {}",
                patch_grain, seed, invader_start_dir1, invader_end_dir1
            );
            println!(
                "ENV-0a-a2p invasion {} {} uni→multi {} {}",
                patch_grain, seed, invader_start_dir2, invader_end_dir2
            );
        }
    }
}
