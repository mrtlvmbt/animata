//! Headless M0 acceptance demo: N dummy entities, two runs of (seed + empty input log) → identical
//! per-tick hash; per-stage timings (with `--features perf`); the stable final hash.

use cli::{run, DEFAULT_ENTITIES, DEFAULT_SEED, DEFAULT_TICKS};

fn main() {
    let seed = DEFAULT_SEED;
    let n = DEFAULT_ENTITIES;
    let ticks = DEFAULT_TICKS;

    let a = run(seed, n, ticks);
    let b = run(seed, n, ticks);
    let identical = a == b;

    println!("animata v2 — M0 walking skeleton");
    println!("seed={seed:#x} entities={n} ticks={ticks}");
    println!("two-run-same-seed identical per tick: {identical}");
    println!("final state hash: {:#018x}", a.last().copied().unwrap_or(0));

    #[cfg(feature = "perf")]
    {
        use sim_core::Sim;
        let mut sim = Sim::new(seed, n);
        for _ in 0..ticks {
            sim.step();
        }
        println!("per-stage perf (total_ns, last_ns_per_entity):");
        for (name, (total, per)) in sim.perf().stages() {
            println!("  {name:<20} {total:>12} ns   {per:>8} ns/ent");
        }
    }

    assert!(identical, "determinism violated: two runs of the same seed diverged");
}
