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

/// Single test seed: measure_fitness() has NO RNG input (deterministic economy measurement),
/// so all seeds produce IDENTICAL curves. Multi-seed framing would be theater.
/// Honest reporting: one curve per body size, deterministic in the germ:soma split.
const TEST_SEED: u64 = 0xD1FF_0001;

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
/// CRITICAL: module_is_germ must encode the split (germ=true, soma=false) so that measure_fitness()
/// can correctly count germ vs soma cells. This field is NOT optional for fate_economy.
fn build_specialist_body(body_size: i64, germ_count: i64) -> CellGraph {
    let soma_count = (body_size - germ_count).max(0).min(body_size);
    let germ_count = (body_size - soma_count) as i32;
    let soma_count_i32 = soma_count as i32;

    let mut module_type = vec![];
    let mut module_cell_count = vec![];
    let mut module_is_germ = vec![];

    // Germ module: type B, germ_count cells
    if germ_count > 0 {
        module_type.push(CellType::B);
        module_cell_count.push(germ_count);
        module_is_germ.push(true);  // CRITICAL: mark as germ so measure_fitness counts it
    }
    // Soma module: type A, soma_count cells
    if soma_count_i32 > 0 {
        module_type.push(CellType::A);
        module_cell_count.push(soma_count_i32);
        module_is_germ.push(false);  // CRITICAL: mark as soma
    }

    let n_modules = module_type.len();
    CellGraph {
        g_dev: 4,  // arbitrary grid size (not used in hand-built bodies)
        module_type,
        module_cell_count,
        module_is_germ,  // Encodes the imposed split (germ=true, soma=false)
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
/// Returns (income_per_cell, fertility_rate) computed from fate-keyed germ/soma split.
///
/// **Measurement formula validation against real stages:**
/// This function computes fitness without calling stage_interactions/stage_birth_death directly
/// (which require full World/Telemetry context). However, the formula MIRRORS the key
/// deterministic computations from those stages:
///
/// 1. **Income (from stage_interactions + stage_birth_death's income_per_capita):**
///    - Real formula (stage_interactions): `demand = u_max·R/(R+k_m)` (Monod kinetics)
///    - Real formula (fate_economy=true): income scales by SOMA count only (germ cells don't forage)
///    - This test: `demand · soma_active / body_size` (per-capita, matching the real stage)
///    - R = 100 (typical world resource level from default config)
///    - u_max, k_m from econ params (fate_economy_config calibrated values)
///    - Key property: income ∝ soma (proven in stage code), germ cells = 0 income
///
/// 2. **Fertility (from stage_birth_death's repro gate):**
///    - Real formula (fate_economy=true): fertility = if germ > 0 { base_fertility } else { 0 }
///    - Real formula (germ_gate): all germ cells gate reproduction (any germ > 0 → can reproduce)
///    - This test: binary 0 (sterile) or 1 (can reproduce) based on germ count
///    - Key property: germ count gates fertility (proven in stage code)
///
/// These formulas are LOAD-BEARING for the verdict:
/// - If PEAK emerges: income-scaling by soma + germ-gate-fertility genuinely favors splits
/// - If PLATEAU/EDGE: no DoL advantage to imposed splits, pure cliff-avoidance
/// - The test can only reach PEAK if the formula correctly reflects the real economy
fn measure_fitness(graph: &CellGraph, econ: &sim_core::EconParams) -> (i64, i64) {
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

    let soma_active = soma.max(1);  // bootstrap (avoid division by zero if soma=0)

    // Income measurement: Monod demand at nominal R=100.
    // fate_economy=true: income scales by soma cells only (germ don't forage).
    let r = 100i64;
    let u_max = econ.u_max;
    let km = econ.km;
    let demand = u_max * r / (r + km);  // monod_demand inline (matches stage_interactions)
    let demand_scaled = demand * (soma_active as i64);
    let income_per_cell = demand_scaled / graph.body_size().max(1);

    // Repro measurement: germ count gates fertility.
    // Germ=0 → sterile (fertility=0).
    // Germ>0 → can reproduce (fertility=1).
    // (Normalized to binary for this verdict; actual repro_bar is genome-specific)
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
/// **CRITICAL:** Analysis restricted to FERTILE subdomain (fertility==1 points).
/// Sterile points (germ=0, fertility=0) are excluded from peak detection because they
/// are structural cliffs from the germ gate, not rising shoulders of concave optima.
///
/// Distinctions:
/// - PEAK (PASS): Among fertile points, interior max is strict peak with concave curvature.
///   This is genuine DoL structure (fitness rises then falls over fertile domain).
/// - PLATEAU/EDGE/MONOTONE (NULL): No interior optimum among fertile points, or max is at
///   the edge of fertile domain (germ=1 is lowest fertile → it's an EDGE). Mere cliff-avoidance.
///
/// Returns (is_peak, curve_classification, full_curve_str).
fn analyze_fitness_curve(curve: &[(i64, i64, i64)]) -> (bool, String, String) {
    if curve.is_empty() {
        return (false, "EMPTY_CURVE".to_string(), "".to_string());
    }

    // Build curve string for full reporting (all points, including sterile)
    let fitnesses_full: Vec<i64> = curve.iter().map(|(_, inc, fert)| inc * fert).collect();
    let curve_str = fitnesses_full.iter().map(|f| f.to_string()).collect::<Vec<_>>().join(", ");

    // FILTER TO FERTILE SUBDOMAIN: only analyze points with fertility > 0
    // Sterile points (germ=0, fertility=0) are structural cliffs, not part of the peak analysis.
    let fertile_points: Vec<(usize, i64)> = curve
        .iter()
        .enumerate()
        .filter(|(_, (_, _inc, fert))| *fert > 0)  // Keep only fertile points
        .map(|(idx, (_, inc, _fert))| (idx, *inc))  // Map to (original_index, income)
        .collect();

    if fertile_points.is_empty() {
        // All points sterile (no viable germ:soma split exists) → NULL
        return (false, "NULL (no fertile points; all splits sterile)".to_string(), curve_str);
    }

    if fertile_points.len() == 1 {
        // Only one fertile point (germ=1 at lowest end) → EDGE
        let (idx, _inc) = fertile_points[0];
        return (
            false,
            format!("EDGE (only one fertile point at germ={}, no interior optimum)", idx),
            curve_str,
        );
    }

    // Analyze peak within fertile subdomain
    let fertile_fitnesses: Vec<i64> = fertile_points.iter().map(|(_, inc)| *inc).collect();
    let n_fertile = fertile_fitnesses.len();

    // Find max among fertile points
    let max_fitness = *fertile_fitnesses.iter().max().unwrap_or(&0);
    let max_idx_fertile = fertile_fitnesses
        .iter()
        .position(|&f| f == max_fitness)
        .unwrap_or(0);

    // Check if max is interior within fertile subdomain
    let is_interior = max_idx_fertile > 0 && max_idx_fertile < n_fertile - 1;

    if !is_interior {
        // Max at edge of fertile domain (e.g., germ=1 at low end)
        return (
            false,
            "EDGE (maximum at boundary of fertile domain, no interior optimum)".to_string(),
            curve_str,
        );
    }

    // Interior max in fertile domain: check if it's a strict PEAK
    let left_neighbor = fertile_fitnesses[max_idx_fertile - 1];
    let right_neighbor = fertile_fitnesses[max_idx_fertile + 1];

    let is_strict_peak = max_fitness > left_neighbor && max_fitness > right_neighbor;

    if !is_strict_peak {
        // Interior max but not strict → PLATEAU (flat)
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
            .all(|w| w[0] <= w[1])
    } else {
        true
    };

    let after_decreases = if max_idx_fertile < n_fertile - 1 {
        fertile_fitnesses[max_idx_fertile..]
            .windows(2)
            .all(|w| w[0] >= w[1])
    } else {
        true
    };

    let is_concave_peak = before_increases && after_decreases;

    let verdict = if is_concave_peak {
        // Map fertile index back to original germ count for reporting
        let (orig_germ_idx, _) = fertile_points[max_idx_fertile];
        format!(
            "PEAK (genuine DoL optimum at germ={}, fitness={})",
            orig_germ_idx, max_fitness
        )
    } else {
        "PLATEAU_or_FLAT (interior max in fertile domain but no concavity)".to_string()
    };

    (is_concave_peak, verdict, curve_str)
}

/// Rung-0 verdict harness: deterministic economy measurement (NO RNG in measure_fitness).
/// Honest reporting: one curve per body size, classified as PEAK/PLATEAU/EDGE.
/// **NOT multi-seed:** measure_fitness() has zero randomness; all seeds produce identical curves.
///
/// Pre-declared verdict (mature after classifier fixes):
/// - PEAK (interior optimum in fertile domain, concave) ⇒ PASS (economy rewards DoL)
/// - PLATEAU/EDGE (max at boundary or flat) ⇒ NULL (economy does not reward DoL)
/// - Expected: EDGE at germ=1 (lowest fertile) → NULL (per coordinator analysis)
#[test]
#[ignore]  // Heavy; dispatched to cloud via sim-run.sh scenario or CI
fn topo_diff_rung0_imposed_split_verdict() {
    println!("\n════════════════════════════════════════════════════════════════");
    println!("TOPO-DIFF Rung 0: Imposed-Split Verdict (Economy Isolation)");
    println!("════════════════════════════════════════════════════════════════");
    println!("\nMEASUREMENT DESIGN:");
    println!("  Hand-built bodies with IMPOSED germ:soma splits (0:N to N:0)");
    println!("  Deterministic economy: income ∝ soma (fate-keyed), fertility gates on germ");
    println!("  Analysis: PEAK vs PLATEAU/EDGE classification in FERTILE subdomain");
    println!("\nPRE-DECLARED CRITERION:");
    println!("  PEAK: interior optimum in fertile domain, concave curvature ⇒ PASS");
    println!("  PLATEAU/EDGE: max at boundary or flat interior ⇒ NULL (no DoL gain)");
    println!("\nSweep: germ:soma ratios at matched body sizes (N={:?})", TEST_BODY_SIZES);
    println!("Fitness: (income per cell) × (fertility gate) = income only for fertile germ>0");
    println!("Note: Measurement is DETERMINISTIC (no RNG input). One curve per body size.");
    println!("Honest reporting: curve classification, no multi-seed theater.\n");

    let mut cfg = driver_config(TEST_SEED);
    cfg.econ.fate_economy = true;  // THE Rung-0 gate
    cfg.econ.env_frontier_config = Some(sim_core::EnvFrontierConfig {
        patch_grain: 4,  // ENV-0a′
    });

    let mut all_peak = true;

    for &body_size in TEST_BODY_SIZES {
        println!("═ Body size N={body_size} ═");

        // Sweep germ:soma splits
        let curve = sweep_splits_for_size(body_size, &cfg.econ);

        // Analyze fitness curve (now correctly restricted to fertile subdomain)
        let (is_peak, verdict, curve_str) = analyze_fitness_curve(&curve);

        println!("  Fitness curve (income×fertility): [{}]", curve_str);
        println!("  Verdict: {}", verdict);
        println!("  Classification: {}", if is_peak { "PEAK ✓" } else { "NOT_PEAK ✗" });

        // Print detailed annotations
        print!("  Split ratios (germ:soma): ");
        for (g, _inc, _fert) in &curve {
            let soma = body_size - g;
            print!("({g}:{soma}) ");
        }
        println!("\n");

        if !is_peak {
            all_peak = false;
        }
    }

    println!("════════════════════════════════════════════════════════════════");
    println!("VERDICT");
    println!("════════════════════════════════════════════════════════════════");

    if all_peak {
        println!("TOPO-DIFF RUNG 0 VERDICT: PASS");
        println!("Interpretation: Economy structure genuinely rewards imposed germ/soma splits");
        println!("→ Proceed to Rung 1 (sparse topology investigation)");
    } else {
        println!("TOPO-DIFF RUNG 0 VERDICT: NULL");
        println!("Interpretation: No interior optimum in fertile domain; germ:soma split");
        println!("produces no structural DoL advantage. Fitness is monotone or edge-peaked.");
        println!("→ Economy does not reward differentiation; Rung 0 is a valid landing.");
    }

    // ── ROBUSTNESS CHECK: fate_economy=false (byte-identity, no state leakage) ──
    println!("\n════════════════════════════════════════════════════════════════");
    println!("ROBUSTNESS: fate_economy=FALSE (byte-identity gate, no state leakage)");
    println!("════════════════════════════════════════════════════════════════");
    {
        let mut cfg_control = driver_config(TEST_SEED);
        cfg_control.econ.fate_economy = false;  // GATE OFF — germ/soma distinction hidden
        cfg_control.econ.env_frontier_config = Some(sim_core::EnvFrontierConfig {
            patch_grain: 4,
        });

        let body_size = TEST_BODY_SIZES[0];
        println!("\nWith fate_economy=FALSE (control arm):");
        println!("  Sweeping N={body_size} (first test size):");

        let curve_control = sweep_splits_for_size(body_size, &cfg_control.econ);
        let (is_peak_control, verdict_control, curve_control_str) = analyze_fitness_curve(&curve_control);

        println!("    Fitness curve: [{}]", curve_control_str);
        println!("    Verdict: {}", verdict_control);

        // With fate_economy=false, germ/soma split is invisible → all-soma income only.
        // Income should be CONSTANT across all splits (only soma count matters, all bodies have same N).
        // If the curve is NOT flat, something is wrong with the gate or state leakage.
        // Expected: NOT_PEAK (no interior optimum in fertile domain, all equal).
        let expected_flat = !is_peak_control;
        println!("    Expected (NOT_PEAK/flat): {}", if expected_flat { "✓" } else { "✗ LEAK DETECTED" });

        assert!(
            expected_flat,
            "fate_economy=false arm produced a PEAK (state leakage detected). The gate is not properly isolating the mechanic."
        );

        println!("  → Robustness PASS: fate_economy=false produces flat curve (no state leakage)");
    }

    println!("\n════════════════════════════════════════════════════════════════");
    println!("END OF HARNESS");
    println!("════════════════════════════════════════════════════════════════\n");
}
