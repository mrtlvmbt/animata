//! The simulation world: creatures, food, and one `step()` of the sim loop.

use crate::behavior::BehaviorKind;
use crate::biome::{Biome, BiomeMap};
use crate::config::*;
use crate::creature::Creature;
use crate::genome::{seed, Appendage, Genome};
use crate::grid::SpatialGrid;
use crate::phylo::Ancestry;
use crate::speciation::Speciation;
use crate::stats::{Snapshot, Stats};
use macroquad::math::Vec2;
use macroquad::rand::gen_range;
use rayon::prelude::*;
use std::time::{Duration, Instant};

/// Accumulated wall-clock time per phase of `step()`, for profiling.
#[derive(Default)]
pub struct Profile {
    pub spawn: Duration,
    pub grids: Duration,
    /// Neighbour/food/threat queries (read-only).
    pub query: Duration,
    /// Brain + movement + biome lookup (mutating).
    pub act: Duration,
    pub eat: Duration,
    pub hunt: Duration,
    pub reproduce: Duration,
    pub cull: Duration,
    pub stats: Duration,
    pub steps: u64,
}

impl Profile {
    /// One-line per-phase breakdown in microseconds per step.
    #[allow(dead_code)] // used by the headless example, not the GUI binary
    pub fn report(&self) -> String {
        let n = self.steps.max(1) as f64;
        let us = |d: Duration| d.as_secs_f64() * 1e6 / n;
        format!(
            "us/step: spawn {:.1}  grids {:.1}  query {:.1}  act {:.1}  eat {:.1}  hunt {:.1}  repro {:.1}  cull {:.1}  stats {:.1}",
            us(self.spawn), us(self.grids), us(self.query), us(self.act), us(self.eat),
            us(self.hunt), us(self.reproduce), us(self.cull), us(self.stats)
        )
    }
}

pub struct World {
    pub creatures: Vec<Creature>,
    pub food: Vec<Vec2>,
    /// Per-pellet flavor (0..1 niche axis), in lockstep with `food`.
    pub flavor: Vec<f32>,
    pub tick: u64,
    pub stats: Stats,
    pub behavior: BehaviorKind,
    /// Counter for handing out unique creature ids.
    pub next_id: u64,
    /// Live-tunable parameters (driven by the in-app sliders).
    pub params: Params,
    /// Procedural biome map (seeded fertility field).
    pub biome: BiomeMap,
    pub biome_seed: u64,
    /// Full ancestry log (births/deaths of all creatures) for the family tree.
    pub ancestry: Ancestry,
    /// Detected species clusters (phenotype-space), refreshed periodically.
    pub speciation: Speciation,
    /// Tick until which a drought is active (0 = none); set stochastically.
    pub drought_until: u64,
    /// Dominant circulating pathogen strain (0..1), random-walking each step so
    /// host resistance has a moving target (the Red Queen).
    pub circulating_strain: f32,
    /// Per-phase timing accumulators for profiling.
    pub profile: Profile,
    // Reusable per-step scratch (pooled to avoid heap churn each step).
    pub g_food: SpatialGrid,
    pub g_cre: SpatialGrid,
    pub buf_cpos: Vec<Vec2>,
    pub buf_carns: Vec<f32>,
    pub buf_targets: Vec<([Option<Vec2>; 3], f32)>,
}

impl World {
    pub fn new(rng_seed: u64, behavior: BehaviorKind) -> Self {
        seed(rng_seed);
        let biome_seed = rng_seed.wrapping_mul(0x9E37_79B9).wrapping_add(1);
        let biome = BiomeMap::new(biome_seed);
        // Diet is a gene, but founders start as herbivores (carnivory gene zeroed)
        // so there's no startup glut of carnivores over-hunting the world into
        // collapse. Carnivory then has to evolve upward via mutation.
        let carn_start = 8 * NT_PER_GENE;
        let mut creatures: Vec<Creature> = (0..START_CREATURES + START_PREDATORS)
            .map(|_| {
                let mut g = Genome::random();
                for k in carn_start..carn_start + NT_PER_GENE {
                    g.nt[k] = 0;
                }
                // Found each creature already adapted to its birthplace's food:
                // its diet niche is set to the local biome flavor. Otherwise a
                // random niche starves everywhere and the population collapses.
                // Divergence then emerges as descendants spread to other biomes.
                let p = land_pos(&biome);
                set_gene(&mut g, 12, biome.props_at(p).flavor);
                // Found with a responsive brain (memory-leak γ=1 -> plain Elman,
                // the proven reactive behavior); slow-memory (low γ) then evolves
                // in only where it pays, instead of sluggish founders dying out.
                set_gene(&mut g, 13, 1.0);
                Creature::new(g, p, START_ENERGY, 0, behavior)
            })
            .collect();
        // Seed an initial disease load: a fraction of founders carry a random
        // strain, so the host-parasite arms race has something to start from.
        let circulating_strain = gen_range(0.0f32, 1.0);
        let n_inf = (creatures.len() as f32 * START_INFECTED_FRAC) as usize;
        for c in creatures.iter_mut().take(n_inf) {
            c.infection = Some((circulating_strain + gen_range(-STRAIN_MUT, STRAIN_MUT)).clamp(0.0, 1.0));
        }
        let (mut food, mut flavor) = (Vec::with_capacity(START_FOOD), Vec::with_capacity(START_FOOD));
        for _ in 0..START_FOOD {
            let p = land_pos(&biome);
            flavor.push(pellet_flavor(&biome, p));
            food.push(p);
        }
        // Found the population with a spread of adult ages (not all newborns),
        // so reproduction starts immediately instead of stalling for a whole
        // juvenile period while predators thin the founders.
        for (i, c) in creatures.iter_mut().enumerate() {
            c.id = i as u64;
            c.lineage = i as u32; // each founder seeds its own lineage
            let maturity = c.pheno.prime * MATURITY_FRAC;
            c.age = gen_range(maturity, c.pheno.prime) as u32;
        }
        let next_id = creatures.len() as u64;
        let mut ancestry = Ancestry::new();
        for c in &creatures {
            ancestry.record_birth(c.id, None, 0, c.lineage);
        }
        let mut w = World {
            creatures,
            food,
            flavor,
            tick: 0,
            stats: Stats::new(),
            behavior,
            next_id,
            params: Params::default(),
            biome,
            biome_seed,
            ancestry,
            speciation: Speciation::new(),
            drought_until: 0,
            circulating_strain,
            profile: Profile::default(),
            g_food: SpatialGrid::default(),
            g_cre: SpatialGrid::default(),
            buf_cpos: Vec::new(),
            buf_carns: Vec::new(),
            buf_targets: Vec::new(),
        };
        w.record_stats();
        w
    }

    fn fresh_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Look up a creature by its stable id (used by the inspector UI).
    pub fn creature_by_id(&self, id: u64) -> Option<&Creature> {
        self.creatures.iter().find(|c| c.id == id)
    }

    /// Seasonal × drought food-spawn multiplier for the current tick.
    pub fn env_food_mult(&self) -> f32 {
        use std::f32::consts::TAU;
        let season = SEASON_BASE + SEASON_AMP * (TAU * self.tick as f32 / SEASON_PERIOD).sin();
        let drought = if self.tick < self.drought_until { DROUGHT_FOOD_MULT } else { 1.0 };
        (season * drought).max(0.0)
    }

    pub fn in_drought(&self) -> bool {
        self.tick < self.drought_until
    }

    /// Seasonal phase in `-1..=1` (>0 = bounty, <0 = lean), for UI.
    pub fn season_phase(&self) -> f32 {
        use std::f32::consts::TAU;
        (TAU * self.tick as f32 / SEASON_PERIOD).sin()
    }

    pub fn step(&mut self) {
        // Maybe begin a drought (only when none is active).
        if self.tick >= self.drought_until && gen_range(0.0f64, 1.0) < DROUGHT_CHANCE {
            self.drought_until = self.tick + DROUGHT_LEN + gen_range(0, DROUGHT_LEN);
        }

        let mut t = Instant::now();
        let mut lap = |slot: &mut Duration| {
            let now = Instant::now();
            *slot += now - t;
            t = now;
        };

        self.spawn_food();
        lap(&mut self.profile.spawn);

        // Take pooled scratch out of `self` so the phase methods can borrow `self`
        // freely; everything is returned at the end of the step (no per-step alloc).
        let mut food_grid = std::mem::take(&mut self.g_food);
        let mut cgrid = std::mem::take(&mut self.g_cre);
        let mut cpos = std::mem::take(&mut self.buf_cpos);
        let mut carns = std::mem::take(&mut self.buf_carns);
        let mut targets = std::mem::take(&mut self.buf_targets);

        cpos.clear();
        cpos.extend(self.creatures.iter().map(|c| c.pos));
        carns.clear();
        carns.extend(self.creatures.iter().map(|c| c.carnivory()));
        // Vertical layer of each creature (its morphological stratum), plus the
        // per-layer population for density-split foraging on the non-surface tiers.
        let clayers: Vec<u8> = self.creatures.iter().map(|c| c.layer).collect();
        let mut layer_pop = [0u32; N_LAYERS];
        for &l in &clayers {
            layer_pop[l as usize] += 1;
        }
        food_grid.rebuild(&self.food, WORLD_W, WORLD_H, GRID_CELL);
        cgrid.rebuild(&cpos, WORLD_W, WORLD_H, GRID_CELL);
        lap(&mut self.profile.grids);

        self.sense_into(&mut targets, &food_grid, &cgrid, &cpos, &carns, &clayers);
        lap(&mut self.profile.query);
        self.act_all(&targets);
        lap(&mut self.profile.act);
        self.eat_food(&food_grid, layer_pop);
        lap(&mut self.profile.eat);
        let killed = self.hunt(&cgrid, &cpos, &carns, &clayers);
        lap(&mut self.profile.hunt);
        // Disease spread runs while grid indices still match `self.creatures`
        // (before cull removes anyone).
        self.infect(&cgrid, &cpos);
        self.cull(killed);
        lap(&mut self.profile.cull);
        self.reproduce();
        lap(&mut self.profile.reproduce);
        self.profile.steps += 1;

        // Return pooled scratch for reuse next step.
        self.g_food = food_grid;
        self.g_cre = cgrid;
        self.buf_cpos = cpos;
        self.buf_carns = carns;
        self.buf_targets = targets;

        self.tick += 1;
        if self.tick % 5 == 0 {
            self.record_stats();
            lap(&mut self.profile.stats);
        }
        // Periodically GC the ancestry log down to the living population's
        // ancestors so it stays bounded and the tree always reaches founders.
        if self.tick % 500 == 0 {
            let living: Vec<u64> = self.creatures.iter().map(|c| c.id).collect();
            self.ancestry.prune(&living);
        }
        // Re-cluster creatures into species periodically (not every step).
        if self.tick % 50 == 0 {
            let mut sp = std::mem::take(&mut self.speciation);
            sp.update(&mut self.creatures);
            self.speciation = sp;
        }
    }

    fn spawn_food(&mut self) {
        // Probabilistic count so non-integer rates work; scaled by season/drought.
        let rate = self.params.food_per_step * self.env_food_mult();
        let mut to_add = rate.floor() as i32;
        if gen_range(0.0, 1.0) < rate.fract() {
            to_add += 1;
        }
        for _ in 0..to_add {
            if self.food.len() >= FOOD_CAP {
                break;
            }
            let p = self.fertile_pos();
            self.flavor.push(pellet_flavor(&self.biome, p));
            self.food.push(p);
        }
    }

    /// A random position, biased toward fertile biomes via rejection sampling.
    /// Falls back to the last candidate after a few tries so spawning never stalls.
    fn fertile_pos(&self) -> Vec2 {
        let mut p = rand_pos();
        for _ in 0..6 {
            let accept = self.biome.props_at(p).food_mult / BIOME_MAX_FOOD_MULT;
            if gen_range(0.0, 1.0) < accept {
                break;
            }
            p = rand_pos();
        }
        p
    }

    /// Read-only pass: for each creature compute the relative offset to its food,
    /// its threat and a similar-diet neighbor. Diet is the carnivory gene:
    /// herbivore-leaning sense pellets, carnivore-leaning sense prey (lower on the
    /// food chain); the threat is the nearest creature that could eat *them*.
    /// Every search is bounded by the creature's sense range (local, not global).
    fn sense_into(
        &self,
        out: &mut Vec<([Option<Vec2>; 3], f32)>,
        food_grid: &SpatialGrid,
        cgrid: &SpatialGrid,
        cpos: &[Vec2],
        carns: &[f32],
        clayers: &[u8],
    ) {
        // Precompute sense ranges + emitted signals so the parallel closure
        // captures only `Sync` data (not `self`/`Creature`, which holds a
        // non-`Sync` `Box<dyn Behavior>`).
        let senses: Vec<f32> = self.creatures.iter().map(|c| c.pheno.sense_range).collect();
        let signals: Vec<f32> = self.creatures.iter().map(|c| c.signal).collect();
        let niches: Vec<f32> = self.creatures.iter().map(|c| c.pheno.diet_niche).collect();
        let pellets: &[Vec2] = &self.food;
        let flavors: &[f32] = &self.flavor;
        // |flavor - niche| beyond this digests below MIN_EAT_EFF -> ignore it.
        let max_flavor_d2 = -2.0 * DIET_WIDTH * DIET_WIDTH * (MIN_EAT_EFF as f32).ln();

        // Read-only and per-creature independent (no RNG, no mutation) -> runs in
        // parallel; this is the heaviest phase at large populations.
        (0..senses.len())
            .into_par_iter()
            .map(|i| {
                let pos = cpos[i];
                let ci = carns[i];
                let sense = senses[i];
                let li = clayers[i];

                // Interactions are within a layer: only same-layer creatures are
                // threats, neighbours or prey, so a non-surface stratum is a refuge.
                let (threat_i, neighbor_i) = cgrid.nearest2_within(
                    cpos,
                    pos,
                    sense,
                    |k| k != i && clayers[k] == li && carns[k] >= ci + PREY_MARGIN,
                    |k| k != i && clayers[k] == li && (carns[k] - ci).abs() < 0.15,
                );
                let food = if ci < 0.5 {
                    // Positioned pellets exist only on the surface; non-surface
                    // herbivores forage their layer's capacity (see eat_food) and
                    // sense no pellets.
                    if li == LAYER_SURFACE {
                        let niche = niches[i];
                        food_grid
                            .nearest_within(pellets, pos, sense, |j| {
                                let d = flavors[j] - niche;
                                d * d <= max_flavor_d2
                            })
                            .map(|j| pellets[j] - pos)
                    } else {
                        None
                    }
                } else {
                    cgrid
                        .nearest_within(cpos, pos, sense, |k| {
                            k != i && clayers[k] == li && carns[k] <= ci - PREY_MARGIN
                        })
                        .map(|k| cpos[k] - pos)
                };
                // Hear the nearest neighbor's emitted signal.
                let heard = neighbor_i.map_or(0.0, |k| signals[k]);
                (
                    [
                        food,
                        threat_i.map(|k| cpos[k] - pos),
                        neighbor_i.map(|k| cpos[k] - pos),
                    ],
                    heard,
                )
            })
            .collect_into_vec(out);
    }

    /// Mutating pass: each creature thinks and moves given its sensed targets and
    /// the biome it stands in. Parallel for the neural brain (no shared state, no
    /// RNG); serial for rule-based (its wander uses the global RNG).
    fn act_all(&mut self, targets: &[([Option<Vec2>; 3], f32)]) {
        let biome = &self.biome;
        if matches!(self.behavior, BehaviorKind::Neural) {
            self.creatures.par_iter_mut().enumerate().for_each(|(i, c)| {
                let b = biome.at(c.pos);
                let bp = b.props();
                let ([food, threat, neighbor], heard) = targets[i];
                c.think_and_act(food, threat, neighbor, heard, bp.move_mult, bp.metab_mult, b.medium());
            });
        } else {
            for i in 0..self.creatures.len() {
                let b = biome.at(self.creatures[i].pos);
                let bp = b.props();
                let ([food, threat, neighbor], heard) = targets[i];
                self.creatures[i].think_and_act(food, threat, neighbor, heard, bp.move_mult, bp.metab_mult, b.medium());
            }
        }
    }

    /// Any creature eats a pellet within reach (one per step), gaining energy
    /// scaled by its herbivory `(1 - carnivory)`. Near-pure carnivores ignore
    /// pellets so they don't waste them.
    fn eat_food(&mut self, grid: &SpatialGrid, layer_pop: [u32; N_LAYERS]) {
        // Per-occupant foraging yield on the non-surface layers: a fixed capacity
        // split among everyone currently there (density-dependent, self-limiting).
        let benthic_yield = BENTHIC_CAPACITY / layer_pop[LAYER_UNDERGROUND as usize].max(1) as f32;
        let aerial_yield = AERIAL_CAPACITY / layer_pop[LAYER_AIR as usize].max(1) as f32;

        let food = &self.food;
        let flavor = &self.flavor;
        let mut eaten = vec![false; food.len()];
        for c in &mut self.creatures {
            let herbivory = 1.0 - c.carnivory();
            if herbivory < 0.1 {
                continue;
            }
            // Off the surface there are no positioned pellets — forage the layer's
            // shared capacity instead (scaled by herbivory, like grazing).
            if c.layer != LAYER_SURFACE {
                let y = if c.layer == LAYER_UNDERGROUND { benthic_yield } else { aerial_yield };
                c.energy += y * herbivory;
                continue;
            }
            let reach = c.pheno.radius + FOOD_RADIUS;
            let reach2 = reach * reach;
            let pos = c.pos;
            // Pick the nearest pellet within reach that this creature can
            // actually digest (efficiency above the threshold); it leaves food
            // outside its niche for the specialists that can use it.
            let mut got: Option<(usize, f32)> = None;
            grid.for_each_near(pos, |idx| {
                if got.is_none() && !eaten[idx] && (food[idx] - pos).length_squared() <= reach2 {
                    let eff = c.pheno.diet_efficiency(flavor[idx]);
                    if eff >= MIN_EAT_EFF {
                        got = Some((idx, eff));
                    }
                }
            });
            if let Some((idx, eff)) = got {
                c.energy += FOOD_ENERGY * herbivory * eff;
                eaten[idx] = true;
            }
        }
        let mut j = 0;
        self.food.retain(|_| {
            let keep = !eaten[j];
            j += 1;
            keep
        });
        let mut k = 0;
        self.flavor.retain(|_| {
            let keep = !eaten[k];
            k += 1;
            keep
        });
    }

    /// Carnivory-driven hunting: a hunter catches a nearby creature lower on the
    /// food chain (carnivory at least `PREY_MARGIN` below its own), gaining energy
    /// scaled by its carnivory. Returns a per-creature "killed this step" mask.
    fn hunt(&mut self, cgrid: &SpatialGrid, cpos: &[Vec2], carns: &[f32], clayers: &[u8]) -> Vec<bool> {
        let mut killed = vec![false; self.creatures.len()];
        for i in 0..self.creatures.len() {
            let ci = carns[i];
            if ci < HUNT_MIN_CARNIVORY {
                continue;
            }
            let pos = self.creatures[i].pos;
            let li = clayers[i];
            let reach = self.creatures[i].pheno.radius + PREDATOR_CATCH_PAD;
            let reach2 = reach * reach;
            let mut victim: Option<usize> = None;
            cgrid.for_each_near(pos, |k| {
                if victim.is_none()
                    && k != i
                    && !killed[k]
                    && clayers[k] == li
                    && carns[k] <= ci - PREY_MARGIN
                    && (cpos[k] - pos).length_squared() <= reach2
                {
                    victim = Some(k);
                }
            });
            if let Some(k) = victim {
                killed[k] = true;
                self.creatures[i].energy += self.params.predator_gain * ci;
            }
        }
        killed
    }

    /// Host–parasite spread (Red Queen): infections drain energy by how poorly the
    /// host's `resistance` matches the pathogen `strain`, spread to healthy
    /// neighbours (the strain drifting on transmission to escape resistance), and
    /// clear over time. `cgrid`/`cpos` index `self.creatures` 1:1 (pre-cull).
    fn infect(&mut self, cgrid: &SpatialGrid, cpos: &[Vec2]) {
        let n = self.creatures.len();
        // The reservoir strain random-walks (reflected at 0/1), so resistance
        // that catches up is eventually escaped — the Red Queen never settles.
        let drifted = self.circulating_strain + gen_range(-STRAIN_DRIFT, STRAIN_DRIFT);
        self.circulating_strain = if drifted < 0.0 {
            -drifted
        } else if drifted > 1.0 {
            2.0 - drifted
        } else {
            drifted
        };
        let circ = self.circulating_strain;
        let strains: Vec<Option<f32>> = self.creatures.iter().map(|c| c.infection).collect();
        let r2 = INFECT_RADIUS * INFECT_RADIUS;

        // Plan new infections (read-only over current state).
        let mut new_inf: Vec<Option<f32>> = vec![None; n];
        for i in 0..n {
            let Some(s) = strains[i] else { continue };
            cgrid.for_each_near(cpos[i], |k| {
                if k != i
                    && strains[k].is_none()
                    && new_inf[k].is_none()
                    && (cpos[k] - cpos[i]).length_squared() <= r2
                    && gen_range(0.0f64, 1.0) < INFECT_CHANCE
                {
                    new_inf[k] = Some((s + gen_range(-STRAIN_MUT, STRAIN_MUT)).clamp(0.0, 1.0));
                }
            });
        }

        // Apply: damage + recovery for the infected, then seat new infections.
        for i in 0..n {
            let c = &mut self.creatures[i];
            if let Some(s) = c.infection {
                let prot = (-(c.pheno.resistance - s).powi(2) / PROTECT_WIDTH).exp();
                c.energy -= INFECTION_DAMAGE * (1.0 - prot);
                if gen_range(0.0f64, 1.0) < RECOVER_CHANCE {
                    c.infection = None;
                }
            } else if let Some(ns) = new_inf[i] {
                c.infection = Some(ns);
            } else if gen_range(0.0f64, 1.0) < BACKGROUND_INFECT {
                // Environmental pickup of the circulating strain.
                c.infection = Some((circ + gen_range(-STRAIN_MUT, STRAIN_MUT)).clamp(0.0, 1.0));
            }
        }
    }

    /// Fertile creatures pair with the nearest fertile same-species partner
    /// (single-point crossover + mutation). With no partner in range they clone
    /// themselves. Handled per species so herbivores and predators never mix.
    fn reproduce(&mut self) {
        let mut babies = Vec::new();
        // Fertile creatures: positions, world indices, carnivory (for assortative
        // mating so distinct diets don't constantly hybridize away).
        let mut pos = Vec::new();
        let mut world = Vec::new();
        let mut carn = Vec::new();
        let mut ornament = Vec::new();
        let mut preference = Vec::new();
        for (i, c) in self.creatures.iter().enumerate() {
            if c.wants_to_reproduce() {
                pos.push(c.pos);
                world.push(i);
                carn.push(c.carnivory());
                ornament.push(c.pheno.ornament);
                preference.push(c.pheno.preference);
            }
        }
        if pos.is_empty() {
            return;
        }
        let grid = SpatialGrid::build(&pos, WORLD_W, WORLD_H, GRID_CELL);
        let mut_rate = self.params.mutation_rate;
        let mate_range2 = MATE_RANGE * MATE_RANGE;
        let mut mated = vec![false; pos.len()];

        for a in 0..pos.len() {
            if mated[a] || self.creatures.len() + babies.len() >= POP_CAP {
                continue;
            }
            // Mate choice: among fertile, unmated, similar-diet candidates in
            // range, pick by sexual-selection score = chooser's preference ×
            // candidate's ornament (minus a tiny distance term as tiebreak). With
            // low preference this reduces to "nearest" — driving Fisherian runaway.
            let (ca, pref) = (carn[a], preference[a]);
            let mut best: Option<(usize, f32)> = None;
            grid.for_each_near(pos[a], |k| {
                if k == a || mated[k] || (carn[k] - ca).abs() >= 0.25 {
                    return;
                }
                let d2 = (pos[k] - pos[a]).length_squared();
                if d2 > mate_range2 {
                    return;
                }
                let score = pref * ornament[k] - d2 * 1e-4;
                if best.map_or(true, |(_, bs)| score > bs) {
                    best = Some((k, score));
                }
            });
            match best {
                Some((b, _)) => {
                    mated[a] = true;
                    mated[b] = true;
                    babies.push(self.breed(world[a], world[b]));
                }
                None => {
                    // No suitable mate nearby: asexual clone.
                    babies.push(self.creatures[world[a]].reproduce(mut_rate));
                }
            }
        }
        for b in &mut babies {
            b.id = self.fresh_id();
            self.ancestry.record_birth(b.id, b.parent_id, self.tick, b.lineage);
        }
        self.creatures.append(&mut babies);
    }

    /// Produce one child of two parents (by world index) and charge both.
    fn breed(&mut self, wa: usize, wb: usize) -> Creature {
        let genome = Genome::crossover(&self.creatures[wa].genome, &self.creatures[wb].genome)
            .mutated(self.params.mutation_rate);
        let pos = (self.creatures[wa].pos + self.creatures[wb].pos) * 0.5
            + Vec2::new(gen_range(-6.0, 6.0), gen_range(-6.0, 6.0));
        let generation = self.creatures[wa].generation.max(self.creatures[wb].generation) + 1;
        let kind = self.creatures[wa].kind;

        let parent_id = Some(self.creatures[wa].id);
        let ea = self.creatures[wa].energy;
        let eb = self.creatures[wb].energy;
        self.creatures[wa].energy = ea * 0.5;
        self.creatures[wb].energy = eb * 0.5;
        let child_energy = (ea + eb) * 0.25;

        let mut child = Creature::new(genome, pos, child_energy, generation, kind);
        child.parent_id = parent_id;
        child.lineage = self.creatures[wa].lineage;
        child.species_id = self.creatures[wa].species_id;
        child
    }

    /// Remove creatures killed by predators this step, starved to death, or
    /// taken by old age. Records each death in the ancestry log.
    fn cull(&mut self, killed: Vec<bool>) {
        // Decide removals once (dies_of_age is random — must not roll twice).
        let mut remove = vec![false; self.creatures.len()];
        for i in 0..self.creatures.len() {
            let dead = {
                let c = &self.creatures[i];
                killed[i] || c.is_dead() || c.dies_of_age()
            };
            if dead {
                remove[i] = true;
                let id = self.creatures[i].id;
                self.ancestry.record_death(id, self.tick);
            }
        }
        let mut i = 0;
        self.creatures.retain(|_| {
            let keep = !remove[i];
            i += 1;
            keep
        });
    }

    fn record_stats(&mut self) {
        // Trait averages over the whole population; "herbivores"/"predators" are
        // now just carnivory buckets (c < 0.5 vs >= 0.5) for the HUD/graph.
        let mut herb = 0u32;
        let mut predators = 0;
        let mut speed = 0.0;
        let mut sense = 0.0;
        let mut radius = 0.0;
        let mut metab = 0.0;
        let mut carn = 0.0;
        let mut ornament = 0.0;
        let mut signal = 0.0;
        let mut resistance = 0.0;
        let mut infected = 0u32;
        let mut memory = 0.0;
        let mut niche = 0.0;
        let mut niche_sq = 0.0;
        let mut segments = 0.0;
        let mut appendaged = 0u32;
        let mut n_under = 0u32;
        let mut n_air = 0u32;
        // Sums of squares of *normalized* traits, for the diversity (std-dev) metric.
        let mut sq = [0.0f32; 4];
        let mut sum_n = [0.0f32; 4];
        let mut gen = 0u32;
        for c in &self.creatures {
            gen = gen.max(c.generation);
            if c.carnivory() < 0.5 {
                herb += 1;
            } else {
                predators += 1;
            }
            speed += c.pheno.max_speed;
            sense += c.pheno.sense_range;
            radius += c.pheno.radius;
            metab += c.pheno.metabolism;
            carn += c.carnivory();
            ornament += c.pheno.ornament;
            signal += c.signal;
            resistance += c.pheno.resistance;
            if c.infection.is_some() {
                infected += 1;
            }
            memory += c.memory_use();
            segments += c.pheno.segments.len() as f32;
            if c.pheno.segments.iter().any(|s| s.appendage != Appendage::None) {
                appendaged += 1;
            }
            match c.layer {
                LAYER_UNDERGROUND => n_under += 1,
                LAYER_AIR => n_air += 1,
                _ => {}
            }
            let dn = c.pheno.diet_niche;
            niche += dn;
            niche_sq += dn * dn;
            let nv = [
                norm(c.pheno.max_speed, SPEED_RANGE),
                norm(c.pheno.sense_range, SENSE_RANGE),
                norm(c.pheno.radius, RADIUS_RANGE),
                norm(c.pheno.metabolism, METAB_RANGE),
            ];
            for k in 0..4 {
                sum_n[k] += nv[k];
                sq[k] += nv[k] * nv[k];
            }
        }
        // Distinct gene-lineages (the "clades" count).
        let mut lin: Vec<u32> = self.creatures.iter().map(|c| c.lineage).collect();
        lin.sort_unstable();
        lin.dedup();
        let lineages = lin.len();
        // Top species by population, for the Muller plot — species stay diverse
        // even after gene-lineages coalesce to one, so the plot is informative.
        let mut counts: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        for c in &self.creatures {
            *counts.entry(c.species_id).or_insert(0) += 1;
        }
        let mut top: Vec<(u32, u32)> = counts.into_iter().collect();
        top.sort_unstable_by(|a, b| b.1.cmp(&a.1)); // by count desc
        top.truncate(12);

        let n = self.creatures.len().max(1) as f32;
        // Diversity = mean over traits of the population std-dev (0 = monoculture).
        let mut std_sum = 0.0;
        for k in 0..4 {
            let mean = sum_n[k] / n;
            let var = (sq[k] / n - mean * mean).max(0.0);
            std_sum += var.sqrt();
        }
        let snap = Snapshot {
            population: self.creatures.len(),
            herbivores: herb as usize,
            predators,
            avg_speed: speed / n,
            avg_sense: sense / n,
            avg_radius: radius / n,
            avg_metabolism: metab / n,
            avg_carnivory: carn / n,
            avg_ornament: ornament / n,
            avg_signal: signal / n,
            avg_resistance: resistance / n,
            infected_frac: infected as f32 / n,
            avg_memory: memory / n,
            avg_segments: segments / n,
            appendaged_frac: appendaged as f32 / n,
            frac_underground: n_under as f32 / n,
            frac_air: n_air as f32 / n,
            avg_niche: niche / n,
            niche_spread: (niche_sq / n - (niche / n) * (niche / n)).max(0.0).sqrt(),
            diversity: std_sum / 4.0,
            lineages,
            species: self.speciation.count(),
            max_generation: gen,
        };
        self.stats.push(snap, top);
    }

    /// Add a food pellet at an arbitrary point (used by mouse input).
    pub fn add_food_at(&mut self, p: Vec2) {
        if self.food.len() < FOOD_CAP {
            self.flavor.push(pellet_flavor(&self.biome, p));
            self.food.push(p);
        }
    }
}

fn rand_pos() -> Vec2 {
    Vec2::new(gen_range(0.0, WORLD_W), gen_range(0.0, WORLD_H))
}

/// Write `value` (0..1) into gene `index` of a genome, encoding it base-4 across
/// its `NT_PER_GENE` nucleotides (matches `Genome::gene_u8`'s big-endian read).
fn set_gene(g: &mut Genome, index: usize, value: f32) {
    let mut x = (value.clamp(0.0, 1.0) * 255.0).round() as u32;
    let start = index * NT_PER_GENE;
    for i in (0..NT_PER_GENE).rev() {
        g.nt[start + i] = (x % 4) as u8;
        x /= 4;
    }
}

/// Flavor of a pellet at `p`: its biome's flavor plus a little noise.
fn pellet_flavor(biome: &BiomeMap, p: Vec2) -> f32 {
    (biome.props_at(p).flavor + gen_range(-FOOD_FLAVOR_NOISE, FOOD_FLAVOR_NOISE)).clamp(0.0, 1.0)
}

/// A random position not in water, so creatures/food don't start stranded mid-river.
fn land_pos(biome: &BiomeMap) -> Vec2 {
    let mut p = rand_pos();
    for _ in 0..8 {
        if biome.at(p) != Biome::Water {
            return p;
        }
        p = rand_pos();
    }
    p
}

/// Normalize a trait value into 0..=1 over its configured range.
fn norm(v: f32, range: (f32, f32)) -> f32 {
    ((v - range.0) / (range.1 - range.0)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sim_runs_without_explosion_or_instant_extinction() {
        let mut w = World::new(123, BehaviorKind::Neural);
        for _ in 0..800 {
            w.step();
        }
        // Not catastrophically broken: some creatures survive, population bounded,
        // food bounded, and a few generations have passed (reproduction works).
        assert!(!w.creatures.is_empty(), "population went extinct");
        assert!(w.creatures.len() <= POP_CAP);
        assert!(w.food.len() <= FOOD_CAP);
        assert!(w.stats.latest().max_generation >= 1, "no reproduction occurred");
    }

    #[test]
    fn ancestry_tree_reaches_a_founder_after_pruning() {
        // Long enough for several prune passes (every 500 ticks).
        let mut w = World::new(7, BehaviorKind::Neural);
        for _ in 0..1600 {
            w.step();
            if w.creatures.is_empty() {
                return; // extinct run; nothing to assert
            }
        }
        let living: Vec<u64> = w.creatures.iter().map(|c| c.id).collect();
        let nodes = w.ancestry.coalescent(&living);
        // The living population's genealogy must reach at least one real root
        // (a founder with no parent) — i.e. the tree isn't a broken forest.
        assert!(
            nodes.iter().any(|n| n.parent.is_none()),
            "coalescent tree never reaches a founder"
        );
        // Pruning keeps the log bounded (no unbounded growth over the run).
        assert!(w.ancestry.len() <= 200_000, "ancestry log not bounded by pruning");
    }

    #[test]
    fn tree_root_count_matches_distinct_lineages() {
        let mut w = World::new(7, BehaviorKind::Neural);
        for _ in 0..3000 {
            w.step();
            if w.creatures.is_empty() {
                return;
            }
        }
        let living: Vec<u64> = w.creatures.iter().map(|c| c.id).collect();
        let nodes = w.ancestry.coalescent(&living);
        let set: std::collections::HashSet<u64> = nodes.iter().map(|n| n.id).collect();
        let true_roots = nodes.iter().filter(|n| n.parent.is_none()).count();
        let broken = nodes
            .iter()
            .filter(|n| n.parent.map_or(false, |p| !set.contains(&p)))
            .count();
        let mut lin: Vec<u32> = w.creatures.iter().map(|c| c.lineage).collect();
        lin.sort_unstable();
        lin.dedup();
        // parent_id and lineage both follow parent A, so the genealogy must be one
        // clean tree per surviving lineage with no broken chains.
        assert_eq!(broken, 0, "tree has {broken} broken chains (pruned ancestors)");
        assert_eq!(true_roots, lin.len(), "roots {} != distinct lineages {}", true_roots, lin.len());
    }

    #[test]
    fn rule_behavior_also_survives() {
        let mut w = World::new(123, BehaviorKind::Rule);
        for _ in 0..800 {
            w.step();
        }
        assert!(!w.creatures.is_empty(), "rule-based population went extinct");
        assert!(w.stats.latest().max_generation >= 1);
    }
}
