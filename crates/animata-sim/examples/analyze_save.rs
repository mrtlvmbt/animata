//! Full-statistics diagnostic over a world snapshot — load a save and dump the population picture
//! (trophic niches, body plans, cell-type mix, energy/age, the predator question) WITHOUT graphics.
//!
//! Usage: `cargo run -p animata-sim --release --example analyze_save -- [path]`
//! (default path: `animata-save.bin`). Read-only: never writes the save.

use animata_sim::config::{CARNIVORE_THRESHOLD, ORGAN_MIN, PHOTO_THETA, REPRO_ENERGY, STRATUM_THETA};
use animata_sim::genome::Phenotype;
use animata_sim::persist::Snapshot;
use std::fs::File;
use std::io::BufReader;

/// Mutually-exclusive food niche by cell mix (precedence matches the energy model: a body that can
/// feed itself off light is an autotroph even with predator cells; else a predatory-enough body is a
/// carnivore; else it grazes).
fn niche(p: &Phenotype) -> &'static str {
    if p.photo_frac() > PHOTO_THETA {
        "autotroph"
    } else if p.carnivory() > CARNIVORE_THRESHOLD {
        "carnivore"
    } else {
        "herbivore"
    }
}

/// Print a labelled fraction-of-population line.
fn pct(label: &str, count: usize, n: usize) {
    println!("  {label:<22}: {count:>6}  ({:.2}%)", 100.0 * count as f32 / n.max(1) as f32);
}

/// A compact bucket histogram for a `[0,1]`-ish series (10 equal bins, blanks skipped).
fn histogram(label: &str, vals: impl Iterator<Item = f32>, bins: usize) {
    let mut h = vec![0u32; bins + 1];
    let mut total = 0u32;
    for v in vals {
        h[((v * bins as f32) as usize).min(bins)] += 1;
        total += 1;
    }
    println!("{label} (n={total}):");
    for (i, k) in h.iter().enumerate() {
        if *k > 0 {
            println!("  [{:.2}-{:.2}) : {k}", i as f32 / bins as f32, (i + 1) as f32 / bins as f32);
        }
    }
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "animata-save.bin".into());
    let snap = Snapshot::read(BufReader::new(File::open(&path).expect("open save"))).expect("read snapshot");
    let cs = &snap.sim.creatures;
    let n = cs.len();
    assert!(n > 0, "empty population");
    let avgf = |f: &dyn Fn(&Phenotype) -> f32| cs.iter().map(|c| f(&c.pheno)).sum::<f32>() / n as f32;

    println!("================ SNAPSHOT ================");
    println!("file        : {path}");
    println!("tick        : {}", snap.tick);
    println!("terrain seed: {}", snap.terrain_seed);
    println!("population  : {n}");
    println!(
        "cumulative  : births {}  deaths {}  kills {}  (kills/death {:.4})",
        snap.sim.births,
        snap.sim.deaths,
        snap.sim.kills,
        snap.sim.kills as f64 / snap.sim.deaths.max(1) as f64
    );

    println!("\n================ TROPHIC NICHES (mutually exclusive, sum=pop) ================");
    for label in ["autotroph", "carnivore", "herbivore"] {
        pct(label, cs.iter().filter(|c| niche(&c.pheno) == label).count(), n);
    }

    println!("\n================ PREDATORS (the question) ================");
    pct("any predator cell", cs.iter().filter(|c| c.pheno.predator > 0).count(), n);
    pct("over carnivore thr 0.2", cs.iter().filter(|c| c.pheno.carnivory() > CARNIVORE_THRESHOLD).count(), n);
    println!("  max predator cells    : {}", cs.iter().map(|c| c.pheno.predator).max().unwrap());
    println!("  max carnivory fraction: {:.3}", cs.iter().map(|c| c.pheno.carnivory()).fold(0.0, f32::max));
    histogram("carnivory (predator-cell fraction of body)", cs.iter().map(|c| c.pheno.carnivory()), 10);

    println!("\n================ BODY PLANS ================");
    pct("multicellular (>1 cell)", cs.iter().filter(|c| c.pheno.n_cells > 1).count(), n);
    pct("complex (>=2 types)", cs.iter().filter(|c| c.pheno.complexity() == 2).count(), n);
    pct("with coherent organ", cs.iter().filter(|c| c.pheno.organ.iter().any(|&l| l >= ORGAN_MIN)).count(), n);
    pct("with emergent axis>=26", cs.iter().filter(|c| c.pheno.axis_order >= 26).count(), n);
    println!("  avg cells   : {:.2}", avgf(&|p| p.n_cells as f32));
    println!("  max cells   : {}", cs.iter().map(|c| c.pheno.n_cells).max().unwrap());
    println!("  avg axis_ord: {:.1} / 255", avgf(&|p| p.axis_order as f32));

    println!("\n================ STRATUM PROXIES (cell-fraction gates, thr {STRATUM_THETA}) ================");
    pct("flight (air)", cs.iter().filter(|c| c.pheno.flight_frac() > STRATUM_THETA).count(), n);
    pct("burrow (underground)", cs.iter().filter(|c| c.pheno.burrow_frac() > STRATUM_THETA).count(), n);
    pct("photo (autotroph)", cs.iter().filter(|c| c.pheno.photo_frac() > PHOTO_THETA).count(), n);

    println!("\n================ CELL-TYPE TOTALS (whole population) ================");
    let sum = |f: &dyn Fn(&Phenotype) -> u32| cs.iter().map(|c| f(&c.pheno) as u64).sum::<u64>();
    let total_cells = sum(&|p| p.n_cells);
    type Col<'a> = (&'a str, &'a dyn Fn(&Phenotype) -> u32);
    let cols: [Col; 8] = [
        ("structural", &|p: &Phenotype| p.structural),
        ("effector", &|p| p.effector),
        ("storage", &|p| p.storage),
        ("sensor", &|p| p.sensor),
        ("predator", &|p| p.predator),
        ("flight", &|p| p.flight),
        ("burrow", &|p| p.burrow),
        ("photo", &|p| p.photo),
    ];
    for (label, f) in cols {
        let t = sum(f);
        println!("  {label:<11}: {t:>8}  ({:.1}% of all cells)", 100.0 * t as f32 / total_cells as f32);
    }
    println!("  TOTAL cells: {total_cells}");

    println!("\n================ ENERGY / AGE ================");
    let energy: Vec<f32> = cs.iter().map(|c| c.energy).collect();
    let ages: Vec<u32> = cs.iter().map(|c| c.age).collect();
    println!("  avg energy : {:.1}  (repro threshold {REPRO_ENERGY})", energy.iter().sum::<f32>() / n as f32);
    println!("  >= repro   : {} ({:.1}%)", energy.iter().filter(|&&e| e >= REPRO_ENERGY).count(), 100.0 * energy.iter().filter(|&&e| e >= REPRO_ENERGY).count() as f32 / n as f32);
    println!("  avg age    : {:.0} ticks  (max {})", ages.iter().sum::<u32>() as f32 / n as f32, ages.iter().max().unwrap());
}
