//! Gas-cycle Phase 1 spike + multi-year probe (go/no-go): does O2 production + toxicity BRAKE the
//! autotroph monoculture vs the oxygen feature OFF, without extinction? Also PRINTS the raw f32 O2
//! field (max/mean/nonzero) to confirm gentle per-creature deposits ACCUMULATE (plan F1/F5) rather
//! than getting absorbed. Two arms on the same seeds:
//!   A off : Features.oxygen = false (today's behaviour, no O2)
//!   B on  : Features.oxygen = true  (production + OxygenToxicity + evolving oxygen_tolerance)
//! GO iff B's autotroph fraction settles BELOW A's across years AND B doesn't crash the population.
//! Prints the ACTUAL engine tick in the body (plan F8 forcing function), not just a filename.
//! Run: cargo run -p animata-sim --release --example probe_oxygen

use animata_sim::sim::Sim;
use animata_sim::sim_config::SimConfig;
use animata_sim::terrain::VoxelTerrain;

fn arm(seed: u64, oxygen: bool, years: u64, year_ticks: u64) {
    let mut t = VoxelTerrain::new(seed);
    let mut cfg = SimConfig::default();
    cfg.features.oxygen = oxygen;
    let mut s = Sim::with_config(seed, &t, cfg);
    let label = if oxygen { "B oxygen-ON " } else { "A oxygen-OFF" };
    let mut tick = 0u64;
    for y in 1..=years {
        for _ in 0..year_ticks {
            s.step(&mut t, tick);
            tick += 1;
        }
        let (omax, omean, onz) = t.oxygen_field_stats();
        println!(
            "  [{label} yr{y}/{years} tick={tick}] autotroph {:>4.1}%  pop {:>6}  O2(max {:.3} mean {:.4} nz {})",
            s.frac_autotroph() * 100.0,
            s.population(),
            omax,
            omean,
            onz
        );
    }
}

fn main() {
    // args: [years] [year_ticks] — defaults to the long multi-generational horizon.
    let mut a = std::env::args().skip(1);
    let years: u64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(4);
    let year_ticks: u64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(60_000);
    let seed = 1u64;
    println!("oxygen probe: seed {seed}, {years}×{year_ticks} ticks (GO iff B autotroph% < A, no crash)");
    arm(seed, false, years, year_ticks);
    arm(seed, true, years, year_ticks);
}
