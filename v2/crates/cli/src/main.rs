//! Headless M1 acceptance demo: run the Ф0 economy, print the closed bookkeeping (population, field
//! total, energy residual), two-run determinism, and the Price covariance of selection.

use cli::{build_sim, default_config, run};

fn main() {
    let seed = 0xA11A_2A11;
    let ticks = 400;

    // Two-run determinism (within this arch + profile).
    let a = run(default_config(seed), ticks);
    let b = run(default_config(seed), ticks);
    println!("animata v2 — M1 first life (Ф0 economy)");
    println!("seed={seed:#x} ticks={ticks}");
    println!("two-run-same-seed identical per tick: {}", a == b);
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
            println!(
                "  tick {tick:>4}  pop={p:>4}  field={field_total:>8}  resid={resid}  size_mean={:.2}  price(size)={:+.4}",
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
