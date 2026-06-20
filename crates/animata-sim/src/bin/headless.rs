//! Headless sim runner — proves `animata-sim` runs standalone, with no graphics.
//!
//! Steps the deterministic fixed-tick sim on a generated world and prints population metrics
//! plus the `state_checksum` (the determinism lock). This is the same fixed-step path the
//! acceptance tests use, so its checksum is the canonical replay value for a (seed, ticks, profile).
//! With `--metrics PATH` it samples the metric registry every 100 ticks and writes the time-series
//! as CSV (for offline graphs / regression baselines).
//!
//! Usage: `cargo run -p animata-sim --bin headless [--release] -- [seed] [ticks] [--metrics out.csv]`

use animata_sim::metrics::{MetricRegistry, SimView};
use animata_sim::sim::{state_checksum, Sim};
use animata_sim::terrain::VoxelTerrain;

fn main() {
    // Positional [seed] [ticks] + optional `--metrics PATH`.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut positional = args.iter().filter(|a| !a.starts_with("--"));
    let seed: u64 = positional.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    let ticks: u64 = positional.next().and_then(|s| s.parse().ok()).unwrap_or(4000);
    let metrics_path = args.iter().position(|a| a == "--metrics").and_then(|i| args.get(i + 1)).cloned();

    let mut terrain = VoxelTerrain::new(seed);
    let mut sim = Sim::new(seed, &terrain);
    let mut metrics = metrics_path.as_ref().map(|_| MetricRegistry::default());

    for tick in 0..ticks {
        sim.step(&mut terrain, tick);
        if let Some(reg) = metrics.as_mut() {
            reg.maybe_sample(&SimView { sim: &sim, terrain: &terrain, tick });
        }
    }

    let (multi, complex) = sim.complexity_mix();
    println!("seed={seed} ticks={ticks}");
    println!("  population   {}", sim.population());
    println!("  avg_energy   {:.2}", sim.avg_energy());
    println!("  avg_biomass  {:.3}", sim.avg_biomass());
    println!("  multi/complex {:.1}% / {:.1}%", multi * 100.0, complex * 100.0);
    println!("  carnivore    {:.1}%", sim.frac_carnivore() * 100.0);
    println!("  autotroph    {:.1}%", sim.frac_autotroph() * 100.0);
    println!("  species      {}", sim.species_count());
    println!("  niches       {}", sim.niche_coverage(&terrain));
    println!("  births/deaths/kills {}/{}/{}", sim.births, sim.deaths, sim.kills);
    println!("  state_checksum 0x{:016x}", state_checksum(&sim, &terrain));

    if let (Some(reg), Some(path)) = (metrics.as_ref(), metrics_path.as_ref()) {
        match std::fs::write(path, reg.to_csv()) {
            Ok(()) => println!("  metrics      {} samples → {path}", reg.len()),
            Err(e) => eprintln!("  metrics write failed ({path}): {e}"),
        }
    }
}
