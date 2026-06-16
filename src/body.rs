//! Locomotion abstraction — the body turns brain drive into actual movement.
//!
//! Capability model (no joint physics, per the macroevolution plan, fork 1): a
//! body yields aggregate [`LocomotionStats`] for a given [`Medium`]. The
//! [`Locomotor`] trait is the seam left for fork 2 — a future joint-physics
//! implementation can replace the capability math behind it without touching the
//! movement code in `creature.rs`.
//!
//! Phase 1 ships a single-segment body that reproduces the old `max_speed`
//! exactly (medium ignored), so behavior is byte-identical. Phase 2 grows the
//! segment chain + appendages and makes `thrust`/`drag` depend on the medium.

use crate::genome::Phenotype;

/// Physical medium a creature is moving through. Drives which body plan is
/// efficient (fins in water, wings in air). Only `Ground` is wired into movement
/// in Phase 1; the others land with biome media in Phase 2.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)] // Water/Air consumed starting in Phase 2 (medium physics).
pub enum Medium {
    Ground,
    Water,
    Air,
}

/// Aggregate locomotion capability of a body in a given medium.
pub struct LocomotionStats {
    /// Top forward speed the body can produce at full drive (px/step), before
    /// terrain drag (`move_mult`) is applied by the caller.
    pub thrust: f32,
}

/// Turns body morphology into locomotion capability. Capability implementation
/// now; a joint-physics implementation can replace it behind this trait (fork 2).
pub trait Locomotor {
    fn locomotion(&self, medium: Medium) -> LocomotionStats;
}

impl Locomotor for Phenotype {
    /// Single-segment Phase-1 body: thrust is just the decoded `max_speed`,
    /// independent of medium — numerically identical to the pre-seam movement.
    fn locomotion(&self, _medium: Medium) -> LocomotionStats {
        LocomotionStats { thrust: self.max_speed }
    }
}
