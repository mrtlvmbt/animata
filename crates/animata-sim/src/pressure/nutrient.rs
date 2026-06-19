//! Nutrient cycle selection pressure: creatures that graze drain nutrient from the soil,
//! which is returned when they die (decomposition).
//!
//! **Note:** The nutrient cycle is special — it's the only pressure that has side effects
//! (mutations to terrain). To preserve purity and determinism:
//! - The pressure's `eval` returns identity (no direct effect on creature channels).
//! - The graze drain is applied INSIDE terrain.graze() (part of the current code).
//! - The death deposit is deferred to an explicit deposit step in the serial apply phase.
//!
//! Thus the cycle is "closed" without giving pressures write access to terrain.

use super::{Effect, SelectionPressure, TickCtx};
use crate::env::EnvSample;
use crate::genome::Genome;
use crate::genome::Phenotype;

pub struct NutrientPressure;

impl SelectionPressure for NutrientPressure {
    fn id(&self) -> &str {
        "nutrient"
    }

    fn fields(&self) -> &[crate::env::Field] {
        &[crate::env::Field::Nutrient]
    }

    fn eval(&self, _env: &mut EnvSample, _pheno: &Phenotype, _genome: &Genome, _ctx: &TickCtx) -> Effect {
        // The nutrient cycle doesn't provide a direct creature effect via the pressure framework.
        // The cycle is closed via:
        // 1. Graze drain: applied inside terrain.graze()
        // 2. Death deposit: explicit call in sim.rs's apply phase
        //
        // This pressure is a placeholder for future features (e.g., nutrient stress on foraging)
        // or for introspection/metrics on the cycle.

        Effect::identity()
    }
}
