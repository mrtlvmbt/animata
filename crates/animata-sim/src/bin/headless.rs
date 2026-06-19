//! Headless sim runner — proves `animata-sim` runs standalone, with no graphics.
//!
//! Steps the deterministic fixed-tick sim on a generated world and prints population metrics
//! plus the `state_checksum` (the determinism lock). This is the same fixed-step path the
//! acceptance tests use, so its checksum is the canonical replay value for a (seed, ticks, profile).
//!
//! Usage: `cargo run -p animata-sim --bin headless [--release] [seed] [ticks]`

use animata_sim::sim::{state_checksum, Sim};
use animata_sim::terrain::VoxelTerrain;

fn main() {
    let mut args = std::env::args().skip(1);
    let seed: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    let ticks: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(4000);

    let mut terrain = VoxelTerrain::new(seed);
    let mut sim = Sim::new(seed, &terrain);
    for tick in 0..ticks {
        sim.step(&mut terrain, tick);
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
}
