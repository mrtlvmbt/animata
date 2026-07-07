//! DC-DIAG: size-ceiling diagnostic — measure equilibrium body size under the strongest existing
//! size-driver (D-5 hazard-refuge predation) across a range of hazard intensities.
//!
//! Question: is equilibrium body size limited by the structural cap (g_dev ≤ 4 → max 16 cells) or by
//! the size-selection driver (weak fitness gradient)? A pure DIAGNOSTIC test-only sweep:
//! - NO new mechanics, NO new EconParams field, NO config change → golden-neutral.
//! - Reuses existing `driver_config` (D-5 hazard-refuge), applying `--set base_hazard=` overrides.
//! - Sweep: base_hazard ∈ {10, 30, 100, 300, 1000} × seed ∈ {1, 2, 3, 4, 5}, ticks=8000.
//! - Record: mean_body_size, max_body_size, multicellular_frac from Telemetry at horizon.
//! - Emit: one descriptive line per (base_hazard, seed) → `mean_cells, max_cells, mc_frac`.
//! - NO PASS/FAIL verdict on outcomes; analysis is PM's. Only structural invariants asserted.

use cli::{apply_overrides, build_sim, driver_config};
use sim_core::BODY_SIZE_SCALE;

const BASE_HAZARD_SWEEP: [i64; 5] = [10, 30, 100, 300, 1000];
const DIAGNOSTIC_SEEDS: [u64; 5] = [1, 2, 3, 4, 5];
const DEFAULT_TICKS: u64 = 8000;

/// DC-DIAG size-ceiling sweep: measure `max_body_size` across hazard intensities.
/// Heavy (5 hazards × 5 seeds × 8000 ticks) — `#[ignore]`d in CI; run via
/// `scripts/sim-run.sh dc-diag` (once this PR merges to main).
#[test]
#[ignore]
fn dc_diag_size_ceiling() {
    let ticks = std::env::var("DC_DIAG_TICKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_TICKS);

    println!("\nDC-DIAG: size-ceiling diagnostic (driver_config D-5 hazard-refuge)");
    println!(
        "Sweep: base_hazard ∈ {:?} × seed ∈ {:?}, ticks={ticks}",
        BASE_HAZARD_SWEEP, DIAGNOSTIC_SEEDS
    );
    println!("{}", "-".repeat(78));
    println!(
        "{:<6} {:>10} {:>12} {:>12} {:>10}",
        "seed", "base_hazard", "mean_cells", "max_cells", "mc_frac%"
    );
    println!("{}", "-".repeat(78));

    for &bh in &BASE_HAZARD_SWEEP {
        for &seed in &DIAGNOSTIC_SEEDS {
            // Apply base_hazard override to driver_config.
            let mut cfg = driver_config(seed);
            let mut econ = cfg.econ.clone();
            apply_overrides(&mut econ, &[("base_hazard".to_string(), bh.to_string())])
                .expect("base_hazard override must be valid");
            cfg.econ = econ;

            // Run simulation to horizon.
            let mut sim = build_sim(cfg);
            for _ in 0..ticks {
                sim.step();
            }

            // Read telemetry at horizon.
            let tel = sim.telemetry();
            let mean_cells = tel.mean_body_size as f64 / BODY_SIZE_SCALE as f64;
            // NOTE: max_body_size is a RAW cell count (sim-core/lib.rs:162), NOT ×BODY_SIZE_SCALE —
            // unlike mean_body_size / multicellular_frac which ARE fixed-point ×SCALE. Do not divide.
            let max_cells = tel.max_body_size as f64;
            let mc_frac_pct = tel.multicellular_frac as f64 / BODY_SIZE_SCALE as f64 * 100.0;
            let pop = tel.population;

            // Structural invariants (outcome-independent): sim runs to horizon without panic,
            // body size stats are self-consistent.
            assert!(
                max_cells >= mean_cells,
                "max_body_size ({}) must be >= mean_body_size ({})",
                max_cells,
                mean_cells
            );
            assert!(
                pop >= 0,
                "population must be non-negative (got {})",
                pop
            );
            assert!(
                (0.0..=32.0).contains(&max_cells),
                "max_cells={} must be in valid range [0, 32]",
                max_cells
            );

            // Emit descriptive map line (no pass/fail).
            println!(
                "DC-DIAG bh={:<4} seed={:<2} mean_cells={:>7.2} max_cells={:>7.2} mc_frac={:>6.1}%",
                bh, seed, mean_cells, max_cells, mc_frac_pct
            );
        }
    }

    println!("{}", "-".repeat(78));
    println!("DC-DIAG map complete. Analysis is PM's; no outcome verdict here.");
}
