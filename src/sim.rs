//! Life simulation — herbivore ecosystem on the voxel world (C0 loop + C1 developmental body).
//!
//! A creature senses the plant-biomass field (S3) around it, a fixed-topology brain with
//! evolvable weights decides throttle + turn, it grazes the column it stands on, pays a
//! Kleiber-scaled metabolic cost, buds a mutated child when well-fed, and dies of starvation
//! or senescence. **C1:** the body is no longer a fixed single cell — it is *grown* from the
//! genome's gene-regulatory network ([`crate::genome`]); biomass = the developed cell count
//! and the cell-type mix modulates the creature's stats (effector→speed, storage→energy cap).
//! The founder's empty GRN develops to one structural cell, recovering the C0 organism.
//!
//! Determinism invariants (see plan): randomness is a pure function of the world seed via
//! [`crate::rng`] (no `rand` crate); creatures live in a `Vec` (stable index); the tick is
//! multi-phase (snapshot/decide read the world unmutated → apply mutates → compact), so the
//! result is independent of iteration order; deaths flag-then-compact (never `swap_remove`
//! mid-apply); over-cap cull is deterministic-random, not tail-truncation.

use macroquad::math::{vec2, Vec2};

use crate::config::*;
use crate::genome::{Genome, Phenotype};
use crate::grid::SpatialGrid;
use crate::rng::{seed_fold, splitmix64, Rng};
use crate::terrain::VoxelTerrain;

// Fixed brain topology: inputs → tanh hidden → tanh outputs. The genome's `brain` vector holds
// the weights (length = `genome::BRAIN_WEIGHTS` = N_INPUTS*N_HIDDEN + N_HIDDEN*N_OUTPUTS).
// Inputs: [plant_here, plant_fwd, plant_left, plant_right, energy, water_dist,
//          prey_prox, prey_bearing, threat_prox, threat_bearing, bias].
const N_INPUTS: usize = 11;
const N_HIDDEN: usize = 6;
const N_OUTPUTS: usize = 2; // [throttle (pre-squash), turn (pre-squash)]

// Seed salts (keep distinct so independent draws on the same (id, tick) don't correlate).
const SALT_FOUNDER: u64 = 0x0F00;
const SALT_MUTATE: u64 = 0x111;
const SALT_CULL: u64 = 0xC011;
const SALT_DEATH: u64 = 0xDEAD;
const SALT_BIRTH: u64 = 0xB127;

/// One creature. Its `genome` (developmental GRN + brain weights) is grown once into `pheno`
/// (the cell body) at creation; biomass and the stat modifiers below read from `pheno`.
pub struct Creature {
    pub id: u64,
    pub founder: u64,
    pub pos: Vec2, // world (x, z) over the ground plane; column = (x/VOX, z/VOX)
    pub heading: f32,
    pub energy: f32,
    pub age: u32,
    alive: bool,
    genome: Genome,
    pub pheno: Phenotype,
}

impl Creature {
    /// Biomass in integer cells = the developed cell count (Kleiber metabolism scales with it,
    /// and it is the energy a predator gets in C2).
    pub fn biomass(&self) -> u32 {
        self.pheno.n_cells
    }

    /// Top speed: effector cells add locomotor thrust (emergent — a body that develops more
    /// contractile cells moves faster, at the metabolic cost of carrying them).
    fn speed(&self) -> f32 {
        CREATURE_SPEED * (1.0 + EFFECTOR_GAIN * self.pheno.effector as f32)
    }

    /// Energy capacity: storage cells enlarge the buffer (survive lean spells, bigger broods).
    fn max_energy(&self) -> f32 {
        MAX_ENERGY + STORAGE_PER_CELL * self.pheno.storage as f32
    }

    /// Grazing throughput: a bigger body crops a little faster (sublinear, so size isn't free).
    fn intake(&self) -> f32 {
        EAT_RATE * (self.pheno.n_cells as f32).sqrt()
    }

    /// Forward brain pass: inputs → tanh hidden → tanh outputs. Returns `(throttle∈[0,1],
    /// turn∈[-1,1])`. Plain matmul (ported shape from the archived `brain.rs`).
    fn think(&self, inputs: &[f32; N_INPUTS]) -> (f32, f32) {
        let w = &self.genome.brain;
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
    pub kills: u64,
    /// Reused spatial index over creature positions, rebuilt each tick for prey/threat queries.
    grid: SpatialGrid,
}

/// Closeness `[0,1]` (1 = adjacent, 0 = at/over the sense range) and the left/right bearing of
/// a target relative to a creature's heading — the two cues the brain needs to steer to/from it.
fn rel(from: Vec2, heading: f32, target: Vec2) -> (f32, f32) {
    let d = target - from;
    let dist = d.length();
    let prox = (1.0 - dist / SENSE_RANGE).clamp(0.0, 1.0);
    let bearing = (d.y.atan2(d.x) - heading).sin();
    (prox, bearing)
}

/// Clamp a continuous world position to an in-world column index (single conversion point —
/// out of bounds would otherwise panic or silently corrupt a neighbour row via `graze`).
pub fn column_index(pos: Vec2) -> (usize, usize) {
    let x = (pos.x / VOX).floor().clamp(0.0, (COLS - 1) as f32) as usize;
    let y = (pos.y / VOX).floor().clamp(0.0, (ROWS - 1) as f32) as usize;
    (x, y)
}

/// Climate match in `[0.1, 1]`: how well a creature feeds at the local temperature given its
/// evolved preference. Matched ⇒ 1 (full food value); fully mismatched ⇒ 0.1 (climate stress
/// cripples foraging). This hits the DOMINANT energy channel (food), so it is a real selective
/// force on the over-provisioned map — cold bands favour low-pref lineages, hot bands high-pref,
/// and lineages sort into the climate they're suited to (C3 habitats / allopatry).
fn climate_match(temp: f32, pref: f32) -> f32 {
    (1.0 - THERMAL_PENALTY * (temp - pref).abs()).clamp(0.1, 1.0)
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
            let genome = Genome::founder(&mut rng); // empty GRN → single cell (== C0)
            let pheno = genome.develop();
            creatures.push(Creature {
                id: i,
                founder: i,
                pos,
                heading: rng.unit() * std::f32::consts::TAU,
                energy: START_ENERGY,
                age: 0,
                alive: true,
                genome,
                pheno,
            });
        }
        Sim {
            creatures,
            world_seed,
            next_id: START_CREATURES as u64,
            births: 0,
            deaths: 0,
            kills: 0,
            grid: SpatialGrid::default(),
        }
    }

    /// One fixed sim tick. Multi-phase so the outcome is independent of iteration order:
    /// (a) snapshot the world + a spatial index, every creature senses (plant field + nearest
    /// prey/threat) and decides; (b) predation pass — resolve hunts by snapshot index, flagging
    /// eaten prey dead and crediting predators (trophic transfer); (c) apply per survivor in
    /// index order (move, graze — diet-scaled, mutates the terrain — metabolise, deaths,
    /// births); (d) compact dead out, append births, cull to the cap deterministically.
    pub fn step(&mut self, terrain: &mut VoxelTerrain, tick: u64) {
        let n = self.creatures.len();
        let (maxx, maxy) = (COLS as f32 * VOX, ROWS as f32 * VOX);
        // (a) snapshot + decide — reads only, terrain unmutated. Snapshot arrays feed the grid
        // predicates without borrowing `self.creatures` inside the closures.
        let pos: Vec<Vec2> = self.creatures.iter().map(|c| c.pos).collect();
        let bm: Vec<u32> = self.creatures.iter().map(|c| c.biomass()).collect();
        let carn: Vec<f32> = self.creatures.iter().map(|c| c.pheno.carnivory()).collect();
        self.grid.rebuild(&pos, maxx, maxy, GRID_CELL);
        // Decision per creature: (throttle, turn, optional prey index to attack THIS tick).
        let mut decisions: Vec<(f32, f32, Option<usize>)> = Vec::with_capacity(n);
        for i in 0..n {
            let predator = carn[i] > CARNIVORE_THRESHOLD;
            let self_bm = bm[i];
            // Nearest edible prey (only if predatory) AND nearest threatening predator, one pass.
            let (prey, threat) = self.grid.nearest2_within(
                &pos,
                pos[i],
                SENSE_RANGE,
                |j| predator && j != i && bm[j] <= self_bm,
                |j| j != i && carn[j] > CARNIVORE_THRESHOLD && bm[j] >= self_bm,
            );
            let c = &self.creatures[i];
            let prey_rel = prey.map(|j| rel(pos[i], c.heading, pos[j]));
            let threat_rel = threat.map(|j| rel(pos[i], c.heading, pos[j]));
            let inputs = self.sense(c, terrain, tick, prey_rel, threat_rel);
            let (throttle, turn) = c.think(&inputs);
            // Attack if the targeted prey is within striking distance at snapshot positions.
            let hunt = prey.filter(|&j| (pos[j] - pos[i]).length() <= ATTACK_RANGE);
            decisions.push((throttle, turn, hunt));
        }
        // (b) predation: resolve hunts by snapshot index. Flag prey dead (never remove mid-pass
        // — indices must stay stable, F7); credit the predator with the trophic-scaled energy.
        for i in 0..n {
            let Some(j) = decisions[i].2 else { continue };
            if !self.creatures[i].alive || !self.creatures[j].alive {
                continue; // predator died, or prey already eaten by a lower-index predator
            }
            let gain = (bm[j] as f32 * CELL_BIOMASS_COST + self.creatures[j].energy.max(0.0))
                * MEAT_EFFICIENCY
                * carn[i];
            self.creatures[j].alive = false;
            self.deaths += 1;
            self.kills += 1;
            let cap = self.creatures[i].max_energy();
            self.creatures[i].energy = (self.creatures[i].energy + gain).min(cap);
        }
        // (c) apply per surviving creature in index order.
        // Logistic birth gate from the population at the START of the tick (deterministic;
        // doesn't shift as births accrue). On the over-provisioned map this aggregate
        // competition term — not food — sets the equilibrium near `SOFT_CAP`.
        let birth_gate = (1.0 - n as f32 / SOFT_CAP).clamp(0.0, 1.0);
        let mut births: Vec<Creature> = Vec::new();
        for (idx, c) in self.creatures.iter_mut().enumerate() {
            if !c.alive {
                continue; // eaten in the predation pass
            }
            let (throttle, turn, _) = decisions[idx];
            // Move.
            c.heading += turn * TURN_RATE * TICK_LEN;
            let step = throttle * c.speed() * TICK_LEN;
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
            // Graze the column (mutates terrain biomass) → energy. Intake scales with body size;
            // a carnivore digests plants poorly (diet efficiency = 1 − carnivory), so the trophic
            // split is a real trade-off, not a free lunch.
            // Food value is scaled by how well the creature's thermal preference matches the
            // local climate — the C3 habitat pressure acts on the dominant (food) channel.
            let taken = terrain.graze(cx, cy, c.intake() * TICK_LEN, tick);
            let herbivory = 1.0 - c.pheno.carnivory();
            let climate = climate_match(terrain.temperature_at(cx, cy), c.genome.thermal_pref);
            c.energy = (c.energy + taken * PLANT_BIOMASS_TO_ENERGY * herbivory * climate).min(c.max_energy());
            // Metabolism: Kleiber (biomass^0.75) + movement effort.
            let kleiber = (c.biomass() as f32).powf(0.75);
            c.energy -= (SIM_BASE_METABOLISM * kleiber + MOVE_COST * throttle) * TICK_LEN;
            c.age += 1;
            // Death by starvation.
            if c.energy <= 0.0 {
                c.alive = false;
                self.deaths += 1;
                continue;
            }
            // Death by senescence: old-age probability rising with age² gives demographic
            // turnover. Scaled by 1/biomass — bigger bodies live longer (a real size benefit),
            // so multicellularity has a gradient to climb against its build + Kleiber costs.
            let sp = SENESCENCE_RATE * (c.age as f32 / LIFESPAN).powi(2) / c.biomass() as f32;
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
                let mut rng = Rng::new(seed_fold(self.world_seed, &[SALT_MUTATE, c.id, tick]));
                // Mutate the genome (brain + GRN) and DEVELOP the child's body up front, so its
                // build cost (energy per cell beyond the first) is known.
                let genome = c.genome.mutate(&mut rng, MUTATION_STD, GRN_MUTATION_STD);
                let pheno = genome.develop();
                c.energy *= 0.5;
                let build = CELL_BIOMASS_COST * pheno.n_cells.saturating_sub(1) as f32;
                let child_energy = c.energy - build;
                // A child the parent can't afford to build is stillborn (the parent still paid
                // half its energy — a real reproductive cost that penalises over-large bodies).
                if child_energy > 0.0 {
                    let child = Creature {
                        id: self.next_id,
                        founder: c.founder,
                        pos: vec2(
                            (c.pos.x + rng.signed() * 2.0).clamp(0.0, maxx),
                            (c.pos.y + rng.signed() * 2.0).clamp(0.0, maxy),
                        ),
                        heading: rng.unit() * std::f32::consts::TAU,
                        energy: child_energy,
                        age: 0,
                        alive: true,
                        genome,
                        pheno,
                    };
                    self.next_id += 1;
                    self.births += 1;
                    births.push(child);
                }
            }
        }
        // (d) compact: drop the dead, append births, cull to the cap.
        self.creatures.retain(|c| c.alive);
        self.creatures.append(&mut births);
        self.cull_to_cap(tick);
    }

    /// Build the brain inputs: the plant-biomass field ahead / left / right (a gradient to
    /// climb), own energy, the column's water distance, the nearest prey + threat cues (closeness
    /// and left/right bearing), and a bias. Read-only on the terrain.
    fn sense(
        &self,
        c: &Creature,
        terrain: &VoxelTerrain,
        tick: u64,
        prey: Option<(f32, f32)>,
        threat: Option<(f32, f32)>,
    ) -> [f32; N_INPUTS] {
        let sample = |angle: f32| {
            let p = vec2(c.pos.x + angle.cos() * SENSE_RADIUS, c.pos.y + angle.sin() * SENSE_RADIUS);
            let (cx, cy) = column_index(p);
            terrain.biomass_at(cx, cy, tick)
        };
        let (cx, cy) = column_index(c.pos);
        let (prey_prox, prey_bearing) = prey.unwrap_or((0.0, 0.0));
        let (threat_prox, threat_bearing) = threat.unwrap_or((0.0, 0.0));
        [
            terrain.biomass_at(cx, cy, tick),
            sample(c.heading),
            sample(c.heading + std::f32::consts::FRAC_PI_2),
            sample(c.heading - std::f32::consts::FRAC_PI_2),
            (c.energy / REPRO_ENERGY).min(1.0),
            terrain.water_dist_at(cx, cy) as f32 / 255.0,
            prey_prox,
            prey_bearing,
            threat_prox,
            threat_bearing,
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

    /// Mean body size (cells) — the emergent biomass; >1 means multicellular bodies took hold.
    pub fn avg_biomass(&self) -> f32 {
        if self.creatures.is_empty() {
            return 0.0;
        }
        self.creatures.iter().map(|c| c.biomass() as f32).sum::<f32>() / self.creatures.len() as f32
    }

    /// Fraction of the population that is multicellular (biomass > 1) and complex (≥2 cell types).
    pub fn complexity_mix(&self) -> (f32, f32) {
        let n = self.creatures.len();
        if n == 0 {
            return (0.0, 0.0);
        }
        let multi = self.creatures.iter().filter(|c| c.pheno.complexity() >= 1).count();
        let complex = self.creatures.iter().filter(|c| c.pheno.complexity() == 2).count();
        (multi as f32 / n as f32, complex as f32 / n as f32)
    }

    /// Allopatry metric: Pearson correlation between each creature's evolved thermal preference
    /// and the actual temperature where it lives. ~0 = no climate adaptation (generalists
    /// everywhere); → 1 = lineages have sorted into the climate band they're suited to (habitats).
    pub fn thermal_correlation(&self, terrain: &VoxelTerrain) -> f32 {
        let n = self.creatures.len();
        if n < 2 {
            return 0.0;
        }
        let mut prefs = Vec::with_capacity(n);
        let mut temps = Vec::with_capacity(n);
        for c in &self.creatures {
            let (cx, cy) = column_index(c.pos);
            prefs.push(c.genome.thermal_pref);
            temps.push(terrain.temperature_at(cx, cy));
        }
        let nf = n as f32;
        let (mp, mt) = (prefs.iter().sum::<f32>() / nf, temps.iter().sum::<f32>() / nf);
        let mut cov = 0.0;
        let mut vp = 0.0;
        let mut vt = 0.0;
        for (&p, &t) in prefs.iter().zip(&temps) {
            cov += (p - mp) * (t - mt);
            vp += (p - mp).powi(2);
            vt += (t - mt).powi(2);
        }
        let denom = (vp * vt).sqrt();
        if denom > 1e-6 {
            cov / denom
        } else {
            0.0
        }
    }

    /// Fraction of the population that is predatory (a second trophic level has appeared).
    pub fn frac_carnivore(&self) -> f32 {
        let n = self.creatures.len();
        if n == 0 {
            return 0.0;
        }
        let carn = self.creatures.iter().filter(|c| c.pheno.carnivory() > CARNIVORE_THRESHOLD).count();
        carn as f32 / n as f32
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

    /// The genome's brain-weight count must match this module's brain topology.
    #[test]
    fn brain_weight_count_matches_topology() {
        assert_eq!(crate::genome::BRAIN_WEIGHTS, N_INPUTS * N_HIDDEN + N_HIDDEN * N_OUTPUTS);
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

    /// C1 acceptance: under the size→longevity gradient, multicellularity EMERGES from the
    /// empty-GRN founders (biomass climbs above 1, a real fraction of the population becomes
    /// multicellular) — the developmental mechanism is exercised live, not just in unit tests —
    /// while the population stays alive and below the cap. Single seed ⇒ deterministic, not flaky.
    #[test]
    fn multicellularity_emerges_under_selection() {
        let mut t = world();
        let mut s = Sim::new(1, &t);
        assert_eq!(s.avg_biomass(), 1.0, "founders must start unicellular (C0 continuity)");
        for tick in 0..5000 {
            s.step(&mut t, tick);
        }
        let (multi, _) = s.complexity_mix();
        let bm = s.avg_biomass();
        eprintln!("after 5000 ticks: pop {} avg_biomass {bm:.3} multi {:.1}%", s.population(), multi * 100.0);
        assert!(bm > 1.1, "multicellularity did not emerge (avg_biomass {bm:.3})");
        assert!(multi > 0.05, "too few multicellular creatures emerged ({:.1}%)", multi * 100.0);
        assert!(s.population() > 100 && s.population() < SIM_POP_CAP, "population unhealthy: {}", s.population());
    }

    /// C2 acceptance: a predatory second trophic level EMERGES — some creatures evolve predator
    /// cells, hunt and kill prey — and predators stay RARER than prey (a trophic pyramid, the
    /// ~10% rule), with the population staying alive. Single seed ⇒ deterministic.
    #[test]
    fn predation_emerges_as_a_trophic_level() {
        let mut t = world();
        let mut s = Sim::new(1, &t);
        for tick in 0..8000 {
            s.step(&mut t, tick);
        }
        let carn = s.frac_carnivore();
        eprintln!("after 8000 ticks: pop {} kills {} carnivore {:.1}%", s.population(), s.kills, carn * 100.0);
        assert!(s.kills > 1000, "no predation happened (kills {})", s.kills);
        assert!(carn > 0.003, "no predator niche persisted ({:.2}%)", carn * 100.0);
        assert!(carn < 0.5, "predators outnumber prey — inverted pyramid ({:.0}%)", carn * 100.0);
        assert!(s.population() > 100 && s.population() < SIM_POP_CAP, "population unhealthy: {}", s.population());
    }

    /// C3-habitats acceptance: lineages sort into the climate band they're adapted to —
    /// the thermal-preference↔local-temperature correlation rises well above 0 (allopatry /
    /// habitats), starting from ~0 (random founders). Single seed ⇒ deterministic.
    #[test]
    fn habitats_emerge_by_climate_adaptation() {
        let mut t = world();
        let mut s = Sim::new(1, &t);
        let start = s.thermal_correlation(&t);
        for tick in 0..6000 {
            s.step(&mut t, tick);
        }
        let end = s.thermal_correlation(&t);
        eprintln!("thermal correlation: start {start:.3} → end {end:.3}");
        assert!(start.abs() < 0.15, "founders should be climate-random (corr {start:.3})");
        assert!(end > 0.3, "no habitat sorting emerged (thermal corr {end:.3})");
    }

    /// Tuning aid (ignored): print the population trajectory for one seed so the energy
    /// constants can be balanced into a food-limited corridor below the cap.
    #[test]
    #[ignore]
    fn tune_trajectory() {
        let mut t = world();
        let mut s = Sim::new(1, &t);
        for tick in 0..12000 {
            s.step(&mut t, tick);
            if tick % 1000 == 0 {
                let (multi, complex) = s.complexity_mix();
                eprintln!(
                    "tick {tick}: pop {} biomass {:.2} multi {:.0}% complex {:.0}% carniv {:.1}% allopatry {:.2} kills {}",
                    s.population(), s.avg_biomass(), multi * 100.0, complex * 100.0, s.frac_carnivore() * 100.0, s.thermal_correlation(&t), s.kills
                );
            }
        }
    }
}
