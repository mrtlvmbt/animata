//! D5-DRIFT: is D-5's ~60% multicellularity drift across a neutral plateau, or selection on a gradient?
//!
//! Pre-registered diagnostic. The question is whether mean body size tracks the computed argmax of
//! net(N) = -drain(N) - c_coord*N, where:
//!   drain(N) = base_hazard * 2^shift / (2^shift + refuge_k * N)
//!   c_coord = 1 (shipped)
//!
//! At D-5 (k=128, base=10), net is **flat on [1,5]**, so its ~60% multicellularity is drift across
//! a neutral plateau, not selection strength. But a **size gradient does exist** at k=128 as base
//! rises — and **DC-DIAG swept base_hazard at k=2, where the refuge is inert**, so the grid cell
//! where a gradient could ever appear **has never been run**.
//!
//! **Arm A:** full grid: refuge_k ∈ {2, 32, 128} × base_hazard ∈ {10, 20, 40} × seeds 1..8.
//! Ticks: 8000. Report: per-cell pop@end, mean_cells, max_cells, mc_frac, body-size histogram (N=1..16),
//! and the computed argmax set (which N values maximize net(N)).
//!
//! **Arm B:** founder control — mspec.g_dev ∈ {1, 2} at k=128, base ∈ {10, 20}, seeds 1..8.
//! Ticks: 8000. Same report format. The two arms differ in **exactly one factor** (founder body size).
//!
//! **Aggregates conditional on survival:** extinct cells (pop@end == 0) are reported as extinct, never as mean=0.
//! A crash confounded with no-drive already cost this project one wrong landmark.
//!
//! **Golden-neutral:** no shipped config, constant or default may change. DRIVER_BASE_HAZARD and
//! DRIVER_REFUGE_K stay untouched. The 3 exact-golden tests must remain green.
//!
//! ### Pre-registered predictions (pin as a comment BEFORE the first run)
//!
//! | refuge_k | base_hazard | argmax | predicted mean | observed |
//! |---|---|---|---|---|
//! | 128 | 10 | {1..5} | ~3.0 | **3.3** (D-5 landmark) |
//! | 128 | 20 | {4,5} | ~4.5 | — |
//! | 128 | 40 | {7} | ~7.0 | — |
//! | 32 | 10 | {1} | ~1.0 | — |
//! | 32 | 20 | {3..7} | ~5.0 | — |
//! | 2 | 10 | {1} | ~1.0 | **1.2** (DC-DIAG) |

use cli::{apply_overrides, build_sim, driver_config};
use sim_core::BODY_SIZE_SCALE;

const DIAGNOSTIC_SEEDS: [u64; 8] = [1, 2, 3, 4, 5, 6, 7, 8];
const DEFAULT_TICKS: u64 = 8000;

/// Compute argmax of net(N) = -drain(N) - c_coord*N for a given refuge_k and base_hazard.
/// Returns a string representation of the set of N values that maximize net(N).
/// shift=11 (DRIVER_REFUGE_SHIFT from lib.rs).
fn compute_argmax_set(base_hazard: i64, refuge_k: i32) -> String {
    const SHIFT: u32 = 11;
    const C_COORD: i64 = 1;
    const MAX_N: i64 = 16;

    let mut max_net = i64::MIN;
    let mut argmax_set = Vec::new();

    for n in 1..=MAX_N {
        // drain(N) = base_hazard * 2^shift / (2^shift + refuge_k * N)
        let shift_val: i128 = (1i128) << SHIFT;
        let k = refuge_k as i128;
        let numer: i128 = (base_hazard as i128) * shift_val;
        let denom: i128 = shift_val + k * (n as i128);
        let denom = denom.max(1);
        let drain = (numer / denom) as i64;

        // net(N) = -drain - c_coord * N
        let net = -drain - C_COORD * n;

        if net > max_net {
            max_net = net;
            argmax_set.clear();
            argmax_set.push(n);
        } else if net == max_net {
            argmax_set.push(n);
        }
    }

    if argmax_set.is_empty() {
        "{1}".to_string()
    } else if argmax_set.len() == 1 {
        format!("{{{}}}", argmax_set[0])
    } else if argmax_set.len() > 1
        && argmax_set[argmax_set.len() - 1] - argmax_set[0] + 1 == argmax_set.len() as i64
    {
        // It's a contiguous range
        format!("{{{}..{}}}", argmax_set[0], argmax_set[argmax_set.len() - 1])
    } else {
        // Non-contiguous set
        let elems: Vec<String> = argmax_set.iter().map(|x| x.to_string()).collect();
        format!("{{{}}}", elems.join(","))
    }
}

/// Compute histogram of body sizes (N=1..16) from a slice of body sizes.
/// Returns a string showing the histogram.
fn compute_histogram(body_sizes: &[i64]) -> String {
    let mut counts = vec![0u64; 17]; // N=0 unused, N=1..16
    for &size in body_sizes {
        if size > 0 && size <= 16 {
            counts[size as usize] += 1;
        }
    }

    let mut parts = Vec::new();
    for n in 1..=16 {
        if counts[n] > 0 {
            parts.push(format!("{}:{}", n, counts[n]));
        }
    }

    if parts.is_empty() {
        "empty".to_string()
    } else {
        parts.join(",")
    }
}

/// D5-DRIFT Arm A: full grid diagnostic (refuge_k × base_hazard × seeds).
/// Heavy (3×3×8×8000 = 576K ticks) — `#[ignore]`d in CI; run via `scripts/sim-run.sh d5-drift`.
#[test]
#[ignore]
fn d5_drift_arm_a() {
    let ticks = std::env::var("D5_DRIFT_TICKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_TICKS);

    let refuge_ks = [2i32, 32, 128];
    let base_hazards = [10i64, 20, 40];

    println!("\nD5-DRIFT ARM A: full grid (driver_config D-5 hazard-refuge)");
    println!(
        "Sweep: refuge_k ∈ {:?} × base_hazard ∈ {:?} × seed ∈ {:?}, ticks={ticks}",
        refuge_ks, base_hazards, DIAGNOSTIC_SEEDS
    );

    for &k in &refuge_ks {
        for &base_hazard in &base_hazards {
            let argmax_str = compute_argmax_set(base_hazard, k);
            println!("Arm A (k={}, base={}) argmax={}", k, base_hazard, argmax_str);

            for &seed in &DIAGNOSTIC_SEEDS {
                // Apply refuge_k and base_hazard overrides to driver_config.
                let mut cfg = driver_config(seed);
                let mut econ = cfg.econ.clone();

                let overrides = vec![
                    ("refuge_k".to_string(), k.to_string()),
                    ("base_hazard".to_string(), base_hazard.to_string()),
                ];

                apply_overrides(&mut econ, &overrides)
                    .expect("refuge_k and base_hazard overrides must be valid");
                cfg.econ = econ;

                // Run simulation to horizon.
                let mut sim = build_sim(cfg);
                for _ in 0..ticks {
                    sim.step();
                }

                // Collect body sizes for histogram.
                let body_sizes = sim.body_size_probe();

                // Read telemetry at horizon.
                let tel = sim.telemetry();
                let pop = tel.population;

                if pop == 0 {
                    // Extinction: emit special report.
                    let histogram = compute_histogram(&[]);
                    println!(
                        "d5_drift a k={:<6} bh={:<4} seed={:<2} pop=0 EXTINCT argmax={} hist={}",
                        k, base_hazard, seed, argmax_str, histogram
                    );
                } else {
                    let mean_cells = tel.mean_body_size as f64 / BODY_SIZE_SCALE as f64;
                    let max_cells = tel.max_body_size as f64;
                    let mc_frac_pct = tel.multicellular_frac as f64 / BODY_SIZE_SCALE as f64 * 100.0;
                    let histogram = compute_histogram(&body_sizes);

                    // Structural invariants (outcome-independent).
                    assert!(
                        max_cells >= mean_cells,
                        "max_body_size ({}) must be >= mean_body_size ({})",
                        max_cells,
                        mean_cells
                    );
                    assert!(
                        (0.0..=32.0).contains(&max_cells),
                        "max_cells={} must be in valid range [0, 32]",
                        max_cells
                    );

                    println!(
                        "d5_drift a k={:<6} bh={:<4} seed={:<2} pop={:<6} mean={:>7.2} max={:>7.2} mc={:>6.1}% argmax={} hist={}",
                        k, base_hazard, seed, pop, mean_cells, max_cells, mc_frac_pct, argmax_str, histogram
                    );
                }
            }
        }
    }

    println!("{}", "-".repeat(150));
    println!("D5-DRIFT ARM A complete. Analysis is PM's; no outcome verdict here.");
}

/// D5-DRIFT Arm B: founder control (g_dev variation).
/// Heavy (2×2×8×8000 = 256K ticks) — `#[ignore]`d in CI; run via `scripts/sim-run.sh d5-drift`.
/// The two arms differ in exactly one factor: founder body size (g_dev ∈ {1, 2}).
#[test]
#[ignore]
fn d5_drift_arm_b() {
    let ticks = std::env::var("D5_DRIFT_TICKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_TICKS);

    let g_devs = [1usize, 2];
    let base_hazards = [10i64, 20];

    println!("\nD5-DRIFT ARM B: founder control (g_dev variation, k=128)");
    println!(
        "Sweep: g_dev ∈ {:?} × base_hazard ∈ {:?} × seed ∈ {:?}, ticks={ticks}, k=128 fixed",
        g_devs, base_hazards, DIAGNOSTIC_SEEDS
    );

    const K: i32 = 128;

    for &g_dev in &g_devs {
        for &base_hazard in &base_hazards {
            let argmax_str = compute_argmax_set(base_hazard, K);
            println!(
                "Arm B (g_dev={}, base={}, k=128) argmax={}",
                g_dev, base_hazard, argmax_str
            );

            for &seed in &DIAGNOSTIC_SEEDS {
                // Apply base_hazard override to driver_config and ALSO override g_dev.
                let mut cfg = driver_config(seed);
                let mut econ = cfg.econ.clone();

                let overrides =
                    vec![("base_hazard".to_string(), base_hazard.to_string())];

                apply_overrides(&mut econ, &overrides)
                    .expect("base_hazard override must be valid");
                cfg.econ = econ;

                // Override g_dev (founder body size) in the morphogen spec.
                if let Some(mspec) = cfg.econ.morphogen.as_mut() {
                    mspec.g_dev = g_dev;
                }

                // Run simulation to horizon.
                let mut sim = build_sim(cfg);
                for _ in 0..ticks {
                    sim.step();
                }

                // Collect body sizes for histogram.
                let body_sizes = sim.body_size_probe();

                // Read telemetry at horizon.
                let tel = sim.telemetry();
                let pop = tel.population;

                if pop == 0 {
                    // Extinction: emit special report.
                    let histogram = compute_histogram(&[]);
                    println!(
                        "d5_drift b g_dev={:<6} bh={:<4} seed={:<2} EXTINCT argmax={} hist={}",
                        g_dev, base_hazard, seed, argmax_str, histogram
                    );
                } else {
                    let mean_cells = tel.mean_body_size as f64 / BODY_SIZE_SCALE as f64;
                    let max_cells = tel.max_body_size as f64;
                    let mc_frac_pct = tel.multicellular_frac as f64 / BODY_SIZE_SCALE as f64 * 100.0;
                    let histogram = compute_histogram(&body_sizes);

                    // Structural invariants (outcome-independent).
                    assert!(
                        max_cells >= mean_cells,
                        "max_body_size ({}) must be >= mean_body_size ({})",
                        max_cells,
                        mean_cells
                    );
                    assert!(
                        (0.0..=32.0).contains(&max_cells),
                        "max_cells={} must be in valid range [0, 32]",
                        max_cells
                    );

                    println!(
                        "d5_drift b g_dev={:<6} bh={:<4} seed={:<2} mean={:>7.2} max={:>7.2} mc={:>6.1}% argmax={} hist={}",
                        g_dev, base_hazard, seed, mean_cells, max_cells, mc_frac_pct, argmax_str, histogram
                    );
                }
            }
        }
    }

    println!("{}", "-".repeat(150));
    println!("D5-DRIFT ARM B complete. Analysis is PM's; no outcome verdict here.");
}
