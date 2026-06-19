//! Climate selection pressure: creatures feed better when their thermal preference
//! matches the local temperature.

use super::{Effect, SelectionPressure, TickCtx};
use crate::config::THERMAL_PENALTY;
use crate::env::{Field, EnvSample};
use crate::genome::Genome;
use crate::genome::Phenotype;

pub struct ClimatePressure;

impl SelectionPressure for ClimatePressure {
    fn id(&self) -> &str {
        "climate"
    }

    fn fields(&self) -> &[Field] {
        &[Field::Temperature]
    }

    fn eval(&self, env: &mut EnvSample, _pheno: &Phenotype, genome: &Genome, _ctx: &TickCtx) -> Effect {
        // Climate match: how well a creature feeds at the local temp given its preference.
        // Matched ⇒ 1.0 (full food value); fully mismatched ⇒ 0.1 (stress cripples foraging).
        let temp = env.sample(Field::Temperature);
        let food_mult = (1.0 - THERMAL_PENALTY * (temp - genome.thermal_pref).abs()).clamp(0.1, 1.0);

        Effect {
            food_mult,
            ..Default::default()
        }
    }
}
