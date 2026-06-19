//! Camouflage selection pressure: creatures that match their background are harder
//! to detect by predators.
//!
//! **F6 note:** This pressure only computes `detection_bias` (the visibility probability).
//! The actual RNG roll `<= p` happens in the predation targeting gate keyed by (predator, prey, tick).
//! The camouflage pressure does NOT see the pair; it only returns the probability based on
//! prey contrast vs ground. The pair-roll stays in predation logic.

use super::{Effect, SelectionPressure, TickCtx};
use crate::env::{Field, EnvSample};
use crate::genome::Genome;
use crate::genome::Phenotype;

pub struct CamouflagePressure;

impl SelectionPressure for CamouflagePressure {
    fn id(&self) -> &str {
        "camouflage"
    }

    fn fields(&self) -> &[Field] {
        &[Field::GroundTone]
    }

    fn eval(&self, env: &mut EnvSample, _pheno: &Phenotype, genome: &Genome, _ctx: &TickCtx) -> Effect {
        // F6: detection_bias is the base probability, contrast-derived.
        // The actual RNG roll (and pair-based seed) stays in predation targeting.
        let ground_tone = env.sample(Field::GroundTone);
        let contrast = (genome.coloration - ground_tone).abs();
        let detection_bias = crate::config::CAMO_BASE_DETECT
            + (1.0 - crate::config::CAMO_BASE_DETECT) * contrast;

        Effect {
            detection_bias,
            ..Default::default()
        }
    }
}
