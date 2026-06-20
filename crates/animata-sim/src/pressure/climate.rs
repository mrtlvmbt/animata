//! Climate / habitat pressure (C3): food value falls as a creature's evolved thermal preference
//! diverges from its local temperature, so lineages sort into the climate band they fit (allopatry).
//! Acts on the dominant energy channel (food), so it actually bites.

use super::{Effect, Sample, SelectionPressure};

pub struct Climate {
    pub thermal_penalty: f32,
}

impl SelectionPressure for Climate {
    fn id(&self) -> &'static str {
        "climate"
    }

    fn eval(&self, s: &Sample) -> Effect {
        // Bit-identical to the former `climate_match(temp, pref)` (param defaults to THERMAL_PENALTY).
        let m = (1.0 - self.thermal_penalty * (s.temperature - s.genome.thermal_pref).abs()).clamp(0.1, 1.0);
        Effect { food_mult: m, ..Effect::identity() }
    }
}
