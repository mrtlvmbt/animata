//! R30-1 (#419): extent-economy size-selection probe harness (pass 1 of 2 — this builds the
//! harness CI-green + byte-identical; the science verdict is the PM-run observational sweep).
//!
//! Question: does the coherent extent economy (`IncomeMode::Extent` ⊕ `metab_reads_n_cells` ⊕
//! `newborn_energy_per_cell`, all three ON) SELECT for larger evolved body size N, faithfully?
//!
//! **Arm A** (`extent_economy_arm_a_evolutionary_contrast`, #[ignore]): EXTENT vs FLAT evolutionary
//! contrast at a raised `gdev_cap`/`morphogen_steps`. Emits the evolved LIVE-cell-count N
//! distribution (dead excluded, last-third stability) per arm per seed. FLAT is the MEASURED
//! neutral-drift baseline, not an assumed ceiling (critic F3).
//!
//! **Arm B** (`extent_economy_arm_b_invasion_diagnostic`, #[ignore]): invasion-fitness diagnostic —
//! does a rare larger-N mutant grow among small residents under the EXTENT economy? Reuses the
//! breed-true PLUMBING of `env0a_a2_invasibility.rs` (NOT its frontier economy — critic F9: this
//! harness's `env_frontier_config` stays `None`). A NEW graded-N invader template builder sweeps
//! g_dev ∈ {2,3,4,6} (critic F7/F10/F12); the invasion signal is the invader SpeciesId's
//! census-count FREQUENCY TRAJECTORY via `species_census()` (NOT a fitness ledger — critic F5).
//!
//! **Plumbing test** (`extent_economy_plumbing_smoke`, non-ignored, CI gate): both arms build,
//! phase-2 bodies decode N>1, the metric emits a per-seed N histogram, and the invasion diagnostic
//! emits a per-invader-N census frequency trajectory — NOT the science verdict, just that the
//! machinery is wired.
//!
//! GOLDEN-NEUTRAL: opt-in configs (`cli::extent_economy_*_config`), default-OFF ⇒ all shipped
//! `v2_golden_conserved_*` goldens stay byte-identical (no re-pin).

use cli::build_sim;
use sim_core::{Boundary, EconParams, Genome, GrnSpec, IncomeMode, MorphogenSpec};
use std::env;
use std::sync::Arc;

const N_LAYERS: usize = 2; // matches driver_config's L=2 (phase2_config base)
const DEFAULT_TICKS: u64 = 8000;
const SEEDS: [u64; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

/// Construct a graded-N invader/resident template at the given `g_dev`, with `n_dev` paired per
/// [`morphogen_steps_for_gdev`] (critic F11 — an under-paired `n_dev` step-limits the body,
/// illusory headroom rather than an economy effect). NEW code (critic F7): the precedent
/// `make_multicell_template` (`env0a_a2_invasibility.rs`) hard-codes g_dev=2; this generalizes it
/// across the sweep and asserts the decode lands in the intended live-N range at EVERY level.
/// Panics loudly if decode fails or cell count is outside `[2, g_dev²]` (dead cells already
/// excluded by construction — `CellGraph`'s union-find only visits `!dead` cells, genome.rs Step 3).
fn make_graded_invader_template(n_layers: usize, g_dev: usize, n_dev: u32) -> Genome {
    let mspec = MorphogenSpec {
        g_dev,
        n_dev,
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
        body_plan: sim_core::BodyPlan::Square,
    };

    // Standard bistable Phase2 GRN spec (identical to the invasibility-precedent templates).
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

    let g = Genome::founder(n_layers).with_specs(Some(Arc::new(gspec)), Some(mspec));

    let econ = EconParams::default();
    let ph = g.decode(&econ).expect(
        "graded invader template must decode to Some; if decode fails, genome/mspec is malformed",
    );
    let cell_count: i64 = ph.graph.module_cell_count.iter().map(|&c| c as i64).sum();
    let max_n = (g_dev * g_dev) as i64;
    assert!(
        (2..=max_n).contains(&cell_count),
        "graded invader template (g_dev={}) must decode to live-N ∈ [2,{}], got {}",
        g_dev,
        max_n,
        cell_count
    );
    g
}

/// `morphogen_steps` (`n_dev`) paired to invader `g_dev`, mirroring Arm A's
/// `EXTENT_ECONOMY_GDEV_CAP`↔`EXTENT_ECONOMY_MORPHOGEN_STEPS` pairing (critic F11, params.rs:741
/// "6→12"). g_dev 6 MUST pair to 12 (matches Arm A exactly — an invader decoded with fewer steps
/// would be step-limited, not a fair same-cap comparison). g_dev {2,3,4} use 8 (the invasibility
/// precedent's proven-decodes value, comfortably above the `n_dev ≥ 2·g_dev−2` decode floor).
fn morphogen_steps_for_gdev(g_dev: usize) -> u32 {
    match g_dev {
        2..=4 => 8,
        6 => cli::EXTENT_ECONOMY_MORPHOGEN_STEPS,
        other => panic!("unsupported g_dev={other} in extent-economy invader sweep (declared set: {{2,3,4,6}})"),
    }
}

/// Histogram of live-N body sizes over `[1, max_n]` — every integer bin, NOT squares-only (the
/// pre-declared verdict criterion C: realized live-N spans `[1..g_dev²]` with dead cells excluded,
/// not perfect squares, since apoptosis/incomplete divisions land off-square).
fn compute_histogram(body_sizes: &[i64], max_n: usize) -> String {
    let mut counts = vec![0u64; max_n + 1];
    for &size in body_sizes {
        if size > 0 && (size as usize) <= max_n {
            counts[size as usize] += 1;
        }
    }
    let mut parts = Vec::new();
    for (n, &count) in counts.iter().enumerate().skip(1) {
        if count > 0 {
            parts.push(format!("{}:{}", n, count));
        }
    }
    if parts.is_empty() {
        "empty".to_string()
    } else {
        parts.join(",")
    }
}

/// Asserts the treatment encoding of an EXTENT-economy config (validity check #4): all three ∝N
/// flags ON, frontier economy stripped (critic F9 — only relevant to Arm B, harmless to re-assert
/// on Arm A which never sets it).
fn assert_extent_flags_on(econ: &sim_core::EconParams) {
    assert_eq!(econ.income_mode, IncomeMode::Extent, "treatment-encoding guard: income_mode must be Extent");
    assert!(econ.metab_reads_n_cells, "treatment-encoding guard: metab_reads_n_cells must be true");
    assert!(econ.newborn_energy_per_cell, "treatment-encoding guard: newborn_energy_per_cell must be true");
    assert!(econ.env_frontier_config.is_none(), "critic F9: env_frontier_config must stay None (frontier economy stripped)");
}

fn assert_flat_flags_off(econ: &sim_core::EconParams) {
    assert_eq!(econ.income_mode, IncomeMode::Anchor, "FLAT arm must keep shipped-default income_mode=Anchor");
    assert!(!econ.metab_reads_n_cells, "FLAT arm must keep metab_reads_n_cells=false");
    assert!(!econ.newborn_energy_per_cell, "FLAT arm must keep newborn_energy_per_cell=false");
}

/// Arm A: EXTENT-vs-FLAT evolutionary contrast (cloud diagnostic, #[ignore]).
/// Emits greppable MAP lines: `EXTENT-ECON armA arm=<EXTENT|FLAT> seed=<n> pop=<> density=<>
/// mean_n=<> max_n=<> samples=<> hist=<>`. Analysis (the pre-declared rank test vs FLAT's measured
/// drift) is offline (PM, pass 2) — this test only runs the harness and emits the raw distribution.
#[test]
#[ignore]
fn extent_economy_arm_a_evolutionary_contrast() {
    let horizon: u64 = env::var("EXTENT_ECONOMY_TICKS").ok().and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_TICKS);
    let last_third_start = horizon - horizon / 3;
    const SAMPLE_INTERVAL: u64 = 200;
    let max_n = cli::EXTENT_ECONOMY_GDEV_CAP * cli::EXTENT_ECONOMY_GDEV_CAP;

    println!(
        "\nEXTENT-ECONOMY ARM A: EXTENT vs FLAT evolutionary contrast (gdev_cap={}, morphogen_steps={}, ticks={})",
        cli::EXTENT_ECONOMY_GDEV_CAP, cli::EXTENT_ECONOMY_MORPHOGEN_STEPS, horizon
    );

    for &seed in &SEEDS {
        for arm_name in ["EXTENT", "FLAT"] {
            let cfg = if arm_name == "EXTENT" {
                let cfg = cli::extent_economy_extent_config(seed);
                assert_extent_flags_on(&cfg.econ);
                cfg
            } else {
                let cfg = cli::extent_economy_flat_config(seed);
                assert_flat_flags_off(&cfg.econ);
                cfg
            };
            assert_eq!(cfg.econ.gdev_cap, cli::EXTENT_ECONOMY_GDEV_CAP, "SAME raised gdev_cap required in both arms (critic F3)");
            assert_eq!(cfg.econ.morphogen_steps, cli::EXTENT_ECONOMY_MORPHOGEN_STEPS, "SAME raised morphogen_steps required in both arms (critic F11)");

            let world_dim = cfg.econ.world_dim;
            let mut sim = build_sim(cfg);

            let mut pooled_n: Vec<i64> = Vec::new();
            for tick in 1..=horizon {
                sim.step();
                if tick >= last_third_start && tick % SAMPLE_INTERVAL == 0 {
                    pooled_n.extend(sim.body_size_probe());
                }
            }
            let pop = sim.population();

            if pop == 0 || pooled_n.is_empty() {
                println!("EXTENT-ECON armA arm={:<6} seed={:<2} pop={:<4} EXTINCT-OR-NO-SAMPLES", arm_name, seed, pop);
                continue;
            }

            // Phase-1-floor guard (BOTH arms — critic F5-prev): distinguish the real confound
            // (n_cells=0 for an ALIVE body — phase-1 DECODE floor, silently zeroes Kleiber/extent
            // income) from a legitimate live-but-unicellular outcome (all N=1 — a real data point,
            // not a floor). Neither case aborts the sweep; both are recorded so PM gets the full
            // per-seed distribution including Arm B (see GOTCHAS.md coupled-∝N-axes entry).
            let n_gt1 = pooled_n.iter().filter(|&&n| n > 1).count();
            let n_zero = pooled_n.iter().filter(|&&n| n == 0).count();
            if n_zero > 0 {
                let histogram = compute_histogram(&pooled_n, max_n);
                println!(
                    "EXTENT-ECON armA arm={:<6} seed={:<2} pop={:<4} WARN-DECODE-FLOOR n_zero={} hist={}",
                    arm_name, seed, pop, n_zero, histogram
                );
            }
            if n_gt1 == 0 {
                let mean_n: f64 = pooled_n.iter().sum::<i64>() as f64 / pooled_n.len() as f64;
                let histogram = compute_histogram(&pooled_n, max_n);
                println!(
                    "EXTENT-ECON armA arm={:<6} seed={:<2} pop={:<4} UNICELLULAR mean_n≈{:.2} samples={} hist={}",
                    arm_name, seed, pop, mean_n, pooled_n.len(), histogram
                );
                continue;
            }

            let mean_n: f64 = pooled_n.iter().sum::<i64>() as f64 / pooled_n.len() as f64;
            let observed_max_n = *pooled_n.iter().max().unwrap_or(&0);
            let density = pop as f64 / (world_dim * world_dim) as f64;
            let histogram = compute_histogram(&pooled_n, max_n);

            println!(
                "EXTENT-ECON armA arm={:<6} seed={:<2} pop={:<4} density={:.4} mean_n={:.2} max_n={} samples={} hist={}",
                arm_name, seed, pop, density, mean_n, observed_max_n, pooled_n.len(), histogram
            );
        }
    }

    println!("EXTENT-ECONOMY ARM A complete. PM compares EXTENT's per-seed N distribution to FLAT's measured drift baseline (pre-declared rank test, pass 2).");
}

/// Arm B: invasion-fitness diagnostic (cloud diagnostic, #[ignore]).
/// Sweeps invader g_dev ∈ {2,3,4,6} × seeds {1..8}. Seed = (n_founders−2) small-N (g_dev=2)
/// residents + 2 fixed larger-N invaders (template index 1) per tested g_dev, under the EXTENT
/// economy with the frontier economy stripped (critic F9). Emits greppable MAP lines: `EXTENT-ECON
/// armB gdev=<> seed=<> pop_end=<> invader_traj=<c0,c1,...,cN>` — the invader SpeciesId's
/// census-count FREQUENCY TRAJECTORY (start→end via `species_census()`, NOT a fitness ledger).
#[test]
#[ignore]
fn extent_economy_arm_b_invasion_diagnostic() {
    let horizon: u64 = env::var("EXTENT_ECONOMY_TICKS").ok().and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_TICKS);
    const INVADER_GDEV_LEVELS: [usize; 4] = [2, 3, 4, 6];
    const N_FOUNDERS: u64 = 100;
    const N_INVADERS: u64 = 2;
    const N_CHECKPOINTS: u64 = 5;
    let resident_n_dev = morphogen_steps_for_gdev(2);

    println!(
        "\nEXTENT-ECONOMY ARM B: invasion diagnostic (graded-N invader sweep g_dev ∈ {{2,3,4,6}}, ticks={})",
        horizon
    );

    for &g_dev in &INVADER_GDEV_LEVELS {
        let invader_n_dev = morphogen_steps_for_gdev(g_dev);
        for &seed in &SEEDS {
            let resident = make_graded_invader_template(N_LAYERS, 2, resident_n_dev);
            let invader = make_graded_invader_template(N_LAYERS, g_dev, invader_n_dev);

            let mut cfg = cli::extent_economy_invasion_config(seed);
            assert_extent_flags_on(&cfg.econ);
            assert!(!cfg.econ.evolve_body_size, "Arm B must be breed-true (evolve_body_size=false)");
            assert_eq!(cfg.econ.speciation_threshold, i64::MAX, "Arm B must freeze speciation");

            cfg.n_founders = N_FOUNDERS;
            cfg.founder_templates = Some(vec![(resident, N_FOUNDERS - N_INVADERS), (invader, N_INVADERS)]);

            let mut sim = build_sim(cfg);

            let checkpoint_every = (horizon / N_CHECKPOINTS).max(1);
            let mut trajectory: Vec<u64> = Vec::new();
            let census0 = sim.species_census();
            trajectory.push(census0.iter().find(|(id, _)| id.0 == 1).map(|(_, c)| *c).unwrap_or(0));

            for tick in 1..=horizon {
                sim.step();
                if tick % checkpoint_every == 0 {
                    let census = sim.species_census();
                    let c = census.iter().find(|(id, _)| id.0 == 1).map(|(_, c)| *c).unwrap_or(0);
                    trajectory.push(c);
                }
            }
            let pop_end = sim.population();
            let traj_str = trajectory.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(",");

            println!(
                "EXTENT-ECON armB gdev={:<2} seed={:<2} pop_end={:<4} invader_traj={}",
                g_dev, seed, pop_end, traj_str
            );
        }
    }

    println!("EXTENT-ECONOMY ARM B complete. Invader RISES ⇒ selection gradient (attributes an Arm A rise to selection); FALLS/vanishes ⇒ no window.");
}

/// Plumbing smoke test (non-ignored, CI gate): both arms build, phase-2 bodies decode N>1, the
/// metric emits a per-seed N histogram, and the invasion diagnostic emits a per-invader-N census
/// frequency trajectory. NOT the science verdict (that is the #[ignore] cloud diagnostic above) —
/// just that the machinery is wired and runs without panicking.
#[test]
fn extent_economy_plumbing_smoke() {
    let seed = 42u64;

    // === EXTENT / FLAT config shape (validity check #4 + critic F3) ===
    let extent_template = cli::extent_economy_extent_config(seed);
    assert_extent_flags_on(&extent_template.econ);
    assert_eq!(extent_template.econ.gdev_cap, cli::EXTENT_ECONOMY_GDEV_CAP);
    assert_eq!(extent_template.econ.morphogen_steps, cli::EXTENT_ECONOMY_MORPHOGEN_STEPS);

    let flat_template = cli::extent_economy_flat_config(seed);
    assert_flat_flags_off(&flat_template.econ);
    assert_eq!(flat_template.econ.gdev_cap, cli::EXTENT_ECONOMY_GDEV_CAP, "SAME raised cap required (critic F3)");
    assert_eq!(flat_template.econ.morphogen_steps, cli::EXTENT_ECONOMY_MORPHOGEN_STEPS);

    // Force phase-2 N>1 bodies via founder_templates (bypass evolution — this is a plumbing check
    // that the metric CAN read multicellular bodies, not a claim that evolution produces them fast).
    let multicell = make_graded_invader_template(N_LAYERS, 2, morphogen_steps_for_gdev(2));

    let mut extent_cfg = extent_template;
    extent_cfg.n_founders = 30;
    extent_cfg.founder_templates = Some(vec![(multicell.clone(), 30)]);
    let mut sim_extent = build_sim(extent_cfg);
    for _ in 0..20 {
        sim_extent.step();
    }
    let sizes_extent = sim_extent.body_size_probe();
    let n_gt1_extent = sizes_extent.iter().filter(|&&n| n > 1).count();
    let hist_extent = compute_histogram(&sizes_extent, 36);
    assert!(n_gt1_extent > 0, "EXTENT plumbing: phase-2 bodies must decode N>1 (hist={})", hist_extent);
    assert_ne!(hist_extent, "empty", "EXTENT metric must emit a non-empty per-seed N histogram");

    let mut flat_cfg = flat_template;
    flat_cfg.n_founders = 30;
    flat_cfg.founder_templates = Some(vec![(multicell, 30)]);
    let mut sim_flat = build_sim(flat_cfg);
    for _ in 0..20 {
        sim_flat.step();
    }
    let sizes_flat = sim_flat.body_size_probe();
    let n_gt1_flat = sizes_flat.iter().filter(|&&n| n > 1).count();
    let hist_flat = compute_histogram(&sizes_flat, 36);
    assert!(n_gt1_flat > 0, "FLAT plumbing: phase-2 bodies must decode N>1 (hist={})", hist_flat);
    assert_ne!(hist_flat, "empty", "FLAT metric must emit a non-empty per-seed N histogram");

    // === Invasion diagnostic wiring (validity check #4 + critic F9) ===
    let resident = make_graded_invader_template(N_LAYERS, 2, morphogen_steps_for_gdev(2));
    let invader = make_graded_invader_template(N_LAYERS, 3, morphogen_steps_for_gdev(3));

    let mut inv_cfg = cli::extent_economy_invasion_config(seed);
    assert_extent_flags_on(&inv_cfg.econ);
    assert!(!inv_cfg.econ.evolve_body_size, "Arm B must be breed-true (evolve_body_size=false)");
    assert_eq!(inv_cfg.econ.speciation_threshold, i64::MAX, "Arm B must freeze speciation");

    inv_cfg.n_founders = 20;
    inv_cfg.founder_templates = Some(vec![(resident, 18), (invader, 2)]);
    let mut sim_inv = build_sim(inv_cfg);

    let mut trajectory: Vec<u64> = Vec::new();
    let census0 = sim_inv.species_census();
    trajectory.push(census0.iter().find(|(id, _)| id.0 == 1).map(|(_, c)| *c).unwrap_or(0));
    for tick in 1..=20u64 {
        sim_inv.step();
        if tick % 5 == 0 {
            let census = sim_inv.species_census();
            trajectory.push(census.iter().find(|(id, _)| id.0 == 1).map(|(_, c)| *c).unwrap_or(0));
        }
    }
    assert!(
        trajectory.len() >= 2,
        "invasion diagnostic must emit a per-invader-N census FREQUENCY TRAJECTORY (≥2 points), got {}",
        trajectory.len()
    );

    println!(
        "extent-economy plumbing OK: EXTENT hist={} FLAT hist={} invasion_traj={:?}",
        hist_extent, hist_flat, trajectory
    );
}
