//! Autotroph-base Phase 1 spike diagnostics: confirm founder develops as `photo`, then trace pop.
//! Run: cargo run -p animata-sim --release --example probe_autotroph [ticks]

use animata_sim::genome::Genome;
use animata_sim::rng::Rng;
use animata_sim::sim::Sim;
use animata_sim::sim_config::SimConfig;
use animata_sim::terrain::VoxelTerrain;

fn main() {
    let ticks: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(10_000);
    // 1) founder phenotype check
    let mut rng = Rng::new(12345);
    let g = Genome::founder(&mut rng);
    let p = g.develop();
    println!("founder pheno: n_cells {} photo {} structural {} effector {} predator {}", p.n_cells, p.photo, p.structural, p.effector, p.predator);

    // 2) multi-seed end-state: survives? heterotrophs appear? autotroph < 100%? stable?
    for seed in 1..=3u64 {
        let mut t = VoxelTerrain::new(seed);
        let mut s = Sim::with_config(seed, &t, SimConfig::default());
        let (mut pmin, mut pmax) = (usize::MAX, 0usize);
        for tick in 0..ticks {
            s.step(&mut t, tick);
            if (tick + 1) % 1000 == 0 {
                let p = s.population();
                pmin = pmin.min(p);
                pmax = pmax.max(p);
            }
        }
        let pop = s.population();
        let verdict = if pop == 0 { "☠ EXTINCT" } else { "ok" };
        println!(
            "  seed {seed}: pop {pop:>6} auto {:>4.0}% carn {:>4.1}% het {:>3.0}% pop[{}-{}] {verdict}",
            s.frac_autotroph() * 100.0, s.frac_carnivore() * 100.0, 100.0 - s.frac_autotroph() * 100.0,
            if pmin == usize::MAX { 0 } else { pmin }, pmax,
        );
    }
}
