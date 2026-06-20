//! Climate / habitat pressure (C3): food value falls as a creature's evolved thermal preference
//! diverges from its local temperature, so lineages sort into the climate band they fit (allopatry).
//! Acts on the dominant energy channel (food), so it actually bites.

use super::{Effect, Sample, SelectionPressure};
use crate::config::THERMAL_PENALTY;

pub struct Climate;

impl SelectionPressure for Climate {
    fn id(&self) -> &'static str {
        "climate"
    }

    fn eval(&self, s: &Sample) -> Effect {
        // Bit-identical to the former `climate_match(temp, pref)`.
        let m = (1.0 - THERMAL_PENALTY * (s.temperature - s.genome.thermal_pref).abs()).clamp(0.1, 1.0);
        Effect { food_mult: m, ..Effect::identity() }
    }
}
