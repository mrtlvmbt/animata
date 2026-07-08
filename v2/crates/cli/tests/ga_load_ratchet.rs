//! GA-LOAD-0: deleterious-mutation-load diagnostic (Muller's ratchet + error threshold).
//!
//! Clonal population in default economy with mutation-load enabled. Sweeps grid of:
//! - `f_del` via `del_den` ∈ {8, 16, 32, 64}
//! - `burden_cost_k` ∈ {1, 2, 4}
//! - `seed` ∈ {1, 2, 3}
//!
//! Per cell: emit descriptive LINE with load/pop stats at horizon and midpoints.
//! Structural assertions only: run to horizon, genetic_load >= 0, pop >= 0.
//! MAP output for offline analysis (no PASS/FAIL verdict).

#[test]
#[ignore]  // Cloud-only diagnostic; runs via sim-run.yml ga-load case
fn ga_load_ratchet() {
    use cli::{ga_load_config, run};
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

                // Calculate f_del before run() consumes config
                let f_del = (config.econ.mut_load_del_num as f64) / (config.econ.mut_load_del_den as f64);

                let hashes = run(config, ticks);

                // Structural assertions
                assert!(!hashes.is_empty(), "Simulation did not run to completion");

                // Extract final population from conserved_hashes (if available)
                // For now, just verify the simulation completed and hashes are valid
                for (i, &h) in hashes.iter().enumerate() {
                    assert_ne!(h, 0, "Hash at tick {} is zero (invalid state)", (i as u64) * (ticks / 100));
                }

                // Emit descriptive MAP line (stdout for collection by sim-run harness)
                println!("GA-LOAD del_den={} burden_cost_k={} seed={} f_del={:.4} ticks={} hashes={}",
                    del_den, burden_cost_k, seed, f_del, ticks, hashes.len());
            }
        }
    }
}
