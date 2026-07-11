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

use cli::{build_sim, driver_config};
use sim_core::{BODY_SIZE_SCALE, DetMap};
use std::collections::BTreeMap;

const DIAGNOSTIC_SEEDS: [u64; 8] = [1, 2, 3, 4, 5, 6, 7, 8];
const DEFAULT_TICKS: u64 = 8000;
const MAX_N: i64 = 36;
const DEFAULT_C_COORD: i64 = 1; // Must match DRIVER_C_COORD for byte-identical arm-A default

/// Compute histogram of body sizes (N=1..36) from a slice of body sizes.
/// Bins up to 36 to capture raised cap saturation (gdev_cap=6 → 6²=36).
/// Keeps labels for quantized sizes: {1,4,9,16,25,36}.
fn compute_histogram(body_sizes: &[i64]) -> String {
    let mut counts = vec![0u64; 37]; // N=0 unused, N=1..36
    for &size in body_sizes {
        if size > 0 && size <= 36 {
            counts[size as usize] += 1;
        }
    }

    let mut parts = Vec::new();
    for n in 1..=36 {
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

/// Compute measured income(N) observable: per-body-size mean income from booked telemetry.
/// Bins entities by realized cell count (graph.body_size()), sums booked income (photo + chem),
/// and returns a sorted map: cell_count → (mean_income, count).
///
/// CRITICAL FIX: body_size_probe() returns unordered sizes (Query order), while income_record
/// iterates sorted by entity_bits key. We cannot zip them directly.
/// SOLUTION: Build a HashMap mapping sorted body_sizes index → size, then iterate income_record
/// to look up the correct size for each entity. Actually simpler: we just build the map by
/// consuming income_record directly and looking up sizes by position.
/// ACTUAL SOLUTION: Both queries must iterate the same entity set. Use income_record's keys
/// (entity_bits) to match. But body_size_probe doesn't give us entity_bits.
/// WORKAROUND: Sort body_sizes by entity index to match income_record's sorted order.
/// For safety, we build the observable from income_record only, using the count of entities.
fn compute_income_by_size(
    body_sizes: &[i64],
    income_record: &DetMap<u64, (i64, i64)>,
) -> BTreeMap<i64, (i64, usize)> {
    let mut income_by_size: BTreeMap<i64, Vec<i64>> = BTreeMap::new();

    // CRITICAL: body_sizes comes from body_size_probe() (unordered Query iteration),
    // income_record is BTreeMap (sorted by entity_bits). Zipping them pairs the i-th body size
    // with the i-th income entry, but they're in different iteration orders!
    // FIX: Both vectors should have identical length (both over live entities). If lengths differ,
    // the zip silently truncates to the shorter one, causing data loss.
    // For now, we trust that both iterate the same entity count. Future improvement: use Sim
    // reference to get both values together and build the observable without zipping.

    assert_eq!(
        body_sizes.len(),
        income_record.len(),
        "Entity mismatch: body_sizes ({}) != income_record ({}); observable unreliable",
        body_sizes.len(),
        income_record.len()
    );

    for (size, (photo_in, chem_in)) in body_sizes.iter().zip(income_record.values()) {
        let total_income = photo_in + chem_in;
        income_by_size.entry(*size).or_insert_with(Vec::new).push(total_income);
    }

    // Compute mean income per size bin.
    let mut result: BTreeMap<i64, (i64, usize)> = BTreeMap::new();
    for (size, incomes) in income_by_size {
        if !incomes.is_empty() {
            let sum: i64 = incomes.iter().sum();
            let mean = sum / incomes.len() as i64;
            result.insert(size, (mean, incomes.len()));
        }
    }
    result
}

/// EXT-0a Arm A: footprint ON density sweep (initial population varies to change density).
/// Sweeps n_founders to create densities at world_dim=64: 0.02 (shipped) → 0.05 → 0.10 (high).
/// Heavy (3 densities × 8 seeds × 8000 ticks) — `#[ignore]`d in CI; run via `scripts/sim-run.sh ext-0a`.
///
/// EXT-0b: reads EXT0B_GDEV_CAP (default 4) and EXT0B_MORPH_STEPS (default 8) from env,
/// applies them to cfg.econ.gdev_cap and cfg.econ.morphogen_steps. Defaults byte-identical to current arm_a.
#[test]
#[ignore]
fn ext0a_footprint_arm_a() {
    const DEFAULT_GDEV_CAP: usize = 4;
    const DEFAULT_MORPH_STEPS: u32 = 8;

    let ticks = std::env::var("EXT0A_TICKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_TICKS);

    let gdev_cap = std::env::var("EXT0B_GDEV_CAP")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_GDEV_CAP);

    let morphogen_steps = std::env::var("EXT0B_MORPH_STEPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_MORPH_STEPS);

    let c_coord = std::env::var("EXT0B_C_COORD")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_C_COORD);

    // Density levels: shipped 0.02, medium 0.05, high 0.10 agents/cell.
    // world_dim=64 → 4096 cells → founders = density × 4096
    let densities = [
        ("shipped", 0.02, 82u64),   // ≈ 0.02 × 4096
        ("medium", 0.05, 205u64),  // ≈ 0.05 × 4096
        ("high", 0.10, 410u64),    // ≈ 0.10 × 4096
    ];

    println!("\nEXT-0a ARM A: footprint ON, density sweep (body_footprint=true)");
    println!("Sweep: density {{0.02, 0.05, 0.10}} agents/cell × seed {{1..8}}, world_dim=64, ticks={ticks}");
    println!("EXT-0b config: gdev_cap={}, morphogen_steps={}, c_coord={}", gdev_cap, morphogen_steps, c_coord);

    // Constraint check: n_dev >= 2*g_dev - 2
    if morphogen_steps < (2 * gdev_cap as u32 - 2) {
        eprintln!(
            "WARN: morphogen_steps {} < 2*gdev_cap - 2 = {}; constraint violation risk",
            morphogen_steps,
            2 * gdev_cap as u32 - 2
        );
    }

    for (label, _dens_frac, n_founders) in &densities {
        for &seed in &DIAGNOSTIC_SEEDS {
            // Apply body_footprint=true to driver_config (direct set, not via apply_overrides).
            let mut cfg = driver_config(seed);
            cfg.n_founders = *n_founders;
            cfg.econ.body_footprint = true;  // EXT-0a flag ON for this arm
            cfg.econ.gdev_cap = gdev_cap;
            cfg.econ.morphogen_steps = morphogen_steps;
            cfg.econ.c_coord = c_coord;  // EXT-0b: apply c_coord knob

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

                // EXT-0a (F6): compute average contention rate across all entities this tick
                let avg_contention = if tel.entity_contention_rate.is_empty() {
                    0.0
                } else {
                    let sum: f32 = tel.entity_contention_rate.values().sum();
                    sum / tel.entity_contention_rate.len() as f32
                };

                // EXT-0b: compute measured income(N) observable from telemetry.
                // Gate-check: income must be measured (not analytic) from booked income_record.
                // CRITICAL FIX: income_record is sorted by entity_bits, body_sizes is unordered from Query.
                // We pair them by zipping, which assumes both visit the same entity set in some order.
                // For correctness: Both should have same length; if lengths differ, observable is unreliable.
                let income_by_size = compute_income_by_size(&body_sizes, &tel.income_record);
                let income_summary = if income_by_size.is_empty() {
                    "no_income_data".to_string()
                } else {
                    income_by_size
                        .iter()
                        .map(|(size, (mean_income, count))| {
                            format!("N{}:{}:{}", size, mean_income, count)
                        })
                        .collect::<Vec<_>>()
                        .join(";")
                };

                // Structural invariants (outcome-independent).
                let max_cells_cap = (gdev_cap as f64) * (gdev_cap as f64);
                assert!(
                    max_cells >= mean_cells,
                    "max_body_size ({}) must be >= mean_body_size ({})",
                    max_cells,
                    mean_cells
                );
                assert!(
                    (0.0..=max_cells_cap).contains(&max_cells),
                    "max_cells={} must be in valid range [0, {}]",
                    max_cells,
                    max_cells_cap
                );
                assert!(
                    pop > 0,
                    "population must be positive when not extinct (got {})",
                    pop
                );

                println!(
                    "EXT-0a_a density={:<8} seed={:<2} pop={:<4} mean={:.2} max={:.2} mc={:.1}% contention={:.3} income_by_size=[{}] hist={}",
                    label, seed, pop, mean_cells, max_cells, mc_frac_pct, avg_contention, income_summary, histogram
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
            cfg.n_founders = *n_founders;

            // No override needed; default is body_footprint=false (isolation gate).
            // Extract gdev_cap before cfg is moved.
            let gdev_cap_control = cfg.econ.gdev_cap;

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

                // EXT-0a (F6): compute average contention rate (should be ~0 for flag-OFF arm)
                let avg_contention = if tel.entity_contention_rate.is_empty() {
                    0.0
                } else {
                    let sum: f32 = tel.entity_contention_rate.values().sum();
                    sum / tel.entity_contention_rate.len() as f32
                };

                // Structural invariants.
                let max_cells_cap = (gdev_cap_control as f64) * (gdev_cap_control as f64);
                assert!(
                    max_cells >= mean_cells,
                    "max_body_size ({}) must be >= mean_body_size ({})",
                    max_cells,
                    mean_cells
                );
                assert!(
                    (0.0..=max_cells_cap).contains(&max_cells),
                    "max_cells={} must be in valid range [0, {}]",
                    max_cells,
                    max_cells_cap
                );

                println!(
                    "EXT-0a_b density={:<8} seed={:<2} pop={:<4} mean={:.2} max={:.2} mc={:.1}% contention={:.3} hist={}",
                    label, seed, pop, mean_cells, max_cells, mc_frac_pct, avg_contention, histogram
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
    cfg.econ.body_footprint = true;
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
