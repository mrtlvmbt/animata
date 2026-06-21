//! Aerobic-respiration pressure (C3 gas cycle, Phase 2). Aerobic respiration burns FOOD (organic
//! carbon) using O2 to extract far more energy from it (~15× the anaerobic yield). So a creature with
//! evolved `aerobic_capacity` in O2-rich water gets a MULTIPLIER on its FOOD income (`food_mult`), NOT
//! free standalone energy from O2 — that distinction is load-bearing: a standalone O2→energy term
//! would reward whoever SITS in O2 (the autotrophs that PRODUCE it), amplifying the monoculture
//! (observed in the Phase-2 spike). Multiplying FOOD income instead rewards the FOOD-eaters
//! (heterotrophs/predators, whose energy is grazing/predation), not the photosynthesisers (whose
//! energy is `energy_add`) — the intended rebalance toward animals. The matching O2 DRAWDOWN (aerobes
//! consume the O2, closing the self-balancing loop) is applied in the serial apply phase. Gated by
//! `Features.aerobic`.

use super::{Effect, Sample, SelectionPressure};

pub struct AerobicRespiration {
    pub gain: f32,
}

impl SelectionPressure for AerobicRespiration {
    fn id(&self) -> &'static str {
        "aerobic"
    }

    fn eval(&self, s: &Sample) -> Effect {
        // Dimensionless multiplier on food income: 1 (anaerobic / no O2) → up to 1 + gain·O2 for a
        // fully-aerobic body in O2-rich water. Disproportionately helps food-reliant heterotrophs
        // (food is their whole income) over photosynthesisers (food is a small fraction of theirs).
        let food_mult = 1.0 + s.oxygen * s.genome.aerobic_capacity * self.gain;
        Effect { food_mult, ..Effect::identity() }
    }
}
