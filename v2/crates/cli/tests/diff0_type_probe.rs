//! DIFF-0-0: zero-economy substrate probe — does cell-type differentiation (T>=2) ever occur
//! under stock `driver_config`?
//!
//! Measures the joint (T, N) distribution under unmodified `driver_config`, where T = number of
//! DISTINCT `CellType`s among a body's modules and N = `CellGraph::body_size()`.
//!
//! The strong prior (from `phase2_config`'s `GrnSpec.input_weights = [0, 0]` — "drive dead") is that
//! every cell resolves the SAME GRN attractor ⇒ same `CellType` ⇒ `T == 1` always. If that holds,
//! the differentiation axis has NO SUBSTRATE under `driver_config`, and pricing it in an economy
//! would have priced a variable the substrate cannot vary. This test measures it.
//!
//! **Stock `driver_config`, unmodified.** Do NOT set `input_weights`, do NOT enable any gated
//! mechanic, do NOT touch `dol_economy`/`env_frontier_config`. The whole point is to measure
//! the config as it ships.
//!
//! **Output format (greppable MAP lines):**
//! ```
//! DIFF0 hist <seed> <tick> T=<t>,N=<n>:<count> …
//! DIFF0 stats <seed> <tick> pop=<p> max_T=<maxT> first_T2_tick=<tick_or_none> min_iw0=<min> mean_iw0=<mean> max_iw0=<max>
//! ```
//! where `iw0` = `|input_weights[0]|` from genomes with GRN specs.

#[test]
#[ignore]  // Cloud-only diagnostic; runs via sim-run.yml diff-probe case
fn diff0_type_probe() {
    use cli::driver_config;
    use std::env;

    let ticks = env::var("DIFF0_TICKS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(8000);

    let seeds = [1u64, 2, 3, 4, 5, 6, 7, 8];

    for seed in &seeds {
        let mut sim = cli::build_sim(driver_config(*seed));

        // Track first T >= 2 across all ticks
        let mut first_t2_tick: Option<u64> = None;

        // Sample at midpoint and horizon
        let sample_ticks = [ticks / 2, ticks];

        for _ in 0..ticks {
            sim.step();

            let current_tick = sim.tick();
            if sample_ticks.contains(&current_tick) {
                // Get probe data
                let (tn_histogram, (max_t, min_iw0, mean_iw0, max_iw0)) = sim.distinct_types_probe();
                let pop = sim.population();

                // Track first observation of T >= 2
                if max_t >= 2 && first_t2_tick.is_none() {
                    first_t2_tick = Some(current_tick);
                }

                // Emit histogram line
                let hist_str = tn_histogram
                    .iter()
                    .map(|((t, n), count)| format!("T={},N={}:{}", t, n, count))
                    .collect::<Vec<_>>()
                    .join(" ");
                println!("DIFF0 hist {} {} {}", seed, current_tick, hist_str);

                // Emit stats line
                let first_t2_str = first_t2_tick.map_or("none".to_string(), |t| t.to_string());
                println!(
                    "DIFF0 stats {} {} pop={} max_T={} first_T2_tick={} min_iw0={} mean_iw0={} max_iw0={}",
                    seed, current_tick, pop, max_t, first_t2_str, min_iw0, mean_iw0, max_iw0
                );
            }
        }
    }
}

/// Smoke test: verify that the driver_config probe builds and steps without panicking.
#[test]
fn diff0_type_probe_smoke() {
    let mut sim = cli::build_sim(cli::driver_config(42u64));
    const SMOKE_TICKS: u64 = 10;
    for _ in 0..SMOKE_TICKS {
        sim.step();
        assert_eq!(sim.conservation_residual(), 0, "energy not conserved at tick {}", sim.tick());
    }
}
