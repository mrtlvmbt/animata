//! Vertical strata selection pressure: each stratum (underground/surface/air/water)
//! has its own foraging niche with density-dependent carrying capacity.

use super::{Effect, SelectionPressure, TickCtx};
use crate::env::{EnvSample, Field};
use crate::genome::Genome;
use crate::genome::Phenotype;

pub struct StrataPressure;

impl SelectionPressure for StrataPressure {
    fn id(&self) -> &str {
        "strata"
    }

    fn fields(&self) -> &[Field] {
        &[] // Strata are computed in sim.rs; this pressure only uses TickCtx.
    }

    fn eval(&self, _env: &mut EnvSample, _pheno: &Phenotype, _genome: &Genome, _ctx: &TickCtx) -> Effect {
        // The stratum a creature occupies is determined by its phenotype + column water status.
        // This is encoded elsewhere; here we assume stratum_of(pheno, is_water) was used to
        // place it in strata[idx]. We need the stratum to compute its foraging yield.
        //
        // However, this pressure as currently called doesn't have access to the stratum directly.
        // That's computed in sim.rs via the strata[] snapshot. For now, return identity;
        // the stratum food value is applied directly in the apply phase (not through a pressure).
        //
        // TODO(PR3): Refactor strata as a true pressure if we have stratum in the eval signature.

        Effect::identity()
    }
}
