//! Ф0 ECS components (R7). Hot/warm split per doc 12 §3.
//!
//! Double-buffered (read `t`, write `t+1`, swapped by stage 10): `Position`/`PositionNext` (in
//! lib.rs), `Velocity`/`VelocityNext`, and the `Intent` Act→Move staging buffer. **`Energy` is NOT
//! double-buffered** — it is an ORDERED multi-writer (Metabolism subtracts, then Interactions adds,
//! with a fork-join barrier between), so no entity is read-and-written at once within a tick.
//! `Sensors` (warm) is NOT buffered — written once in Sense, read in Act.

use crate::{Vec2Fixed, BRAIN_HIDDEN, BRAIN_OUTPUTS};
use bevy_ecs::prelude::Component;

/// Horizontal velocity (cells/tick), integer. Double-buffered with [`VelocityNext`].
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Velocity(pub Vec2Fixed);

/// Write-side of the [`Velocity`] double buffer.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct VelocityNext(pub Vec2Fixed);

/// Energy ledger of one organism, fixed-point integer `eu` (R13). The conserved currency.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Energy(pub i64);

/// Act→Move intent buffer: the desired velocity chemotaxis chose this tick.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Intent(pub Vec2Fixed);

/// Warm sensor cache (read-old): the sampled CONSERVED resource gradient (integer) + local amount +
/// the cached effective sensing expression. Written by Sense (stage 1), consumed by Brain (stage 2)
/// and Metabolism (stage 5). Not double-buffered.
/// `signal_gradient` was removed (M3/F3): the signal field is intentionally not fed to the integer
/// brain in M3. Now pure-integer → derives `Eq`.
///
/// `effort` is derived state (= `Genome::sense_range_eff(local_resource)`) — re-computed each
/// Sense tick, NOT itself hashed (F9 covers the genome fields that produce it, not the cache).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sensors {
    pub gradient: Vec2Fixed,
    pub local_resource: i64,
    /// Cached effective sensing expression for this tick (D-slice GRN seed).
    /// Written once in `stage_sense`, read by BOTH `stage_sense` (gradient radius) AND
    /// `stage_metabolism` (sense cost) — single computation, two reads, cost and benefit coupled.
    pub effort: i32,
}

/// Species tag (cold). Inherited by offspring; speciation check in stage_birth_death.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct SpeciesId(pub u32);

/// Marker placed on every newly born entity by stage_birth_death. Consumed and removed by
/// `Sim::process_pending_speciation()` (runs after all stages) which computes the L1
/// brain-weight distance and finalises the SpeciesId. Never enters state_hash.
#[derive(Component, Default)]
pub struct PendingSpeciation;

/// Recurrent hidden state of the brain (M3 / D-Brain-2) — a per-entity **double buffer** of the
/// `H = BRAIN_HIDDEN` hidden units (`FixedI16` Q8.8). All recurrent edges read `h_old` and write
/// `h_new`; the buffers are swapped **only on Brain ticks** (1/K), so between Brain ticks the hidden
/// state is frozen (the replay reproduces that). In the ECS each field is a SoA archetype column, so
/// this is the per-entity equivalent of the plan's "whole-array pointer swap".
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrainState {
    pub h_old: [i16; BRAIN_HIDDEN],
    pub h_new: [i16; BRAIN_HIDDEN],
}

impl BrainState {
    /// A freshly-zeroed hidden state (`h = 0`). The spawn contract (D-Brain-2a) hands every newborn
    /// THIS, in BOTH buffers, so no prior occupant's hidden state can leak through a reused ECS slot.
    pub const fn zeroed() -> Self {
        BrainState { h_old: [0; BRAIN_HIDDEN], h_new: [0; BRAIN_HIDDEN] }
    }
}

impl Default for BrainState {
    fn default() -> Self {
        Self::zeroed()
    }
}

/// The motor decision the brain produced on its last Brain tick (M3 / D-Brain-4) — `O = BRAIN_OUTPUTS`
/// `FixedI16` (Q8.8) outputs. Act reads it at the BASE rhythm (every tick) and it PERSISTS between
/// Brain ticks (1/K). Zeroed on spawn (D-Brain-2a) so a reused slot can never act on a corpse's
/// command, and so a newborn born off-phase is frozen (neutral) until its first global Brain tick.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrainOutput {
    pub out: [i16; BRAIN_OUTPUTS],
}

impl BrainOutput {
    /// A neutral (no-op) motor command — the newborn / between-Brain-tick default.
    pub const fn zeroed() -> Self {
        BrainOutput { out: [0; BRAIN_OUTPUTS] }
    }
}

impl Default for BrainOutput {
    fn default() -> Self {
        Self::zeroed()
    }
}
