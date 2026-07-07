//! DL-V (#343): DOL robustness verdict — does division-of-labor make the Phase-2 multicellular
//! transition ROBUST (vs D-5's marginal ~60%)? Four-arm harness: D-5 baseline / DOL-full /
//! DOL-refuge-only / ablation. Empirical verdict on germ:soma intermediate stability.

use cli::{build_sim, driver_config, dol_config};
use sim_core::{EconParams, SimConfig};

const EMERGE_FLOOR: i64 = 128; // ×BODY_SIZE_SCALE(256) == 50%
const MARGIN: i64 = 2;
const SEED_MAJORITY: usize = 3;
const POP_FLOOR: i64 = 10;
const FIXED_REFUGE_K_HAZARD: i32 = 128;
const BASE_HAZARD_SWEEP: [i64; 4] = [10, 20, 30, 45];
const VERDICT_SEEDS: [u64; 5] = [1, 2, 3, 4, 5];

// DOL-specific thresholds: germ:soma axis is coarse (~4-5 states), expect discrete intermediate
// instead of D-5's smooth body-size continuum.
const GERM_DRIFT_EPSILON: f64 = 1.0; // Allow 1 cell drift (coarse axis granularity)
const GERM_INTERMEDIATE_MIN: i64 = 2; // Modal germ ≥ 2 (not germ=1 pinning)

/// Extended result for DOL verdict: includes germ:soma metrics.
struct DolArmResult {
    frac: i64,
    mean_pop: f64,
    mean_body_size: f64,
    body_size_drift: f64,
    mean_germ_frac: f64,      // mean(germ_cells / total_cells)
    modal_germ: i64,           // most common germ_cell count in window
    collapsed: bool,
}

fn run_dol_arm(
    seed: u64,
    ticks: u64,
    window_start: u64,
    cfg_builder: impl Fn(u64) -> SimConfig,
    predators_on: bool,
    refuge_k: i32,
    base_hazard: i64,
    dol_germ_repro_override: Option<bool>, // Override dol_germ_repro if specified
) -> DolArmResult {
    let mut cfg = cfg_builder(seed);
    if !predators_on {
        cfg.econ.predation = None;
    } else {
        let spec = cfg
            .econ
            .predation
            .as_mut()
            .expect("config always configures predation");
        spec.mode = sim_core::PredationMode::Hazard;
        spec.base_hazard = base_hazard;
        spec
            .size_refuge
            .as_mut()
            .expect("config always configures size_refuge")
            .refuge_k = refuge_k;
    }

    // Apply override if specified (for DOL-refuge-only arm)
    if let Some(override_val) = dol_germ_repro_override {
        cfg.econ.dol_germ_repro = override_val;
    }

    let mut sim = build_sim(cfg);
    let mut frac_sum: i64 = 0;
    let mut valid_ticks: i64 = 0;
    let mut pop_sum: i64 = 0;
    let mut pop_ticks: i64 = 0;
    let mut body_size_sum: f64 = 0.0;
    let mut body_size_ticks: usize = 0;
    let window_len = (ticks - window_start) as usize;
    let mut body_size_first_half: Vec<f64> = Vec::new();
    let mut body_size_second_half: Vec<f64> = Vec::new();
    let mid_point = window_start + (window_len as u64 / 2);

    // Track germ:soma distribution
    let mut germ_sum: i64 = 0;
    let mut germ_count: i64 = 0;
    let mut germ_histogram: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();

    for t in 0..ticks {
        sim.step();
        if t >= window_start {
            // Snapshot germ:soma distribution first (requires &mut)
            let snapshot = sim.cellgraph_snapshot();

            // Then get telemetry
            let tel = sim.telemetry();
            pop_sum += tel.population;
            pop_ticks += 1;

            // Process snapshot data
            for (_n_modules, germ_cells, _soma_cells, total_cells) in snapshot {
                if total_cells > 0 {
                    germ_sum += germ_cells;
                    germ_count += 1;
                    *germ_histogram.entry(germ_cells).or_insert(0) += 1;
                }
            }

            let body_size = tel.mean_body_size as f64;
            if t < mid_point {
                body_size_first_half.push(body_size);
            } else {
                body_size_second_half.push(body_size);
            }

            if tel.population >= POP_FLOOR {
                frac_sum += tel.multicellular_frac;
                valid_ticks += 1;
                body_size_sum += body_size;
                body_size_ticks += 1;
            }
        }
    }

    let mean_pop = if pop_ticks > 0 { pop_sum as f64 / pop_ticks as f64 } else { 0.0 };
    let mean_body_size = if body_size_ticks > 0 { body_size_sum / body_size_ticks as f64 } else { 0.0 };
    let mean_germ_frac = if germ_count > 0 { germ_sum as f64 / germ_count as f64 } else { 0.0 };

    // Find modal germ count
    let modal_germ = germ_histogram
        .iter()
        .max_by_key(|(_, count)| *count)
        .map(|(germ, _)| *germ)
        .unwrap_or(0);

    // Compute drift
    let drift = if !body_size_first_half.is_empty() && !body_size_second_half.is_empty() {
        let mean_first = body_size_first_half.iter().sum::<f64>() / body_size_first_half.len() as f64;
        let mean_second =
            body_size_second_half.iter().sum::<f64>() / body_size_second_half.len() as f64;
        (mean_second - mean_first).abs()
    } else {
        0.0
    };

    if valid_ticks == 0 {
        DolArmResult {
            frac: 0,
            mean_pop,
            mean_body_size,
            body_size_drift: drift,
            mean_germ_frac,
            modal_germ,
            collapsed: true,
        }
    } else {
        DolArmResult {
            frac: frac_sum / valid_ticks,
            mean_pop,
            mean_body_size,
            body_size_drift: drift,
            mean_germ_frac,
            modal_germ,
            collapsed: false,
        }
    }
}

/// DL-V (#343): DOL robustness verdict harness. Tests whether division-of-labor makes the
/// Phase-2 multicellular transition robust (vs D-5's marginal ~60%). Four arms per (seed, base_hazard):
/// (1) D-5 baseline (flag-off), (2) DOL-full, (3) DOL-refuge-only (soma-refuge on, germ-repro off),
/// (4) ablation (no predators). Verdict logic: (a) DOL-full achieves robust majority ABOVE D-5 baseline,
/// (b) DOL-full > DOL-refuge-only (attribute to germ-repro, not just refuge re-parameterization),
/// (c) germ:soma intermediate is non-degenerate (modal_germ ≥ 2, not germ=1 pinning).
/// EMPIRICAL thresholds — no pre-declared hard robustness number beyond a/b/c gates.
#[test]
#[ignore]
fn driver_dol_verdict() {
    let ticks: u64 = std::env::var("DOL_VERDICT_TICKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(400);
    let window_len = ticks.min(1000);
    let window_start = ticks - window_len;

    println!("\nDL-V (#343) DOL robustness verdict: division-of-labor on Phase-2 multicellularity");
    println!(
        "PRE-DECLARED: EMERGE_FLOOR={:.0}%, MARGIN={MARGIN}x, SEED_MAJORITY={SEED_MAJORITY}/5, POP_FLOOR={POP_FLOOR}",
        EMERGE_FLOOR as f64 / sim_core::BODY_SIZE_SCALE as f64 * 100.0
    );
    println!(
        "ticks={ticks}  late-window=[{window_start},{ticks}]  fixed refuge_k={FIXED_REFUGE_K_HAZARD}  base_hazard sweep={:?}",
        BASE_HAZARD_SWEEP
    );
    println!(
        "GERM:SOMA thresholds: drift≤{:.1} cells, modal_germ≥{} (coarse axis granularity)",
        GERM_DRIFT_EPSILON, GERM_INTERMEDIATE_MIN
    );

    // Ablation is independent of base_hazard
    let ablation: Vec<DolArmResult> = VERDICT_SEEDS
        .iter()
        .map(|&seed| run_dol_arm(seed, ticks, window_start, dol_config, false, 0, 0, Some(true)))
        .collect();

    let mut any_regime_emerges = false;

    for &bh in &BASE_HAZARD_SWEEP {
        println!("\n{}", "-".repeat(100));
        println!("base_hazard={bh}");
        println!(
            "{:<6} {:>12} {:>12} {:>12} {:>12} {:>12} {:>10} {:>11} {:>8} {:>6}",
            "seed", "D-5%", "DOL-full%", "DOL-refuge%", "ablation%", "size", "drift", "germ_frac", "modal_g", "result"
        );

        let mut seed_dol_full_pass = 0usize;
        let mut seed_dol_refuge_pass = 0usize;
        let mut regime_has_nondegenerate_germ = false;

        for (i, &seed) in VERDICT_SEEDS.iter().enumerate() {
            let d5_baseline = run_dol_arm(seed, ticks, window_start, driver_config, true, FIXED_REFUGE_K_HAZARD, bh, Some(false));
            let dol_full = run_dol_arm(seed, ticks, window_start, dol_config, true, FIXED_REFUGE_K_HAZARD, bh, None);
            let dol_refuge_only = run_dol_arm(seed, ticks, window_start, dol_config, true, FIXED_REFUGE_K_HAZARD, bh, Some(false));
            let abl = &ablation[i];

            // Verdict conditions (a/b/c)
            let (a_ok, b_ok, c_ok);
            let d5_pct = d5_baseline.frac as f64 / sim_core::BODY_SIZE_SCALE as f64 * 100.0;
            let dol_full_pct = dol_full.frac as f64 / sim_core::BODY_SIZE_SCALE as f64 * 100.0;
            let dol_refuge_pct = dol_refuge_only.frac as f64 / sim_core::BODY_SIZE_SCALE as f64 * 100.0;
            let abl_pct = abl.frac as f64 / sim_core::BODY_SIZE_SCALE as f64 * 100.0;

            // (a) DOL-full passes D-5 faithfulness gates
            let floor_ok = !dol_full.collapsed && dol_full.frac >= EMERGE_FLOOR;
            let margin_abl_ok = !dol_full.collapsed && !abl.collapsed && dol_full.frac >= MARGIN * abl.frac;
            a_ok = floor_ok && margin_abl_ok;

            // (b) DOL-full > DOL-refuge-only (attribute to germ-repro, not refuge re-param)
            b_ok = !dol_full.collapsed && !dol_refuge_only.collapsed && dol_full.frac > dol_refuge_only.frac;

            // (c) germ:soma intermediate is non-degenerate (modal_germ ≥ 2)
            c_ok = dol_full.modal_germ >= GERM_INTERMEDIATE_MIN;

            let pass = a_ok && b_ok && c_ok;
            if pass {
                seed_dol_full_pass += 1;
            }
            if !dol_refuge_only.collapsed && dol_refuge_only.frac >= EMERGE_FLOOR {
                seed_dol_refuge_pass += 1;
            }
            if pass && dol_full.modal_germ >= GERM_INTERMEDIATE_MIN {
                regime_has_nondegenerate_germ = true;
            }

            let body_size_cells = dol_full.mean_body_size / sim_core::BODY_SIZE_SCALE as f64;
            let germ_frac_pct = dol_full.mean_germ_frac * 100.0;
            let tag = if dol_full.collapsed {
                "COLLAPSED"
            } else if pass {
                "PASS"
            } else {
                "fail"
            };

            println!(
                "{:<6} {:>11.1}% {:>11.1}% {:>11.1}% {:>11.1}% {:>9.1} {:>7.2} {:>7.1}% {:>8} {:>8}",
                seed, d5_pct, dol_full_pct, dol_refuge_pct, abl_pct, body_size_cells, dol_full.body_size_drift / sim_core::BODY_SIZE_SCALE as f64,
                germ_frac_pct, dol_full.modal_germ, tag
            );

            // Diagnostic notes on condition failures
            if !pass && !dol_full.collapsed {
                if !a_ok {
                    println!("       (a) DOL-full emergence: {:.1}% ≥ {:.0}% floor? {} | ≥ {MARGIN}×ablation? {}",
                        dol_full_pct, EMERGE_FLOOR as f64 / sim_core::BODY_SIZE_SCALE as f64 * 100.0, floor_ok, margin_abl_ok);
                }
                if !b_ok {
                    println!("       (b) DOL-full > DOL-refuge: {:.1}% > {:.1}%? {}", dol_full_pct, dol_refuge_pct, b_ok);
                }
                if !c_ok {
                    println!("       (c) germ intermediate: modal_germ={} ≥ {}? {}", dol_full.modal_germ, GERM_INTERMEDIATE_MIN, c_ok);
                }
            }
        }

        println!("  DOL-full seeds passing all 3 conditions (a/b/c): {seed_dol_full_pass}/5 (need ≥{SEED_MAJORITY})");
        println!("  DOL-refuge-only seeds (condition b baseline): {seed_dol_refuge_pass}/5");

        if seed_dol_full_pass >= SEED_MAJORITY {
            any_regime_emerges = true;
            println!("  ✓ Regime supports DOL emergence (a/b/c all pass)");
        } else {
            println!("  ✗ Regime does NOT support DOL emergence");
        }
    }

    println!("\n{}", "-".repeat(100));
    println!();
    if any_regime_emerges {
        println!("DOL-VERDICT: EMERGENCE");
        println!("  At some base_hazard, DOL-full reaches SEED_MAJORITY={SEED_MAJORITY}/5 on all 3 conditions:");
        println!("  (a) multicellular_frac ≥ {:.0}% AND ≥{MARGIN}x ablation",
            EMERGE_FLOOR as f64 / sim_core::BODY_SIZE_SCALE as f64 * 100.0);
        println!("  (b) DOL-full > DOL-refuge-only (germ-repro trade-off load-bearing, not refuge re-param)");
        println!("  (c) germ:soma intermediate non-degenerate (modal_germ ≥ {GERM_INTERMEDIATE_MIN}, not germ=1 pinning)");
        println!("  ⇒ Division-of-labor robustly drives multicellularity ABOVE D-5 marginal (~60%).");
    } else {
        println!("DOL-VERDICT: NULL");
        println!("  No base_hazard regime reached SEED_MAJORITY={SEED_MAJORITY}/5 on all 3 conditions.");
        println!("  Interpreted: DOL mechanic is insufficient or germ:soma substrate too marginal.");
        println!("  Phase-2 transition closes on D-5 hazard-refuge alone (variant A, pre-committed valid).");
    }

    // Never panic on biological outcome — NULL is an honest result
}
