//! Seasonality (C3) — a TIME-varying environmental pressure (the others vary in SPACE). Food
//! availability swings over the year: rich in summer, lean in winter. This is the same `food_mult`
//! channel climate uses, but driven by the global seasonal phase (a pure function of the tick)
//! rather than a per-column field. A lean winter is a recurring famine — selecting for storage and
//! making the whole population breathe with the seasons.
//!
//! Default OFF (an opt-in environmental mode), so the baseline world stays aseasonal and the
//! determinism golden is unchanged.

use super::{Effect, Sample, SelectionPressure};

pub struct Seasonality {
    pub amplitude: f32,
}

impl SelectionPressure for Seasonality {
    fn id(&self) -> &'static str {
        "seasonality"
    }

    fn eval(&self, s: &Sample) -> Effect {
        // food ×= 1 + amplitude·sin(year_phase): summer (>0) richer, winter (<0) leaner.
        Effect { food_mult: 1.0 + self.amplitude * s.season_phase, ..Effect::identity() }
    }
}
