//! Density-cap sweep: find the gentlest cap that BOUNDS (no explosion) while sparing the zones corridor.
//! Runs to 25000t with early-abort on explosion (pop>200k). Reports zones@8000 + final/max pop.
//! Run: --example probe_cap -- [cap] [seed]
use animata_sim::sim::Sim;
use animata_sim::terrain::VoxelTerrain;
fn main() {
    let cap: f32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(50.0);
    let seed: u64 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(1);
    let mut t = VoxelTerrain::new(seed);
    let mut s = Sim::new(seed, &t);
    s.density_cap = cap;
    s.density_lethality = 0.2;
    let mut zones8k = 0.0f32;
    let mut mx = 0usize;
    let mut exploded = false;
    for tick in 0..25000u64 {
        s.step(&mut t, tick);
        if tick == 7999 { zones8k = s.frac_with_zones(); }
        let p = s.population(); mx = mx.max(p);
        if (tick+1) % 1000 == 0 && p > 200_000 { exploded = true; break; }
    }
    let bound = if exploded { "✗ EXPLODES" } else { "✓ bounded" };
    let zpass = if zones8k > 0.05 { "zones✓" } else { "zones✗" };
    println!("CAP {cap} seed {seed}: {bound} (max {mx}) | zones@8k {:.1}% {zpass} | final {}", zones8k*100.0, s.population());
}
