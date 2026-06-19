//! Predation selection pressure: predatory creatures (with predator cells) hunt
//! smaller prey and gain trophic energy.
//!
//! Note: This pressure is primarily observational. The actual hunting logic (targeting,
//! energy transfer) is in sim.rs and involves RNG rolls, so it's not a pure pressure eval.
//! Here we return identity; the hunting happens during the predation pass in sim.rs.

use super::{Effect, SelectionPressure, TickCtx};
use crate::env::EnvSample;
use crate::genome::Genome;
use crate::genome::Phenotype;

pub struct PredationPressure;

impl SelectionPressure for PredationPressure {
    fn id(&self) -> &str {
        "predation"
    }

    fn fields(&self) -> &[crate::env::Field] {
        &[] // Predation uses creature-to-creature interactions, not environment fields.
    }

    fn eval(&self, _env: &mut EnvSample, _pheno: &Phenotype, _genome: &Genome, _ctx: &TickCtx) -> Effect {
        // Predation doesn't have a pure effect; it's handled as a special case in sim.rs
        // because it involves creature-to-creature interactions and RNG rolls.
        // Return identity here; hunting is resolved in the predation pass.
        Effect::identity()
    }
}
