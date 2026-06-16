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

use crate::genome::{Appendage, Phenotype};

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
    /// Capability locomotion: base thrust is the `max_speed` gene, scaled by how
    /// well the body's appendages suit the medium. Fins drive swimming, legs
    /// drive walking, wings flying; the wrong appendage (or none) leaves a body
    /// sluggish in that medium. A finless single-segment founder scores exactly
    /// 1.0 on the ground (movement unchanged) and poorly in water.
    fn locomotion(&self, medium: Medium) -> LocomotionStats {
        let (mut fins, mut legs, mut wings) = (0.0f32, 0.0f32, 0.0f32);
        for s in &self.segments {
            match s.appendage {
                Appendage::Fin => fins += 1.0,
                Appendage::Leg => legs += 1.0,
                Appendage::Wing => wings += 1.0,
                Appendage::None | Appendage::Burrow => {}
            }
        }
        // Diminishing returns: a few well-suited appendages give most of the
        // benefit, so there's no incentive to fill every segment with limbs — the
        // chain settles at an interior optimum once per-segment upkeep bites.
        let legs_eff = legs.min(3.0);
        let fins_eff = fins.min(3.0);
        let wings_eff = wings.min(3.0);
        // A finless, legless body scores 1.0 in both ground and water (water's
        // sluggishness still comes from the biome's move_mult, as before), so the
        // medium change doesn't turn rivers into death traps before there's any
        // aquatic food to reward fins (that arrives with Phase 2.3 niches). The
        // right appendage adds a bonus; the wrong one a mild penalty. Air stays
        // gated on wings (no air layer exists to enter yet).
        let factor = match medium {
            Medium::Ground => (1.0 + 0.30 * legs_eff - 0.10 * fins).clamp(0.30, 1.9),
            Medium::Water => (1.0 + 0.45 * fins_eff - 0.10 * legs).clamp(0.30, 1.9),
            Medium::Air => (0.10 + 0.55 * wings_eff).clamp(0.05, 1.9),
        };
        LocomotionStats { thrust: self.max_speed * factor }
    }
}
