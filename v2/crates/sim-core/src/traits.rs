//! Trait boundaries — fixed AS TYPES in `sim-core` (R1: the core depends on NO backend crate; the
//! concrete `world`/`fields` backends are injected as boxed trait objects by `cli`). The cargo dep
//! graph therefore guarantees no render/IO/world types leak into the deterministic core.

use crate::Vec2Fixed;
use bevy_ecs::prelude::Resource;

/// Read-mostly query interface to the world (R29). The CPU heightmap backend lives in `world`.
/// `height`/`biome`/`resource` may be derived from float worldgen noise behind a feature — that is
/// the float that makes the M1 trajectory arch-dependent (hence the arm64-only golden). The
/// *conserved* layer (energy + the resource field amounts) stays pure integer regardless.
pub trait WorldView: Send + Sync {
    fn is_solid(&self, pos: Vec2Fixed) -> bool;
    fn height(&self, x: i64, z: i64) -> i64;
    fn biome(&self, pos: Vec2Fixed) -> u8;
    /// Static resource POTENTIAL at a position (the per-cell regeneration cap). The DYNAMIC amount
    /// lives in [`FieldStore`].
    fn resource(&self, pos: Vec2Fixed) -> i64;
}

/// The conserved resource field (R13). **Fixed-point integer end-to-end** — every agent↔field
/// exchange is an exact integer add/sub, so conservation holds by construction and the integer merge
/// is associative (R14: thread-count independent, though M1 applies serially). No float here, ever.
pub trait FieldStore: Send + Sync {
    /// Voxels per field cell — integer, immutable for the run, checked on load (R8).
    fn m_field(&self) -> i64;
    /// Map a world position to a field-cell linear index (integer division, no float rounding).
    fn cell_index(&self, pos: Vec2Fixed) -> usize;
    /// Current amount in the cell containing `pos`.
    fn amount_at(&self, pos: Vec2Fixed) -> i64;
    /// Integer central-difference gradient of the resource over `±range` cells. Drives chemotaxis.
    fn gradient_at(&self, pos: Vec2Fixed, range: i64) -> (i64, i64);
    /// Remove up to `amount` from the cell containing `pos`; returns the EXACT amount removed
    /// (≤ cell content). Sequential calls = first-come, deterministic when ordered by Entity-id.
    fn take_at(&mut self, pos: Vec2Fixed, amount: i64) -> i64;
    /// Stage a conserved contribution into the cell (applied to the NEXT tick — R17).
    fn scatter_at(&mut self, pos: Vec2Fixed, amount: i64);
    /// Commit staged scatter into the live field (the FieldScatter→next-tick boundary).
    fn apply_scatter(&mut self);
    /// Regenerate toward the per-cell cap; returns total injected (the explicit conservation SOURCE).
    fn regenerate(&mut self) -> i64;
    /// Conservative integer flux diffusion (§5.1) — Σ field is invariant EXACTLY (no ε).
    fn diffuse(&mut self);
    /// Σ of the whole field — for the energy-conservation audit.
    fn total(&self) -> i64;
    /// R8 load check: the build-time `M_field` must match the expected value or refuse.
    fn check_meta(&self, expected_m_field: i64) -> Result<(), String>;
}

/// Per-creature controller. Real neuro-inference is M3; the seam exists so the core never hard-codes
/// a control policy. Unused in M1 (chemotaxis in stage Act is brain-less).
pub trait Brain: Send + Sync {
    fn decide(&self, sensors: &[i64], out: &mut [i64]);
}

/// Boxed world backend, injected by `cli` and stored as an ECS resource (keeps R1).
#[derive(Resource)]
pub struct WorldRes(pub Box<dyn WorldView>);

/// Boxed conserved-field backend, injected by `cli` and stored as an ECS resource (keeps R1).
#[derive(Resource)]
pub struct FieldRes(pub Box<dyn FieldStore>);
