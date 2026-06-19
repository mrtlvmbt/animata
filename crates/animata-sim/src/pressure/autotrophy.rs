//! Autotrophy selection pressure: photosynthetic creatures gain energy from light,
//! scaled by photo cell fraction and shaded by population density.

use super::{Effect, SelectionPressure, TickCtx};
use crate::config::PHOTO_THETA;
use crate::env::EnvSample;
use crate::genome::Genome;
use crate::genome::Phenotype;

pub struct AutotrophyPressure;

impl SelectionPressure for AutotrophyPressure {
    fn id(&self) -> &str {
        "autotrophy"
    }

    fn fields(&self) -> &[crate::env::Field] {
        &[] // Light is computed by sim; autotrophs use TickCtx for shading.
    }

    fn eval(&self, _env: &mut EnvSample, pheno: &Phenotype, _genome: &Genome, _ctx: &TickCtx) -> Effect {
        // Autotrophs gain direct energy if they have enough photo cells.
        // Light is computed elsewhere (needs stratum/latitude/time info from sim).
        // Here we just factor in the photo fraction and population shading.
        let photo_frac = pheno.photo_frac();
        if photo_frac <= PHOTO_THETA {
            return Effect::identity();
        }

        // The pressure itself doesn't compute energy (that's context-dependent);
        // instead, it returns a multiplier on the photosynthesis yield.
        // The actual light * shading is applied in sim.rs after eval.
        // For now, return identity; the photo_gain is calculated directly in sim.rs.

        Effect::identity()
    }
}
