//! Autotrophy pressure (C3): photosynthetic cells make energy from light, on top of any foraging
//! (so a mixotroph is possible). Light is 0 underground and at night; self-shading (`autotroph_shading`,
//! a per-tick aggregate) makes the niche self-limit. The cell slots photo takes are the trade-off.

use super::{Effect, Sample, SelectionPressure};
use crate::config::TICK_LEN;

pub struct Autotrophy {
    pub photo_rate: f32,
}

impl SelectionPressure for Autotrophy {
    fn id(&self) -> &'static str {
        "autotrophy"
    }

    fn eval(&self, s: &Sample) -> Effect {
        // Bit-identical to the former inline `photo_gain` (param defaults to PHOTO_RATE).
        let photo = s.pheno.photo as f32;
        let energy_add = if photo > 0.0 {
            self.photo_rate * photo * s.light * s.autotroph_shading * TICK_LEN
        } else {
            0.0
        };
        Effect { energy_add, ..Effect::identity() }
    }
}
