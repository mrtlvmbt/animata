//! GA-LOAD-0: deleterious-mutation-load diagnostic (Muller's ratchet + error threshold).
//!
//! Clonal population in default economy with mutation-load enabled. Sweeps grid of:
//! - `f_del` via `del_den` ∈ {8, 16, 32, 64}
//! - `burden_cost_k` ∈ {1, 2, 4}
//! - `seed` ∈ {1, 2, 3}
//!
//! Per cell: emit descriptive LINE with load/pop stats at horizon and midpoints (25%, 50%, 75%, 100%).
//! Structural assertions only: run to horizon, genetic_load >= 0, pop >= 0.
//! MAP output for offline analysis (no PASS/FAIL verdict).
//!
//! Output format (space-separated fields):
//!   del_den burden_cost_k seed f_del ticks
//!   t25/pop/load_mean/load_max t50/pop/load_mean/load_max t75/pop/load_mean/load_max t100/pop/load_mean/load_max

#[test]
#[ignore]  // Cloud-only diagnostic; runs via sim-run.yml ga-load case
fn ga_load_ratchet() {
    use cli::ga_load_config;
    use std::env;

    let ticks = env::var("GA_LOAD_TICKS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(8000);

    // Sweep grid: del_den × burden_cost_k × seed
    let del_dens = [8u32, 16, 32, 64];
    let burden_costs = [1i64, 2, 4];
    let seeds = [1u64, 2, 3];

    for &del_den in &del_dens {
        for &burden_cost_k in &burden_costs {
            for &seed in &seeds {
                let mut config = ga_load_config(seed);
                // Override sweep parameters
                config.econ.mut_load_del_den = del_den as i32;
                config.econ.burden_cost_k = burden_cost_k;

                // Calculate f_del before consuming config
                let f_del = (config.econ.mut_load_del_num as f64) / (config.econ.mut_load_del_den as f64);

                // Build sim and step directly to capture samples
                let mut sim = cli::build_sim(config);

                // Sample at 25%, 50%, 75%, 100% of ticks
                let sample_ticks = [ticks / 4, ticks / 2, 3 * ticks / 4, ticks];
                let mut samples = Vec::new();

                for tick_idx in 0..ticks {
                    sim.step();
                    let residual = sim.conservation_residual();
                    assert_eq!(residual, 0, "ENERGY CONSERVATION VIOLATED at tick {}: residual={residual}", sim.tick());
                    assert!(sim.signal_finite(), "SIGNAL NaN/Inf at tick {}", sim.tick());

                    let current_tick = sim.tick();
                    if sample_ticks.contains(&current_tick) {
                        let pop = sim.population();
                        let (load_mean, load_max) = sim.genetic_load_stats();
                        samples.push((current_tick, pop, load_mean, load_max));
                    }
                }

                // Structural assertions
                assert_eq!(samples.len(), 4, "Expected 4 samples, got {}", samples.len());
                for (t, pop, lm, lx) in &samples {
                    assert!(*pop >= 0, "Population at tick {} is negative: {}", t, pop);
                    assert!(*lm >= 0, "Load mean at tick {} is negative: {}", t, lm);
                    assert!(*lx >= 0, "Load max at tick {} is negative: {}", t, lx);
                }

                // Emit MAP line with all sample data
                let sample_str = samples
                    .iter()
                    .map(|(t, pop, lm, lx)| format!("t{}/pop={}/lm={}/lx={}", t, pop, lm, lx))
                    .collect::<Vec<_>>()
                    .join(" ");

                println!("GA-LOAD del_den={} burden_cost_k={} seed={} f_del={:.4} ticks={} {}",
                    del_den, burden_cost_k, seed, f_del, ticks, sample_str);
            }
        }
    }
}
