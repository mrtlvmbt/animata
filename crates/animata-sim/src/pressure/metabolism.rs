//! Stratum metabolic pressure (C3): the vertical niche a creature lives in scales its metabolic
//! cost — flight is dear (lift), burrowing cheap (sheltered), surface/water neutral. This is the
//! home of the per-stratum cost (it used to be `Stratum::metab_mult`).

use super::{Effect, Sample, SelectionPressure};
use crate::config::{AIR_METAB_MULT, UNDERGROUND_METAB_MULT};
use crate::sim::Stratum;

pub struct Metabolism;

impl SelectionPressure for Metabolism {
    fn id(&self) -> &'static str {
        "metabolism"
    }

    fn eval(&self, s: &Sample) -> Effect {
        // Bit-identical to the former `Stratum::metab_mult`.
        let metab_mult = match s.layer {
            Stratum::Air => AIR_METAB_MULT,
            Stratum::Underground => UNDERGROUND_METAB_MULT,
            _ => 1.0,
        };
        Effect { metab_mult, ..Effect::identity() }
    }
}
