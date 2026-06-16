//! Pluggable creature behavior: turn perception into action.
//!
//! A creature's decision-making is a [`Behavior`] strategy. The neural-net brain
//! is one implementation; the rule-based steering is another. Both read genes
//! (via the decoded weights), so evolution still acts on them.
//!
//! To add a new variant: implement [`Behavior`], add a [`BehaviorKind`] arm, and
//! wire it into [`BehaviorKind::build`]. Nothing else needs to change.

use crate::brain::Brain;
use crate::config::*;
use crate::genome::{Phenotype, Synapse};
use macroquad::rand::gen_range;
use std::f32::consts::PI;

/// Behavior used by default when the app starts (toggle in-app with `B`).
pub const DEFAULT_BEHAVIOR: BehaviorKind = BehaviorKind::Neural;

/// What a creature perceives this step (engine-level, behavior-agnostic).
///
/// "Food" is the thing it eats (pellets for herbivores, herbivores for
/// predators). "Threat" is the nearest thing that eats it (a predator); it is
/// always absent for predators.
pub struct Senses {
    /// Angle to the nearest food relative to heading (radians), if in range.
    pub food_rel_angle: Option<f32>,
    /// Closeness to nearest food, `0..=1` (1 == on top of it); 0 if none in range.
    pub food_prox: f32,
    /// Angle to the nearest threat relative to heading (radians), if in range.
    pub threat_rel_angle: Option<f32>,
    /// Closeness to nearest threat, `0..=1`; 0 if none in range.
    pub threat_prox: f32,
    /// Angle to the nearest same-species neighbor (radians), if in range.
    pub neighbor_rel_angle: Option<f32>,
    /// Closeness to nearest same-species neighbor, `0..=1`; 0 if none in range.
    pub neighbor_prox: f32,
    /// Loudness of the signal heard from the nearest neighbor, `0..=1`.
    pub heard: f32,
    /// Own energy normalized to the reproduction threshold, `0..=1`.
    pub energy: f32,
}

/// A creature's chosen action.
pub struct Action {
    /// Forward drive; only the positive part moves the creature.
    pub throttle: f32,
    /// Turn rate, scaled by [`MAX_TURN`] when applied.
    pub turn: f32,
    /// Emitted signal loudness this step, `0..=1` (a "call" others can hear).
    pub signal: f32,
}

/// Strategy turning [`Senses`] into an [`Action`].
pub trait Behavior {
    /// `&mut self` because a recurrent brain updates its memory each step.
    fn decide(&mut self, senses: &Senses) -> Action;

    /// Realized recurrent-memory reliance, 0..1 (0 for memoryless behaviors).
    fn memory_use(&self) -> f32 {
        0.0
    }
}

/// Which behavior implementation the simulation uses.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BehaviorKind {
    /// Feed-forward neural net, weights from DNA.
    Neural,
    /// Hand-written steering whose gains are read from DNA.
    Rule,
}

impl BehaviorKind {
    /// Construct the behavior for a decoded genome.
    pub fn build(self, pheno: &Phenotype) -> Box<dyn Behavior + Send> {
        match self {
            BehaviorKind::Neural => {
                Box::new(NeuralBehavior::new(&pheno.synapses, pheno.leak, pheno.n_hidden))
            }
            BehaviorKind::Rule => Box::new(RuleBehavior::new(&pheno.synapses)),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            BehaviorKind::Neural => "neural-net",
            BehaviorKind::Rule => "rule-based",
        }
    }

    /// Next kind in the cycle (for the in-app toggle).
    pub fn next(self) -> Self {
        match self {
            BehaviorKind::Neural => BehaviorKind::Rule,
            BehaviorKind::Rule => BehaviorKind::Neural,
        }
    }

    /// Single-char code for save files.
    pub fn code(self) -> char {
        match self {
            BehaviorKind::Neural => 'n',
            BehaviorKind::Rule => 'r',
        }
    }

    pub fn from_code(c: char) -> Option<Self> {
        match c {
            'n' => Some(BehaviorKind::Neural),
            'r' => Some(BehaviorKind::Rule),
            _ => None,
        }
    }
}

// ---- Neural network brain ----

struct NeuralBehavior {
    brain: Brain,
}

impl NeuralBehavior {
    fn new(synapses: &[Synapse], leak: f32, n_hidden: usize) -> Self {
        NeuralBehavior {
            brain: Brain::from_synapses(synapses, leak, n_hidden),
        }
    }
}

impl Behavior for NeuralBehavior {
    fn decide(&mut self, s: &Senses) -> Action {
        let dir = |a: Option<f32>| match a {
            Some(a) => (a.sin(), a.cos()),
            None => (0.0, 0.0),
        };
        let (fsin, fcos) = dir(s.food_rel_angle);
        let (tsin, tcos) = dir(s.threat_rel_angle);
        let (nsin, ncos) = dir(s.neighbor_rel_angle);
        let inputs = [
            s.food_prox, fsin, fcos,
            s.threat_prox, tsin, tcos,
            s.neighbor_prox, nsin, ncos,
            s.heard, s.energy, 1.0,
        ];
        let [throttle, turn, signal] = self.brain.forward(&inputs);
        // Signal is an emitted loudness in 0..1.
        Action { throttle, turn, signal: signal.max(0.0) }
    }

    fn memory_use(&self) -> f32 {
        self.brain.mem_use
    }
}

// ---- Rule-based steering ----

/// Steer toward the nearest visible food; wander otherwise. The three gains are
/// decoded from the genome's weight region, so they mutate and evolve too.
struct RuleBehavior {
    steer_gain: f32,
    wander: f32,
    hunger_throttle: f32,
}

impl RuleBehavior {
    fn new(synapses: &[Synapse]) -> Self {
        // Read the first few synapse weights (-WEIGHT_SCALE..=WEIGHT_SCALE) as the
        // gains, so the rule behavior still mutates and evolves with the genome.
        let u = |i: usize| {
            let w = synapses.get(i).map_or(0.0, |s| s.w);
            (w / WEIGHT_SCALE + 1.0) * 0.5 // 0..=1
        };
        RuleBehavior {
            steer_gain: 0.5 + u(0) * 1.5,       // 0.5..=2.0
            wander: u(1) * 0.6,                 // 0..=0.6
            hunger_throttle: 0.4 + u(2) * 0.6,  // 0.4..=1.0
        }
    }
}

impl Behavior for RuleBehavior {
    fn decide(&mut self, s: &Senses) -> Action {
        // Fleeing a threat overrides foraging: steer away at full speed, and
        // sound an alarm (also flee if a neighbor is already calling).
        if s.threat_rel_angle.is_some() || s.heard > 0.5 {
            let away = s.threat_rel_angle.unwrap_or(0.0);
            return Action {
                throttle: 1.0,
                turn: (-away / PI * self.steer_gain).clamp(-1.0, 1.0),
                signal: if s.threat_rel_angle.is_some() { 1.0 } else { 0.0 },
            };
        }
        match s.food_rel_angle {
            Some(rel) => Action {
                throttle: 1.0,
                turn: (rel / PI * self.steer_gain).clamp(-1.0, 1.0),
                signal: 0.0,
            },
            None => Action {
                throttle: self.hunger_throttle,
                turn: gen_range(-self.wander, self.wander),
                signal: 0.0,
            },
        }
    }
}
