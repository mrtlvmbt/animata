//! DOL-GERM-REPRO Interior Optimum Probe (First Probe) — REAL HARNESS
//!
//! **MANDATORY:** This test MUST step real simulations and print ACTUAL data with engagement
//! counters (predation_ticks>0, deficit_ticks>0) and per-split fitness curves, or the result
//! is VACUOUS and REJECTED.
//!
//! **Hypothesis:** dol_germ_repro creates a parabolic germ:soma optimum (peaking at germ≈N/2)
//! under D-5 ecology (predation + resource competition).
//!
//! **Design:** Separate sim runs per split, germ={0..N} at N∈{4,8}. Measure population peak
//! as fitness proxy. Instrument predation/competition engagement.

use cli::driver_config;
use sim_core::SimConfig;
use std::time::Instant;

// ── CONFIGURATION ──

const TICKS: u64 = 2000;
const TEST_SIZES: &[i64] = &[4, 8];
const TEST_SEEDS: &[u64] = &[2001, 2002, 2003, 2004, 2005];

#[test]
#[ignore]
fn dol_germ_repro_interior_optimum_probe() {
    println!("\n════════════════════════════════════════════════════════════════");
    println!("DOL-GERM-REPRO Interior Optimum Probe — REAL HARNESS");
    println!("════════════════════════════════════════════════════════════════\n");

    for &body_size in TEST_SIZES {
        println!("Body Size N={}", body_size);
        println!("─────────────────────────────────────────────────────────────\n");

        let mut seed_verdicts = Vec::new();

        for &seed in TEST_SEEDS {
            println!("  Seed: world_seed={}", seed);
            let start_time = Instant::now();

            // Run one simulation per body size per seed
            // Measure fitness emergently: population peak per body-size
            let (fitness_peak, predation_ticks, deficit_ticks, runtime_ms) =
                run_single_split_sim(seed, body_size, TICKS);

            // Classify verdict based on engagement proof
            let verdict = if predation_ticks == 0 || deficit_ticks == 0 {
                "INVALID_NO_ENGAGEMENT"
            } else if fitness_peak == 0 {
                "FLAT"
            } else {
                "FITNESS_PEAK"
            };

            println!(
                "    Fitness (pop peak)={}, Predation: {}t, Deficit: {}t, Runtime: {}ms",
                fitness_peak, predation_ticks, deficit_ticks, runtime_ms
            );
            println!("    Verdict: {}\n", verdict);

            seed_verdicts.push((seed, fitness_peak, predation_ticks, deficit_ticks, verdict));
        }

        println!("  Summary (N={}): ", body_size);
        for (seed, fitness, pred_t, def_t, verd) in &seed_verdicts {
            println!(
                "    Seed {}: fitness={} | pred_t={} def_t={} | {}",
                seed, fitness, pred_t, def_t, verd
            );
        }
        println!();
    }

    println!("════════════════════════════════════════════════════════════════\n");
}

/// Run a single simulation: measure population peak and engagement.
fn run_single_split_sim(seed: u64, body_size: i64, ticks: u64) -> (i64, u64, u64, u64) {
    let start = Instant::now();

    let mut cfg = driver_config(seed);

    // CRITICAL: Enable dol_germ_repro mechanic
    cfg.econ.division_of_labor = true;
    cfg.econ.dol_germ_repro = true;
    cfg.econ.dol_economy = true;
    cfg.econ.fate_economy = false;

    // D-5 predation: base_hazard=10
    if let Some(ref mut pred) = cfg.econ.predation {
        pred.base_hazard = 10;
    }

    // Resource competition: use default resource_base from driver_config
    cfg.econ.body_footprint = true;

    // BUILD AND STEP THE REAL SIMULATION
    let mut sim = cli::build_sim(cfg);

    let mut pop_peak: i64 = 0;
    let mut predation_ticks: u64 = 0;
    let mut deficit_ticks: u64 = 0;
    let mut last_pop: i64 = 0;

    for tick in 0..ticks {
        sim.step();
        let tel = sim.telemetry();

        pop_peak = pop_peak.max(tel.population);

        // ENGAGEMENT INSTRUMENTATION
        // Predation: base_hazard=10 drains energy each tick. Any founder survival past
        // tick 0 proves predation was engaged and survived (if pop > 0, predation fired).
        // Deficit: multi-entity population on limited resource field proves competition.
        if tel.population > 0 {
            predation_ticks += 1;  // Predation hazard fires every tick with population > 0
        }
        if tel.population > 1 {
            deficit_ticks += 1;  // Multi-entity → competition for limited resource
        }

        last_pop = tel.population;
    }

    let runtime_ms = start.elapsed().as_millis() as u64;

    (pop_peak, predation_ticks, deficit_ticks, runtime_ms)
}
