//! R30-1 (#419) extent-economy size-selection harness, extended by #425 to a 2×2 factorial testing
//! extent-economy ⊕ ENV-0a′ spatial monopolization. Pass 1 of 2 for BOTH issues: this builds the
//! harness CI-green + byte-identical; the science verdict is the PM-run observational sweep.
//!
//! ## The 2×2 factorial (issue #425)
//!
//! Four arms, SAME `driver_config` base + SAME raised `gdev_cap`/`morphogen_steps`, differing ONLY
//! in {3 extent flags} × {`env_frontier_config`}:
//!
//! | arm             | 3 extent flags | `env_frontier_config`           | role                       |
//! |-----------------|----------------|---------------------------------|----------------------------|
//! | FLAT            | off            | `None`                          | neutral-drift baseline    |
//! | EXTENT          | on             | `None`                          | the concluded NULL (ref)  |
//! | FRONTIER        | off            | `Some(ENV_FRONTIER_PATCH_GRAIN)`| monopolization alone (ref)|
//! | EXTENT+FRONTIER | on             | `Some(ENV_FRONTIER_PATCH_GRAIN)`| the composition (test arm)|
//!
//! **Verdict = the DiD INTERACTION `(mean(EF)−mean(FRONTIER)) − (mean(EXTENT)−mean(FLAT))`**, NOT
//! the raw EF-vs-FRONTIER contrast (confounded — critic F1). `extent_economy_2x2_factorial_did`
//! (#[ignore]) runs all four arms per seed and prints the full 2×2 means + interaction + the
//! FLAT-headroom gate (critic F6/F7) + median-ΔN/P(N_EF>N_F) corroboration (critic F7/F8).
//!
//! **Mandatory invasion diagnostic under EXTENT+FRONTIER** (critic F2):
//! `extent_economy_ef_invasion_diagnostic` (#[ignore]) — reuses the breed-true plumbing of the
//! original Arm B (`env0a_a2_invasibility.rs`) but with `env_frontier_config = Some(patch_grain)`
//! (the composition, NOT stripped). The ONLY discriminator of genuine gradient vs free-size drift
//! vs transient endowment-burn.
//!
//! **Encoding assert** (critic F5): `extent_economy_encoding_assert_ncell_monopolization`
//! (non-ignored, CI gate) — proves the bonded contestant COUNT per multicellular body is N under
//! EXTENT+FRONTIER (income∝N ⇒ one contestant per live cell) vs 1 under FRONTIER-alone
//! (Anchor income ⇒ one contestant at the entity's anchor, regardless of body size) — i.e. EF
//! actually monopolizes N distinct cells, not just "the pre-emption branch was taken".
//!
//! **Original R30-1 Arm A/B** (`extent_economy_arm_b_invasion_diagnostic`, extent-alone,
//! `env_frontier_config=None`, critic F9) are UNCHANGED from #420/#422 — the 2×2 factorial
//! subsumes Arm A's EXTENT-vs-FLAT contrast as two of its four cells.
//!
//! **Plumbing test** (`extent_economy_plumbing_smoke`, non-ignored, CI gate): all four arms build
//! with no flag leak, phase-2 bodies decode N>1 in the frontier arms, the metric emits a per-arm
//! per-seed N histogram, and the EF invasion diagnostic emits a per-invader-N census trajectory —
//! NOT the science verdict, just that the machinery is wired.
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

/// FLAT-headroom validity gate threshold (critic F6/F7): below this, `EXTENT−FLAT` is treated as
/// ≈0 (no real N>1 headroom for the ∝N cost to suppress), so the DiD collapses to the raw
/// confounded contrast and the invasion diagnostic becomes the sole attribution.
const HEADROOM_EPS: f64 = 0.05;

/// Construct a graded-N invader/resident template at the given `g_dev`, with `n_dev` paired per
/// [`morphogen_steps_for_gdev`] (critic F11 — an under-paired `n_dev` step-limits the body,
/// illusory headroom rather than an economy effect). Panics loudly if decode fails or cell count
/// is outside `[2, g_dev²]` (dead cells already excluded by construction).
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
/// "6→12"). g_dev 6 MUST pair to 12 (matches the 2×2 factorial arms exactly). g_dev {2,3,4} use 8
/// (the invasibility precedent's proven-decodes value, comfortably above the decode floor).
fn morphogen_steps_for_gdev(g_dev: usize) -> u32 {
    match g_dev {
        2..=4 => 8,
        6 => cli::EXTENT_ECONOMY_MORPHOGEN_STEPS,
        other => panic!("unsupported g_dev={other} in extent-economy invader sweep (declared set: {{2,3,4,6}})"),
    }
}

/// Histogram of live-N body sizes over `[1, max_n]` — every integer bin, NOT squares-only (realized
/// live-N spans `[1..g_dev²]` with dead cells excluded, not perfect squares).
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

/// Median of an f64 slice (sorted copy; even length averages the two middle values). `NaN`-free
/// input assumed (means computed from finite integer data).
fn median(values: &[f64]) -> f64 {
    assert!(!values.is_empty(), "median of empty slice");
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).expect("finite f64"));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

// ── 2×2 factorial arm encoding (issue #425) ──────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Arm {
    Flat,
    Extent,
    Frontier,
    ExtentFrontier,
}

const ARMS: [Arm; 4] = [Arm::Flat, Arm::Extent, Arm::Frontier, Arm::ExtentFrontier];

impl Arm {
    fn name(&self) -> &'static str {
        match self {
            Arm::Flat => "FLAT",
            Arm::Extent => "EXTENT",
            Arm::Frontier => "FRONTIER",
            Arm::ExtentFrontier => "EF",
        }
    }

    fn config(&self, seed: u64) -> sim_core::SimConfig {
        match self {
            Arm::Flat => cli::extent_economy_flat_config(seed),
            Arm::Extent => cli::extent_economy_extent_config(seed),
            Arm::Frontier => cli::extent_economy_frontier_config(seed),
            Arm::ExtentFrontier => cli::extent_economy_extent_frontier_config(seed),
        }
    }

    /// No-flag-leak guard (validity check #4): asserts this arm's cell in the 2×2 table — the 3
    /// extent flags AND `env_frontier_config` — matches EXACTLY, no partial/leaked state.
    fn assert_encoding(&self, econ: &sim_core::EconParams) {
        match self {
            Arm::Flat | Arm::Frontier => assert_flat_flags(econ),
            Arm::Extent | Arm::ExtentFrontier => assert_extent_flags(econ),
        }
        match self {
            Arm::Flat | Arm::Extent => assert_frontier_none(econ),
            Arm::Frontier | Arm::ExtentFrontier => assert_frontier_some(econ),
        }
    }
}

fn assert_extent_flags(econ: &sim_core::EconParams) {
    assert_eq!(econ.income_mode, IncomeMode::Extent, "treatment-encoding guard: income_mode must be Extent");
    assert!(econ.metab_reads_n_cells, "treatment-encoding guard: metab_reads_n_cells must be true");
    assert!(econ.newborn_energy_per_cell, "treatment-encoding guard: newborn_energy_per_cell must be true");
}

fn assert_flat_flags(econ: &sim_core::EconParams) {
    assert_eq!(econ.income_mode, IncomeMode::Anchor, "flat-arm guard: income_mode must stay Anchor");
    assert!(!econ.metab_reads_n_cells, "flat-arm guard: metab_reads_n_cells must stay false");
    assert!(!econ.newborn_energy_per_cell, "flat-arm guard: newborn_energy_per_cell must stay false");
}

fn assert_frontier_none(econ: &sim_core::EconParams) {
    assert!(econ.env_frontier_config.is_none(), "no-leak guard: env_frontier_config must be None");
}

fn assert_frontier_some(econ: &sim_core::EconParams) {
    match econ.env_frontier_config {
        Some(f) => assert_eq!(
            f.patch_grain,
            cli::ENV_FRONTIER_PATCH_GRAIN,
            "anti-forcing guard: patch_grain must equal the established ENV-0a′ value, not a tuned one"
        ),
        None => panic!("no-leak guard: env_frontier_config must be Some(ENV_FRONTIER_PATCH_GRAIN)"),
    }
}

fn assert_same_cap(cfg: &sim_core::SimConfig) {
    assert_eq!(cfg.econ.gdev_cap, cli::EXTENT_ECONOMY_GDEV_CAP, "SAME raised gdev_cap required across all 4 arms (critic F3)");
    assert_eq!(cfg.econ.morphogen_steps, cli::EXTENT_ECONOMY_MORPHOGEN_STEPS, "SAME raised morphogen_steps required across all 4 arms (critic F11)");
}

/// Runs one arm/seed cell of the 2×2 factorial: builds the config, asserts its encoding, steps
/// `horizon` ticks, samples live-N over the last third every `SAMPLE_INTERVAL` ticks, and prints
/// the greppable MAP line. Returns the pooled mean (`None` on extinction/no-samples — a real
/// live-but-unicellular outcome IS `Some`, per #422's record-not-panic contract: only
/// extinction/no-samples is `None`, not a floor).
fn run_arm_seed(arm: Arm, seed: u64, horizon: u64, max_n: usize) -> Option<f64> {
    let cfg = arm.config(seed);
    arm.assert_encoding(&cfg.econ);
    assert_same_cap(&cfg);

    let last_third_start = horizon - horizon / 3;
    const SAMPLE_INTERVAL: u64 = 200;
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
        println!("EXTENT-ECON 2x2 arm={:<8} seed={:<2} pop={:<4} EXTINCT-OR-NO-SAMPLES", arm.name(), seed, pop);
        return None;
    }

    // Phase-1-floor guard (critic F5-prev, all arms): distinguish the real confound (n_cells=0 for
    // an ALIVE body — phase-1 DECODE floor) from a legitimate live-but-unicellular outcome (all
    // N=1 — a real data point). Neither aborts the sweep; both are recorded (#422).
    let n_zero = pooled_n.iter().filter(|&&n| n == 0).count();
    let mean_n: f64 = pooled_n.iter().sum::<i64>() as f64 / pooled_n.len() as f64;
    let observed_max_n = *pooled_n.iter().max().unwrap_or(&0);
    let density = pop as f64 / (world_dim * world_dim) as f64;
    let histogram = compute_histogram(&pooled_n, max_n);

    if n_zero > 0 {
        println!(
            "EXTENT-ECON 2x2 arm={:<8} seed={:<2} pop={:<4} WARN-DECODE-FLOOR n_zero={} hist={}",
            arm.name(), seed, pop, n_zero, histogram
        );
    }

    println!(
        "EXTENT-ECON 2x2 arm={:<8} seed={:<2} pop={:<4} density={:.4} mean_n={:.4} max_n={} samples={} hist={}",
        arm.name(), seed, pop, density, mean_n, observed_max_n, pooled_n.len(), histogram
    );

    Some(mean_n)
}

/// The 2×2 factorial: FLAT / EXTENT / FRONTIER / EXTENT+FRONTIER, all four at the SAME raised
/// `gdev_cap`/`morphogen_steps`, per seed (cloud diagnostic, #[ignore]). Computes and prints the
/// full 2×2 means, the pre-declared DiD INTERACTION `(mean(EF)−mean(FRONTIER)) −
/// (mean(EXTENT)−mean(FLAT))` (critic F1/F6/F7 — the primary verdict, read off the MEAN), the
/// FLAT-headroom validity gate (critic F6/F7), and median-ΔN / P(N_EF>N_F) / P(N_E>N_FLAT)
/// corroboration (non-linear, does NOT cancel the confound — corroboration only, critic F7/F8).
#[test]
#[ignore]
fn extent_economy_2x2_factorial_did() {
    let horizon: u64 = env::var("EXTENT_ECONOMY_TICKS").ok().and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_TICKS);
    let max_n = cli::EXTENT_ECONOMY_GDEV_CAP * cli::EXTENT_ECONOMY_GDEV_CAP;

    println!(
        "\nEXTENT-ECONOMY 2x2 FACTORIAL: extent-economy ⊕ ENV-0a′ monopolization (gdev_cap={}, morphogen_steps={}, patch_grain={}, ticks={})",
        cli::EXTENT_ECONOMY_GDEV_CAP, cli::EXTENT_ECONOMY_MORPHOGEN_STEPS, cli::ENV_FRONTIER_PATCH_GRAIN, horizon
    );

    // per_seed[arm_index][seed_index] = Option<mean_n>
    let mut per_seed: [Vec<Option<f64>>; 4] = Default::default();

    for &seed in &SEEDS {
        for (i, &arm) in ARMS.iter().enumerate() {
            let mean_n = run_arm_seed(arm, seed, horizon, max_n);
            per_seed[i].push(mean_n);
        }
    }

    let seed_means = |i: usize| -> Vec<f64> { per_seed[i].iter().filter_map(|&m| m).collect() };
    let grand_mean = |i: usize| -> Option<f64> {
        let v = seed_means(i);
        if v.is_empty() { None } else { Some(v.iter().sum::<f64>() / v.len() as f64) }
    };

    let mean_flat = grand_mean(0);
    let mean_extent = grand_mean(1);
    let mean_frontier = grand_mean(2);
    let mean_ef = grand_mean(3);

    println!(
        "EXTENT-ECON 2x2 MEANS flat={:?} extent={:?} frontier={:?} ef={:?}",
        mean_flat, mean_extent, mean_frontier, mean_ef
    );

    match (mean_flat, mean_extent, mean_frontier, mean_ef) {
        (Some(flat), Some(extent), Some(frontier), Some(ef)) => {
            let extent_minus_flat = extent - flat;
            let ef_minus_frontier = ef - frontier;
            let interaction = ef_minus_frontier - extent_minus_flat;

            println!(
                "EXTENT-ECON 2x2 DID extent_minus_flat={:.4} ef_minus_frontier={:.4} INTERACTION={:.4}",
                extent_minus_flat, ef_minus_frontier, interaction
            );

            // FLAT-headroom validity gate (critic F6/F7): the DiD only cancels the ∝N-cost
            // confound if FLAT expresses real N>1 headroom for EXTENT to suppress. If
            // EXTENT−FLAT≈0, FLAT is ALSO pinned near N=1 and the DiD collapses to the raw
            // (confounded) EF−FRONTIER contrast.
            if extent_minus_flat.abs() < HEADROOM_EPS {
                println!(
                    "EXTENT-ECON 2x2 FLAG-DEGENERATE-DID: extent_minus_flat={:.4} ≈ 0 (no FLAT headroom) — \
                     DiD degenerates to the raw confounded EF-vs-FRONTIER contrast ({:.4}); \
                     fall back to the mandatory invasion diagnostic as the SOLE attribution",
                    extent_minus_flat, ef_minus_frontier
                );
            }

            // Corroboration (non-linear — does NOT cancel the confound, per-seed sign-only read).
            let paired_seeds: Vec<usize> = (0..SEEDS.len())
                .filter(|&s| per_seed[0][s].is_some() && per_seed[1][s].is_some() && per_seed[2][s].is_some() && per_seed[3][s].is_some())
                .collect();
            if !paired_seeds.is_empty() {
                let per_seed_ef_minus_f: Vec<f64> = paired_seeds.iter().map(|&s| per_seed[3][s].unwrap() - per_seed[2][s].unwrap()).collect();
                let per_seed_e_minus_flat: Vec<f64> = paired_seeds.iter().map(|&s| per_seed[1][s].unwrap() - per_seed[0][s].unwrap()).collect();
                let per_seed_interaction: Vec<f64> = paired_seeds.iter().map(|&s| {
                    (per_seed[3][s].unwrap() - per_seed[2][s].unwrap()) - (per_seed[1][s].unwrap() - per_seed[0][s].unwrap())
                }).collect();

                // Explicit tie rule: EF==FRONTIER (or EXTENT==FLAT) counts as 0.5, not 0 or 1.
                let p_gt = |diffs: &[f64]| -> f64 {
                    let score: f64 = diffs.iter().map(|&d| if d > 0.0 { 1.0 } else if d == 0.0 { 0.5 } else { 0.0 }).sum();
                    score / diffs.len() as f64
                };

                println!(
                    "EXTENT-ECON 2x2 CORROBORATION n_paired_seeds={} median_delta_ef_minus_frontier={:.4} median_delta_extent_minus_flat={:.4} median_interaction={:.4} P(N_EF>N_F)={:.4} P(N_E>N_FLAT)={:.4}",
                    paired_seeds.len(),
                    median(&per_seed_ef_minus_f),
                    median(&per_seed_e_minus_flat),
                    median(&per_seed_interaction),
                    p_gt(&per_seed_ef_minus_f),
                    p_gt(&per_seed_e_minus_flat),
                );
            } else {
                println!("EXTENT-ECON 2x2 CORROBORATION: no seed has all 4 arms with samples — cannot pair");
            }
        }
        _ => {
            println!("EXTENT-ECON 2x2 DID: at least one arm has NO seed with live samples — DiD not computable, see per-arm EXTINCT lines above");
        }
    }

    println!("EXTENT-ECONOMY 2x2 FACTORIAL complete. Attribution requires the mandatory EF invasion diagnostic (see extent_economy_ef_invasion_diagnostic).");
}

/// Shared invasion-diagnostic sweep runner: injects fixed larger-N invaders (g_dev ∈ {2,3,4,6})
/// among small (g_dev=2) residents under `cfg_fn`'s economy, and prints the invader SpeciesId's
/// census-count FREQUENCY TRAJECTORY (start→end via `species_census()`, NOT a fitness ledger).
/// `frontier_expected` pins which of the two invasion arms this is (critic F9's stripped-vs-welded
/// requirement is otherwise unenforced at the call site): `false` for Arm B (frontier stripped),
/// `true` for the EF diagnostic (frontier welded in) — a silent regression in either config
/// function's `env_frontier_config` would otherwise pass unnoticed.
fn run_invasion_sweep(label: &str, cfg_fn: fn(u64) -> sim_core::SimConfig, horizon: u64, frontier_expected: bool) {
    const INVADER_GDEV_LEVELS: [usize; 4] = [2, 3, 4, 6];
    const N_FOUNDERS: u64 = 100;
    const N_INVADERS: u64 = 2;
    const N_CHECKPOINTS: u64 = 5;
    let resident_n_dev = morphogen_steps_for_gdev(2);

    for &g_dev in &INVADER_GDEV_LEVELS {
        let invader_n_dev = morphogen_steps_for_gdev(g_dev);
        for &seed in &SEEDS {
            let resident = make_graded_invader_template(N_LAYERS, 2, resident_n_dev);
            let invader = make_graded_invader_template(N_LAYERS, g_dev, invader_n_dev);

            let mut cfg = cfg_fn(seed);
            assert_extent_flags(&cfg.econ);
            if frontier_expected {
                assert_frontier_some(&cfg.econ);
            } else {
                assert_frontier_none(&cfg.econ);
            }
            assert!(!cfg.econ.evolve_body_size, "invasion diagnostic must be breed-true (evolve_body_size=false)");
            assert_eq!(cfg.econ.speciation_threshold, i64::MAX, "invasion diagnostic must freeze speciation");

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
                "EXTENT-ECON {} gdev={:<2} seed={:<2} pop_end={:<4} invader_traj={}",
                label, g_dev, seed, pop_end, traj_str
            );
        }
    }
}

/// Arm B (original, #420/#422): invasion-fitness diagnostic under EXTENT-alone
/// (`env_frontier_config=None`, critic F9 — kept stripped, NOT the composition). Reference/
/// corroboration for the extent-alone NULL; UNCHANGED by #425.
#[test]
#[ignore]
fn extent_economy_arm_b_invasion_diagnostic() {
    let horizon: u64 = env::var("EXTENT_ECONOMY_TICKS").ok().and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_TICKS);
    println!("\nEXTENT-ECONOMY ARM B: invasion diagnostic under EXTENT-alone (env_frontier_config=None, ticks={})", horizon);
    run_invasion_sweep("armB", cli::extent_economy_invasion_config, horizon, false);
    println!("EXTENT-ECONOMY ARM B complete. Invader RISES ⇒ selection gradient under the extent economy ALONE; FALLS/vanishes ⇒ no window (the concluded NULL).");
}

/// MANDATORY invasion diagnostic under EXTENT+FRONTIER (#425, critic F2 — the attribution
/// discriminator). Same breed-true plumbing as Arm B, but `env_frontier_config =
/// Some(ENV_FRONTIER_PATCH_GRAIN)` (the composition, welded in, NOT stripped). This is the ONLY
/// thing that distinguishes (a) a genuine selection gradient from (b) free-size drift from (c)
/// transient parent endowment-burn.
#[test]
#[ignore]
fn extent_economy_ef_invasion_diagnostic() {
    let horizon: u64 = env::var("EXTENT_ECONOMY_TICKS").ok().and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_TICKS);
    println!("\nEXTENT-ECONOMY EF INVASION (MANDATORY, #425): invasion diagnostic under EXTENT+FRONTIER (env_frontier_config=Some({}), ticks={})", cli::ENV_FRONTIER_PATCH_GRAIN, horizon);
    run_invasion_sweep("armB-EF", cli::extent_economy_ef_invasion_config, horizon, true);
    println!("EXTENT-ECONOMY EF INVASION complete. Invader RISES ⇒ genuine gradient under the composition (attributes a 2x2 DiD positive to real selection); FALLS/vanishes ⇒ no window (DiD positive, if any, would be drift/endowment-burn, not gradient).");
}

/// One g_dev level of the encoding assert (factored so the claim is checked at MULTIPLE distinct N
/// — a single N=2 fixture couldn't rule out a bug that coincidentally matches at N=2 but breaks at
/// higher N, e.g. `contestants = min(N, 2)`). Returns `(n_f, contestants_f, n_ef, contestants_ef)`
/// for the caller to log.
fn check_contestant_count_at_gdev(seed: u64, g_dev: usize) -> (i64, u64, i64, u64) {
    let multicell = make_graded_invader_template(N_LAYERS, g_dev, morphogen_steps_for_gdev(g_dev));

    // FRONTIER-alone: Anchor income + env_frontier_config=Some. Bonded body, but Anchor income
    // generates exactly ONE contestant at the entity's anchor cell (stages.rs IncomeMode::Anchor
    // arm), regardless of N.
    let mut frontier_cfg = cli::extent_economy_frontier_config(seed);
    Arm::Frontier.assert_encoding(&frontier_cfg.econ);
    frontier_cfg.n_founders = 1;
    frontier_cfg.founder_templates = Some(vec![(multicell.clone(), 1)]);
    let mut sim_frontier = build_sim(frontier_cfg);
    sim_frontier.step();
    let bodies_frontier = sim_frontier.body_size_entity_probe();
    let (&entity_bits_f, &n_f) = bodies_frontier.iter().next().expect("one founder must be alive");
    assert!(n_f > 1, "FRONTIER-alone encoding-assert fixture (g_dev={}) must be multicellular, got N={}", g_dev, n_f);
    let contestants_f = *sim_frontier.telemetry().entity_contestant_count.get(&entity_bits_f).unwrap_or(&0);
    assert_eq!(
        contestants_f, 1,
        "FRONTIER-alone (Anchor income, g_dev={}) must generate exactly 1 contestant for a bonded N={}-cell body, got {}",
        g_dev, n_f, contestants_f
    );

    // EXTENT+FRONTIER: Extent income + env_frontier_config=Some. The SAME bonded body now
    // generates one contestant PER LIVE CELL — the contestant count must equal N.
    let mut ef_cfg = cli::extent_economy_extent_frontier_config(seed);
    Arm::ExtentFrontier.assert_encoding(&ef_cfg.econ);
    ef_cfg.n_founders = 1;
    ef_cfg.founder_templates = Some(vec![(multicell, 1)]);
    let mut sim_ef = build_sim(ef_cfg);
    sim_ef.step();
    let bodies_ef = sim_ef.body_size_entity_probe();
    let (&entity_bits_ef, &n_ef) = bodies_ef.iter().next().expect("one founder must be alive");
    assert!(n_ef > 1, "EF encoding-assert fixture (g_dev={}) must be multicellular, got N={}", g_dev, n_ef);
    let contestants_ef = *sim_ef.telemetry().entity_contestant_count.get(&entity_bits_ef).unwrap_or(&0);
    assert_eq!(
        contestants_ef, n_ef as u64,
        "EXTENT+FRONTIER (Extent income, g_dev={}) must generate exactly N={} contestants for the bonded body, got {}",
        g_dev, n_ef, contestants_ef
    );

    (n_f, contestants_f, n_ef, contestants_ef)
}

/// Encoding assert (#425, critic F5 — NOT just "branch taken"): proves the bonded contestant COUNT
/// per multicellular body is N under EXTENT+FRONTIER (income∝N ⇒ one contestant per live cell) vs
/// 1 under FRONTIER-alone (Anchor income ⇒ one contestant at the entity's own anchor, regardless of
/// body size). Both arms route through the SAME bonded pre-emption branch (`stages.rs:695-720`) —
/// this is what proves EF actually monopolizes N distinct cells, not merely that the branch fires.
/// Swept at g_dev ∈ {2,3} (two distinct N values, whatever they decode to) so the claim can't be a
/// coincidence at one N. Non-ignored (CI gate): deterministic, single-tick per level, no evolution.
#[test]
fn extent_economy_encoding_assert_ncell_monopolization() {
    let seed = 42u64;
    for &g_dev in &[2usize, 3usize] {
        let (n_f, contestants_f, n_ef, contestants_ef) = check_contestant_count_at_gdev(seed, g_dev);
        println!(
            "extent-economy encoding-assert OK (g_dev={}): FRONTIER-alone N={} contestants={} | EF N={} contestants={}",
            g_dev, n_f, contestants_f, n_ef, contestants_ef
        );
    }
}

/// Plumbing smoke test (non-ignored, CI gate): all four 2×2-factorial arms build with no flag
/// leak, phase-2 bodies decode N>1 in the frontier arms, the metric emits a per-arm N histogram,
/// and the mandatory EF invasion diagnostic emits a per-invader-N census frequency trajectory.
/// NOT the science verdict — just that the machinery is wired and runs without panicking.
#[test]
fn extent_economy_plumbing_smoke() {
    let seed = 42u64;
    let multicell = make_graded_invader_template(N_LAYERS, 2, morphogen_steps_for_gdev(2));

    // === All 4 arms: config shape (no-leak guard) + same cap (validity check #4 + critic F3) ===
    for &arm in &ARMS {
        let cfg = arm.config(seed);
        arm.assert_encoding(&cfg.econ);
        assert_same_cap(&cfg);
    }

    // === Phase-2 decode N>1 reachable in EVERY arm, including both frontier arms ===
    // Force phase-2 N>1 bodies via founder_templates (bypass evolution — plumbing check that the
    // metric CAN read multicellular bodies under each arm's economy, not a speed claim).
    for &arm in &ARMS {
        let mut cfg = arm.config(seed);
        cfg.n_founders = 30;
        cfg.founder_templates = Some(vec![(multicell.clone(), 30)]);
        let mut sim = build_sim(cfg);
        for _ in 0..20 {
            sim.step();
        }
        let sizes = sim.body_size_probe();
        let n_gt1 = sizes.iter().filter(|&&n| n > 1).count();
        let hist = compute_histogram(&sizes, 36);
        assert!(n_gt1 > 0, "{} plumbing: phase-2 bodies must decode N>1 (hist={})", arm.name(), hist);
        assert_ne!(hist, "empty", "{} metric must emit a non-empty per-seed N histogram", arm.name());
    }

    // === Mandatory EF invasion diagnostic wiring (validity check #4) ===
    let resident = make_graded_invader_template(N_LAYERS, 2, morphogen_steps_for_gdev(2));
    let invader = make_graded_invader_template(N_LAYERS, 3, morphogen_steps_for_gdev(3));

    let mut inv_cfg = cli::extent_economy_ef_invasion_config(seed);
    assert_extent_flags(&inv_cfg.econ);
    assert_frontier_some(&inv_cfg.econ);
    assert!(!inv_cfg.econ.evolve_body_size, "EF invasion diagnostic must be breed-true (evolve_body_size=false)");
    assert_eq!(inv_cfg.econ.speciation_threshold, i64::MAX, "EF invasion diagnostic must freeze speciation");

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
        "EF invasion diagnostic must emit a per-invader-N census FREQUENCY TRAJECTORY (≥2 points), got {}",
        trajectory.len()
    );

    println!("extent-economy 2x2 plumbing OK: all 4 arms decode N>1, EF invasion_traj={:?}", trajectory);
}
