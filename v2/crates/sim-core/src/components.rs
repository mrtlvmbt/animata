//! Ф0 ECS components (R7). Hot/warm split per doc 12 §3.
//!
//! Double-buffered (read `t`, write `t+1`, swapped by stage 10): `Position`/`PositionNext` (in
//! lib.rs), `Velocity`/`VelocityNext`, and the `Intent` Act→Move staging buffer. **`Energy` is NOT
//! double-buffered** — it is an ORDERED multi-writer (Metabolism subtracts, then Interactions adds,
//! with a fork-join barrier between), so no entity is read-and-written at once within a tick.
//! `Sensors` (warm) is NOT buffered — written once in Sense, read in Act.

use crate::Vec2Fixed;
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

/// Warm sensor cache (read-old): the sampled resource gradient + local amount. Written by Sense,
/// consumed by Act. Not buffered.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Sensors {
    pub gradient: Vec2Fixed,
    pub local_resource: i64,
}

/// Species tag (cold). Inherited by offspring; no speciation logic in Ф0.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SpeciesId(pub u32);
