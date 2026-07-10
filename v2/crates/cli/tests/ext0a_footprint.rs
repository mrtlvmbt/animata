//! EXT-0a: footprint harvest diagnostic — measure income growth with body size across density levels.
//!
//! Question: does non-lethal body-size income gradient emerge from spatial footprint harvest and
//! between-body contention? The field's per-cell income must be linear in isolation but become
//! concave under contention (finite cell resource shared by multiple bodies).
//!
//! Mechanism: body occupies side² cells (side=g_dev.max(1)), harvests from each independently.
//! At sparse density (shipped ~0.02-0.03 agents/cell), contention is rare → gradient weak/absent.
//! At higher density, contention strong → optimum interior and smaller N* than at lower density.
//!
//! GOLDEN-NEUTRAL: flag OFF byte-identical to main. Sweep uses driver_config + body_footprint=true overrides.
//! NO size-reward constant, no base_hazard bump — income is Σ R_cell over footprint cells, nothing more.
//! A NULL result (bodies pile at cap regardless of density) is valid → indicates no contention drove size selection.
//!
//! **Arm A:** density sweep (vary initial population n_founders), fixed world_dim=64.
//! - Densities (agents/cell): 0.02 (shipped), 0.05, 0.10 (HIGH).
//! - Each density: seeds 1..8, ticks=8000.
//! - Emits: R̄, mean_cells, max_cells, mc_frac, body-size histogram per (density, seed).
//!
//! **Arm B (control):** flag OFF at the same densities as Arm A.
//! - Same sweep structure, body_footprint=false (should reproduce D5-DRIFT's N*≈3.3 at shipped density).
//!
//! No PASS/FAIL verdict; PM interprets density response of N* to diagnose mechanism.
//! Conservation assertion: footprint cells must sum to zero residual (R15).

use cli::{apply_overrides, build_sim, driver_config};
use sim_core::BODY_SIZE_SCALE;

const DIAGNOSTIC_SEEDS: [u64; 8] = [1, 2, 3, 4, 5, 6, 7, 8];
const DEFAULT_TICKS: u64 = 8000;
const MAX_N: i64 = 16;

/// Compute histogram of body sizes (N=1..16) from a slice of body sizes.
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

/// EXT-0a Arm A: footprint ON density sweep (initial population varies to change density).
/// Sweeps n_founders to create densities at world_dim=64: 0.02 (shipped) → 0.05 → 0.10 (high).
/// Heavy (3 densities × 8 seeds × 8000 ticks) — `#[ignore]`d in CI; run via `scripts/sim-run.sh ext-0a`.
#[test]
#[ignore]
fn ext0a_footprint_arm_a() {
    let ticks = std::env::var("EXT0A_TICKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_TICKS);

    // Density levels: shipped 0.02, medium 0.05, high 0.10 agents/cell.
    // world_dim=64 → 4096 cells → founders = density × 4096
    let densities = [
        ("shipped", 0.02, 82u64),   // ≈ 0.02 × 4096
        ("medium", 0.05, 205u64),  // ≈ 0.05 × 4096
        ("high", 0.10, 410u64),    // ≈ 0.10 × 4096
    ];

    println!("\nEXT-0a ARM A: footprint ON, density sweep (body_footprint=true)");
    println!("Sweep: density {{0.02, 0.05, 0.10}} agents/cell × seed {{1..8}}, world_dim=64, ticks={ticks}");

    for (label, _dens_frac, n_founders) in &densities {
        for &seed in &DIAGNOSTIC_SEEDS {
            // Apply body_footprint=true override to driver_config.
            let mut cfg = driver_config(seed);
            let mut econ = cfg.econ.clone();
            cfg.n_founders = n_founders;

            apply_overrides(&mut econ, &[("body_footprint".to_string(), "true".to_string())])
                .expect("body_footprint override must be valid");
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
                // Extinction: emit special report (exclude from aggregates).
                let histogram = compute_histogram(&[]);
                println!(
                    "EXT-0a_a density={:<8} seed={:<2} pop=0 EXTINCT hist={}",
                    label, seed, histogram
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
                assert!(
                    pop > 0,
                    "population must be positive when not extinct (got {})",
                    pop
                );

                println!(
                    "EXT-0a_a density={:<8} seed={:<2} pop={:<4} mean={:.2} max={:.2} mc={:.1}% hist={}",
                    label, seed, pop, mean_cells, max_cells, mc_frac_pct, histogram
                );
            }
        }
    }

    println!("EXT-0a ARM A complete. PM interprets density-dependent N* shift (→ contention signal).");
}

/// EXT-0a Arm B (control): footprint OFF at the same densities as Arm A.
/// Should reproduce D5-DRIFT's drift centre N*≈3.3 at shipped density (no footprint income gradient).
/// Same structure as Arm A; body_footprint=false (default, byte-identical to main).
#[test]
#[ignore]
fn ext0a_footprint_arm_b_control() {
    let ticks = std::env::var("EXT0A_TICKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_TICKS);

    // Same densities as Arm A.
    let densities = [
        ("shipped", 0.02, 82u64),
        ("medium", 0.05, 205u64),
        ("high", 0.10, 410u64),
    ];

    println!("\nEXT-0a ARM B (CONTROL): footprint OFF, density sweep (body_footprint=false)");
    println!("Sweep: density {{0.02, 0.05, 0.10}} agents/cell × seed {{1..8}}, world_dim=64, ticks={ticks}");
    println!("Expected: N* ≈ 3.3 at all densities (no income gradient; D5-DRIFT baseline)");

    for (label, _dens_frac, n_founders) in &densities {
        for &seed in &DIAGNOSTIC_SEEDS {
            // Driver config with body_footprint=false (explicit default, byte-identical).
            let mut cfg = driver_config(seed);
            cfg.n_founders = n_founders;

            // No override needed; default is body_footprint=false (isolation gate).

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
                let histogram = compute_histogram(&[]);
                println!(
                    "EXT-0a_b density={:<8} seed={:<2} pop=0 EXTINCT hist={}",
                    label, seed, histogram
                );
            } else {
                let mean_cells = tel.mean_body_size as f64 / BODY_SIZE_SCALE as f64;
                let max_cells = tel.max_body_size as f64;
                let mc_frac_pct = tel.multicellular_frac as f64 / BODY_SIZE_SCALE as f64 * 100.0;
                let histogram = compute_histogram(&body_sizes);

                // Structural invariants.
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
                    "EXT-0a_b density={:<8} seed={:<2} pop={:<4} mean={:.2} max={:.2} mc={:.1}% hist={}",
                    label, seed, pop, mean_cells, max_cells, mc_frac_pct, histogram
                );
            }
        }
    }

    println!("EXT-0a ARM B complete. Comparison to Arm A reveals footprint effect (or absence).");
}

/// EXT-0a conservation test: footprint must conserve energy (R15).
/// A multicellular body's side² footprint contestants, when applied, must have zero residual:
/// Σ conserved_take(cell_i) == Σ gained + Σ lost (dissipated).
/// This test is compact (1 seed, 100 ticks) — check ON a multicellular body, OFF control.
#[test]
fn ext0a_footprint_conservation() {
    println!("\nEXT-0a CONSERVATION TEST");
    println!("Verify: footprint harvest does not leak/create energy (R15 residual = 0).");

    // Build a sim with footprint ON on a simple driver config.
    let mut cfg = driver_config(1);
    let mut econ = cfg.econ.clone();
    apply_overrides(&mut econ, &[("body_footprint".to_string(), "true".to_string())])
        .expect("body_footprint override must be valid");
    cfg.econ = econ;
    cfg.n_founders = 10;

    let mut sim = build_sim(cfg);

    // Let sim run for 100 ticks (enough for multicellular bodies to evolve).
    for _ in 0..100 {
        sim.step();
    }

    // Read final conservation residual.
    let residual = sim.conservation_residual();

    // R15: residual must be EXACTLY 0 (integer conservation, no float error).
    println!("Conservation residual (footprint ON): {} eu", residual);
    assert_eq!(residual, 0, "Footprint must conserve energy (residual must be 0)");

    // Control: OFF should also conserve.
    let mut cfg_off = driver_config(1);
    cfg_off.n_founders = 10;

    let mut sim_off = build_sim(cfg_off);
    for _ in 0..100 {
        sim_off.step();
    }

    let residual_off = sim_off.conservation_residual();
    println!("Conservation residual (footprint OFF): {} eu", residual_off);
    assert_eq!(residual_off, 0, "Baseline must conserve energy (residual must be 0)");

    println!("✓ Conservation OK: both ON and OFF maintain R15.");
}
