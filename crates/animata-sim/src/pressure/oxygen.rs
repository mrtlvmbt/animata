//! Oxygen-toxicity pressure (C3 gas cycle, Phase 1). Dissolved O2 (a photosynthesis byproduct that
//! accumulates where autotrophs are dense, terrain.rs `deposit_oxygen`) beyond a creature's evolved
//! `oxygen_tolerance` inflicts a per-tick death hazard — recapitulating the Great Oxygenation Event:
//! O2 is a poison (reactive-oxygen damage) to the unadapted, and tolerance must EVOLVE. This both
//! filters for O2-tolerant lineages AND brakes the autotroph density that produced the O2 (the
//! monoculture fix). Verbatim the `toxicity` template — hazard on `mortality_add`, the death roll
//! happens in `Sim::step` per (id, tick). Gated by `Features.oxygen` (off ⇒ no O2 at all).
//! Phase 2 will add an aerobic ENERGY boost (a second gene) on the `energy_add` channel.

use super::{Effect, Sample, SelectionPressure};

pub struct OxygenToxicity {
    pub lethality: f32,
}

impl SelectionPressure for OxygenToxicity {
    fn id(&self) -> &'static str {
        "oxygen"
    }

    fn eval(&self, s: &Sample) -> Effect {
        let excess = (s.oxygen - s.genome.oxygen_tolerance).max(0.0);
        Effect { mortality_add: excess * self.lethality, ..Effect::identity() }
    }
}
