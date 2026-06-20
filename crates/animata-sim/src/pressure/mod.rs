//! Selection-pressure framework — the engine of evolution made extensible.
//!
//! An *environmental selection pressure* is a pure function of a creature's body + genome and the
//! environment it stands in, producing an [`Effect`]: a bias on a small, fixed set of energy-budget
//! **channels**. The channels are the safety boundary — a pressure can only touch sanctioned
//! currencies (food intake, direct energy, metabolic cost), never arbitrary creature or world
//! state. Effects compose (multiply the mults, sum the adds), so the order is irrelevant and the
//! result is deterministic and parallel-safe.
//!
//! **Adding a pressure** = one module implementing [`SelectionPressure`] + a line in
//! [`PressureRegistry::default`]. `Sim::step` is not touched. New environment input → add a field to
//! [`Sample`]; new evolvable trait → add to the genome/phenotype. Existing pressures stay put.
//!
//! Scope note: only the pure *energy-budget* pressures live here (climate, autotrophy, metabolism).
//! The mechanic pressures — predation (a multi-creature pass), camouflage (a per-pair detection
//! roll), the nutrient cycle (it mutates the terrain) — are NOT pure per-creature channel effects,
//! so they remain explicit phases in `Sim::step` (see the critic findings F2/F6). More channels
//! (detection / mortality / reproduction) join `Effect` when/if those migrate.

use crate::genome::{Genome, Phenotype};
use crate::sim::Stratum;
use crate::sim_config::{Features, Params};

mod autotrophy;
mod climate;
mod metabolism;
mod toxicity;

/// Per-creature inputs a pressure may read: an environment sample at the creature's column plus its
/// body, genome and niche context. Cheap scalars are filled eagerly by the caller (only a couple,
/// so there is no eager-union cost to worry about — cf. F3).
pub struct Sample<'a> {
    pub pheno: &'a Phenotype,
    pub genome: &'a Genome,
    pub layer: Stratum,
    /// Local temperature `[0,1]` at the creature's column (env field).
    pub temperature: f32,
    /// Light reaching the creature's stratum at this tick `[0,1]` (env field; 0 underground/night).
    pub light: f32,
    /// Ground toxicity `[0,1]` at the creature's column (env field).
    pub toxicity: f32,
    /// Autotroph self-shading multiplier (a per-tick population aggregate; see `TickCtx` in `step`).
    pub autotroph_shading: f32,
}

/// The sanctioned channels a pressure can bias. `Default`/[`Effect::identity`] = no effect, so a
/// pressure only writes the channels it cares about. Composition: mults multiply, adds sum.
#[derive(Clone, Copy)]
pub struct Effect {
    /// Multiplies foraging income (e.g. climate stress on food value).
    pub food_mult: f32,
    /// Direct energy income on top of foraging (e.g. photosynthesis).
    pub energy_add: f32,
    /// Multiplies the metabolic cost (e.g. the stratum a creature lives in).
    pub metab_mult: f32,
    /// Additive per-tick death hazard `[0,1]` (e.g. ground toxicity beyond a creature's tolerance).
    pub mortality_add: f32,
}

impl Effect {
    pub fn identity() -> Self {
        Effect { food_mult: 1.0, energy_add: 0.0, metab_mult: 1.0, mortality_add: 0.0 }
    }

    /// Fold another effect in. Identity element under `compose`, and — because multiplying by `1.0`
    /// and adding `0.0` are exact for finite floats — composing a single contributor with identities
    /// reproduces that contributor bit-for-bit (the determinism-lock invariant, cf. F4).
    fn compose(self, o: Effect) -> Effect {
        Effect {
            food_mult: self.food_mult * o.food_mult,
            energy_add: self.energy_add + o.energy_add,
            metab_mult: self.metab_mult * o.metab_mult,
            mortality_add: self.mortality_add + o.mortality_add,
        }
    }
}

/// A pure environmental selection pressure. `eval` reads only `Sample` and returns an `Effect` — no
/// side effects, no RNG, no `&mut`. That purity is what makes the registry safe to extend and
/// (later) parallel.
pub trait SelectionPressure: Send + Sync {
    /// Stable identifier (for introspection / the dev bridge in a later PR).
    fn id(&self) -> &'static str;
    fn eval(&self, s: &Sample) -> Effect;
}

/// The set of active pressures. `step` composes them per creature into one `Effect`.
pub struct PressureRegistry {
    active: Vec<Box<dyn SelectionPressure>>,
}

impl PressureRegistry {
    /// Build the active set from a config: a pure energy-budget pressure is present iff its feature
    /// is on (membership = toggle). Metabolism is intrinsic (every body pays it), so it is always
    /// present — its per-stratum variation is what the `strata` feature governs (via `stratum_of`).
    /// Each pressure is given its tunable parameters from `Params` (defaults reproduce the consts).
    pub fn build(f: &Features, p: &Params) -> Self {
        let mut active: Vec<Box<dyn SelectionPressure>> = Vec::new();
        if f.climate {
            active.push(Box::new(climate::Climate { thermal_penalty: p.thermal_penalty }));
        }
        if f.autotrophy {
            active.push(Box::new(autotrophy::Autotrophy { photo_rate: p.photo_rate }));
        }
        active.push(Box::new(metabolism::Metabolism {
            air: p.air_metab_mult,
            underground: p.underground_metab_mult,
        }));
        if f.toxicity {
            active.push(Box::new(toxicity::Toxicity { lethality: p.toxin_lethality }));
        }
        PressureRegistry { active }
    }

    /// Compose every active pressure's effect for one creature.
    pub fn eval_all(&self, s: &Sample) -> Effect {
        let mut e = Effect::identity();
        for p in &self.active {
            e = e.compose(p.eval(s));
        }
        e
    }

    /// Ids of the active pressures, in evaluation order (for introspection / tests).
    pub fn ids(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.active.iter().map(|p| p.id())
    }
}

impl Default for PressureRegistry {
    /// The all-on, default-params set (the determinism golden).
    fn default() -> Self {
        PressureRegistry::build(&Features::default(), &Params::default())
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
