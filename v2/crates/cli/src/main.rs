//! Headless CLI driver — M4 perf foundation.
//!
//! Usage: `v2-sim [seed [ticks]] [--bench-pop N] [--profile] [--timelapse <interval>] [--set KEY=VALUE ...]`
//!
//! * No flags → default demo (two-run determinism, R14, per-50-tick telemetry).
//! * `seed ticks` positional → override the defaults (seed as decimal or 0x hex).
//! * `--bench-pop N` → run the perf-gate bench scenario (world_dim=128, N founders).
//! * `--profile`     → print per-stage wall-clock ns (requires `--features perf`).
//! * `--timelapse I` → emit telemetry as parseable CSV every I ticks instead of
//!   the default human-readable per-50-tick summary.
//! * `--set KEY=VALUE` → override one EconParams calibration knob before the run.
//!   Repeatable (each `--set` overrides one field). Validated before any sim runs;
//!   invalid keys or out-of-range values exit with `error:` and code 1.
//!   Whitelisted knobs: km, u_max, base_metab, c_div, e_cell, k_size_metab, k_move_cost,
//!   k_sense_cost, excrete, recycle_num, speciation_threshold, brain_period, metab_period,
//!   d0_scaled, pheromone, reg_gain_max, c_coord, refuge_k, bite_shift, base_hazard,
//!   gdev_cap, morphogen_steps. Structural fields (n_layers, world_dim, …) are rejected.
//!
//! **R27**: `dt` is fixed at `DT_MICROS` (1/64 s). Time-acceleration in headless mode means
//! running ticks as fast as the CPU allows (no vsync). The multi-rate K/N periods
//! (`EconParams::brain_period` / `metab_period`) are the ONLY per-system rate dials.
//! A time-scale multiplier on `dt` would violate determinism and is not part of the v2 core.

use cli::{
    apply_overrides, bench_config, build_sim, config_with, run, run_conserved_hashes,
    DEFAULT_THREADS,
};
use sim_core::{EconParams, MergeStrategy};
use telemetry::{compute_with_census, guild_csv_header, guild_csv_row};

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = raw.iter().map(String::as_str).collect();
    let (seed, ticks, bench_pop, do_profile, timelapse_interval, sets) = parse_args(&args);

    // Validate ALL overrides once, before any sim run — clean exit on bad params.
    let mut econ = EconParams::default();
    if let Err(msg) = apply_overrides(&mut econ, &sets) {
        eprintln!("{msg}");
        std::process::exit(1);
    }

    if let Some(n_pop) = bench_pop {
        run_bench(seed, n_pop, ticks, do_profile, &sets);
        return;
    }

    run_demo(seed, ticks, do_profile, timelapse_interval, econ);
}

/// Parse positional `[seed [ticks]]` plus flags. Returns sets as `Vec<(key, val)>`.
fn parse_args(args: &[&str]) -> (u64, u64, Option<u64>, bool, Option<u64>, Vec<(String, String)>) {
    let mut seed = 0xA11A_2A11u64;
    let mut ticks = 400u64;
    let mut bench_pop: Option<u64> = None;
    let mut do_profile = false;
    let mut timelapse: Option<u64> = None;
    let mut sets: Vec<(String, String)> = Vec::new();
    let mut positional = 0usize;
    let mut i = 0usize;
    while i < args.len() {
        match args[i] {
            "--bench-pop" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("error: --bench-pop requires a value (positive integer)");
                    std::process::exit(1);
                }
                let n: u64 = args[i].parse().unwrap_or_else(|_| {
                    eprintln!("error: --bench-pop value must be a positive integer, got {:?}", args[i]);
                    std::process::exit(1);
                });
                if n == 0 {
                    eprintln!("error: --bench-pop must be ≥ 1");
                    std::process::exit(1);
                }
                bench_pop = Some(n);
            }
            "--profile" => {
                do_profile = true;
            }
            "--timelapse" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("error: --timelapse requires a value (positive integer interval)");
                    std::process::exit(1);
                }
                let interval: u64 = args[i].parse().unwrap_or_else(|_| {
                    eprintln!("error: --timelapse value must be a positive integer, got {:?}", args[i]);
                    std::process::exit(1);
                });
                if interval == 0 {
                    eprintln!("error: --timelapse interval must be ≥ 1 (0 would cause a divide-by-zero)");
                    std::process::exit(1);
                }
                timelapse = Some(interval);
            }
            "--set" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("error: --set requires a value in KEY=VALUE form");
                    std::process::exit(1);
                }
                let kv = args[i];
                let eq = kv.find('=').unwrap_or_else(|| {
                    eprintln!("error: --set value must be KEY=VALUE (no '=' found in {:?})", kv);
                    std::process::exit(1);
                });
                sets.push((kv[..eq].to_string(), kv[eq + 1..].to_string()));
            }
            arg if !arg.starts_with('-') => {
                match positional {
                    0 => {
                        seed = arg
                            .parse()
                            .or_else(|_| u64::from_str_radix(arg.trim_start_matches("0x"), 16))
                            .unwrap_or_else(|_| {
                                eprintln!("error: invalid seed {:?} (expected decimal or 0x hex)", arg);
                                std::process::exit(1);
                            });
                    }
                    1 => {
                        ticks = arg.parse().unwrap_or_else(|_| {
                            eprintln!("error: invalid ticks {:?} (expected positive integer)", arg);
                            std::process::exit(1);
                        });
                    }
                    _ => {
                        eprintln!("error: unexpected positional argument {:?}", arg);
                        std::process::exit(1);
                    }
                }
                positional += 1;
            }
            arg => {
                eprintln!("error: unknown argument {:?}", arg);
                std::process::exit(1);
            }
        }
        i += 1;
    }
    (seed, ticks, bench_pop, do_profile, timelapse, sets)
}

/// Build a `SimConfig` from `config_with` with `econ` patched in.
/// When `econ == EconParams::default()` the result is field-for-field identical to
/// `config_with(seed, threads, merge)` — the no-override path is byte-identical.
fn patched_cfg(seed: u64, threads: usize, merge: MergeStrategy, econ: EconParams) -> sim_core::SimConfig {
    let mut cfg = config_with(seed, threads, merge);
    cfg.econ = econ;
    cfg
}

/// Standard demo: two-run determinism, R14 1-vs-N, telemetry loop.
/// Behaviourally identical to the pre-M4 hard-coded binary when run with no flags (`econ == default()`).
fn run_demo(seed: u64, ticks: u64, do_profile: bool, timelapse_interval: Option<u64>, econ: EconParams) {
    // ── Two-run determinism + R14 ────────────────────────────────────────────────────────────────
    let a = run(patched_cfg(seed, DEFAULT_THREADS, MergeStrategy::Canonical, econ.clone()), ticks);
    let b = run(patched_cfg(seed, DEFAULT_THREADS, MergeStrategy::Canonical, econ.clone()), ticks);
    println!("animata v2 — M4 perf foundation (integer brain + multi-rate)");
    println!(
        "seed={seed:#x} ticks={ticks} sim_threads={DEFAULT_THREADS} K(brain)={} N(metab)={}",
        econ.brain_period, econ.metab_period
    );
    println!("two-run-same-seed identical per tick: {}", a == b);
    let c1 = run_conserved_hashes(patched_cfg(seed, 1, MergeStrategy::Canonical, econ.clone()), ticks);
    let cn = run_conserved_hashes(
        patched_cfg(seed, DEFAULT_THREADS, MergeStrategy::Canonical, econ.clone()),
        ticks,
    );
    println!("R14 conserved hash 1-vs-{DEFAULT_THREADS} identical: {}", c1 == cn);
    println!("final state hash: {:#018x}", a.last().copied().unwrap_or(0));

    // ── Telemetry loop ───────────────────────────────────────────────────────────────────────────
    let mut sim = build_sim(patched_cfg(seed, DEFAULT_THREADS, MergeStrategy::Canonical, econ));
    let mut pop_min = u64::MAX;
    let mut pop_max = 0u64;

    // Timelapse CSV header (emitted once before the loop).
    // Guild columns are generated from Guild::ALL — the same source as the data row — so the
    // column count cannot drift if guilds are added in the future.
    if timelapse_interval.is_some() {
        println!(
            "tick,population,\
             metabolism_eff_mean,move_speed_mean,sense_range_mean,size_mean,repro_threshold_mean,mutation_rate_mean,\
             uptake_layer_mean,excrete_layer_mean,\
             metabolism_eff_price,move_speed_price,sense_range_price,size_price,repro_threshold_price,mutation_rate_price,\
             uptake_layer_price,excrete_layer_price,\
             trait_var_diversity,field_total,signal_total,species_count,\
             {guild_hdr},shannon,simpson",
            guild_hdr = guild_csv_header(),
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
            let tele = sim.telemetry();
            let rep = compute_with_census(&tele.samples, &tele.species_census, sim.econ().detritus_layer);
            let field_total = tele.field_total;
            let signal = tele.signal_total;
            let species_count = tele.species_count;
            if timelapse_interval.is_some() {
                // CSV row — parseable, arch-observational (signal_total is f32 → arch-bound).
                // Guild columns come from guild_csv_row (same Guild::ALL source as the header).
                let m = &rep.means;
                let pc = &rep.price_cov;
                let guilds = guild_csv_row(&rep);
                println!(
                    "{tick},{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:+.6},{:+.6},{:+.6},{:+.6},{:+.6},{:+.6},{:+.6},{:+.6},{:.6},{field_total},{signal:.4},{species_count},{guilds},{:.6},{:.6}",
                    rep.population,
                    m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7],
                    pc[0], pc[1], pc[2], pc[3], pc[4], pc[5], pc[6], pc[7],
                    rep.diversity,
                    rep.shannon,
                    rep.simpson,
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

    // ── Final-horizon summary (always printed; parseable key=value line for batch reducers) ──────
    {
        let tele = sim.telemetry();
        let rep = compute_with_census(&tele.samples, &tele.species_census, sim.econ().detritus_layer);
        let (mean_body_size, max_body_size, multicellular_frac) =
            (tele.mean_body_size, tele.max_body_size, tele.multicellular_frac);
        let resid = sim.conservation_residual();
        println!(
            "HORIZON tick={ticks} pop={} resid={resid} size_mean={:.4} trait_var_diversity={:.6} shannon={:.6} simpson={:.6} mean_body_size={mean_body_size} max_body_size={max_body_size} multicellular_frac={multicellular_frac}",
            rep.population, rep.means[3], rep.diversity, rep.shannon, rep.simpson,
        );
    }

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
/// Calibration overrides in `sets` are threaded into the bench econ (structural fields are already
/// rejected by `parse_args`/`main` validation; `world_dim=128` is preserved by `bench_config`).
/// Prints per-stage perf summary when `--features perf`; exits cleanly.
fn run_bench(seed: u64, n_pop: u64, ticks: u64, do_profile: bool, sets: &[(String, String)]) {
    let mut cfg = bench_config(seed, n_pop);
    // Thread calibration overrides into bench econ (already validated; world_dim=128 stays
    // because world_dim is in the structural-reject list and can't appear in `sets`).
    if !sets.is_empty() {
        apply_overrides(&mut cfg.econ, sets).expect("overrides already validated in main");
    }
    println!(
        "animata v2 bench — seed={seed:#x} n_founders={n_pop} ticks={ticks} world_dim={}",
        cfg.econ.world_dim
    );
    let mut sim = build_sim(cfg);
    let mut peak_pop = 0u64;
    let mut min_pop = u64::MAX;
    // born_total (E-2): count of entities ACTUALLY SPAWNED over the run — the numerator the
    // `births/tick × N_dev × G_dev²` morphogen perf ceiling (E-3 precondition) needs. Deliberately
    // NOT `WorkCounters::birth_death_iters` (F11): that counts every-entity-every-tick gate
    // iterations (≈ N×ticks), which would inflate this denominator by orders of magnitude. Read
    // purely from `sim.telemetry()` (already-public, read-only, outside the sim hot-path) — no
    // sim-core change, no effect on the trajectory or any golden.
    let mut born_total = 0u64;
    for _ in 0..ticks {
        sim.step();
        let p = sim.population();
        peak_pop = peak_pop.max(p);
        min_pop = min_pop.min(p);
        born_total += sim.telemetry().samples.iter().map(|s| s.offspring as u64).sum::<u64>();
    }
    let mean_births_per_tick = born_total as f64 / ticks as f64;
    println!(
        "peak_population={peak_pop}  min_population={min_pop}  born_total={born_total}  mean_births_per_tick={mean_births_per_tick:.3}"
    );

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
    assert!(min_pop > 0, "population collapsed to zero during bench scenario");
}
