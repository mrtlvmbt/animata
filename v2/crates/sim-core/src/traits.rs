//! Trait boundaries fixed AS TYPES in `sim-core` from M0 (no empty `world`/`fields`/`brain` crates
//! yet — F6). R1 (pure core: no render/IO/backend types in `sim-core`) holds by the cargo dependency
//! graph: these signatures reference only core integer types. The CPU/GPU backends implementing them
//! land at M1 (`world`, `fields`) and M3 (`brain`).
//!
//! M0 has no biology, so the traits are declared but unused — that is intentional (the seam is
//! reserved, not built).

use crate::Vec2Fixed;

/// Read-mostly query interface to the world. Backend (heightmap/voxel) arrives at M1.
pub trait WorldView {
    fn is_solid(&self, pos: Vec2Fixed) -> bool;
    fn height(&self, x: i64, z: i64) -> i64;
}

/// Environment resource/signal field store. CPU backend at M1; a GPU backend later implements the
/// same trait without touching callers.
pub trait FieldStore {
    /// Conserved resource amount at a cell (fixed-point integer domain).
    fn resource_at(&self, pos: Vec2Fixed) -> i64;
}

/// Per-creature controller. Real neuro-inference lands at M3; the seam exists now so the core never
/// hard-codes a control policy.
pub trait Brain {
    fn decide(&self, sensors: &[i64], out: &mut [i64]);
}
