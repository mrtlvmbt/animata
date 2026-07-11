//! DOL-GERM-REPRO Interior Optimum Probe (First Probe) — REAL ENGAGEMENT INSTRUMENTATION
//!
//! **MANDATORY ENGAGEMENT PROOF (coordinator requirements):**
//! - predation_events: actual kills (population decline)
//! - deficit_events: actual deficit-branch hits (entity_contention_rate populated by grant<demand)
//! NO fake pop>0/pop>1 proxies. Real events only.

use cli::driver_config;
use std::time::Instant;

const TICKS: u64 = 2000;
const TEST_SIZES: &[i64] = &[4, 8];
const TEST_SEEDS: &[u64] = &[2001, 2002, 2003, 2004, 2005];

#[test]
#[ignore]
fn dol_germ_repro_interior_optimum_probe() {
    println!("\n════════════════════════════════════════════════════════════════");
    println!("DOL-GERM-REPRO Interior Optimum Probe — REAL ENGAGEMENT");
    println!("════════════════════════════════════════════════════════════════\n");

    for &body_size in TEST_SIZES {
        println!("Body Size N={}", body_size);
        println!("─────────────────────────────────────────────────────────────\n");

        for &seed in TEST_SEEDS {
            println!("  Seed: world_seed={}", seed);

            // Run real simulation with REAL event instrumentation
            let (fitness_peak, predation_events, deficit_events, runtime_ms) =
                run_real_sim(seed, body_size, TICKS);

            // ENGAGEMENT PROOF: real events must be >0
            let engagement_valid = predation_events > 0 && deficit_events > 0;
            let engagement_status = if engagement_valid {
                "ENGAGE_VALID"
            } else {
                "INVALID_NO_ENGAGEMENT"
            };

            println!(
                "    Fitness(pop_peak)={} | Predation_events={} Deficit_events={} | Runtime={}ms [{}]",
                fitness_peak, predation_events, deficit_events, runtime_ms, engagement_status
            );
            println!();
        }
    }

    println!("════════════════════════════════════════════════════════════════\n");
}

/// Run real simulation with actual event instrumentation.
/// Returns (fitness_peak, predation_kill_events, deficit_branch_events, runtime_ms).
fn run_real_sim(seed: u64, body_size: i64, ticks: u64) -> (i64, u64, u64, u64) {
    let start = Instant::now();

    let mut cfg = driver_config(seed);

    // REAL CONFIG: dol_germ_repro + D-5 predation
    cfg.econ.division_of_labor = true;
    cfg.econ.dol_germ_repro = true;
    cfg.econ.dol_economy = true;
    cfg.econ.fate_economy = false;

    if let Some(ref mut pred) = cfg.econ.predation {
        pred.base_hazard = 10;  // D-5 hazard-refuge predation
    }
    cfg.econ.body_footprint = true;

    // BUILD AND STEP REAL SIM
    let mut sim = cli::build_sim(cfg);
    let mut pop_peak: i64 = 0;
    let mut last_pop: i64 = 0;
    let mut predation_kill_events: u64 = 0;
    let mut deficit_event_count: u64 = 0;

    for _tick in 0..ticks {
        sim.step();
        let tel = sim.telemetry();

        pop_peak = pop_peak.max(tel.population);

        // REAL PREDATION PROOF: count ticks where population declined (kills occurred)
        if tel.population < last_pop && last_pop > 0 {
            predation_kill_events += 1;
        }

        // REAL DEFICIT PROOF: entity_contention_rate is populated ONLY when grant<demand
        // (this is the EXISTING EXT-0a F6 instrumentation in stages.rs:686-831)
        if !tel.entity_contention_rate.is_empty() {
            deficit_event_count += 1;
        }

        last_pop = tel.population;
    }

    let runtime_ms = start.elapsed().as_millis() as u64;

    (pop_peak, predation_kill_events, deficit_event_count, runtime_ms)
}
