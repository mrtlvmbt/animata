//! Ground-toxicity pressure (C3): a new abiotic selection axis. A creature standing where the
//! ground toxicity exceeds its evolved `toxin_resistance` takes an extra per-tick death hazard
//! proportional to the unresisted excess — so toxic belts filter for resistant lineages (allopatry
//! on a non-thermal axis). Resistant creatures pay nothing; the unresistant in clean ground pay
//! nothing either. The hazard is written to the `mortality_add` channel; the death roll happens in
//! `Sim::step` (the mortality channel is consumed there, per (id, tick) so it stays deterministic).

use super::{Effect, Sample, SelectionPressure};

pub struct Toxicity {
    pub lethality: f32,
}

impl SelectionPressure for Toxicity {
    fn id(&self) -> &'static str {
        "toxicity"
    }

    fn eval(&self, s: &Sample) -> Effect {
        let excess = (s.toxicity - s.genome.toxin_resistance).max(0.0);
        Effect { mortality_add: excess * self.lethality, ..Effect::identity() }
    }
}
