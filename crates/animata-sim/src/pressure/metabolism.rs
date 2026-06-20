//! Stratum metabolic pressure (C3): the vertical niche a creature lives in scales its metabolic
//! cost — flight is dear (lift), burrowing cheap (sheltered), surface/water neutral. This is the
//! home of the per-stratum cost (it used to be `Stratum::metab_mult`).

use super::{Effect, Sample, SelectionPressure};
use crate::sim::Stratum;

pub struct Metabolism {
    pub air: f32,
    pub underground: f32,
}

impl SelectionPressure for Metabolism {
    fn id(&self) -> &'static str {
        "metabolism"
    }

    fn eval(&self, s: &Sample) -> Effect {
        // Bit-identical to the former `Stratum::metab_mult` (params default to the AIR/UNDERGROUND consts).
        let metab_mult = match s.layer {
            Stratum::Air => self.air,
            Stratum::Underground => self.underground,
            _ => 1.0,
        };
        Effect { metab_mult, ..Effect::identity() }
    }
}
