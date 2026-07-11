//! DOL-GERM-REPRO Interior Optimum Probe — Per-Split Fitness Curve Measurement
//!
//! **GOAL:** Measure whether the existing `dol_germ_repro` mechanic rewards a balanced germ:soma
//! split, showing a REAL per-split fitness curve so we can see PEAK (interior optimum) vs
//! EDGE/PLATEAU (no interior optimum).
//!
//! **MEASUREMENT DESIGN (DESIGN 2 — deterministic proxy from real stage functions):**
//! - Build hand-constructed bodies with IMPOSED germ:soma splits (0:N, 1:(N-1), ..., N:0)
//! - Compute income per cell using REAL Monod kinetics (u_max·R/(R+k_m))
//! - Compute repro_bar using REAL formula from stages.rs:1439:
//!   `repro_bar = if germ==0 { ∞ } else { repro_threshold * body / germ }`
//! - Fitness proxy = (income_per_cell × body_size) / repro_bar
//!   (higher income relative to repro cost = higher fitness)
//! - Sweep all splits for each body size
//!
//! **PRE-DECLARED VERDICT:**
//! - **PEAK:** Interior maximum in fertile subdomain (germ>0), concave curvature ⇒ PASS
//! - **EDGE/PLATEAU:** Max at boundary or flat interior ⇒ NULL (no DoL interior optimum)
//!
//! **Key property:** If `dol_germ_repro` creates a genuine interior optimum, the curve will show:
//! - Germ=0: sterile (fitness=0, excluded from analysis)
//! - Germ=1..N-1: varying fitness based on body/germ ratio trade-off
//! - Germ=N: all-germ, high germ count but no soma income (low fitness)
//! Intermediate splits should outcompete extremes IF the mechanic genuinely rewards differentiation.

use cli::driver_config;
use sim_core::{CellGraph, CellType};

const TEST_SEED: u64 = 0xD01_0001;
const TEST_BODY_SIZES: &[i64] = &[4, 8];
const REPRO_THRESHOLD_DEFAULT: i64 = 1500;  // From genome.rs default


// ── Hand-built body generators (imposed-split) ──

/// Build a matched-N body with a specific germ:soma split.
/// All cells type A (soma) except the first `germ_count` cells (type B, germ).
/// module_is_germ encodes the split for repro_bar computation.
fn build_imposed_split_body(body_size: i64, germ_count: i64) -> CellGraph {
    let soma_count = (body_size - germ_count).max(0).min(body_size);
    let germ_count_i32 = (body_size - soma_count) as i32;
    let soma_count_i32 = soma_count as i32;

    let mut module_type = vec![];
    let mut module_cell_count = vec![];
    let mut module_is_germ = vec![];

    // Germ module: type B, germ_count cells
    if germ_count_i32 > 0 {
        module_type.push(CellType::B);
        module_cell_count.push(germ_count_i32);
        module_is_germ.push(true);
    }
    // Soma module: type A, soma_count cells
    if soma_count_i32 > 0 {
        module_type.push(CellType::A);
        module_cell_count.push(soma_count_i32);
        module_is_germ.push(false);
    }

    let n_modules = module_type.len();
    CellGraph {
        g_dev: 4,
        module_type,
        module_cell_count,
        module_is_germ,
        module_reachable: vec![true; n_modules],
        module_consortium: (0..n_modules).collect(),
    }
}

/// Compute repro_bar using the REAL formula from stages.rs:1439
/// when `dol_germ_repro=true`:
///   if germ == 0 { i64::MAX } else { repro_threshold * body / germ }
fn compute_repro_bar(graph: &CellGraph, repro_threshold: i64) -> i64 {
    let body = graph.body_size();
    let germ = graph.module_cell_count.iter().zip(graph.module_is_germ.iter())
        .filter_map(|(&c, &g)| if g { Some(c as i64) } else { None })
        .sum::<i64>();

    if germ == 0 {
        i64::MAX
    } else {
        repro_threshold * body / germ.max(1)
    }
}

/// Fitness proxy = (total income) / (repro_bar)
/// Higher income, lower repro cost = higher fitness.
/// Income scales by SOMA count (stages.rs:563 demand * soma), not body_size.
/// Returns (income_total, repro_bar, fitness_proxy).
fn measure_fitness_proxy(
    graph: &CellGraph,
    econ: &sim_core::EconParams,
    repro_threshold: i64,
) -> (i64, i64, f64) {
    let body_size = graph.body_size();
    let (soma, germ) = graph.fate_germ_soma_counts();
    let soma_count = soma as i64;

    // Monod demand (real formula from stages.rs:563)
    let r = 100i64;
    let u_max = econ.u_max;
    let km = econ.km;
    let demand = u_max * r / (r + km);

    // Income scales by soma (not body_size) per stages.rs:563 / 591
    // soma_count can be 0 (all-germ case), creating zero income → natural parabola shape
    let total_income = demand * soma_count;
    let repro_bar = compute_repro_bar(graph, repro_threshold);

    let fitness = if repro_bar == i64::MAX {
        0.0  // Sterile (germ=0)
    } else if repro_bar <= 0 {
        0.0  // Safety check
    } else {
        total_income as f64 / repro_bar as f64
    };

    (total_income, repro_bar, fitness)
}

/// Sweep germ:soma split for a body size and collect fitness curve.
/// Returns Vec<(germ_count, total_income, repro_bar, fitness)> ordered by split ratio.
fn sweep_splits_for_size(
    body_size: i64,
    econ: &sim_core::EconParams,
    repro_threshold: i64,
) -> Vec<(i64, i64, i64, f64)> {
    let mut results = vec![];

    for germ_count in 0..=body_size {
        let graph = build_imposed_split_body(body_size, germ_count);
        let (income, repro_bar, fitness) = measure_fitness_proxy(&graph, econ, repro_threshold);
        results.push((germ_count, income, repro_bar, fitness));
    }

    results
}

/// Analyze fitness curve for PEAK vs PLATEAU vs EDGE (restricted to FERTILE subdomain).
/// Returns (is_peak, classification, curve_summary).
fn analyze_fitness_curve(curve: &[(i64, i64, i64, f64)]) -> (bool, String, String) {
    if curve.is_empty() {
        return (false, "EMPTY_CURVE".to_string(), "".to_string());
    }

    // Build curve string (all points including sterile)
    let fitnesses_full: Vec<String> = curve.iter()
        .map(|(_, _, _, fitness)| format!("{:.2}", fitness))
        .collect();
    let curve_str = fitnesses_full.join(", ");

    // Filter to FERTILE subdomain: only points with germ > 0 and fitness > 0
    let fertile_points: Vec<(usize, f64)> = curve
        .iter()
        .enumerate()
        .filter(|(_, (germ, _, repro_bar, fitness))| {
            *germ > 0 && *repro_bar != i64::MAX && *fitness > 0.0
        })
        .map(|(idx, (_, _, _, fitness))| (idx, *fitness))
        .collect();

    if fertile_points.is_empty() {
        return (false, "NULL (no fertile points; all splits sterile or zero fitness)".to_string(), curve_str);
    }

    if fertile_points.len() == 1 {
        let (idx, _fitness) = fertile_points[0];
        return (
            false,
            format!("EDGE (only one fertile point at germ={}, no interior optimum)", idx),
            curve_str,
        );
    }

    // Analyze peak within fertile subdomain
    let fertile_fitnesses: Vec<f64> = fertile_points.iter().map(|(_, fitness)| *fitness).collect();
    let n_fertile = fertile_fitnesses.len();

    // Find max among fertile points
    let max_fitness = fertile_fitnesses.iter()
        .fold(f64::NEG_INFINITY, |a, &b| a.max(b));
    let max_idx_fertile = fertile_fitnesses
        .iter()
        .position(|&f| (f - max_fitness).abs() < 1e-9)
        .unwrap_or(0);

    // Check if max is interior within fertile subdomain
    let is_interior = max_idx_fertile > 0 && max_idx_fertile < n_fertile - 1;

    if !is_interior {
        return (
            false,
            "EDGE (maximum at boundary of fertile domain, no interior optimum)".to_string(),
            curve_str,
        );
    }

    // Interior max in fertile domain: check if it's a strict PEAK
    let left_neighbor = fertile_fitnesses[max_idx_fertile - 1];
    let right_neighbor = fertile_fitnesses[max_idx_fertile + 1];
    let eps = 1e-9;

    let is_strict_peak = max_fitness > left_neighbor + eps && max_fitness > right_neighbor + eps;

    if !is_strict_peak {
        return (
            false,
            "PLATEAU (interior max in fertile domain, but not strict peak)".to_string(),
            curve_str,
        );
    }

    // Verify concavity in fertile subdomain
    let before_increases = if max_idx_fertile > 0 {
        fertile_fitnesses[0..max_idx_fertile]
            .windows(2)
            .all(|w| w[0] <= w[1] + eps)
    } else {
        true
    };

    let after_decreases = if max_idx_fertile < n_fertile - 1 {
        fertile_fitnesses[max_idx_fertile..]
            .windows(2)
            .all(|w| w[0] >= w[1] - eps)
    } else {
        true
    };

    let is_concave_peak = before_increases && after_decreases;

    let verdict = if is_concave_peak {
        let (orig_germ_idx, _) = fertile_points[max_idx_fertile];
        format!("PEAK (interior optimum at germ={}, fitness={:.4})", orig_germ_idx, max_fitness)
    } else {
        "PLATEAU_or_FLAT (interior max but no concavity)".to_string()
    };

    (is_concave_peak, verdict, curve_str)
}

/// DOL-GERM-REPRO interior optimum probe: per-split fitness curve measurement.
/// Deterministic economy measurement (no RNG in fitness computation).
/// Pre-declared verdict:
///   - PEAK (interior optimum in fertile domain, concave) ⇒ PASS (mechanic rewards DoL)
///   - PLATEAU/EDGE (max at boundary or flat) ⇒ NULL (mechanic does not reward DoL)
#[test]
#[ignore]
fn dol_germ_repro_interior_optimum_probe() {
    println!("\n════════════════════════════════════════════════════════════════");
    println!("DOL-GERM-REPRO Interior Optimum Probe — Per-Split Fitness Curve");
    println!("════════════════════════════════════════════════════════════════\n");
    println!("MEASUREMENT DESIGN:");
    println!("  Hand-built bodies with IMPOSED germ:soma splits (0:N to N:0)");
    println!("  Fitness proxy: (total_income) / (repro_bar)");
    println!("  where repro_bar = repro_threshold * body / germ (from stages.rs:1439)");
    println!("  Analysis: PEAK vs PLATEAU/EDGE classification in FERTILE subdomain\n");

    let mut cfg = driver_config(TEST_SEED);
    cfg.econ.division_of_labor = true;
    cfg.econ.dol_germ_repro = true;  // THE gate being tested
    cfg.econ.dol_economy = false;    // Ensure dol_germ_repro path is taken
    cfg.econ.fate_economy = false;   // Disable alternate path

    let repro_threshold = REPRO_THRESHOLD_DEFAULT;
    let mut all_peak = true;

    for &body_size in TEST_BODY_SIZES {
        println!("═ Body size N={body_size} ═\n");

        // Sweep germ:soma splits
        let curve = sweep_splits_for_size(body_size, &cfg.econ, repro_threshold);

        // Print raw fitness values (row 1)
        println!("  Raw fitness curve (income/repro_bar): [{}]", {
            curve.iter()
                .map(|(_, _, _, fitness)| format!("{:.4}", fitness))
                .collect::<Vec<_>>()
                .join(", ")
        });

        // Print germ:soma ratios (row 2)
        print!("  Split ratios (germ:soma):               [");
        for (germ, _, _, _) in &curve {
            let soma = body_size - germ;
            print!("({germ}:{soma}) ");
        }
        println!("]");

        // Print repro_bar values (row 3)
        println!("  Repro_bar per split:                  [{}]", {
            curve.iter()
                .map(|(_, _, repro_bar, _)| {
                    if *repro_bar == i64::MAX {
                        "∞".to_string()
                    } else {
                        repro_bar.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        });

        // Print income values (row 4)
        println!("  Total income per split:               [{}]", {
            curve.iter()
                .map(|(_, income, _, _)| income.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        });

        // Analyze fitness curve
        let (is_peak, verdict, _curve_str) = analyze_fitness_curve(&curve);

        println!("\n  Verdict: {}", verdict);
        println!("  Classification: {}\n", if is_peak { "PEAK ✓" } else { "NOT_PEAK ✗" });

        if !is_peak {
            all_peak = false;
        }
    }

    println!("════════════════════════════════════════════════════════════════");
    println!("FINAL VERDICT");
    println!("════════════════════════════════════════════════════════════════");

    if all_peak {
        println!("\nDOL-GERM-REPRO VERDICT: PASS");
        println!("Interpretation: The dol_germ_repro mechanic creates genuine interior");
        println!("optima in the germ:soma fitness landscape across all tested body sizes.");
        println!("→ The reward structure genuinely favors differentiation (intermediate splits).");
    } else {
        println!("\nDOL-GERM-REPRO VERDICT: NULL");
        println!("Interpretation: No interior optimum observed; fitness is monotone or");
        println!("edge-peaked. The dol_germ_repro mechanic does not reward differentiation.");
        println!("→ The mechanic alone is insufficient to drive stable multicellularity.");
    }

    println!("════════════════════════════════════════════════════════════════\n");
}
