//! Gas-cycle Phase 2 spike (go/no-go), MULTI-SEED: does the aerobic energy windfall (O2 → a FOOD
//! multiplier for creatures with `aerobic_capacity`, who are also O2-immune) raise the carnivore /
//! predator fraction vs Phase 1 alone — ROBUSTLY across seeds, not single-seed noise? Both arms have
//! oxygen ON; they differ only in `Features.aerobic`. Phase 2's job is the PREDATOR-energy enablement
//! (raise the energy ceiling so high-cost lifestyles pay), NOT the monoculture fix (that's Phase 1's
//! brake + Phase 3's CO2 loop) — so the headline metric is carnivore%, autotroph% is secondary.
//! Run: cargo run -p animata-sim --release --example probe_aerobic [years] [year_ticks]

use animata_sim::sim::Sim;
use animata_sim::sim_config::SimConfig;
use animata_sim::terrain::VoxelTerrain;

/// Run one arm to `years*year_ticks` on `seed`; return (autotroph%, carnivore%) at the end.
fn run(seed: u64, aerobic: bool, total_ticks: u64) -> (f32, f32) {
    let mut t = VoxelTerrain::new(seed);
    let mut cfg = SimConfig::default();
    cfg.features.oxygen = true;
    cfg.features.aerobic = aerobic;
    let mut s = Sim::with_config(seed, &t, cfg);
    for tick in 0..total_ticks {
        s.step(&mut t, tick);
    }
    (s.frac_autotroph() * 100.0, s.frac_carnivore() * 100.0)
}

fn main() {
    let mut a = std::env::args().skip(1);
    let years: u64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(3);
    let year_ticks: u64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(30_000);
    let nseeds: u64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(3);
    let total = years * year_ticks;
    let seeds: Vec<u64> = (1..=nseeds).collect();
    println!("aerobic probe (multi-seed): seeds {seeds:?}, {total} ticks/arm (GO iff mean B carnivore% > A)");
    let (mut sa, mut sb) = (0.0f32, 0.0f32);
    for &seed in &seeds {
        let (aa, ac) = run(seed, false, total);
        let (ba, bc) = run(seed, true, total);
        sa += ac;
        sb += bc;
        println!("  seed {seed}: A(off) auto {aa:>4.1}% carn {ac:>4.2}%   |   B(on) auto {ba:>4.1}% carn {bc:>4.2}%   (Δcarn {:+.2})", bc - ac);
    }
    let n = seeds.len() as f32;
    println!("MEAN carnivore: A(off) {:.2}%   B(on) {:.2}%   Δ {:+.2}pp", sa / n, sb / n, (sb - sa) / n);
}
