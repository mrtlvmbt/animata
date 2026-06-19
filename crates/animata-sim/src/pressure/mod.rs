//! Selection pressure framework — data-driven registry of pure functions that modify
//! creature traits via a fixed dictionary of effect channels (food_mult, energy_add, etc.).
//!
//! **Design (F2 determinism-safe):**
//! - Each pressure is a pure function `eval(env, pheno, genome, ctx) -> Effect`
//! - Effects compose via channel accumulators (mults multiply, adds sum)
//! - Density-dependent aggregates (e.g., n_auto, stratum counts) pre-computed in `TickCtx`
//! - Pressures never mutate terrain or make RNG calls (purity boundary)

use crate::env::Field;
use crate::genome::{Genome, Phenotype};

pub mod camouflage;
pub mod climate;
pub mod strata;
pub mod predation;
pub mod autotrophy;
pub mod nutrient;

/// Fixed effect dictionary — the communication channel between pressures and apply.
/// Default (all identity) = no effect.
#[derive(Clone, Copy, Debug, Default)]
pub struct Effect {
    /// Multiplier on food intake (plant grazing or stratum foraging).
    pub food_mult: f32,
    /// Direct energy addition (e.g., photosynthesis).
    pub energy_add: f32,
    /// Multiplier on metabolic cost.
    pub metab_mult: f32,
    /// Detection bias [0,1] — shift in predator detection probability of prey (camouflage).
    pub detection_bias: f32,
    /// Additive mortality hazard per tick.
    pub mortality_add: f32,
    /// Multiplier on reproduction gate.
    pub repro_mult: f32,
}

impl Effect {
    /// Identity effect (no change to any channel).
    pub fn identity() -> Self {
        Self {
            food_mult: 1.0,
            energy_add: 0.0,
            metab_mult: 1.0,
            detection_bias: 0.0,
            mortality_add: 0.0,
            repro_mult: 1.0,
        }
    }

    /// Compose two effects: mults multiply, adds sum.
    pub fn compose(self, other: Effect) -> Effect {
        Effect {
            food_mult: self.food_mult * other.food_mult,
            energy_add: self.energy_add + other.energy_add,
            metab_mult: self.metab_mult * other.metab_mult,
            detection_bias: self.detection_bias + other.detection_bias,
            mortality_add: self.mortality_add + other.mortality_add,
            repro_mult: self.repro_mult * other.repro_mult,
        }
    }
}

/// Density-dependent state aggregates computed once per tick before eval,
/// used by pressures to self-limit (e.g., stratum carrying capacity, photosynthesis shading).
#[derive(Clone, Debug)]
pub struct TickCtx {
    /// Population count per stratum (by Stratum::idx()).
    pub stratum_count: [f32; 4],
    /// Autotroph count, for self-shading.
    pub n_auto: usize,
    /// Autotroph shading multiplier: `1.0 / (1.0 + n_auto / PHOTO_SOFTCAP)`.
    pub autotroph_shading: f32,
}

/// Trait for a selection pressure: eval(env, pheno, genome, ctx) -> Effect.
pub trait SelectionPressure: Send + Sync {
    /// Unique identifier for this pressure (e.g., "climate", "camouflage").
    fn id(&self) -> &str;

    /// Fields this pressure requires (for introspection and batched gathering).
    fn fields(&self) -> &[Field];

    /// Whether this pressure is enabled by the config (placeholder for PR4).
    fn enabled(&self) -> bool {
        true // Default: always enabled. PR4 will add config gating.
    }

    /// Pure evaluation: given environment, phenotype, genome, and per-tick aggregates,
    /// return an effect on this creature's channels. No side effects, no RNG.
    fn eval(
        &self,
        env: &mut crate::env::EnvSample,
        pheno: &Phenotype,
        genome: &Genome,
        ctx: &TickCtx,
    ) -> Effect;
}

/// Registry of active selection pressures.
pub struct PressureRegistry {
    pub pressures: Vec<Box<dyn SelectionPressure>>,
}

impl PressureRegistry {
    /// Create a new registry with all default pressures enabled.
    pub fn new() -> Self {
        Self {
            pressures: vec![
                Box::new(climate::ClimatePressure),
                Box::new(camouflage::CamouflagePressure),
                Box::new(strata::StrataPressure),
                Box::new(autotrophy::AutotrophyPressure),
                Box::new(predation::PredationPressure),
                Box::new(nutrient::NutrientPressure),
            ],
        }
    }

    /// Evaluate all pressures on a creature.
    pub fn eval_all(
        &self,
        env: &mut crate::env::EnvSample,
        pheno: &Phenotype,
        genome: &Genome,
        ctx: &TickCtx,
    ) -> Effect {
        let mut result = Effect::identity();
        for pressure in &self.pressures {
            if pressure.enabled() {
                let effect = pressure.eval(env, pheno, genome, ctx);
                result = result.compose(effect);
            }
        }
        result
    }
}

impl Default for PressureRegistry {
    fn default() -> Self {
        Self::new()
    }
}
