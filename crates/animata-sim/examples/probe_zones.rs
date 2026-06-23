//! Multi-seed zones-emergence check (the corridor's measure, per seed) — to decide §5 re-validation
//! of `zones_emerge_under_selection` under the density death-sink. Run: --example probe_zones -- [seed]
use animata_sim::sim::Sim;
use animata_sim::terrain::VoxelTerrain;
fn main() {
    let seed: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let mut t = VoxelTerrain::new(seed);
    let mut s = Sim::new(seed, &t);
    for tick in 0..8000u64 { s.step(&mut t, tick); }
    let frac = s.frac_with_zones();
    println!(
        "SEED {seed}: zones {:.1}% (bar 5.0%)  avg_zones {:.2}  corr {:.3}  pop {}  {}",
        frac * 100.0, s.avg_zones(), s.zones_size_correlation(), s.population(),
        if frac > 0.05 { "PASS" } else { "FAIL" }
    );
}
