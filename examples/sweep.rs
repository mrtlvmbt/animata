//! Headless parameter sweep: run the sim across a grid of parameters × seeds,
//! single-threaded (macroquad's RNG is global), and write one row of outcomes
//! per run to `sweep.csv` for offline analysis / tuning.
//!
//! Run: `cargo run --release --example sweep [steps] [seeds]`
//!   e.g. `cargo run --release --example sweep 20000 5`

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
use std::io::Write as _;
use world::World;

fn main() {
    let steps: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(20_000);
    let seeds: u64 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(5);

    // Parameter grid (the most impactful knobs).
    let food_grid = [1.5f32, 2.2, 3.0];
    let mut_grid = [0.006f64, 0.012, 0.024];

    let mut out = String::from(
        "food_per_step,mutation_rate,seed,survived,steps_survived,pop,herb,pred,\
avg_speed,avg_sense,avg_carnivory,diversity,species,clades,max_gen\n",
    );
    let total = food_grid.len() * mut_grid.len() * seeds as usize;
    let mut done = 0;

    for &food in &food_grid {
        for &mut_rate in &mut_grid {
            for seed in 0..seeds {
                let mut w = World::new(seed, BehaviorKind::Neural);
                w.params.food_per_step = food;
                w.params.mutation_rate = mut_rate;

                let mut survived_steps = steps;
                for step in 0..steps {
                    w.step();
                    if w.creatures.is_empty() {
                        survived_steps = step;
                        break;
                    }
                }
                let survived = !w.creatures.is_empty();
                let s = w.stats.latest();
                out.push_str(&format!(
                    "{:.2},{:.4},{},{},{},{},{},{},{:.3},{:.1},{:.3},{:.4},{},{},{}\n",
                    food, mut_rate, seed, survived as u8, survived_steps, s.population,
                    s.herbivores, s.predators, s.avg_speed, s.avg_sense, s.avg_carnivory,
                    s.diversity, s.species, s.lineages, s.max_generation
                ));
                done += 1;
                println!(
                    "[{done}/{total}] food {food:.1} mut {mut_rate:.3} seed {seed}: \
{} pop {} ({} steps)",
                    if survived { "survived" } else { "EXTINCT" },
                    s.population,
                    survived_steps
                );
            }
        }
    }

    let path = "sweep.csv";
    match std::fs::File::create(path).and_then(|mut f| f.write_all(out.as_bytes())) {
        Ok(()) => println!("\nwrote {total} rows to {path}"),
        Err(e) => eprintln!("failed to write {path}: {e}"),
    }
}
