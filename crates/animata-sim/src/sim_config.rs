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
    AIR_METAB_MULT, CAMO_BASE_DETECT, PHOTO_RATE, SEASON_AMPLITUDE, SEASON_LEN, THERMAL_PENALTY,
    TOXIN_LETHALITY, UNDERGROUND_METAB_MULT,
};

/// Which simulation features are active. All default to `true`.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
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
    /// Ground toxicity selection (toxic belts kill the unresistant). Off ⇒ no toxic mortality.
    pub toxicity: bool,
    /// Seasonality (food swings over the year). **Default OFF** — an opt-in environmental mode, so
    /// the baseline world (and the determinism golden) stays aseasonal.
    pub seasonality: bool,
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
            toxicity: true,
            seasonality: false,
        }
    }
}

impl Features {
    /// `(name, on)` for every feature — the introspection surface (get_config / the dev bridge).
    pub fn pairs(&self) -> [(&'static str, bool); 8] {
        [
            ("climate", self.climate),
            ("autotrophy", self.autotrophy),
            ("strata", self.strata),
            ("predation", self.predation),
            ("camouflage", self.camouflage),
            ("development", self.development),
            ("toxicity", self.toxicity),
            ("seasonality", self.seasonality),
        ]
    }

    /// Set a feature by name; returns `false` for an unknown name (caller can report it).
    pub fn set(&mut self, name: &str, on: bool) -> bool {
        match name {
            "climate" => self.climate = on,
            "autotrophy" => self.autotrophy = on,
            "strata" => self.strata = on,
            "predation" => self.predation = on,
            "camouflage" => self.camouflage = on,
            "development" => self.development = on,
            "toxicity" => self.toxicity = on,
            "seasonality" => self.seasonality = on,
            _ => return false,
        }
        true
    }
}

/// Tunable numeric parameters (mirror of the `config.rs` constants used in the hot path). Defaults
/// equal those constants, so `Params::default()` is bit-identical to the pre-config behaviour.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
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
    /// Toxicity: per-tick death hazard per unit of unresisted ground toxicity.
    pub toxin_lethality: f32,
    /// Seasonality: food swing amplitude over the year, and the year length in sim-seconds.
    pub season_amplitude: f32,
    pub season_len: f32,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            thermal_penalty: THERMAL_PENALTY,
            photo_rate: PHOTO_RATE,
            air_metab_mult: AIR_METAB_MULT,
            underground_metab_mult: UNDERGROUND_METAB_MULT,
            camo_base_detect: CAMO_BASE_DETECT,
            toxin_lethality: TOXIN_LETHALITY,
            season_amplitude: SEASON_AMPLITUDE,
            season_len: SEASON_LEN,
        }
    }
}

impl Params {
    /// `(name, value)` for every parameter — the introspection surface.
    pub fn pairs(&self) -> [(&'static str, f32); 8] {
        [
            ("thermal_penalty", self.thermal_penalty),
            ("photo_rate", self.photo_rate),
            ("air_metab_mult", self.air_metab_mult),
            ("underground_metab_mult", self.underground_metab_mult),
            ("camo_base_detect", self.camo_base_detect),
            ("toxin_lethality", self.toxin_lethality),
            ("season_amplitude", self.season_amplitude),
            ("season_len", self.season_len),
        ]
    }

    /// Set a parameter by name; returns `false` for an unknown name.
    pub fn set(&mut self, name: &str, v: f32) -> bool {
        match name {
            "thermal_penalty" => self.thermal_penalty = v,
            "photo_rate" => self.photo_rate = v,
            "air_metab_mult" => self.air_metab_mult = v,
            "underground_metab_mult" => self.underground_metab_mult = v,
            "camo_base_detect" => self.camo_base_detect = v,
            "toxin_lethality" => self.toxin_lethality = v,
            "season_amplitude" => self.season_amplitude = v,
            "season_len" => self.season_len = v,
            _ => return false,
        }
        true
    }
}

/// The full runtime config: feature flags + parameters. Default = all-on, constants — the golden.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SimConfig {
    pub features: Features,
    pub params: Params,
}

impl SimConfig {
    /// Parse a `SimConfig` from RON (e.g. `assets/config/sim.ron`). Missing fields fall back to the
    /// defaults (the consts), so a file may specify only what it wants to override.
    pub fn from_ron(s: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(s)
    }
}

#[cfg(test)]
#[path = "sim_config_tests.rs"]
mod tests;
