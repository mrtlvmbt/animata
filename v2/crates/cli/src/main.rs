//! Headless CLI driver — M4 perf foundation.
//!
//! Usage: `v2-sim [seed [ticks]] [--bench-pop N] [--profile] [--timelapse <interval>]`
//!
//! * No flags → default demo (two-run determinism, R14, per-50-tick telemetry).
//! * `seed ticks` positional → override the defaults (seed as decimal or 0x hex).
//! * `--bench-pop N` → run the perf-gate bench scenario (world_dim=128, N founders).
//! * `--profile`     → print per-stage wall-clock ns (requires `--features perf`).
//! * `--timelapse I` → emit telemetry as parseable CSV every I ticks instead of
//!   the default human-readable per-50-tick summary.
//!
//! **R27**: `dt` is fixed at `DT_MICROS` (1/64 s). Time-acceleration in headless mode means
//! running ticks as fast as the CPU allows (no vsync). The multi-rate K/N periods
//! (`EconParams::brain_period` / `metab_period`) are the ONLY per-system rate dials.
//! A time-scale multiplier on `dt` would violate determinism and is not part of the v2 core.

use cli::{bench_config, build_sim, build_sim_bench, default_config, run, run_conserved_hashes, DEFAULT_THREADS};
use sim_core::{EconParams, MergeStrategy};

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = raw.iter().map(String::as_str).collect();
    let (seed, ticks, bench_pop, do_profile, timelapse_interval) = parse_args(&args);

    if let Some(n_pop) = bench_pop {
        run_bench(seed, n_pop, ticks, do_profile);
        return;
    }

    run_demo(seed, ticks, do_profile, timelapse_interval);
}

/// Parse positional `[seed [ticks]]` plus flags.
fn parse_args(args: &[&str]) -> (u64, u64, Option<u64>, bool, Option<u64>) {
    let mut seed = 0xA11A_2A11u64;
    let mut ticks = 400u64;
    let mut bench_pop: Option<u64> = None;
    let mut do_profile = false;
    let mut timelapse: Option<u64> = None;
    let mut positional = 0usize;
    let mut i = 0usize;
    while i < args.len() {
        match args[i] {
            "--bench-pop" => {
                i += 1;
                bench_pop = Some(args[i].parse().expect("--bench-pop requires a number"));
            }
            "--profile" => { do_profile = true; }
            "--timelapse" => {
                i += 1;
                timelapse = Some(args[i].parse().expect("--timelapse requires a number"));
            }
            arg if !arg.starts_with('-') => {
                match positional {
                    0 => {
                        seed = arg.parse()
                            .or_else(|_| u64::from_str_radix(arg.trim_start_matches("0x"), 16))
                            .unwrap_or_else(|_| panic!("invalid seed: {arg}"));
                    }
                    1 => { ticks = arg.parse().unwrap_or_else(|_| panic!("invalid ticks: {arg}")); }
                    _ => panic!("unexpected positional argument: {arg}"),
                }
                positional += 1;
            }
            arg => panic!("unknown argument: {arg}"),
        }
        i += 1;
    }
    (seed, ticks, bench_pop, do_profile, timelapse)
}

/// Standard demo: two-run determinism, R14 1-vs-N, telemetry loop.
/// Behaviourally identical to the pre-M4 hard-coded binary when run with no flags.
fn run_demo(seed: u64, ticks: u64, do_profile: bool, timelapse_interval: Option<u64>) {
    let econ = EconParams::default();
    let a = run(default_config(seed), ticks);
    let b = run(default_config(seed), ticks);
    println!("animata v2 — M4 perf foundation (integer brain + multi-rate)");
    println!(
        "seed={seed:#x} ticks={ticks} sim_threads={DEFAULT_THREADS} K(brain)={} N(metab)={}",
        econ.brain_period, econ.metab_period
    );
    println!("two-run-same-seed identical per tick: {}", a == b);
    let c1 = run_conserved_hashes(seed, 1, MergeStrategy::Canonical, ticks);
    let cn = run_conserved_hashes(seed, DEFAULT_THREADS, MergeStrategy::Canonical, ticks);
    println!("R14 conserved hash 1-vs-{DEFAULT_THREADS} identical: {}", c1 == cn);
    println!("final state hash: {:#018x}", a.last().copied().unwrap_or(0));

    let mut sim = build_sim(default_config(seed));
    let mut pop_min = u64::MAX;
    let mut pop_max = 0u64;

    // Timelapse CSV header (emitted once before the loop).
    if timelapse_interval.is_some() {
        println!(
            "tick,population,\
             metabolism_eff_mean,move_speed_mean,sense_range_mean,size_mean,repro_threshold_mean,mutation_rate_mean,\
             metabolism_eff_price,move_speed_price,sense_range_price,size_price,repro_threshold_price,mutation_rate_price,\
             diversity,field_total,signal_total"
        );
    }

    for t in 0..ticks {
        sim.step();
        let p = sim.population();
        pop_min = pop_min.min(p);
        pop_max = pop_max.max(p);

        let emit = match timelapse_interval {
            Some(interval) => (t + 1) % interval == 0,
            None => t % 50 == 49,
        };
        if emit {
            let tick = sim.tick();
            let rep = telemetry::compute(&sim.telemetry().samples);
            let field_total = sim.telemetry().field_total;
            let signal = sim.telemetry().signal_total;
            if timelapse_interval.is_some() {
                // CSV row — parseable, arch-observational (signal_total is f32 → arch-bound).
                let m = &rep.means;
                let pc = &rep.price_cov;
                println!(
                    "{tick},{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:+.6},{:+.6},{:+.6},{:+.6},{:+.6},{:+.6},{:.6},{field_total},{signal:.4}",
                    rep.population,
                    m[0], m[1], m[2], m[3], m[4], m[5],
                    pc[0], pc[1], pc[2], pc[3], pc[4], pc[5],
                    rep.diversity
                );
            } else {
                let resid = sim.conservation_residual();
                println!(
                    "  tick {tick:>4}  pop={p:>4}  field={field_total:>8}  signal={signal:>8.0}  resid={resid}  size_mean={:.2}  price(size)={:+.4}",
                    rep.means[3], rep.price_cov[3],
                );
            }
        }
    }
    println!("population range over run: [{pop_min}, {pop_max}] (bounded, no extinction/explosion)");

    if do_profile {
        #[cfg(feature = "perf")]
        {
            println!("per-stage perf (total_ns, last_ns_per_entity):");
            for (name, (total, per)) in sim.perf().stages() {
                println!("  {name:<20} {total:>12} ns   {per:>8} ns/ent");
            }
        }
        #[cfg(not(feature = "perf"))]
        eprintln!("warning: --profile requires --features perf");
    }

    assert!(a == b, "determinism violated");
    assert!(pop_min > 0, "population went extinct");
}

/// Bench scenario: world_dim=128, `n_pop` founders, `ticks` ticks.
/// Prints per-stage perf summary when `--features perf`; exits cleanly.
fn run_bench(seed: u64, n_pop: u64, ticks: u64, do_profile: bool) {
    let cfg = bench_config(seed, n_pop);
    println!(
        "animata v2 bench — seed={seed:#x} n_founders={n_pop} ticks={ticks} world_dim={}",
        cfg.econ.world_dim
    );
    let mut sim = build_sim_bench(seed, n_pop);
    let mut peak_pop = 0u64;
    for _ in 0..ticks {
        sim.step();
        peak_pop = peak_pop.max(sim.population());
    }
    println!("peak_population={peak_pop}");

    if do_profile {
        #[cfg(feature = "perf")]
        {
            println!("per-stage perf (total_ns, last_ns_per_entity):");
            for (name, (total, per)) in sim.perf().stages() {
                println!("  {name:<20} {total:>12} ns   {per:>8} ns/ent");
            }
            let wc = sim.perf().work;
            println!(
                "work counters: brain_infer={} field_takes={} birth_death_iters={} scatter_deposits={}",
                wc.brain_infer, wc.field_takes, wc.birth_death_iters, wc.scatter_deposits
            );
        }
        #[cfg(not(feature = "perf"))]
        eprintln!("warning: --profile requires --features perf");
    }
    assert!(peak_pop > 0, "population went extinct in bench scenario");
}
