//! Ф0 ECS components (R7). Hot/warm split per doc 12 §3.
//!
//! Double-buffered (read `t`, write `t+1`, swapped by stage 10): `Position`/`PositionNext` (in
//! lib.rs), `Velocity`/`VelocityNext`, and the `Intent` Act→Move staging buffer. **`Energy` is NOT
//! double-buffered** — it is an ORDERED multi-writer (Metabolism subtracts, then Interactions adds,
//! with a fork-join barrier between), so no entity is read-and-written at once within a tick.
//! `Sensors` (warm) is NOT buffered — written once in Sense, read in Act.

use crate::{Vec2Fixed, BRAIN_HIDDEN, BRAIN_OUTPUTS};
use bevy_ecs::prelude::{Component, Entity};

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

/// Warm sensor cache (read-old): the sampled CONSERVED resource gradient (integer) + local amount.
/// Written by Sense (stage 1), consumed by Brain (stage 2). Not double-buffered.
/// `signal_gradient` was removed (M3/F3): the signal field is intentionally not fed to the integer
/// brain in M3; the dead per-tick f32 compute was eliminated. Signal still contributes to
/// `state_hash` via `signal_hash()` (observational). Now pure-integer → derives `Eq`.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sensors {
    pub gradient: Vec2Fixed,
    pub local_resource: i64,
}

/// Species tag (cold). Inherited by offspring; speciation check in stage_birth_death.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct SpeciesId(pub u32);

/// Marker placed on every newly born entity by stage_birth_death. Consumed and removed by
/// `Sim::process_pending_speciation()` (runs after all stages) which computes the L1
/// brain-weight distance and finalises the SpeciesId. Never enters state_hash.
#[derive(Component, Default)]
pub struct PendingSpeciation;

/// Per-entity mineral quota stock (D′-3a). Present only when `EconParams.mineral_layer.is_some()`.
/// Mineral flows: field → quota (Monod uptake, stage_mineral_feed); quota → spent at division
/// (`q_mineral` deducted); quota → field on death (recycle fraction). Overflow: when
/// energy-ready but quota < q_mineral, the cell burns `overflow_delta` energy as heat.
/// Founders and newborns start at 0 (no inherited mineral).
/// Conservation: every mineral-eu in quota + field_mineral = conserved (tracked in EnergyLedger
/// alongside energy, since all conserved layers are unified in `conserved_total_all()`).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct MineralQuota(pub i64);

/// P-1 propagule growth primitive (#429): current MATERIALISED cell count (≤
/// `Phenotype.graph.growth_cells.len()`, the decoded target). Initialised at EVERY spawn seam —
/// founders and non-propagule newborns start full (`Grown = target`); a propagule child starts at
/// `Grown = n_eff`. Warm — incremented by `stage_grow`, at most ONE cell per metabolism tick, only
/// when `EconParams.enable_propagule`. Always present (unlike `MineralQuota`, which is Option-
/// gated) but hash-folded ONLY under that flag (`Sim::state_hash`) — off ⇒ present but unfolded ⇒
/// byte-identical goldens.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Grown(pub u8);

/// P-2b provisioning (#448): the child's link back to its parent, recorded at birth. Present ONLY
/// when `EconParams.enable_provision` — founders/world-init get no `Parent` link (they have no
/// parent). NOT folded into `state_hash` (critic F119): folding raw entity bits would pin any
/// future flag-on golden to Bevy's `Entities` index/generation reuse, an allocator internal, not
/// simulation state. A stale/recycled parent (despawned) is detected at READ time (`q.get(parent)
/// → Err`), never dereferenced blindly.
#[derive(Component, Clone, Copy, Debug)]
pub struct Parent(pub Entity);

/// P-2b provisioning (#448): a still-growing child's non-liquid energy bank, filled by the parent
/// (`5a_provision`) and spent ONLY by `stage_grow` (never by metabolism/repro — the F3/F5 firewall:
/// provisioning funds the offspring's BODY, never its liquid `Energy`/premature reproduction).
/// Present ONLY when `EconParams.enable_provision`; init `0`. Folded into `state_hash` ONLY when
/// non-zero (same pattern as `MineralQuota`) — a TRIPWIRE, not a live signal: under the same-tick-
/// drain invariant (all-or-nothing grants, critic F131/F133) `Provisioned` is provably `0` at
/// every `state_hash` boundary, so the fold is vacuous while the invariant holds and only
/// perturbs the hash if it is ever broken (critic F138).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Provisioned(pub i64);

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
