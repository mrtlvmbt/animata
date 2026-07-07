//! DR-0 (#347): economy-coupled division-of-labor bootstrapping diagnostic.
//! Tests the redesign's core precondition: does the economy-coupled mechanic (income∝soma,
//! germ=flat fertility) bootstrap multi-soma bodies from a unicellular founder under the D-5 hazard
//! economy? Answers two questions:
//! (Q-boot) Founder survival: run dr0_config at VERDICT_SEEDS [1..5] with resource-rich start,
//!          track founder lineage survival to first reproduction by ~tick 100-200.
//! (Q-grow) Soma growth: over a late window (ticks 2000-4000), report mean soma_cells, mean germ_cells,
//!          modal germ, mean body_size, multicellular_frac. Compare vs D-5 baseline PER-SEED.
//! Verdict: PROCEED iff founder-survival majority (≥3/5) AND seed-majority reaches soma≥2 with
//!          germ-minority AND multicellular_frac ≥ D-5 baseline (same seed). Else STOP.

use cli::{build_sim, dr0_config, driver_config};
use sim_core::SimConfig;

const VERDICT_SEEDS: [u64; 5] = [1, 2, 3, 4, 5];
const BOOT_WINDOW_START: u64 = 0;
const BOOT_WINDOW_END: u64 = 200;   // Track founder survival by tick 200
const GROW_WINDOW_START: u64 = 2000; // Late window: tick 2000-4000
const GROW_WINDOW_END: u64 = 4000;
const POP_FLOOR: i64 = 10;          // Population must reach 10+ to count as surviving

#[derive(Clone)]
struct Dr0Result {
    seed: u64,
    dr0_survived: bool,           // Did founder lineage survive to reproduction?
    dr0_mean_soma_cells: f64,
    dr0_mean_germ_cells: f64,
    dr0_mean_body_size: f64,
    dr0_modal_germ: i64,
    dr0_multicellular_frac: i64,  // ×BODY_SIZE_SCALE
    d5_multicellular_frac: i64,   // D-5 baseline at same seed, same window
}

/// Run DR-0 arm and D-5 baseline at the same seed (for per-seed comparison).
fn run_dr0_with_d5_baseline(seed: u64, ticks: u64) -> Dr0Result {
    // ── DR-0 arm (economy-coupled) ──
    let cfg = dr0_config(seed);
    let mut sim = build_sim(cfg);

    let mut founder_survived = false;
    let mut soma_cells_sum: f64 = 0.0;
    let mut germ_cells_sum: f64 = 0.0;
    let mut total_cells_sum: f64 = 0.0;
    let mut grow_window_count: usize = 0;
    let mut germ_histogram: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    let mut dr0_frac_sum: i64 = 0;
    let mut dr0_frac_count: usize = 0;

    for t in 0..ticks {
        sim.step();

        // Q-boot: founder survival to ~tick 200
        if t >= BOOT_WINDOW_START && t < BOOT_WINDOW_END {
            let tel = sim.telemetry();
            if tel.population >= POP_FLOOR {
                founder_survived = true;
            }
        }

        // Q-grow: soma growth metrics over late window
        if t >= GROW_WINDOW_START && t < GROW_WINDOW_END {
            let snapshot = sim.cellgraph_snapshot();
            let tel = sim.telemetry();

            // Accumulate snapshot data
            let mut window_soma: i64 = 0;
            let mut window_germ: i64 = 0;
            let mut window_total: i64 = 0;
            for (_n_modules, germ_cells, soma_cells, total_cells) in snapshot {
                window_soma += soma_cells;
                window_germ += germ_cells;
                window_total += total_cells;
                *germ_histogram.entry(germ_cells).or_insert(0) += 1;
            }

            soma_cells_sum += window_soma as f64;
            germ_cells_sum += window_germ as f64;
            total_cells_sum += window_total as f64;
            grow_window_count += 1;

            // Multicellular fraction (DR-0)
            if tel.population >= POP_FLOOR {
                dr0_frac_sum += tel.multicellular_frac;
                dr0_frac_count += 1;
            }
        }
    }

    let mean_soma = if grow_window_count > 0 {
        soma_cells_sum / grow_window_count as f64
    } else {
        0.0
    };
    let mean_germ = if grow_window_count > 0 {
        germ_cells_sum / grow_window_count as f64
    } else {
        0.0
    };
    let mean_total = if grow_window_count > 0 {
        total_cells_sum / grow_window_count as f64
    } else {
        0.0
    };
    let modal_germ = germ_histogram
        .iter()
        .max_by_key(|(_, count)| *count)
        .map(|(g, _)| *g)
        .unwrap_or(0);
    let dr0_multi_frac = if dr0_frac_count > 0 {
        dr0_frac_sum as i64 / dr0_frac_count as i64
    } else {
        0
    };

    // ── D-5 baseline arm (at the SAME seed) ──
    let d5_cfg = driver_config(seed);
    let mut d5_sim = build_sim(d5_cfg);
    let mut d5_frac_sum: i64 = 0;
    let mut d5_frac_count: usize = 0;

    for t in 0..ticks {
        d5_sim.step();
        if t >= GROW_WINDOW_START && t < GROW_WINDOW_END {
            let tel = d5_sim.telemetry();
            if tel.population >= POP_FLOOR {
                d5_frac_sum += tel.multicellular_frac;
                d5_frac_count += 1;
            }
        }
    }

    let d5_multi_frac = if d5_frac_count > 0 {
        d5_frac_sum as i64 / d5_frac_count as i64
    } else {
        0
    };

    Dr0Result {
        seed,
        dr0_survived: founder_survived,
        dr0_mean_soma_cells: mean_soma,
        dr0_mean_germ_cells: mean_germ,
        dr0_mean_body_size: mean_total,
        dr0_modal_germ: modal_germ,
        dr0_multicellular_frac: dr0_multi_frac,
        d5_multicellular_frac: d5_multi_frac,
    }
}

#[test]
#[ignore]
fn dr0_bootstrap_diag() {
    let ticks: u64 = std::env::var("DR0_TICKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(400);

    println!("\nDR-0 (#347) economy-coupled bootstrapping diagnostic");
    println!("Question (Q-boot): founder survival to reproduction by tick 200?");
    println!("Question (Q-grow): soma growth over late window [2000, 4000]?");
    println!("Per-seed D-5 baseline comparison (FIX 1: seed-i DR-0 vs seed-i D-5)");
    println!("ticks={ticks}");
    println!();

    // Q-boot: founder survival
    println!("Q-boot (founder survival):");
    println!("{:<6} {:>10}", "seed", "survived");
    let mut founder_survive_count = 0;
    let results: Vec<Dr0Result> = VERDICT_SEEDS
        .iter()
        .map(|&seed| {
            let result = run_dr0_with_d5_baseline(seed, ticks);
            if result.dr0_survived {
                founder_survive_count += 1;
            }
            println!("DR-0: {:>6} {:>10}", seed, if result.dr0_survived { "YES" } else { "no" });
            result
        })
        .collect();

    let founder_survival_pct = (founder_survive_count * 100) / VERDICT_SEEDS.len();
    println!(
        "DR-0: founder-survival majority: {}/{} = {}%",
        founder_survive_count,
        VERDICT_SEEDS.len(),
        founder_survival_pct
    );
    println!();

    // Q-grow: soma growth (per-seed comparison)
    println!("Q-grow (soma growth in late window + per-seed D-5 baseline):");
    println!(
        "{:<6} {:>12} {:>12} {:>12} {:>12} {:>10} {:>10} {:>10}",
        "seed", "soma_cells", "germ_cells", "modal_germ", "body_size", "DR0%", "D5%", "DR0>=D5"
    );

    let mut soma_ge_2_count = 0;
    let mut germ_minority_count = 0;
    let mut multi_frac_ge_baseline_count = 0;

    for result in &results {
        let soma_ge_2 = result.dr0_mean_soma_cells >= 2.0;
        let germ_minority = result.dr0_mean_germ_cells >= 1.0 && result.dr0_mean_germ_cells < result.dr0_mean_soma_cells;
        let multi_frac_ok = result.dr0_multicellular_frac >= result.d5_multicellular_frac;

        if soma_ge_2 {
            soma_ge_2_count += 1;
        }
        if germ_minority {
            germ_minority_count += 1;
        }
        if multi_frac_ok {
            multi_frac_ge_baseline_count += 1;
        }

        let body_size_cells = result.dr0_mean_body_size / sim_core::BODY_SIZE_SCALE as f64;
        let dr0_pct = result.dr0_multicellular_frac as f64 / sim_core::BODY_SIZE_SCALE as f64 * 100.0;
        let d5_pct = result.d5_multicellular_frac as f64 / sim_core::BODY_SIZE_SCALE as f64 * 100.0;
        let baseline_ok = if multi_frac_ok { "YES" } else { "no" };
        println!(
            "DR-0: {:>6} {:>12.2} {:>12.2} {:>12} {:>12.2} {:>9.0}% {:>9.0}% {:>10}",
            result.seed,
            result.dr0_mean_soma_cells,
            result.dr0_mean_germ_cells,
            result.dr0_modal_germ,
            body_size_cells,
            dr0_pct,
            d5_pct,
            baseline_ok
        );
    }

    println!();
    println!("GATE SUMMARY (per-seed evaluation):");
    println!("  founder-survival ≥3/5: {} ({})", if founder_survive_count >= 3 { "PASS" } else { "FAIL" }, founder_survive_count);
    println!("  soma≥2 ≥3/5: {} ({})", if soma_ge_2_count >= 3 { "PASS" } else { "FAIL" }, soma_ge_2_count);
    println!("  germ-minority ≥3/5: {} ({})", if germ_minority_count >= 3 { "PASS" } else { "FAIL" }, germ_minority_count);
    println!("  multicell(seed-i)≥baseline(seed-i) ≥3/5: {} ({})", if multi_frac_ge_baseline_count >= 3 { "PASS" } else { "FAIL" }, multi_frac_ge_baseline_count);
    println!();

    // Verdict
    let founder_majority = founder_survive_count >= 3;
    let soma_majority = soma_ge_2_count >= 3;
    let germ_minority_majority = germ_minority_count >= 3;
    let baseline_majority = multi_frac_ge_baseline_count >= 3;

    let verdict_proceed = founder_majority && soma_majority && germ_minority_majority && baseline_majority;

    if verdict_proceed {
        println!("DR-0-VERDICT: PROCEED");
    } else {
        let failed_gates: Vec<&str> = vec![
            if !founder_majority { "founder-survival" } else { "" },
            if !soma_majority { "soma≥2" } else { "" },
            if !germ_minority_majority { "germ-minority" } else { "" },
            if !baseline_majority { "multicell≥baseline(seed-i)" } else { "" },
        ]
        .into_iter()
        .filter(|g| !g.is_empty())
        .collect();
        println!("DR-0-VERDICT: STOP (failed: {})", failed_gates.join(", "));
    }
}
