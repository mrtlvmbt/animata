//! Runtime simulation configuration — feature toggles + tunable parameters.
//!
//! `SimConfig` is part of the sim's INPUT (alongside the world seed), so a run is reproducible from
//! `(seed, cfg)`: the determinism golden is taken at `SimConfig::default()`. Every feature defaults
//! ON and every parameter defaults to its `config.rs` constant, so the default config reproduces the
//! pre-toggle behaviour bit-for-bit. A later PR wires these to the dev bridge for live control.
//!
//! Toggling: the pure energy-budget pressures are turned off by DROPPING them from the
//! [`crate::pressure::PressureRegistry`] (membership = feature); the mechanic features
//! (strata / predation / camouflage / development) are gated inside `Sim::step`.

use crate::config::{
    AIR_METAB_MULT, CAMO_BASE_DETECT, PHOTO_RATE, THERMAL_PENALTY, UNDERGROUND_METAB_MULT,
};

/// Which simulation features are active. All default to `true`.
#[derive(Clone, Copy, Debug)]
pub struct Features {
    /// Climate stress on food value (temperature vs evolved preference). Off ⇒ `food_mult = 1`.
    pub climate: bool,
    /// Photosynthesis from photo cells. Off ⇒ no autotroph energy income.
    pub autotrophy: bool,
    /// Vertical strata (air / underground / water niches). Off ⇒ every creature is Surface.
    pub strata: bool,
    /// Predation (the trophic web). Off ⇒ no hunting / kills.
    pub predation: bool,
    /// Camouflage (crypsis gates predator detection). Off ⇒ prey is always detectable.
    pub camouflage: bool,
    /// Developmental bodies (the GRN grows multicellular forms). Off ⇒ children are single cells.
    pub development: bool,
}

impl Default for Features {
    fn default() -> Self {
        Features {
            climate: true,
            autotrophy: true,
            strata: true,
            predation: true,
            camouflage: true,
            development: true,
        }
    }
}

/// Tunable numeric parameters (mirror of the `config.rs` constants used in the hot path). Defaults
/// equal those constants, so `Params::default()` is bit-identical to the pre-config behaviour.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// Climate: how steeply food value falls with temperature mismatch.
    pub thermal_penalty: f32,
    /// Autotrophy: energy per photo cell per sim-second at full light.
    pub photo_rate: f32,
    /// Metabolism: per-stratum cost multipliers.
    pub air_metab_mult: f32,
    pub underground_metab_mult: f32,
    /// Camouflage: detection probability of a perfectly cryptic prey (the floor).
    pub camo_base_detect: f32,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            thermal_penalty: THERMAL_PENALTY,
            photo_rate: PHOTO_RATE,
            air_metab_mult: AIR_METAB_MULT,
            underground_metab_mult: UNDERGROUND_METAB_MULT,
            camo_base_detect: CAMO_BASE_DETECT,
        }
    }
}

/// The full runtime config: feature flags + parameters. Default = all-on, constants — the golden.
#[derive(Clone, Copy, Debug, Default)]
pub struct SimConfig {
    pub features: Features,
    pub params: Params,
}
