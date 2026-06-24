//! Headless M3 acceptance demo: creatures run fixed-point INTEGER recurrent brains (behaviour every K
//! ticks), metabolism every N ticks, reproduction event-driven — and the whole multi-rate trajectory
//! replays bit-for-bit. Prints closed bookkeeping, two-run determinism, the Price covariance, the
//! signal total, and the R14 1-vs-N conserved-field equality.

use cli::{build_sim, default_config, run, run_conserved_hashes, DEFAULT_THREADS};
use sim_core::{EconParams, MergeStrategy};

fn main() {
    let seed = 0xA11A_2A11;
    let ticks = 400;
    let econ = EconParams::default();

    // Two-run determinism (within this arch + profile, fixed sim-thread N).
    let a = run(default_config(seed), ticks);
    let b = run(default_config(seed), ticks);
    println!("animata v2 — M3 brain + multi-rate (integer recurrent inference)");
    println!(
        "seed={seed:#x} ticks={ticks} sim_threads={DEFAULT_THREADS} K(brain)={} N(metab)={}",
        econ.brain_period, econ.metab_period
    );
    println!("two-run-same-seed identical per tick: {}", a == b);
    // R14: conserved-field hash identical on 1 vs N threads.
    let c1 = run_conserved_hashes(seed, 1, MergeStrategy::Canonical, ticks);
    let cn = run_conserved_hashes(seed, DEFAULT_THREADS, MergeStrategy::Canonical, ticks);
    println!("R14 conserved hash 1-vs-{DEFAULT_THREADS} identical: {}", c1 == cn);
    println!("final state hash: {:#018x}", a.last().copied().unwrap_or(0));

    // Replay the trajectory once more to print the emergence telemetry.
    let mut sim = build_sim(default_config(seed));
    let mut pop_min = u64::MAX;
    let mut pop_max = 0u64;
    for t in 0..ticks {
        sim.step();
        let p = sim.population();
        pop_min = pop_min.min(p);
        pop_max = pop_max.max(p);
        if t % 50 == 49 {
            let resid = sim.conservation_residual();
            let tick = sim.tick();
            let rep = telemetry::compute(&sim.telemetry().samples);
            let field_total = sim.telemetry().field_total;
            let signal = sim.telemetry().signal_total;
            println!(
                "  tick {tick:>4}  pop={p:>4}  field={field_total:>8}  signal={signal:>8.0}  resid={resid}  size_mean={:.2}  price(size)={:+.4}",
                rep.means[3], rep.price_cov[3],
            );
        }
    }
    println!("population range over run: [{pop_min}, {pop_max}] (bounded, no extinction/explosion)");

    #[cfg(feature = "perf")]
    {
        println!("per-stage perf (total_ns, last_ns_per_entity):");
        for (name, (total, per)) in sim.perf().stages() {
            println!("  {name:<20} {total:>12} ns   {per:>8} ns/ent");
        }
    }

    assert!(a == b, "determinism violated");
    assert!(pop_min > 0, "population went extinct");
}
