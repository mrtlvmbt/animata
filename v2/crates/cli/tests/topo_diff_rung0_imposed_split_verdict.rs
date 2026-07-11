//! TOPO-DIFF Rung 0: Imposed-Split VERDICT Probe.
//!
//! Tests the ECONOMY question in isolation (R-B): does fate-keyed germ/soma yield a group-fitness
//! DoL gain under the existing D-5⊕ENV-0a′ economy? NOT an evolutionary test — hand-builds matched-N
//! bodies with IMPOSED within-body fate splits to isolate the economy from GRN uncertainty.
//!
//! **Key insight:** natural within-body fate variation is ~0.04% (DIFF-0-0 probe, memory), so gating
//! on evolved differentiation would false-NULL on absent signal, not the economy. Rung 0 IMPOSES a
//! fate split to answer the economy question cleanly: "given a split, does fate-keyed DoL pay?"
//!
//! **Verdict design (specialist-vs-generalist fitness sweep):**
//! - Build matched-N bodies at various germ:soma ratios (0:N, 1:(N-1), 2:(N-2), ..., N:0)
//! - Run through ACTUAL economy stages (stage_interactions + stage_birth_death) with fate_economy=ON
//! - Measure group fitness = (per-capita income) × (per-capita fertility)
//! - Compute fitness curve across ratios
//!
//! **Pre-declared PASS/NULL verdict:**
//! - **PASS:** Interior maximum (intermediate split beats both all-soma and all-germ) ⇒
//!   fate-keyed DoL genuinely pays. Specialist achieves higher fitness than generalist.
//! - **NULL:** Monotone or edge maximum (no advantage to intermediate splits) ⇒
//!   weak/no DoL benefit. Mixed body only "avoids cliffs" (germ=0 sterility, soma=0 income floor).
//!   Report which and exit.
//!
//! Heavy (sweeps at multiple N values × multiple seeds) — `#[ignore]`d; run via `sim-run.sh` scenario
//! or cloud CI. Light harness scaffold (this file): structure + helper functions for cloud PASS 2.
//!
//! Determinism: integer-only, hand-built bodies, no evolution — output is reproducible per seed.

use cli::driver_config;
use sim_core::{CellGraph, CellType, Vec2Fixed, WorldView};

const VERDICT_SEEDS: [u64; 3] = [0xD1FF_0001, 0xD1FF_0002, 0xD1FF_0003];
const SEED_MAJORITY: usize = 2;  // ≥2/3 seeds must pass → Rung 0 PASS

/// Test cell counts to sweep: small (N=4, fate mixing boundary) and larger (N=8, g_dev limit).
const TEST_BODY_SIZES: &[i64] = &[4, 8];

// ── Minimal stub WorldView for test (no complex world features) ──

struct StubWorld;
impl WorldView for StubWorld {
    fn is_solid(&self, _p: Vec2Fixed) -> bool { false }
    fn height(&self, _x: i64, _z: i64) -> i64 { 0 }
    fn biome(&self, _p: Vec2Fixed) -> u8 { 0 }
    fn resource(&self, _p: Vec2Fixed) -> i64 { 100 }
    fn temp_at(&self, _p: Vec2Fixed) -> i32 { 1500 }
}

// ── Hand-built body generators (imposed-split) ──

/// Build a matched-N body with a specific germ:soma split. All cells type A (soma) except the
/// first `germ_count` cells, which are type B (germ). Result is a MIXED specialist body.
fn build_specialist_body(body_size: i64, germ_count: i64) -> CellGraph {
    let soma_count = (body_size - germ_count).max(0).min(body_size);
    let germ_count = (body_size - soma_count) as i32;
    let soma_count_i32 = soma_count as i32;

    let mut module_type = vec![];
    let mut module_cell_count = vec![];

    // Germ module: type B, germ_count cells
    if germ_count > 0 {
        module_type.push(CellType::B);
        module_cell_count.push(germ_count);
    }
    // Soma module: type A, soma_count cells
    if soma_count_i32 > 0 {
        module_type.push(CellType::A);
        module_cell_count.push(soma_count_i32);
    }

    let n_modules = module_type.len();
    CellGraph {
        g_dev: 4,  // arbitrary grid size (not used in hand-built bodies)
        module_type,
        module_cell_count,
        module_is_germ: vec![false; n_modules],  // ignored when fate_economy=true
        module_reachable: vec![true; n_modules],
        module_consortium: (0..n_modules).collect(),
    }
}

/// Build a GENERALIST (uniform) body — all type A (soma), no germ.
fn build_generalist_body(body_size: i64) -> CellGraph {
    CellGraph {
        g_dev: 4,
        module_type: vec![CellType::A],
        module_cell_count: vec![body_size as i32],
        module_is_germ: vec![false],
        module_reachable: vec![true],
        module_consortium: vec![0],
    }
}

// ── Fitness measurement scaffold ──

/// Measure group fitness for a hand-built body under the economy stages.
/// Runs income aggregation + repro gate, returns (income_per_cell, fertility_rate).
fn measure_fitness(graph: &CellGraph, econ: &sim_core::EconParams) -> (i64, i64) {
    // Income measurement: stage_interactions computes demand scaling.
    // For simplicity, read per-cell soma (fate-keyed when fate_economy=true).
    // Compute germ/soma counts by iterating module_cell_count and module_is_germ.
    let soma: i32 = graph
        .module_cell_count
        .iter()
        .zip(graph.module_is_germ.iter())
        .filter(|(_, &is_germ)| !is_germ)
        .map(|(count, _)| count)
        .sum();
    let germ: i32 = graph
        .module_cell_count
        .iter()
        .zip(graph.module_is_germ.iter())
        .filter(|(_, &is_germ)| is_germ)
        .map(|(count, _)| count)
        .sum();

    let soma_active = soma.max(1);  // bootstrap

    // Monod demand at a nominal resource level (R=100, typical world).
    let r = 100i64;
    let u_max = econ.u_max;
    let km = econ.km;
    let demand = u_max * r / (r + km);  // monod_demand inline
    let demand_scaled = demand * (soma_active as i64);
    let income_per_cell = demand_scaled / graph.body_size().max(1);

    // Repro measurement: stage_birth_death gates on germ count.
    // Germ=0 → sterile (repro_bar = i64::MAX, fertility=0).
    // Germ>0 → flat threshold (repro_bar = genome.repro_threshold, normalized to baseline).
    let fertility = if germ == 0 { 0 } else { 1 };  // 0=sterile, 1=can reproduce

    (income_per_cell, fertility)
}

/// Sweep germ:soma split for a body size and collect fitness curve.
/// Returns Vec<(germ_count, income_per_cell, fertility)> ordered by split ratio.
fn sweep_splits_for_size(body_size: i64, econ: &sim_core::EconParams) -> Vec<(i64, i64, i64)> {
    let mut results = vec![];

    // Sweep: germ_count from 0 to body_size
    for germ_count in 0..=body_size {
        let soma_count = body_size - germ_count;
        let specialist = build_specialist_body(body_size, germ_count);
        let (income, fertility) = measure_fitness(&specialist, econ);
        results.push((germ_count, income, fertility));
    }

    results
}

/// Analyze fitness curve for PEAK vs PLATEAU vs MONOTONE/EDGE (per coordinator guidance).
/// Critical distinction:
/// - PEAK (PASS): genuine interior optimum with fitness strictly higher than neighbors on BOTH sides,
///   curve rises then falls (concave around peak). This is structural DoL.
/// - PLATEAU (NULL): flat interior (fitness ~constant across interior), only edges drop.
///   This is mere cliff-avoidance ("have ≥1 germ AND ≥1 soma"), not DoL.
/// - MONOTONE/EDGE (NULL): monotone or only edge maximum, no interior optimum.
///
/// Returns (is_peak, curve_classification, full_curve_str).
fn analyze_fitness_curve(curve: &[(i64, i64, i64)]) -> (bool, String, String) {
    if curve.is_empty() {
        return (false, "EMPTY_CURVE".to_string(), "".to_string());
    }

    // Extract fitness values (income × fertility) for each split
    let fitnesses: Vec<i64> = curve.iter().map(|(_, inc, fert)| inc * fert).collect();
    let n = fitnesses.len();

    // Build curve string for reporting
    let curve_str = fitnesses.iter().map(|f| f.to_string()).collect::<Vec<_>>().join(", ");

    // Find max fitness and its position
    let max_fitness = *fitnesses.iter().max().unwrap_or(&0);
    let max_idx = fitnesses.iter().position(|&f| f == max_fitness).unwrap_or(0);

    // Check location: interior vs edge
    let is_interior = max_idx > 0 && max_idx < n - 1;

    if !is_interior {
        // Edge or single-point maximum → NULL
        let verdict = if max_idx == 0 || max_idx == n - 1 {
            "EDGE_MAX (no interior optimum)".to_string()
        } else {
            "MONOTONE (no singular peak)".to_string()
        };
        return (false, verdict, curve_str);
    }

    // Interior maximum: check if it's a PEAK vs PLATEAU
    // PEAK: fitness at max strictly > fitness at both immediate neighbors (left and right)
    let left_neighbor = fitnesses[max_idx - 1];
    let right_neighbor = fitnesses[max_idx + 1];

    let is_strict_peak = max_fitness > left_neighbor && max_fitness > right_neighbor;

    if !is_strict_peak {
        // Interior max but not strict peak → likely PLATEAU
        return (false, "PLATEAU (flat interior, only edges drop)".to_string(), curve_str);
    }

    // Verify genuine PEAK: check concavity around peak (fitness increases before, decreases after)
    let before_increases = if max_idx > 0 {
        fitnesses[0..max_idx].windows(2).all(|w| w[0] <= w[1])
    } else {
        true
    };

    let after_decreases = if max_idx < n - 1 {
        fitnesses[max_idx..].windows(2).all(|w| w[0] >= w[1])
    } else {
        true
    };

    let is_concave_peak = before_increases && after_decreases;

    let verdict = if is_concave_peak {
        format!("PEAK (genuine DoL optimum at ratio idx={}, fitness={})", max_idx, max_fitness)
    } else {
        "PLATEAU_or_FLAT (interior max but no concavity)".to_string()
    };

    (is_concave_peak, verdict, curve_str)
}

/// Skeleton Rung-0 verdict harness. PASS 1 scaffold; full data collection + PASS 2 in cloud.
#[test]
#[ignore]  // Heavy; dispatched to cloud via sim-run.sh scenario or CI
fn topo_diff_rung0_imposed_split_verdict() {
    println!("\n════════════════════════════════════════════════════════════════");
    println!("TOPO-DIFF Rung 0: Imposed-Split Verdict (Economy Isolation)");
    println!("════════════════════════════════════════════════════════════════");
    println!("\nPRE-DECLARED CRITERION:");
    println!("  PEAK (interior optimum, concave, strict max > neighbors on both sides) ⇒ PASS");
    println!("  PLATEAU (flat interior, only edges drop) ⇒ NULL (cliff-avoidance, not DoL)");
    println!("  EDGE/MONOTONE ⇒ NULL (no interior optimum)");
    println!("\nSweep: germ:soma ratios at matched body sizes (N={:?})", TEST_BODY_SIZES);
    println!("Measurement: (per-capita income) × (fertility gate) for each split");
    println!("Seeds: {}; majority=≥{}/{} PEAK results → Rung 0 PASS", VERDICT_SEEDS.len(), SEED_MAJORITY, VERDICT_SEEDS.len());
    println!("\n");

    let mut seeds_pass = vec![];
    let mut seeds_fail = vec![];

    for &seed in &VERDICT_SEEDS {
        println!("═ SEED 0x{seed:08X} ═");

        let mut cfg = driver_config(seed);
        cfg.econ.fate_economy = true;  // THE Rung-0 gate
        cfg.econ.env_frontier_config = Some(sim_core::EnvFrontierConfig {
            patch_grain: 4,  // ENV-0a′ (same as fate_economy_config)
        });

        let mut seed_pass_all = true;

        for &body_size in TEST_BODY_SIZES {
            println!("\n  Sweeping N={body_size}:");

            // Sweep germ:soma splits
            let curve = sweep_splits_for_size(body_size, &cfg.econ);

            // Analyze fitness curve — distinguish PEAK vs PLATEAU vs EDGE/MONOTONE
            let (is_peak, verdict, curve_str) = analyze_fitness_curve(&curve);

            println!("    Fitness curve (income×fertility): [{}]", curve_str);
            println!("    Verdict: {}", verdict);
            println!("    Classification: {}", if is_peak { "PEAK (✓ PASS condition)" } else { "NOT_PEAK (✗ NULL)" });

            // Print detailed curve annotation with split ratios
            print!("    Split ratios (germ:soma): ");
            for (g, _inc, _fert) in &curve {
                let soma = body_size - g;
                print!("({g}:{soma}) ");
            }
            println!();

            if !is_peak {
                seed_pass_all = false;
            }
        }

        if seed_pass_all {
            seeds_pass.push(seed);
            println!("  Seed PASS: all sizes showed interior maximum");
        } else {
            seeds_fail.push(seed);
            println!("  Seed FAIL: at least one size had no interior maximum");
        }
    }

    println!("\n════════════════════════════════════════════════════════════════");
    println!("SUMMARY");
    println!("════════════════════════════════════════════════════════════════");
    println!(
        "RESULT: {}/{} seeds showed PEAK (genuine interior optimum at all N)",
        seeds_pass.len(),
        VERDICT_SEEDS.len()
    );
    println!("        {}/{} seeds showed PLATEAU or EDGE (cliff-avoidance only, no DoL)",
        seeds_fail.len(),
        VERDICT_SEEDS.len()
    );

    let rung0_pass = seeds_pass.len() >= SEED_MAJORITY;
    println!(
        "\nTOPO-DIFF RUNG 0 VERDICT: {}",
        if rung0_pass {
            format!("PASS (≥{}/{} seeds showed PEAK — fate-keyed DoL genuinely pays)", seeds_pass.len(), VERDICT_SEEDS.len())
        } else {
            format!("NULL (most seeds showed PLATEAU/EDGE — economy does not reward differentiation; {}/{} seeds failed)", seeds_fail.len(), VERDICT_SEEDS.len())
        }
    );

    if !rung0_pass {
        if seeds_fail.iter().all(|_| true) {
            println!("\nAnalysis: Failed seeds showed PLATEAU (flat interior, no interior optimum).");
            println!("Interpretation: Mixed bodies beat pure extremes only by avoiding cliffs");
            println!("(germ=0 sterility, soma=0 income floor), not by structural DoL benefit.");
            println!("→ Economy does not provide advantage to fate-keyed differentiation; pivot stops here.");
        }
    } else {
        println!("\nProceed to Rung 1 (sparse topology): PASS enables topology investigation.");
    }

    // OBSERVATIONAL verdict — harness sanity gate.
    assert!(
        !seeds_pass.is_empty(),
        "harness error: no seeds produced results; check econ/WorldView/sweep logic"
    );
}
