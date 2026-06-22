//! Climate / habitat pressure (C3): the metabolic (respiration) cost RISES as a creature's evolved
//! thermal preference diverges from its local temperature, so lineages sort into the climate band they
//! fit (allopatry → habitats). Acts on `metab_mult` — a UNIVERSAL lever that bites BOTH autotrophs and
//! heterotrophs. (The old food-only form went inert under the autotroph-base once free grazing was
//! removed — food=0 ⇒ no climate selection.) Respiration is the more temperature-sensitive process
//! (Q10≈2 > photosynthesis' ~0.32 eV — it drives the thermal optimum), so a metabolic cost is the
//! faithful channel for the producer base.

use super::{Effect, Sample, SelectionPressure};

pub struct Climate {
    pub thermal_penalty: f32,
}

impl SelectionPressure for Climate {
    fn id(&self) -> &'static str {
        "climate"
    }

    fn eval(&self, s: &Sample) -> Effect {
        // 1.0 at a perfect thermal match, rising with mismatch (|temp − pref| ∈ [0,1] ⇒ metab_mult ∈
        // [1, 1+penalty]). A creature living off its thermal optimum pays more to survive ⇒ selection
        // for thermal_pref tracking local temperature (the habitats axis), now on the producer base too.
        let m = 1.0 + self.thermal_penalty * (s.temperature - s.genome.thermal_pref).abs();
        Effect { metab_mult: m, ..Effect::identity() }
    }
}
