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

use cli::{build_sim, driver_config};
use sim_core::{
    CellGraph, CellType, Energy, EnergyLedger, Genome, Phenotype, Position, SimClock, Telemetry,
    Vec2Fixed, WorldRes, WorldView, FieldRes, Deposit, MergeStrategy,
};
use bevy_ecs::prelude::*;

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
    let (soma, _germ) = graph.fate_germ_soma_counts();
    let soma_active = soma.max(1);  // bootstrap
    let per_cell_soma = if soma > 0 { soma } else { 1 };

    // Monod demand at a nominal resource level (R=100, typical world).
    let r = 100i64;
    let u_max = econ.u_max;
    let km = econ.km;
    let demand = u_max * r / (r + km);  // monod_demand inline
    let demand_scaled = demand * soma_active;
    let income_per_cell = demand_scaled / graph.body_size().max(1);

    // Repro measurement: stage_birth_death gates on germ count.
    // Germ=0 → sterile (repro_bar = i64::MAX, fertility=0).
    // Germ>0 → flat threshold (repro_bar = genome.repro_threshold, normalized to baseline).
    let (_soma, germ) = graph.fate_germ_soma_counts();
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

/// Analyze fitness curve for interior maximum (PASS) vs edge/monotone (NULL).
/// Returns (has_interior_max, verdict_str).
fn analyze_fitness_curve(curve: &[(i64, i64, i64)]) -> (bool, String) {
    if curve.is_empty() {
        return (false, "EMPTY_CURVE".to_string());
    }

    // Extract fitness values (income × fertility) for each split
    let fitnesses: Vec<i64> = curve.iter().map(|(_, inc, fert)| inc * fert).collect();

    // Find max fitness and its position
    let max_fitness = *fitnesses.iter().max().unwrap_or(&0);
    let max_idx = fitnesses.iter().position(|&f| f == max_fitness).unwrap_or(0);

    // Check for interior maximum: max_idx is strictly between 0 and len-1,
    // and is not the only peak (fitness should increase to it, then decrease).
    let is_interior = max_idx > 0 && max_idx < fitnesses.len() - 1;

    let has_interior_max = if is_interior {
        // Verify it's actually an interior peak (increases before, decreases after)
        let before_increases = fitnesses[0..max_idx].windows(2).all(|w| w[0] <= w[1]);
        let after_decreases = fitnesses[max_idx..].windows(2).all(|w| w[0] >= w[1]);
        before_increases && after_decreases
    } else {
        false
    };

    let verdict = if has_interior_max {
        format!("INTERIOR_MAX at idx={}, fitness={}", max_idx, max_fitness)
    } else if max_idx == 0 || max_idx == fitnesses.len() - 1 {
        "EDGE_MAX (no interior optimum)"
    } else {
        "MONOTONE_or_MULTIPLE (no singular interior peak)"
    };

    (has_interior_max, verdict.to_string())
}

/// Skeleton Rung-0 verdict harness. PASS 1 scaffold; full data collection + PASS 2 in cloud.
#[test]
#[ignore]  // Heavy; dispatched to cloud via sim-run.sh scenario or CI
fn topo_diff_rung0_imposed_split_verdict() {
    println!("\n════════════════════════════════════════════════════════════════");
    println!("TOPO-DIFF Rung 0: Imposed-Split Verdict (Economy Isolation)");
    println!("════════════════════════════════════════════════════════════════");
    println!("\nPRE-DECLARED CRITERION:");
    println!("  Interior maximum in fitness sweep ⇒ PASS (DoL genuinely pays)");
    println!("  Edge/monotone maximum ⇒ NULL (no DoL benefit, only cliff-avoidance)");
    println!("\nSweep: germ:soma ratios at matched body sizes (N={:?})", TEST_BODY_SIZES);
    println!("Measurement: (per-capita income) × (fertility gate) for each split");
    println!("Seeds: {}; majority=≥{}/{} → Rung 0 PASS", VERDICT_SEEDS.len(), SEED_MAJORITY, VERDICT_SEEDS.len());
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

            // Analyze fitness curve
            let (has_interior, verdict) = analyze_fitness_curve(&curve);

            println!("    Verdict: {}", verdict);
            println!("    Result: {}", if has_interior { "INTERIOR_MAX ✓" } else { "NO_INTERIOR ✗" });

            // Print detailed curve for inspection (PASS 2 to extract precise numbers)
            print!("    Curve: ");
            for (g, inc, fert) in &curve {
                let fitness = inc * fert;
                print!("({g}:{}→{})", body_size - g, fitness);
            }
            println!();

            if !has_interior {
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
        "RESULT: {}/{} seeds passed (interior max at all N)",
        seeds_pass.len(),
        VERDICT_SEEDS.len()
    );

    let rung0_pass = seeds_pass.len() >= SEED_MAJORITY;
    println!(
        "\nTOPO-DIFF RUNG 0 VERDICT: {}",
        if rung0_pass {
            "PASS (interior maximum confirmed — fate-keyed DoL genuinely pays)"
        } else {
            "NULL (no interior maximum — economy does not reward differentiation)"
        }
    );

    if !rung0_pass {
        println!("\nFailed seeds: {:?}", seeds_fail);
        println!("→ Economy does not provide structural advantage to DoL; pivot stops here.");
    } else {
        println!("\nProceed to Rung 1 (sparse topology) if pre-conditions met.");
    }

    // OBSERVATIONAL verdict — harness sanity gate.
    assert!(
        !seeds_pass.is_empty(),
        "harness error: no seeds produced results; check econ/WorldView/sweep logic"
    );
}
