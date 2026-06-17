//! Run the simulation without a window and print population/trait trends.
//! Useful for tuning constants. Run: `cargo run --example headless`

#[path = "../src/config.rs"]
mod config;
#[path = "../src/biome.rs"]
mod biome;
#[path = "../src/brain.rs"]
mod brain;
#[path = "../src/body.rs"]
mod body;
#[path = "../src/behavior.rs"]
mod behavior;
#[path = "../src/genome.rs"]
mod genome;
#[path = "../src/grid.rs"]
mod grid;
#[path = "../src/marker.rs"]
mod marker;
#[path = "../src/phylo.rs"]
mod phylo;
#[path = "../src/speciation.rs"]
mod speciation;
#[path = "../src/stats.rs"]
mod stats;
#[path = "../src/creature.rs"]
mod creature;
#[path = "../src/world.rs"]
mod world;

use behavior::BehaviorKind;
use world::World;

fn main() {
    // Args: [neural|rule] [seed]. Pass "rule" to compare the rule-based behavior.
    let kind = match std::env::args().nth(1).as_deref() {
        Some("rule") => BehaviorKind::Rule,
        _ => BehaviorKind::Neural,
    };
    let seed: u64 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(1);
    println!("behavior: {}  seed: {}", kind.label(), seed);
    let steps: u64 = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(4000);
    let window = std::env::args().nth(4).and_then(|s| s.parse().ok()).unwrap_or(20_000u64);
    let mut w = World::new(seed, kind);
    println!("step      herb  carn  gen    diet  orn   div   clades  spec   resist inf%  mem   nspr  seg   app%  und%  air%  hid  fin% us/step(window) | mk_emit mk_listen ch_meaning");
    let mut t = std::time::Instant::now();
    for step in 0..=steps {
        w.step();
        if step % window == 0 && step > 0 {
            let us = t.elapsed().as_secs_f64() * 1e6 / window as f64;
            let s = w.stats.latest();
            let cm = s.channel_meaning.iter().map(|v| format!("{:+.2}", v)).collect::<Vec<_>>().join(",");
            println!(
                "{:8}  {:4}  {:4}  {:5}  {:.2}  {:.2}  {:.3}  {:3}    {:3}    {:.2}   {:3.0}  {:.2}  {:.3}  {:.2}  {:3.0}  {:3.0}  {:3.0}  {:.1} {:3.0} {:.1} | {:.3} {:.3} [{}] cryp={:.3}",
                step, s.herbivores, s.predators, s.max_generation, s.avg_carnivory, s.avg_ornament, s.diversity, s.lineages, s.species, s.avg_resistance, s.infected_frac * 100.0, s.avg_memory, s.niche_spread, s.avg_segments, s.appendaged_frac * 100.0, s.frac_underground * 100.0, s.frac_air * 100.0, s.avg_hidden, s.frac_finned * 100.0, us, s.marker_emit, s.marker_listener_frac, cm, s.avg_color_contrast
            );
            println!("         spd={:.2} sns={:.0} cryp_all={:.3} cryp_pred={:.3} und%={:.0} air%={:.0} pred={}", s.avg_speed, s.avg_sense, s.avg_color_contrast, s.avg_color_contrast_pred, s.frac_underground*100.0, s.frac_air*100.0, s.predators);
            t = std::time::Instant::now();
        }
        if w.creatures.is_empty() {
            println!("EXTINCT at step {step}");
            break;
        }
    }
    println!("{}", w.profile.report());

    // Morpho-fragility gate: do fresh body-plan mutants die in early life more
    // often than their peers? A novel/baseline ratio well above 1 (with adequate
    // novel N) is the evidence that would justify MIP.
    let m = w.morpho;
    let nm = m.novel_mortality() * 100.0;
    let bm = m.baseline_mortality() * 100.0;
    let r = if bm > 0.0 { nm / bm } else { f32::NAN };
    println!(
        "morpho-fragility: novel infant mortality {:.1}% ({}/{}), baseline {:.1}% ({}/{}), ratio {:.2}",
        nm, m.died_young_novel, m.born_novel, bm, m.died_young_normal, m.born_normal, r
    );
}
