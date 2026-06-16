//! A single organism: position, heading, energy, its genome and decoded body,
//! plus a pluggable [`Behavior`] that drives it.

use crate::behavior::{Behavior, BehaviorKind, Senses};
use crate::body::{Locomotor, Medium};
use crate::config::*;
use crate::genome::{Appendage, Genome, Phenotype};
use macroquad::math::Vec2;
use macroquad::rand::gen_range;

/// Coarse diet class, derived from the continuous carnivory gene — used for
/// coloring and stats only; mechanics use the float directly.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Diet {
    Herbivore,
    Omnivore,
    Carnivore,
}

pub struct Creature {
    /// Stable unique id (assigned by the world), so the UI can track one
    /// individual across reproduction/death even as the vector reorders.
    pub id: u64,
    /// Id of the parent this creature budded/crossed from, if any.
    pub parent_id: Option<u64>,
    /// Founder lineage this creature descends from (inherited from a parent).
    /// Lets the UI track clades and watch lineages coalesce over time.
    pub lineage: u32,
    /// Detected species cluster (assigned at runtime by speciation; not saved).
    pub species_id: u32,
    /// Signal loudness emitted this step (0..1), heard by nearby creatures.
    pub signal: f32,
    /// Current infection's pathogen strain (0..1), or `None` if healthy.
    pub infection: Option<f32>,
    pub pos: Vec2,
    /// Vertical layer the creature currently occupies (default surface). Sensing,
    /// eating and hunting happen within a layer; morphology gates which layers it
    /// can reach (see [`Phenotype::layer_access`]).
    pub layer: u8,
    pub heading: f32,
    pub energy: f32,
    pub age: u32,
    pub generation: u32,
    pub genome: Genome,
    pub pheno: Phenotype,
    /// Which behavior kind this creature uses (kept so children match).
    pub kind: BehaviorKind,
    mind: Box<dyn Behavior + Send>,
}

/// Linear interpolation.
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

impl Creature {
    pub fn new(genome: Genome, pos: Vec2, energy: f32, generation: u32, kind: BehaviorKind) -> Self {
        let pheno = genome.decode();
        let mind = kind.build(&pheno);
        Creature {
            id: 0,
            parent_id: None,
            lineage: 0,
            species_id: 0,
            signal: 0.0,
            infection: None,
            pos,
            layer: pheno.primary_layer(),
            heading: gen_range(0.0, std::f32::consts::TAU),
            energy,
            age: 0,
            generation,
            genome,
            pheno,
            kind,
            mind,
        }
    }

    /// Random-genome creature (used by tests; the world biases founders' diet).
    #[allow(dead_code)]
    pub fn spawn_random(pos: Vec2, kind: BehaviorKind) -> Self {
        Self::new(Genome::random(), pos, START_ENERGY, 0, kind)
    }

    /// Rebuild a creature from saved state (phenotype and brain are re-derived
    /// from the genome). Used when loading a world from disk.
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        id: u64,
        parent_id: Option<u64>,
        lineage: u32,
        genome: Genome,
        pos: Vec2,
        heading: f32,
        energy: f32,
        age: u32,
        generation: u32,
        kind: BehaviorKind,
    ) -> Self {
        let pheno = genome.decode();
        let mind = kind.build(&pheno);
        Creature {
            id,
            parent_id,
            lineage,
            species_id: 0,
            signal: 0.0,
            infection: None,
            pos,
            layer: pheno.primary_layer(),
            heading,
            energy,
            age,
            generation,
            genome,
            pheno,
            kind,
            mind,
        }
    }

    /// Diet on a 0..1 herbivore→carnivore scale (the gene).
    pub fn carnivory(&self) -> f32 {
        self.pheno.carnivory
    }

    /// Realized recurrent-memory reliance (0..1) from the behavior/brain.
    pub fn memory_use(&self) -> f32 {
        self.mind.memory_use()
    }


    /// Coarse diet class for coloring/stats.
    pub fn diet(&self) -> Diet {
        let c = self.pheno.carnivory;
        if c < DIET_HERBIVORE_MAX {
            Diet::Herbivore
        } else if c >= DIET_CARNIVORE_MIN {
            Diet::Carnivore
        } else {
            Diet::Omnivore
        }
    }


    /// Sense the nearest food, threat and same-species neighbor (relative
    /// offsets, or None), decide, and move. Energy spent this step is applied
    /// to `self.energy`.
    /// `move_mult` / `metab_mult` come from the biome the creature stands in
    /// (terrain drag and climate).
    #[allow(clippy::too_many_arguments)]
    pub fn think_and_act(
        &mut self,
        nearest_food: Option<Vec2>,
        nearest_threat: Option<Vec2>,
        nearest_neighbor: Option<Vec2>,
        heard: f32,
        move_mult: f32,
        metab_mult: f32,
        medium: Medium,
    ) {
        let senses = self.sense(nearest_food, nearest_threat, nearest_neighbor, heard);
        let action = self.mind.decide(&senses);
        self.signal = action.signal; // emit this step's call (heard next step)

        // Turning is coupled to forward drive: a creature can only steer while
        // moving (like a car). This kills frantic spinning-in-place when idle.
        let drive = action.throttle.max(0.0); // 0..=1
        self.heading += action.turn * MAX_TURN * drive;
        // Carnivores get the predator multipliers; herbivores get 1.0; omnivores
        // interpolate by carnivory.
        let c = self.pheno.carnivory;
        let speed_mult = lerp(1.0, PREDATOR_SPEED_MULT, c);
        // Old age (senescence) saps speed.
        let age_mult = 1.0 - SENESCENCE_SPEED_DROP * self.senescence();
        // Terrain drag scales the distance actually covered, but the creature
        // still pays the movement cost for the effort it tried to make. Thrust
        // comes through the Locomotor seam, scaled by how well the body's
        // appendages suit the medium it's in (fins in water, legs on land). Each
        // appendage segment has its own brain-driven actuator port (drives[k]).
        let n_app = self
            .pheno
            .segments
            .iter()
            .filter(|s| s.appendage != Appendage::None)
            .count();
        let thrust = self.pheno.locomotion(medium, &action.drives[..n_app]).thrust;
        let intent = drive * thrust * speed_mult * age_mult;
        let speed = intent * move_mult;
        let dir = Vec2::new(self.heading.cos(), self.heading.sin());
        self.pos += dir * speed;

        // Toroidal world: wrap around edges.
        self.pos.x = self.pos.x.rem_euclid(WORLD_W);
        self.pos.y = self.pos.y.rem_euclid(WORLD_H);

        // Vertical migration: the brain can climb (toward air) or descend (toward
        // underground) one stratum per step, gated by which layers the body can
        // reach. A surface-only body (no wings/burrow) can't move — so founders
        // stay put and the baseline is preserved; a winged or burrowing body can
        // exploit two strata, foraging where its layer is richest.
        let step = if action.vertical > LAYER_SWITCH_DEADZONE {
            1
        } else if action.vertical < -LAYER_SWITCH_DEADZONE {
            -1
        } else {
            0
        };
        if step != 0 {
            let target = self.layer as i32 + step;
            if (0..N_LAYERS as i32).contains(&target) {
                let target = target as u8;
                if self.pheno.layer_access() & (1 << target) != 0 {
                    self.layer = target;
                }
            }
        }

        // Energy upkeep: metabolism (climate-scaled) + movement effort, scaled by
        // body size and (for predators) a species multiplier. Each body segment
        // and appendage adds upkeep, so a longer/limbed body must earn its keep
        // through locomotion — the brake that keeps chain length at an interior
        // optimum (a finless single-segment founder pays neither term).
        let body = 1.0
            + self.pheno.radius * 0.08
            + SEGMENT_UPKEEP * self.pheno.segments.len() as f32
            + APPENDAGE_UPKEEP * n_app as f32
            + NEURON_UPKEEP * (self.pheno.n_hidden as f32 - FOUNDER_HIDDEN as f32);
        let diet_mult = lerp(1.0, PREDATOR_METAB_MULT, c);
        // A showy ornament is a survival handicap (extra upkeep). Squared so the
        // marginal cost rises with size — a nonlinear brake on Fisherian runaway.
        let o = self.pheno.ornament;
        let ornament_cost = 1.0 + ORNAMENT_COST * o * o;
        let upkeep = BASE_METABOLISM * self.pheno.metabolism * body * metab_mult * ornament_cost;
        let move_cost = MOVE_COST * intent * body;
        self.energy -= (upkeep + move_cost) * diet_mult;
        self.age += 1;
    }

    fn sense(
        &self,
        nearest_food: Option<Vec2>,
        nearest_threat: Option<Vec2>,
        nearest_neighbor: Option<Vec2>,
        heard: f32,
    ) -> Senses {
        // Old age dulls the senses too.
        let sense = self.pheno.sense_range * (1.0 - SENESCENCE_SENSE_DROP * self.senescence());
        // Map an offset to (relative angle, proximity) if it's within range.
        let channel = |offset: Option<Vec2>| match offset {
            Some(off) => {
                let dist = off.length();
                if dist <= sense && dist > 0.0 {
                    (Some(off.y.atan2(off.x) - self.heading), 1.0 - dist / sense)
                } else {
                    (None, 0.0)
                }
            }
            None => (None, 0.0),
        };
        let (food_rel_angle, food_prox) = channel(nearest_food);
        let (threat_rel_angle, threat_prox) = channel(nearest_threat);
        let (neighbor_rel_angle, neighbor_prox) = channel(nearest_neighbor);
        // Proprioceptive pacemakers: one CPG oscillator per appendage segment, in
        // body order. Each runs at a gene-tuned rate (segment flexibility) and is
        // phase-staggered by its position, so a multi-limb body feels a travelling
        // rhythm it can wire into gait. Founders (no limbs) get none.
        let mut proprioception = [0.0f32; MAX_SEGMENTS];
        let mut n_sensors = 0usize;
        for seg in self.pheno.segments.iter() {
            if seg.appendage == Appendage::None || n_sensors >= MAX_SEGMENTS {
                continue;
            }
            let freq = OSC_FREQ_BASE * (0.5 + seg.flexibility);
            let stagger = n_sensors as f32 * 0.25; // quarter-cycle between limbs
            let phase = self.age as f32 * freq + stagger;
            proprioception[n_sensors] = (phase * std::f32::consts::TAU).sin();
            n_sensors += 1;
        }
        Senses {
            food_rel_angle,
            food_prox,
            threat_rel_angle,
            threat_prox,
            neighbor_rel_angle,
            neighbor_prox,
            heard,
            energy: (self.energy / REPRO_ENERGY).min(1.0),
            proprioception,
            n_sensors,
        }
    }

    /// Senescence factor in `0..=1`: 0 through the prime, ramping to 1 over
    /// `SENESCENCE_SCALE` steps afterwards. Single source for all aging effects.
    /// Carnivores age slower (later onset + gentler ramp).
    pub fn senescence(&self) -> f32 {
        let life = lerp(1.0, PREDATOR_LONGEVITY_MULT, self.pheno.carnivory);
        let over = self.age as f32 - self.pheno.prime * life;
        if over <= 0.0 {
            0.0
        } else {
            (over / (SENESCENCE_SCALE * life)).min(1.0)
        }
    }

    pub fn wants_to_reproduce(&self) -> bool {
        // Late maturity is the cost of a long lifespan; carnivores mature faster.
        let frac = lerp(MATURITY_FRAC, PREDATOR_MATURITY_FRAC, self.pheno.carnivory);
        self.energy >= REPRO_ENERGY && self.age as f32 >= self.pheno.prime * frac
    }

    pub fn is_dead(&self) -> bool {
        self.energy <= 0.0
    }

    /// Random death from old age; probability rises with senescence².
    pub fn dies_of_age(&self) -> bool {
        let s = self.senescence();
        gen_range(0.0f64, 1.0) < AGE_MORTALITY * (s * s) as f64
    }

    /// Split off a mutated child of the same behavior kind; halves own energy.
    /// The child's id is left 0 for the world to assign.
    pub fn reproduce(&mut self, mut_rate: f64) -> Creature {
        self.energy *= 0.5;
        let jitter = Vec2::new(gen_range(-6.0, 6.0), gen_range(-6.0, 6.0));
        let child_pos = self.pos + jitter;
        let mut child = Creature::new(
            self.genome.mutated(mut_rate),
            child_pos,
            self.energy,
            self.generation + 1,
            self.kind,
        );
        child.parent_id = Some(self.id);
        child.lineage = self.lineage;
        child.species_id = self.species_id; // refreshed at next speciation update
        child
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::seed;

    #[test]
    fn senescence_zero_in_prime_and_full_when_old() {
        seed(1);
        let mut c = Creature::spawn_random(Vec2::ZERO, BehaviorKind::Neural);
        c.age = 0;
        assert_eq!(c.senescence(), 0.0);
        c.age = c.pheno.prime as u32 + SENESCENCE_SCALE as u32 + 10;
        assert_eq!(c.senescence(), 1.0);
    }

    #[test]
    fn juveniles_cannot_reproduce() {
        seed(2);
        let mut c = Creature::spawn_random(Vec2::ZERO, BehaviorKind::Neural);
        c.energy = REPRO_ENERGY + 10.0;
        c.age = 0;
        assert!(!c.wants_to_reproduce(), "newborn should be immature");
        c.age = c.pheno.prime as u32; // well past maturity
        assert!(c.wants_to_reproduce());
    }
}
