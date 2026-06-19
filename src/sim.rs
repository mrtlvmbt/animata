//! Life simulation — phase C0: a single-cell herbivore ecosystem on the voxel world.
//!
//! The simplest possible living population, built to validate the energy / biomass / trophic
//! loop and the world integration BEFORE any developmental complexity (that is C1). A creature
//! is one cell: it senses the plant-biomass field (S3) around it, a tiny fixed-topology brain
//! with **evolvable weights** decides throttle + turn, it grazes the column it stands on,
//! pays a Kleiber-scaled metabolic cost, buds a mutated child when well-fed, and dies at zero
//! energy. No development, no morphology, no inter-creature interaction yet.
//!
//! Determinism invariants (see plan): randomness is a pure function of the world seed via
//! [`crate::rng`] (no `rand` crate); creatures live in a `Vec` (stable index); the tick is
//! multi-phase (snapshot/decide read the world unmutated → apply mutates → compact), so the
//! result is independent of iteration order; deaths flag-then-compact (never `swap_remove`
//! mid-apply); over-cap cull is deterministic-random, not tail-truncation.

use macroquad::math::{vec2, Vec2};

use crate::config::*;
use crate::rng::{seed_fold, splitmix64, Rng};
use crate::terrain::VoxelTerrain;

// Fixed brain topology for C0 (the genome is just the weight vector — evolvable, fixed length).
const N_INPUTS: usize = 7; // [biomass_here, fwd, left, right, energy, water_dist, bias]
const N_HIDDEN: usize = 6;
const N_OUTPUTS: usize = 2; // [throttle (pre-squash), turn (pre-squash)]
const N_WEIGHTS: usize = N_INPUTS * N_HIDDEN + N_HIDDEN * N_OUTPUTS;

// Seed salts (keep distinct so independent draws on the same (id, tick) don't correlate).
const SALT_FOUNDER: u64 = 0x0F00;
const SALT_MUTATE: u64 = 0x111;
const SALT_CULL: u64 = 0xC011;
const SALT_DEATH: u64 = 0xDEAD;
const SALT_BIRTH: u64 = 0xB127;

/// One creature. In C0 every creature is a single cell, so biomass is the constant 1; the
/// `weights` are its heritable genome.
pub struct Creature {
    pub id: u64,
    pub founder: u64,
    pub pos: Vec2, // world (x, z) over the ground plane; column = (x/VOX, z/VOX)
    pub heading: f32,
    pub energy: f32,
    pub age: u32,
    alive: bool,
    weights: Vec<f32>,
}

impl Creature {
    /// Biomass in integer cells. Fixed at 1 for C0; C1's development makes this Σ cells.
    pub fn biomass(&self) -> u32 {
        1
    }

    /// Forward brain pass: inputs → tanh hidden → tanh outputs. Returns `(throttle∈[0,1],
    /// turn∈[-1,1])`. Plain matmul (ported shape from the archived `brain.rs`).
    fn think(&self, inputs: &[f32; N_INPUTS]) -> (f32, f32) {
        let w = &self.weights;
        let mut hidden = [0.0f32; N_HIDDEN];
        for (h, hv) in hidden.iter_mut().enumerate() {
            let mut sum = 0.0;
            for (i, &iv) in inputs.iter().enumerate() {
                sum += iv * w[h * N_INPUTS + i];
            }
            *hv = sum.tanh();
        }
        let base = N_INPUTS * N_HIDDEN;
        let mut out = [0.0f32; N_OUTPUTS];
        for (o, ov) in out.iter_mut().enumerate() {
            let mut sum = 0.0;
            for (h, &hv) in hidden.iter().enumerate() {
                sum += hv * w[base + o * N_HIDDEN + h];
            }
            *ov = sum.tanh();
        }
        ((out[0] + 1.0) * 0.5, out[1])
    }
}

/// The whole creature population + the deterministic id counter and cumulative stats.
pub struct Sim {
    pub creatures: Vec<Creature>,
    world_seed: u64,
    next_id: u64,
    pub births: u64,
    pub deaths: u64,
}

/// Clamp a continuous world position to an in-world column index (single conversion point —
/// out of bounds would otherwise panic or silently corrupt a neighbour row via `graze`).
pub fn column_index(pos: Vec2) -> (usize, usize) {
    let x = (pos.x / VOX).floor().clamp(0.0, (COLS - 1) as f32) as usize;
    let y = (pos.y / VOX).floor().clamp(0.0, (ROWS - 1) as f32) as usize;
    (x, y)
}

/// Colder columns cost more to live in (a mild climate tax that seeds habitat pressure later).
fn climate_factor(temp: f32) -> f32 {
    1.0 + 0.6 * (1.0 - temp)
}

impl Sim {
    /// Spawn the founder population on land columns, deterministically from `world_seed`.
    pub fn new(world_seed: u64, terrain: &VoxelTerrain) -> Self {
        let mut creatures = Vec::with_capacity(START_CREATURES);
        for i in 0..START_CREATURES as u64 {
            let mut rng = Rng::new(seed_fold(world_seed, &[SALT_FOUNDER, i]));
            // Place on land: a few tries to dodge water, else accept (clamped) wherever.
            let mut pos = vec2(0.0, 0.0);
            for _ in 0..8 {
                pos = vec2(rng.unit() * COLS as f32 * VOX, rng.unit() * ROWS as f32 * VOX);
                let (cx, cy) = column_index(pos);
                if !terrain.is_water(cx, cy) {
                    break;
                }
            }
            let weights = (0..N_WEIGHTS).map(|_| rng.signed()).collect();
            creatures.push(Creature {
                id: i,
                founder: i,
                pos,
                heading: rng.unit() * std::f32::consts::TAU,
                energy: START_ENERGY,
                age: 0,
                alive: true,
                weights,
            });
        }
        Sim { creatures, world_seed, next_id: START_CREATURES as u64, births: 0, deaths: 0 }
    }

    /// One fixed sim tick. Multi-phase so the outcome is independent of iteration order:
    /// (a/b) all creatures sense the unmutated world and decide; (c) apply in index order
    /// (move, graze — which mutates the terrain — metabolise, mark deaths, buffer births);
    /// (d) compact dead out, append births, cull to the cap deterministically.
    pub fn step(&mut self, terrain: &mut VoxelTerrain, tick: u64) {
        let n = self.creatures.len();
        // (a/b) snapshot + decide — reads only, terrain unmutated.
        let mut decisions: Vec<(f32, f32)> = Vec::with_capacity(n);
        for c in &self.creatures {
            let inputs = self.sense(c, terrain, tick);
            decisions.push(c.think(&inputs));
        }
        // (c) apply in index order.
        let (maxx, maxy) = (COLS as f32 * VOX, ROWS as f32 * VOX);
        // Logistic birth gate from the population at the START of the tick (deterministic;
        // doesn't shift as births accrue). On the over-provisioned map this aggregate
        // competition term — not food — sets the equilibrium near `SOFT_CAP`.
        let birth_gate = (1.0 - n as f32 / SOFT_CAP).clamp(0.0, 1.0);
        let mut births: Vec<Creature> = Vec::new();
        for (idx, c) in self.creatures.iter_mut().enumerate() {
            let (throttle, turn) = decisions[idx];
            // Move.
            c.heading += turn * TURN_RATE * TICK_LEN;
            let step = throttle * CREATURE_SPEED * TICK_LEN;
            c.pos.x += c.heading.cos() * step;
            c.pos.y += c.heading.sin() * step;
            // Map edge = wall (reflect), via the single clamp helper for the column.
            if c.pos.x < 0.0 {
                c.pos.x = 0.0;
                c.heading = std::f32::consts::PI - c.heading;
            } else if c.pos.x > maxx {
                c.pos.x = maxx;
                c.heading = std::f32::consts::PI - c.heading;
            }
            if c.pos.y < 0.0 {
                c.pos.y = 0.0;
                c.heading = -c.heading;
            } else if c.pos.y > maxy {
                c.pos.y = maxy;
                c.heading = -c.heading;
            }
            let (cx, cy) = column_index(c.pos);
            // Graze the column (mutates terrain biomass) → energy.
            let taken = terrain.graze(cx, cy, EAT_RATE * TICK_LEN, tick);
            c.energy = (c.energy + taken * PLANT_BIOMASS_TO_ENERGY).min(MAX_ENERGY);
            // Metabolism: Kleiber (biomass^0.75) × climate, + movement effort.
            let kleiber = (c.biomass() as f32).powf(0.75);
            let metab = SIM_BASE_METABOLISM * kleiber * climate_factor(terrain.temperature_at(cx, cy));
            c.energy -= (metab + MOVE_COST * throttle) * TICK_LEN;
            c.age += 1;
            // Death by starvation.
            if c.energy <= 0.0 {
                c.alive = false;
                self.deaths += 1;
                continue;
            }
            // Death by senescence: old-age probability rising with age² gives demographic
            // turnover (so death isn't only the over-cap cull) and selection to reproduce young.
            let sp = SENESCENCE_RATE * (c.age as f32 / LIFESPAN).powi(2);
            if sp > 0.0 && Rng::new(seed_fold(self.world_seed, &[SALT_DEATH, c.id, tick])).unit() < sp {
                c.alive = false;
                self.deaths += 1;
                continue;
            }
            // Reproduction: bud a mutated child, splitting energy in half. Gated by the logistic
            // birth term (population self-limits near SOFT_CAP) on top of the energy threshold.
            let lucky = birth_gate >= 1.0
                || Rng::new(seed_fold(self.world_seed, &[SALT_BIRTH, c.id, tick])).unit() < birth_gate;
            if c.energy >= REPRO_ENERGY && lucky {
                c.energy *= 0.5;
                let mut rng = Rng::new(seed_fold(self.world_seed, &[SALT_MUTATE, c.id, tick]));
                let weights = c.weights.iter().map(|&w| w + rng.signed() * MUTATION_STD).collect();
                let child = Creature {
                    id: self.next_id,
                    founder: c.founder,
                    pos: vec2(
                        (c.pos.x + rng.signed() * 2.0).clamp(0.0, maxx),
                        (c.pos.y + rng.signed() * 2.0).clamp(0.0, maxy),
                    ),
                    heading: rng.unit() * std::f32::consts::TAU,
                    energy: c.energy,
                    age: 0,
                    alive: true,
                    weights,
                };
                self.next_id += 1;
                self.births += 1;
                births.push(child);
            }
        }
        // (d) compact: drop the dead, append births, cull to the cap.
        self.creatures.retain(|c| c.alive);
        self.creatures.append(&mut births);
        self.cull_to_cap(tick);
    }

    /// Sense the plant-biomass field ahead / left / right (a gradient to climb), plus own
    /// energy, the column's water distance and a bias. Read-only on the terrain.
    fn sense(&self, c: &Creature, terrain: &VoxelTerrain, tick: u64) -> [f32; N_INPUTS] {
        let sample = |angle: f32| {
            let p = vec2(c.pos.x + angle.cos() * SENSE_RADIUS, c.pos.y + angle.sin() * SENSE_RADIUS);
            let (cx, cy) = column_index(p);
            terrain.biomass_at(cx, cy, tick)
        };
        let (cx, cy) = column_index(c.pos);
        [
            terrain.biomass_at(cx, cy, tick),
            sample(c.heading),
            sample(c.heading + std::f32::consts::FRAC_PI_2),
            sample(c.heading - std::f32::consts::FRAC_PI_2),
            (c.energy / REPRO_ENERGY).min(1.0),
            terrain.water_dist_at(cx, cy) as f32 / 255.0,
            1.0,
        ]
    }

    /// Deterministic-random cull down to `SIM_POP_CAP` (sort the living by a splitmix key, drop
    /// the lowest). NOT tail-truncation — that would systematically kill the freshest newborns
    /// and bias selection against reproduction (which is the engine of diversity).
    fn cull_to_cap(&mut self, tick: u64) {
        if self.creatures.len() <= SIM_POP_CAP {
            return;
        }
        let seed = self.world_seed ^ SALT_CULL;
        let key = |c: &Creature| splitmix64(seed.wrapping_add(splitmix64(tick).wrapping_add(c.id)));
        self.creatures.sort_unstable_by_key(key);
        let removed = self.creatures.len() - SIM_POP_CAP;
        self.deaths += removed as u64;
        self.creatures.truncate(SIM_POP_CAP);
    }

    pub fn population(&self) -> usize {
        self.creatures.len()
    }

    pub fn avg_energy(&self) -> f32 {
        if self.creatures.is_empty() {
            return 0.0;
        }
        self.creatures.iter().map(|c| c.energy).sum::<f32>() / self.creatures.len() as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> VoxelTerrain {
        VoxelTerrain::new(1)
    }

    #[test]
    fn column_index_clamps_out_of_world() {
        assert_eq!(column_index(vec2(-100.0, -100.0)), (0, 0));
        assert_eq!(column_index(vec2(1e9, 1e9)), (COLS - 1, ROWS - 1));
    }

    /// A run is reproducible from the world seed: two sims stepped the same number of fixed
    /// ticks have an identical population and identical leading creatures.
    #[test]
    fn deterministic_replay() {
        let (mut t1, mut t2) = (world(), world());
        let (mut a, mut b) = (Sim::new(42, &t1), Sim::new(42, &t2));
        for tick in 0..300 {
            a.step(&mut t1, tick);
            b.step(&mut t2, tick);
        }
        assert_eq!(a.population(), b.population());
        assert_eq!(a.births, b.births);
        assert_eq!(a.deaths, b.deaths);
        for (x, y) in a.creatures.iter().zip(b.creatures.iter()).take(50) {
            assert_eq!(x.id, y.id);
            assert_eq!(x.pos, y.pos);
            assert_eq!(x.energy, y.energy);
        }
    }

    /// The lock metric: over a headless run the herbivore population neither dies out nor pins
    /// the cap — a living, self-limiting ecosystem on the new world. (Tuning target for C0.)
    #[test]
    fn population_stays_in_a_living_corridor() {
        for &seed in &[1u64, 2, 3] {
            let mut t = world();
            let mut s = Sim::new(seed, &t);
            for tick in 0..4000 {
                s.step(&mut t, tick);
            }
            let pop = s.population();
            eprintln!("seed {seed}: pop {pop}, avg_energy {:.1}, births {}, deaths {}", s.avg_energy(), s.births, s.deaths);
            assert!(pop > 100, "population collapsed for seed {seed}: {pop}");
            assert!(pop < SIM_POP_CAP, "population pinned the cap for seed {seed}: {pop}");
        }
    }

    /// Tuning aid (ignored): print the population trajectory for one seed so the energy
    /// constants can be balanced into a food-limited corridor below the cap.
    #[test]
    #[ignore]
    fn tune_trajectory() {
        let mut t = world();
        let mut s = Sim::new(1, &t);
        for tick in 0..6000 {
            s.step(&mut t, tick);
            if tick % 500 == 0 {
                eprintln!("tick {tick}: pop {} avg_E {:.1} births {} deaths {}", s.population(), s.avg_energy(), s.births, s.deaths);
            }
        }
    }
}
