//! P3-3 thermal-niche faithful-verdict (a-d criteria): does niche partitioning emerge from
//! thermal gradient (with ambient_tolerance) vs random drift (ablation-baseline)? Harness scaffolding
//! for PASS 1: collect data, print per-seed gates + VERDICT, output calibration breadth_cost_k for
//! PASS 2 (cloud-run).
//!
//! Two arms on the SAME high-gradient verdict config (thermal_verdict_temps override):
//!   WITH     (ambient_tolerance=Some): the full tolerance economy — species diverge along thermal gradient.
//!   ABLATION (ambient_tolerance=None):  tolerance disabled (bytewise identical to shipped) — control.
//!
//! Four channels (a-d): reversibility, intermediate-stable, measurable-cost (calibration), ablation margin.
//! Channel-isolated: each criterion tests a different aspect of faithful niche-partitioning.
//!
//! Heavy (6 arms × N seeds × 8000-tick horizon) — `#[ignore]`d; run via `sim-run.sh` scenario
//! or locally with `TICKS=8000` override.
//!
//! Scaffold PASS 1 (this file): harness structure, data collection, output format.
//! Full PASS 2 (cloud): run verdict, collect measured penalty, calibrate breadth_cost_k, finalize report.

use cli::{build_sim, p3_verdict_config};
use sim_core::{AmbientToleranceSpec, EconParams};

const VERDICT_SEEDS: [u64; 5] = [0xB0B0_0001, 0xB0B0_0002, 0xB0B0_0003, 0xB0B0_0004, 0xB0B0_0005];
const SEED_MAJORITY: usize = 3; // ≥3/5 seeds must pass all 4 gates → EMERGENCE

// ── Pre-declared constants (drifted from design-spec §2.2 measurement; to be finalized in PASS 1) ──

/// Criterion (a): reversibility — if flattened to constant T=1500, population tol_optimum → 1500
/// over REVERSIBILITY_FLAT_TICKS ticks (selection-driven convergence, not per-lineage re-evolution).
/// PROVISIONAL — measure first run, finalize.
const REVERSIBILITY_FLAT_TICKS: u64 = 500;
const REVERSIBILITY_THRESHOLD: i32 = 100;  // mean optimum within ±100 cd of 1500

/// Criterion (b): intermediate-stable — in window [4000..8000], cold-adapted (tol_optimum ≤ +10°C)
/// and hot-adapted (tol_optimum ≥ +20°C) persist at ≥15% each, and mean drifts ≤±50cd.
const COLD_ADAPTED_THRESHOLD: i32 = 1000;   // ≤ +10°C
const HOT_ADAPTED_THRESHOLD: i32 = 2000;    // ≥ +20°C
const COLD_FRACTION_THRESHOLD: f64 = 0.15;
const HOT_FRACTION_THRESHOLD: f64 = 0.15;
const DRIFT_EPSILON_OPTIMUM: i32 = 50;      // max drift in [4000..8000]

/// Criterion (c): measurable-cost — specialist income-loss vs generalist, 2-5% range.
/// SPLIT into (c)-CALIBRATION (definitional knob-tuning) and (c)-GATE (emergent breadth-narrowing).
/// PASS 1 harness collects the measurement; PASS 2 adjusts k and gates it.
const SPECIALIST_INCOME_LOSS_MIN: f64 = 0.02;  // 2%
const SPECIALIST_INCOME_LOSS_MAX: f64 = 0.05;  // 5%

/// Criterion (d): ablation margin — WITH divergence ≥ 1.5× ABLATION divergence (early 2000 ticks).
/// Absolute floor: DIV_FLOOR (from design-spec §2.2, finalized in measurement).
const WITH_vs_ABLATION_MARGIN: f64 = 1.5;
const WITH_DIVERGENCE_WINDOW_START: u64 = 0;
const WITH_DIVERGENCE_WINDOW_END: u64 = 2000;

/// Per-entity minimum population to count it in measurement (avoid rare stragglers).
const POP_FLOOR: i64 = 1;

// ── Helper structures for verdict logic ──

struct ArmResult {
    /// Mean tol_optimum over window [4000..8000] (criterion b).
    mean_tol_optimum: i32,
    /// Std dev of tol_optimum.
    std_tol_optimum: i32,
    /// Cold-adapted (tol_optimum ≤ 1000) fraction in window.
    cold_fraction: f64,
    /// Hot-adapted (tol_optimum ≥ 2000) fraction in window.
    hot_fraction: f64,
    /// Divergence score (criterion d): sum of squared deviations from founder 1500 in [0..2000].
    divergence_early: f64,
    /// Mean population in window.
    mean_pop: i64,
}

struct VerdictSeedResult {
    gate_a: bool,  // reversibility
    gate_b: bool,  // intermediate-stable
    gate_c: bool,  // measurable-cost (PASS 1: collect, PASS 2: gate)
    gate_d: bool,  // ablation margin
    with_result: ArmResult,
    abl_result: ArmResult,
}

/// Skeleton harness — PASS 1 scaffold, not executed locally (hook forbids sim runs).
#[test]
#[ignore]  // Heavy; dispatched to cloud via sim-run.sh scenario
fn p3_thermal_niche_verdict() {
    let ticks: u64 = std::env::var("P3_VERDICT_TICKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8000);
    let window_start = ticks.saturating_sub(4000).max(1);

    println!("\n════════════════════════════════════════════════════════════════");
    println!("P3-3: Thermal-Niche Faithful-Verdict (a-d Criteria + Calibration)");
    println!("════════════════════════════════════════════════════════════════");
    println!("\nPRE-DECLARED CRITERIA:");
    println!("  (a) Reversibility: mid-run flatten T→1500 → tol_optimum→1500 in {REVERSIBILITY_FLAT_TICKS}t");
    println!("  (b) Intermediate-Stable: cold≥{:.0}%, hot≥{:.0}%, drift≤±{DRIFT_EPSILON_OPTIMUM}cd in window",
        COLD_FRACTION_THRESHOLD * 100.0, HOT_FRACTION_THRESHOLD * 100.0);
    println!("  (c) Measurable-Cost: specialist penalty 2–5% income vs generalist → calibrate k");
    println!("  (d) Ablation: WITH divergence ≥{WITH_vs_ABLATION_MARGIN}× ABLATION in [0..2000]t");
    println!("\nConfig: p3_verdict_config (high-gradient, ambient_tolerance=Some/None)");
    println!("Seeds: 5 (VERDICT_SEEDS); majority=≥{SEED_MAJORITY}/5 → EMERGENCE");
    println!("Ticks: {ticks};  window: [{window_start},{ticks}]");
    println!("\n");

    let mut seeds_pass = vec![];
    let mut seeds_fail = vec![];
    let mut seeds_both_alive = 0usize;

    for &seed in &VERDICT_SEEDS {
        println!("═ SEED 0x{seed:08X} ═");

        // Run WITH arm (ambient_tolerance=Some, high-gradient world)
        let with_result = run_verdict_arm(seed, ticks, window_start, true);

        // Run ABLATION arm (ambient_tolerance=None, stock temps, control)
        let abl_result = run_verdict_arm(seed, ticks, window_start, false);

        // Apply gates (a-d) — currently scaffolded (placeholder logic)
        let gate_a = true;  // TODO: implement reversibility gate
        let gate_b = true;  // TODO: implement intermediate-stable gate
        let gate_c = true;  // TODO: implement measurable-cost gate (CALIBRATION)
        let gate_d = with_result.divergence_early >= abl_result.divergence_early * WITH_vs_ABLATION_MARGIN;

        let seed_pass = gate_a && gate_b && gate_c && gate_d;

        println!(
            "  a:{} b:{} c:{} d:{} | overall:{}",
            if gate_a { "✓" } else { "✗" },
            if gate_b { "✓" } else { "✗" },
            if gate_c { "✓" } else { "✗" },
            if gate_d { "✓" } else { "✗" },
            if seed_pass { "PASS" } else { "FAIL" }
        );
        println!("  WITH div={:.0}, ABL div={:.0}, margin={:.2}×",
            with_result.divergence_early,
            abl_result.divergence_early,
            if abl_result.divergence_early > 0.0 {
                with_result.divergence_early / abl_result.divergence_early
            } else {
                0.0
            }
        );

        if with_result.mean_pop > 0 && abl_result.mean_pop > 0 {
            seeds_both_alive += 1;
        }

        if seed_pass {
            seeds_pass.push(seed);
        } else {
            seeds_fail.push(seed);
        }
    }

    println!("\n════════════════════════════════════════════════════════════════");
    println!("SUMMARY");
    println!("════════════════════════════════════════════════════════════════");
    println!(
        "RESULT: {}/{} seeds passed all 4 gates (both-alive: {}/{})",
        seeds_pass.len(),
        VERDICT_SEEDS.len(),
        seeds_both_alive,
        VERDICT_SEEDS.len()
    );

    let faithful = seeds_pass.len() >= SEED_MAJORITY;
    println!(
        "\nTHERMAL-NICHE-VERDICT: {}",
        if faithful { "EMERGENCE (a-d aligned)" } else { "NULL (gates did not align)" }
    );

    if faithful {
        // TODO: PASS 2 — compute final breadth_cost_k from criterion (c) measurement
        println!("CALIBRATED breadth_cost_k = [TODO: extract from criterion (c) measurement]");
    } else {
        println!("\nFailed seeds: {:?}", seeds_fail);
        println!("Both-alive: {}/{}", seeds_both_alive, VERDICT_SEEDS.len());
        println!("→ Repeat with adjusted config / larger horizon if harness is viable");
    }

    // OBSERVATIONAL verdict — harness sanity gate only.
    assert!(
        seeds_both_alive >= SEED_MAJORITY,
        "harness failure: {}/{} seeds had viable arms; cannot read verdict (raise ticks / check config)",
        seeds_both_alive,
        VERDICT_SEEDS.len()
    );
}

/// Run one arm (WITH or ABLATION) and collect verdict telemetry over the full ticks.
/// Placeholder — PASS 1 scaffold; actual data collection in PASS 2.
fn run_verdict_arm(seed: u64, ticks: u64, window_start: u64, with_tolerance: bool) -> ArmResult {
    let mut cfg = p3_verdict_config(seed);
    if !with_tolerance {
        // ABLATION: disable ambient_tolerance
        cfg.econ.ambient_tolerance = None;
    }

    let mut sim = build_sim(cfg);

    let mut pop_sum: i64 = 0;
    let mut pop_ticks: i64 = 0;
    let mut opt_sum: i64 = 0;
    let mut opt_sq_sum: i64 = 0;
    let mut opt_window_count: i64 = 0;
    let mut cold_count: i64 = 0;
    let mut hot_count: i64 = 0;
    let mut total_in_window: i64 = 0;
    let mut div_early_sum: f64 = 0.0;

    for t in 0..ticks {
        sim.step();

        // Collect window telemetry [window_start, ticks]
        if t >= window_start {
            let _tel = sim.telemetry();
            pop_sum += _tel.population;
            pop_ticks += 1;

            // TODO: collect per-entity tol_optimum / tol_breadth from genomes
            // For now, placeholder: assume stub values
            if _tel.population > 0 {
                // Placeholder: assume mean optimum ≈ 1500 (neutral) in both arms
                opt_sum += 1500;
                opt_sq_sum += 1500 * 1500;
                opt_window_count += 1;
                // Placeholder divergence count
                cold_count += 0;
                hot_count += 0;
                total_in_window += _tel.population;
            }
        }

        // Collect early divergence [0, 2000]
        if t < 2000 {
            // TODO: collect per-entity divergence from founder (1500)
            // Placeholder: assume zero divergence in both arms (pre-measurement)
            div_early_sum += 0.0;
        }
    }

    let mean_pop = if pop_ticks > 0 { pop_sum / pop_ticks } else { 0 };
    let mean_opt = if opt_window_count > 0 {
        (opt_sum / opt_window_count) as i32
    } else {
        1500
    };
    let var_opt = if opt_window_count > 0 {
        ((opt_sq_sum / opt_window_count) - (opt_sum / opt_window_count) * (opt_sum / opt_window_count)).max(0)
    } else {
        0
    };
    let std_opt = (var_opt as f64).sqrt() as i32;

    let cold_frac = if total_in_window > 0 {
        cold_count as f64 / total_in_window as f64
    } else {
        0.0
    };
    let hot_frac = if total_in_window > 0 {
        hot_count as f64 / total_in_window as f64
    } else {
        0.0
    };

    ArmResult {
        mean_tol_optimum: mean_opt,
        std_tol_optimum: std_opt,
        cold_fraction: cold_frac,
        hot_fraction: hot_frac,
        divergence_early: div_early_sum,
        mean_pop,
    }
}
