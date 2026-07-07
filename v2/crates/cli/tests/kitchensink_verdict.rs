//! DL-C (#345): composition verdict — does the ECOLOGY of D-5 hazard-refuge + settling +
//! O₂-hypoxia + DOL compose to a robust multicellular transition? Three-arm harness: D-5 baseline /
//! kitchensink (all mechanics ON) / ablation (no predators). Empirical verdict on whether bodies grow
//! past germ_threshold(5) so soma appears and the DOL bootstrap succeeds.

use cli::{build_sim, driver_config, kitchensink_config};
use sim_core::SimConfig;

const EMERGE_FLOOR: i64 = 128; // ×BODY_SIZE_SCALE(256) == 50%
const MARGIN: i64 = 2;
const SEED_MAJORITY: usize = 3;
const POP_FLOOR: i64 = 10;
const FIXED_REFUGE_K_HAZARD: i32 = 128;
const BASE_HAZARD_SWEEP: [i64; 4] = [10, 20, 30, 45];
const VERDICT_SEEDS: [u64; 5] = [1, 2, 3, 4, 5];

// DL-C-specific threshold: does soma appear (modal_germ ≥ 2)?
const GERM_INTERMEDIATE_MIN: i64 = 2; // Modal germ ≥ 2 (not germ=1 pinning)

/// Extended result for DL-C verdict: includes germ:soma metrics.
struct DlcArmResult {
    frac: i64,
    mean_pop: f64,
    mean_body_size: f64,
    body_size_drift: f64,
    mean_germ_frac: f64,      // mean(germ_cells / total_cells)
    modal_germ: i64,           // most common germ_cell count in window
    collapsed: bool,
}

fn run_dlc_arm(
    seed: u64,
    ticks: u64,
    window_start: u64,
    cfg_builder: impl Fn(u64) -> SimConfig,
    predators_on: bool,
    refuge_k: i32,
    base_hazard: i64,
) -> DlcArmResult {
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
    let mut germ_frac_sum: f64 = 0.0;
    let mut germ_frac_count: i64 = 0;
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
                    let frac = germ_cells as f64 / total_cells as f64;
                    germ_frac_sum += frac;
                    germ_frac_count += 1;
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
    let mean_germ_frac = if germ_frac_count > 0 { germ_frac_sum / germ_frac_count as f64 } else { 0.0 };

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
        DlcArmResult {
            frac: 0,
            mean_pop,
            mean_body_size,
            body_size_drift: drift,
            mean_germ_frac,
            modal_germ,
            collapsed: true,
        }
    } else {
        DlcArmResult {
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

/// DL-C (#345): composition verdict harness. Tests whether the ECOLOGY of D-5 + settling +
/// O₂-hypoxia + DOL composes to a robust multicellular transition. Three arms per (seed, base_hazard):
/// (1) D-5 baseline (flag-off), (2) kitchensink (all mechanics ON), (3) ablation (no predators).
/// Verdict logic (EMPIRICAL — not pre-declared): composition reaches EMERGENCE if kitchensink
/// achieves a robust MAJORITY that exceeds D-5 baseline AND bodies grow past germ_threshold
/// (modal_germ ≥ 2, so soma appears, not germ=1 pinning). Honest NULL if composition insufficient.
#[test]
#[ignore]
fn kitchensink_verdict() {
    let ticks: u64 = std::env::var("DLC_VERDICT_TICKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(400);
    let window_len = ticks.min(1000);
    let window_start = ticks - window_len;

    println!("\nDL-C (#345) composition verdict: D-5 + settling + O₂-hypoxia + DOL");
    println!(
        "PRE-DECLARED: EMERGE_FLOOR={:.0}%, MARGIN={MARGIN}x, SEED_MAJORITY={SEED_MAJORITY}/5, POP_FLOOR={POP_FLOOR}",
        EMERGE_FLOOR as f64 / sim_core::BODY_SIZE_SCALE as f64 * 100.0
    );
    println!(
        "ticks={ticks}  late-window=[{window_start},{ticks}]  fixed refuge_k={FIXED_REFUGE_K_HAZARD}  base_hazard sweep={:?}",
        BASE_HAZARD_SWEEP
    );
    println!(
        "GERM:SOMA thresholds: modal_germ≥{} (soma appears, not germ=1 pinning)",
        GERM_INTERMEDIATE_MIN
    );

    // Ablation is independent of base_hazard
    let ablation: Vec<DlcArmResult> = VERDICT_SEEDS
        .iter()
        .map(|&seed| run_dlc_arm(seed, ticks, window_start, kitchensink_config, false, 0, 0))
        .collect();

    let mut any_regime_emerges = false;

    for &bh in &BASE_HAZARD_SWEEP {
        println!("\n{}", "-".repeat(100));
        println!("base_hazard={bh}");
        println!(
            "{:<6} {:>12} {:>14} {:>12} {:>12} {:>10} {:>11} {:>8} {:>6}",
            "seed", "D-5%", "kitchensink%", "ablation%", "size_cells", "drift", "germ_frac", "modal_g", "result"
        );

        let mut seed_d5_baseline_pass = 0usize;
        let mut seed_kitchen_pass = 0usize;
        let mut regime_has_nondegenerate_germ = false;

        for (i, &seed) in VERDICT_SEEDS.iter().enumerate() {
            let d5_baseline = run_dlc_arm(seed, ticks, window_start, driver_config, true, FIXED_REFUGE_K_HAZARD, bh);
            let kitchen = run_dlc_arm(seed, ticks, window_start, kitchensink_config, true, FIXED_REFUGE_K_HAZARD, bh);
            let abl = &ablation[i];

            // Verdict conditions (a/b)
            let (a_ok, b_ok);
            let d5_pct = d5_baseline.frac as f64 / sim_core::BODY_SIZE_SCALE as f64 * 100.0;
            let kitchen_pct = kitchen.frac as f64 / sim_core::BODY_SIZE_SCALE as f64 * 100.0;
            let abl_pct = abl.frac as f64 / sim_core::BODY_SIZE_SCALE as f64 * 100.0;

            // D-5 baseline pass check (same emergence conditions as kitchen)
            let d5_floor_ok = !d5_baseline.collapsed && d5_baseline.frac >= EMERGE_FLOOR;
            let d5_margin_abl_ok = !d5_baseline.collapsed && !abl.collapsed && d5_baseline.frac >= MARGIN * abl.frac;
            let d5_baseline_pass = d5_floor_ok && d5_margin_abl_ok;
            if d5_baseline_pass {
                seed_d5_baseline_pass += 1;
            }

            // (a) kitchensink passes emergence gates
            let floor_ok = !kitchen.collapsed && kitchen.frac >= EMERGE_FLOOR;
            let margin_abl_ok = !kitchen.collapsed && !abl.collapsed && kitchen.frac >= MARGIN * abl.frac;
            a_ok = floor_ok && margin_abl_ok;

            // (b) germ:soma intermediate non-degenerate (modal_germ ≥ 2, soma appears)
            b_ok = kitchen.modal_germ >= GERM_INTERMEDIATE_MIN;

            let pass = a_ok && b_ok;
            if pass {
                seed_kitchen_pass += 1;
            }
            if pass && kitchen.modal_germ >= GERM_INTERMEDIATE_MIN {
                regime_has_nondegenerate_germ = true;
            }

            let body_size_cells = kitchen.mean_body_size / sim_core::BODY_SIZE_SCALE as f64;
            let germ_frac_pct = kitchen.mean_germ_frac * 100.0;
            let tag = if kitchen.collapsed {
                "COLLAPSED"
            } else if pass {
                "PASS"
            } else {
                "fail"
            };

            println!(
                "{:<6} {:>11.1}% {:>13.1}% {:>11.1}% {:>9.1} {:>7.2} {:>7.1}% {:>8} {:>8}",
                seed, d5_pct, kitchen_pct, abl_pct, body_size_cells, kitchen.body_size_drift / sim_core::BODY_SIZE_SCALE as f64,
                germ_frac_pct, kitchen.modal_germ, tag
            );

            // Diagnostic notes on condition failures
            if !pass && !kitchen.collapsed {
                if !a_ok {
                    println!("       (a) kitchen emergence: {:.1}% ≥ {:.0}% floor? {} | ≥ {MARGIN}×ablation? {}",
                        kitchen_pct, EMERGE_FLOOR as f64 / sim_core::BODY_SIZE_SCALE as f64 * 100.0, floor_ok, margin_abl_ok);
                }
                if !b_ok {
                    println!("       (b) germ intermediate: modal_germ={} ≥ {}? {}", kitchen.modal_germ, GERM_INTERMEDIATE_MIN, b_ok);
                }
            }
        }

        println!("  D-5 baseline seeds passing faithfulness: {seed_d5_baseline_pass}/5 (marginal ~60%)");
        println!("  kitchensink seeds passing all 2 conditions (a/b): {seed_kitchen_pass}/5 (need ≥{SEED_MAJORITY})");

        // Verdict condition: kitchensink reaches SEED_MAJORITY AND exceeds D-5 baseline (robustness lift)
        if seed_kitchen_pass >= SEED_MAJORITY && seed_kitchen_pass > seed_d5_baseline_pass {
            any_regime_emerges = true;
            println!("  ✓ Regime supports composition emergence (a/b both pass & robustness lift over D-5)");
        } else {
            println!("  ✗ Regime does NOT support composition emergence");
        }
    }

    println!("\n{}", "-".repeat(100));
    println!();
    if any_regime_emerges {
        println!("DL-C-VERDICT: EMERGENCE");
        println!("  At some base_hazard, kitchensink reaches SEED_MAJORITY={SEED_MAJORITY}/5 on all 2 conditions:");
        println!("  (a) multicellular_frac ≥ {:.0}% AND ≥{MARGIN}x ablation",
            EMERGE_FLOOR as f64 / sim_core::BODY_SIZE_SCALE as f64 * 100.0);
        println!("  (b) germ:soma intermediate non-degenerate (modal_germ ≥ {GERM_INTERMEDIATE_MIN}, soma appears)");
        println!("  ⇒ Composition of D-5 + settling + O₂ + DOL robustly drives multicellularity ABOVE D-5 marginal (~60%).");
    } else {
        println!("DL-C-VERDICT: NULL");
        println!("  No base_hazard regime reached SEED_MAJORITY={SEED_MAJORITY}/5 on all 2 conditions.");
        println!("  Interpreted: composition of existing mechanics is insufficient for robust soma appearance.");
        println!("  The multi-mechanism thesis (complexity from ecology, not single driver) needs NEW substrate.");
    }

    // Never panic on biological outcome — NULL is an honest result
}
