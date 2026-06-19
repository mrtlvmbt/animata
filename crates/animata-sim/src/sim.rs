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

use glam::{vec2, Vec2};

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
const SALT_CAMO: u64 = 0xCA30;

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

    /// Evolved body coloration `[0,1]` (for the camouflage render tint).
    pub fn coloration(&self) -> f32 {
        self.genome.coloration
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

/// Dimensions of the species feature vector: 7 cell-type fractions (the developmental body plan)
/// plus normalised size. Speciation is by BODY, per the plan ("topological speciation on the
/// developmental bodies") — climate/colour are continuous within-species niche traits, not here.
const FEATURES: usize = 8;

/// The body-plan feature vector a creature is clustered by into a species — its cell-type
/// composition and size. Each component is ~`[0,1]`.
fn feature(c: &Creature) -> [f32; FEATURES] {
    let p = &c.pheno;
    [
        p.effector as f32 / p.n_cells as f32,
        p.storage as f32 / p.n_cells as f32,
        p.sensor as f32 / p.n_cells as f32,
        p.predator as f32 / p.n_cells as f32,
        p.flight as f32 / p.n_cells as f32,
        p.burrow as f32 / p.n_cells as f32,
        p.photo as f32 / p.n_cells as f32,
        (p.n_cells as f32 / crate::genome::MAX_CELLS as f32).min(1.0),
    ]
}

/// Squared Euclidean distance between two feature vectors.
fn feature_dist2(a: &[f32; FEATURES], b: &[f32; FEATURES]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y).powi(2)).sum()
}

/// Pearson correlation of two equal-length samples (`0` if undefined). Shared by the niche
/// metrics — how well an evolved trait tracks the local environment (allopatry, crypsis).
fn pearson(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len() as f32;
    if a.len() < 2 {
        return 0.0;
    }
    let (ma, mb) = (a.iter().sum::<f32>() / n, b.iter().sum::<f32>() / n);
    let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
    for (&x, &y) in a.iter().zip(b) {
        cov += (x - ma) * (y - mb);
        va += (x - ma).powi(2);
        vb += (y - mb).powi(2);
    }
    let denom = (va * vb).sqrt();
    if denom > 1e-6 {
        cov / denom
    } else {
        0.0
    }
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

/// Vertical strata (C3). Which one a creature occupies is set by its morphology + where it
/// stands: flight cells → Air, burrow cells → Underground, fins over a water column → Water,
/// else the Surface base layer. Each is a distinct niche — its own food source and a predator
/// refuge (predators only hunt within their own stratum).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Stratum {
    Underground,
    Surface,
    Air,
    Water,
}

impl Stratum {
    fn idx(self) -> usize {
        match self {
            Stratum::Underground => 0,
            Stratum::Surface => 1,
            Stratum::Air => 2,
            Stratum::Water => 3,
        }
    }

    /// Metabolic multiplier for living here — flight is dear (lift), burrowing cheap (sheltered).
    fn metab_mult(self) -> f32 {
        match self {
            Stratum::Air => AIR_METAB_MULT,
            Stratum::Underground => UNDERGROUND_METAB_MULT,
            _ => 1.0,
        }
    }

    /// Total non-surface foraging yield (energy / sim-second) of this stratum, split among its
    /// occupants (so an empty stratum richly rewards the first colonisers, then self-limits).
    /// Surface returns 0 here — it feeds from the positioned S3 plant field instead.
    fn capacity(self) -> f32 {
        match self {
            Stratum::Underground => UNDERGROUND_CAPACITY,
            Stratum::Air => AIR_CAPACITY,
            Stratum::Water => WATER_CAPACITY,
            Stratum::Surface => 0.0,
        }
    }
}

/// The stratum a creature occupies, from its body and whether its column is water. Priority
/// Air > Underground > Water > Surface (a body able to fly uses the air even over water).
fn stratum_of(pheno: &Phenotype, is_water_col: bool) -> Stratum {
    if pheno.flight_frac() > STRATUM_THETA {
        Stratum::Air
    } else if pheno.burrow_frac() > STRATUM_THETA {
        Stratum::Underground
    } else if is_water_col && pheno.fin_frac() > STRATUM_THETA {
        Stratum::Water
    } else {
        Stratum::Surface
    }
}

/// Light available to photosynthesis at a creature's stratum, latitude row and tick, in `[0,1]`.
/// Activates the dormant S2 day/night (a sinusoid in `day_frac` with a dim-night floor) and the
/// S1 latitude (poles dimmer). Underground = no light; water = attenuated; surface/air = full.
fn light_for(stratum: Stratum, cy: usize, tick: u64) -> f32 {
    if stratum == Stratum::Underground {
        return 0.0;
    }
    let day_frac = (tick as f64 * TICK_LEN as f64 / DAY_LEN as f64).fract() as f32;
    let daylight = LIGHT_NIGHT_FLOOR
        + (1.0 - LIGHT_NIGHT_FLOOR) * (0.5 + 0.5 * (std::f32::consts::TAU * day_frac).sin());
    let lat = 1.0 - (2.0 * cy as f32 / ROWS as f32 - 1.0).abs(); // 0 poles .. 1 equator
    let l = daylight * (0.4 + 0.6 * lat);
    if stratum == Stratum::Water {
        l * WATER_LIGHT_MULT
    } else {
        l
    }
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
        let color: Vec<f32> = self.creatures.iter().map(|c| c.genome.coloration).collect();
        // Ground tone each creature stands on (camouflage background, snapshot — read-only).
        let bg: Vec<f32> = self
            .creatures
            .iter()
            .map(|c| {
                let (cx, cy) = column_index(c.pos);
                terrain.ground_tone_at(cx, cy)
            })
            .collect();
        // Each creature's stratum (from its body + whether its column is water) and the per-stratum
        // headcount (for the density-split non-surface food). A stratum is a predator refuge:
        // hunting only reaches prey in the SAME stratum.
        let strata: Vec<Stratum> = self
            .creatures
            .iter()
            .map(|c| {
                let (cx, cy) = column_index(c.pos);
                stratum_of(&c.pheno, terrain.is_water(cx, cy))
            })
            .collect();
        let mut stratum_count = [0.0f32; 4];
        for s in &strata {
            stratum_count[s.idx()] += 1.0;
        }
        // Autotroph self-shading (F3: computed in the snapshot phase, order-independent). Light is
        // a finite flux, so more autotrophs ⇒ less photosynthesis per head ⇒ the niche self-limits.
        let n_auto = self.creatures.iter().filter(|c| c.pheno.photo_frac() > PHOTO_THETA).count();
        let autotroph_shading = 1.0 / (1.0 + n_auto as f32 / PHOTO_SOFTCAP);
        self.grid.rebuild(&pos, maxx, maxy, GRID_CELL);
        // Decision per creature: (throttle, turn, optional prey index to attack THIS tick).
        let mut decisions: Vec<(f32, f32, Option<usize>)> = Vec::with_capacity(n);
        for i in 0..n {
            let predator = carn[i] > CARNIVORE_THRESHOLD;
            let self_bm = bm[i];
            let self_layer = strata[i];
            // Nearest edible prey (only if predatory) AND nearest threatening predator, one pass.
            // Both restricted to the SAME stratum — a creature in another layer is out of reach.
            // Camouflage gates DETECTION: a predator only sees (and so only targets) a prey it
            // spots, with probability rising in the contrast of the prey's coloration vs its
            // ground. A cryptic prey is often invisible — strong, habitat-specific selection for
            // matching the background (coevolution). Deterministic per (predator, prey, tick).
            let detected = |j: usize| {
                let contrast = (color[j] - bg[j]).abs();
                let p = CAMO_BASE_DETECT + (1.0 - CAMO_BASE_DETECT) * contrast;
                Rng::new(seed_fold(self.world_seed, &[SALT_CAMO, i as u64, j as u64, tick])).unit() <= p
            };
            let (prey, threat) = self.grid.nearest2_within(
                &pos,
                pos[i],
                SENSE_RANGE,
                |j| predator && j != i && bm[j] <= self_bm && strata[j] == self_layer && detected(j),
                |j| j != i && carn[j] > CARNIVORE_THRESHOLD && bm[j] >= self_bm && strata[j] == self_layer,
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
            // (Camouflage already gated targeting at sensing — a chosen prey was detectable.)
            let gain = (bm[j] as f32 * CELL_BIOMASS_COST + self.creatures[j].energy.max(0.0))
                * MEAT_EFFICIENCY
                * carn[i];
            self.creatures[j].alive = false;
            self.deaths += 1;
            self.kills += 1;
            let cap = self.creatures[i].max_energy();
            self.creatures[i].energy = (self.creatures[i].energy + gain).min(cap);
            // The predator gained ENERGY; the prey's MATTER returns to the nutrient pool at its
            // column (a kill site fertilises the ground — energy and matter are separate currencies).
            let (dx, dy) = column_index(pos[j]);
            terrain.deposit_nutrient(dx, dy, bm[j] as f32 * NUTRIENT_PER_CELL, tick);
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
            let layer = strata[idx];
            // Food value is scaled by how well the creature's thermal preference matches the
            // local climate — the C3 habitat pressure acts on the dominant (food) channel.
            let climate = climate_match(terrain.temperature_at(cx, cy), c.genome.thermal_pref);
            let food = if layer == Stratum::Surface {
                // Surface feeds on the positioned S3 plant field; intake scales with body size, a
                // carnivore digests plants poorly (efficiency = 1 − carnivory).
                let taken = terrain.graze(cx, cy, c.intake() * TICK_LEN, tick);
                taken * PLANT_BIOMASS_TO_ENERGY * (1.0 - c.pheno.carnivory())
            } else {
                // Non-surface strata: a fixed foraging capacity split among occupants (density-
                // dependent → an empty stratum richly rewards colonisers, then self-limits).
                layer.capacity() / stratum_count[layer.idx()].max(1.0) * TICK_LEN
            };
            // Photosynthesis (C3 autotrophs): light-driven energy from photo cells, on top of any
            // foraging — so a mixotroph (photo + grazing) is possible. Light is 0 underground and
            // at night, so an autotroph must hold light (surface/shallow, daytime) — that, plus
            // the cell slots photo takes, is the trade-off against mobility/predation.
            let photo = c.pheno.photo as f32;
            let photo_gain = if photo > 0.0 {
                PHOTO_RATE * photo * light_for(layer, cy, tick) * autotroph_shading * TICK_LEN
            } else {
                0.0
            };
            c.energy = (c.energy + food * climate + photo_gain).min(c.max_energy());
            // Metabolism: Kleiber (biomass^0.75) × stratum cost + movement effort.
            let kleiber = (c.biomass() as f32).powf(0.75);
            c.energy -= (SIM_BASE_METABOLISM * kleiber * layer.metab_mult() + MOVE_COST * throttle) * TICK_LEN;
            c.age += 1;
            // Death by starvation. The creature's matter returns to the nutrient pool here
            // (decomposition) — closing the cycle and re-fertilising the death site.
            if c.energy <= 0.0 {
                c.alive = false;
                self.deaths += 1;
                let (dx, dy) = column_index(c.pos);
                terrain.deposit_nutrient(dx, dy, c.biomass() as f32 * NUTRIENT_PER_CELL, tick);
                continue;
            }
            // Death by senescence: old-age probability rising with age² gives demographic
            // turnover. Scaled by 1/biomass — bigger bodies live longer (a real size benefit),
            // so multicellularity has a gradient to climb against its build + Kleiber costs.
            let sp = SENESCENCE_RATE * (c.age as f32 / LIFESPAN).powi(2) / c.biomass() as f32;
            if sp > 0.0 && Rng::new(seed_fold(self.world_seed, &[SALT_DEATH, c.id, tick])).unit() < sp {
                c.alive = false;
                self.deaths += 1;
                let (dx, dy) = column_index(c.pos);
                terrain.deposit_nutrient(dx, dy, c.biomass() as f32 * NUTRIENT_PER_CELL, tick);
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
        pearson(&prefs, &temps)
    }

    /// Fraction of the population in each stratum `[underground, surface, air, water]` — shows
    /// whether vertical niches (burrowers / fliers / swimmers) have been colonised.
    pub fn stratum_mix(&self, terrain: &VoxelTerrain) -> [f32; 4] {
        let n = self.creatures.len();
        if n == 0 {
            return [0.0; 4];
        }
        let mut m = [0.0f32; 4];
        for c in &self.creatures {
            let (cx, cy) = column_index(c.pos);
            m[stratum_of(&c.pheno, terrain.is_water(cx, cy)).idx()] += 1.0;
        }
        for v in &mut m {
            *v /= n as f32;
        }
        m
    }

    /// Mean nutrient level (`[0,1]`) at the columns creatures occupy — the realised fertility of
    /// the inhabited landscape. Falls where grazing strips the ground, rises where deaths return
    /// matter + weathering replenishes; a healthy bounded value means the cycle is self-sustaining.
    pub fn avg_nutrient(&self, terrain: &VoxelTerrain, tick: u64) -> f32 {
        if self.creatures.is_empty() {
            return 0.0;
        }
        let s: f32 = self
            .creatures
            .iter()
            .map(|c| {
                let (cx, cy) = column_index(c.pos);
                terrain.nutrient_at(cx, cy, tick)
            })
            .sum();
        s / self.creatures.len() as f32
    }

    /// Niche coverage: how many DISTINCT ecological niches are occupied — the cross-product of
    /// stratum × diet (herbivore/carnivore) × autotrophy × climate band × complexity tier, counted
    /// over distinct occupied combinations. Rises as the population radiates into the niche space
    /// C3 built; a single-niche monoculture would score ~1. Cheap (O(N) + a small set).
    pub fn niche_coverage(&self, terrain: &VoxelTerrain) -> usize {
        let mut seen = std::collections::HashSet::new();
        for c in &self.creatures {
            let (cx, cy) = column_index(c.pos);
            let stratum = stratum_of(&c.pheno, terrain.is_water(cx, cy)).idx() as u32;
            let carn = (c.pheno.carnivory() > CARNIVORE_THRESHOLD) as u32;
            let auto = (c.pheno.photo_frac() > PHOTO_THETA) as u32;
            let climate = (c.genome.thermal_pref * 2.99) as u32; // 0 cold .. 2 hot
            let cplx = c.pheno.complexity() as u32;
            seen.insert(stratum + 4 * (carn + 2 * (auto + 2 * (climate + 3 * cplx))));
        }
        seen.len()
    }

    /// Species count by LEADER clustering on a phenotype feature vector (body-type composition +
    /// size + climate + colour): a creature joins the first leader within `SPECIES_THRESHOLD`,
    /// else founds a new one. So distinct body plans / niches separate into clades. O(N × species)
    /// — call occasionally (it backs a throttled HUD readout + the dev bridge), not every tick.
    pub fn species_count(&self) -> usize {
        let mut leaders: Vec<[f32; FEATURES]> = Vec::new();
        for c in &self.creatures {
            let f = feature(c);
            if !leaders.iter().any(|l| feature_dist2(l, &f) <= SPECIES_THRESHOLD * SPECIES_THRESHOLD) {
                leaders.push(f);
            }
        }
        leaders.len()
    }

    /// Crypsis metric: Pearson correlation between each creature's coloration and the ground tone
    /// where it lives. ~0 = random colours; → 1 = creatures have evolved to match their background
    /// (camouflage), differently per habitat — a coevolutionary outcome of the detection channel.
    pub fn crypsis_correlation(&self, terrain: &VoxelTerrain) -> f32 {
        let n = self.creatures.len();
        if n < 2 {
            return 0.0;
        }
        let (mut cols, mut tones) = (Vec::with_capacity(n), Vec::with_capacity(n));
        for c in &self.creatures {
            let (cx, cy) = column_index(c.pos);
            cols.push(c.genome.coloration);
            tones.push(terrain.ground_tone_at(cx, cy));
        }
        pearson(&cols, &tones)
    }

    /// Fraction of the population that is autotrophic (photosynthesises — a producer tier inside
    /// the creature substrate, not just the exogenous plant field).
    pub fn frac_autotroph(&self) -> f32 {
        let n = self.creatures.len();
        if n == 0 {
            return 0.0;
        }
        let auto = self.creatures.iter().filter(|c| c.pheno.photo_frac() > PHOTO_THETA).count();
        auto as f32 / n as f32
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

/// Full-state determinism checksum (PR1 lock, F1/F7): an integer fold of the COMPLETE
/// deterministic sim+terrain state — every creature's identity, kinematics, energy, age,
/// liveness, full genome and developed phenotype, the id/counter state, and the mutable terrain
/// fields. Floats are folded by `f32::to_bits` (never float-add — F2). This is the bit-exact lock
/// every later refactor is checked against: counts can collide by luck, a full-state hash cannot.
/// (Used by the determinism-checksum tests now; by the metrics-registry checksum metric in PR5.)
#[allow(dead_code)]
pub fn state_checksum(sim: &Sim, terrain: &VoxelTerrain) -> u64 {
    use crate::rng::{fnv_fold_u32, fnv_fold_u64, FNV_OFFSET};
    let mut h = FNV_OFFSET;
    fnv_fold_u64(&mut h, sim.next_id);
    fnv_fold_u64(&mut h, sim.births);
    fnv_fold_u64(&mut h, sim.deaths);
    fnv_fold_u64(&mut h, sim.kills);
    for c in &sim.creatures {
        fnv_fold_u64(&mut h, c.id);
        fnv_fold_u64(&mut h, c.founder);
        fnv_fold_u32(&mut h, c.pos.x.to_bits());
        fnv_fold_u32(&mut h, c.pos.y.to_bits());
        fnv_fold_u32(&mut h, c.heading.to_bits());
        fnv_fold_u32(&mut h, c.energy.to_bits());
        fnv_fold_u32(&mut h, c.age);
        fnv_fold_u64(&mut h, c.alive as u64);
        fnv_fold_u64(&mut h, c.genome.checksum());
        let p = &c.pheno;
        for v in [p.n_cells, p.effector, p.storage, p.sensor, p.predator, p.flight, p.burrow, p.photo, p.structural] {
            fnv_fold_u32(&mut h, v);
        }
    }
    fnv_fold_u64(&mut h, terrain.mut_state_checksum());
    h
}

/// Golden checksum for `Sim::new(42)` stepped 300 fixed ticks on `VoxelTerrain::new(1)`. Pinned
/// in PR1 so any later change that perturbs the trajectory is caught at the introducing PR.
/// **Profile-specific:** debug and release do NOT produce bit-identical floats (LLVM fuses a*b+c
/// into an FMA in release, not debug) — the trajectory and this hash differ by profile. The sim is
/// deterministic *within* a profile, so we pin one golden per profile via `cfg!(debug_assertions)`.
/// Canonical verification profile is **release** (acceptance corridors are tuned there).
#[allow(dead_code)]
pub const GOLDEN_CHECKSUM_SEED42_300: u64 = if cfg!(debug_assertions) {
    16631596019518872104 // debug profile
} else {
    4293681612149572219 // release profile (FMA contraction shifts the trajectory)
};

#[cfg(test)]
#[path = "sim_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "pressure_tests.rs"]
mod pressure_tests;

