//! Trait boundaries — fixed AS TYPES in `sim-core` (R1). Backends (`world`/`fields`) are injected as
//! boxed trait objects by `cli`. M2 expands `FieldStore` to TWO field classes (doc 14 §1):
//! conserved (fixed-point integer, in the energy balance, thread-count-independent) and signal (f32,
//! NOT in the balance, diffuses/decays, deterministic only under a fixed reduction order).

use crate::Vec2Fixed;
use bevy_ecs::prelude::Resource;

/// One agent's scatter contribution for stage 8. Carries the canonical sort key `(morton, entity)`
/// so `commit_merge` can fold in `(Morton → Entity-id)` order regardless of how threads partitioned.
#[derive(Clone, Copy, Debug)]
pub struct Deposit {
    pub cell: usize,
    pub morton: u32,
    pub entity_bits: u64,
    /// Conserved (integer) contribution — agent→field excretion (exact, in the energy balance).
    pub conserved: i64,
    /// Signal (f32) contribution — pheromone deposit (NOT in the balance).
    pub signal: f32,
}

/// How `commit_merge` folds the per-thread deposit batches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeStrategy {
    /// The correct path: flatten → sort by `(morton, entity)` → apply in that single serial order.
    /// Conserved integer add is associative ⇒ the result is **identical for any thread count** (R14).
    Canonical,
    /// A DELIBERATELY BROKEN path for the R14 negative test: folds the N per-thread partial sums with
    /// a non-associative, count-sensitive combine ⇒ the conserved result DEPENDS on the thread count
    /// ⇒ the R14 gate goes RED. Proves the gate has teeth (F1). Never used in production.
    NonAssociative,
}

/// Read-mostly world query (R29). Float worldgen (heightmap noise) may live behind a feature in the
/// `world` backend — that float is what makes the trajectory arch-dependent.
pub trait WorldView: Send + Sync {
    fn is_solid(&self, pos: Vec2Fixed) -> bool;
    fn height(&self, x: i64, z: i64) -> i64;
    fn biome(&self, pos: Vec2Fixed) -> u8;
    fn resource(&self, pos: Vec2Fixed) -> i64;
}

/// The field backend — one CONSERVED resource field (fixed-point integer) and one SIGNAL field (f32)
/// coexisting in a tick (R13). Scatter (stage 8) is multithreaded; the conserved merge is integer-
/// associative (thread-count-independent, R14), the signal merge is float in a fixed serial order.
pub trait FieldStore: Send + Sync {
    // ── meta ──────────────────────────────────────────────────────────────────────────────────────
    fn m_field(&self) -> i64;
    fn cell_index(&self, pos: Vec2Fixed) -> usize;
    /// Morton (Z-order) code of the cell containing `pos` — the primary canonical merge key.
    fn cell_morton(&self, pos: Vec2Fixed) -> u32;
    fn check_meta(&self, expected_m_field: i64) -> Result<(), String>;

    // ── conserved field (integer, in the energy balance) ────────────────────────────────────────────
    fn conserved_at(&self, pos: Vec2Fixed) -> i64;
    fn conserved_gradient(&self, pos: Vec2Fixed, range: i64) -> (i64, i64);
    /// Remove up to `amount` from the cell; returns the EXACT amount removed.
    fn conserved_take(&mut self, pos: Vec2Fixed, amount: i64) -> i64;
    fn conserved_total(&self) -> i64;
    /// Deterministic hash of the conserved grid (integer, canonical cell order) — the R14 subject.
    fn conserved_hash(&self) -> u64;

    // ── signal field (f32, NOT in the balance) ──────────────────────────────────────────────────────
    /// Bilinear sample of the signal field.
    fn signal_at(&self, pos: Vec2Fixed) -> f32;
    /// Finite-difference signal gradient (smooth chemotaxis).
    fn signal_gradient(&self, pos: Vec2Fixed) -> (f32, f32);
    /// Σ signal — a SERIAL reduction (no parallel float fold, F2). For telemetry.
    fn signal_total(&self) -> f32;
    /// Hash of the signal grid (f32 bits) — arch-bound, folded into the golden only.
    fn signal_hash(&self) -> u64;
    /// NaN/Inf guard — every signal cell must be finite (always-on in the release harness).
    fn signal_all_finite(&self) -> bool;

    // ── scatter + between-tick solver (stage 8) ─────────────────────────────────────────────────────
    /// Merge the per-thread deposit batches into the staging buffers per `strategy`. Conserved =
    /// integer associative; signal = float in canonical `(morton, entity)` serial order. No
    /// float-atomic anywhere.
    fn commit_merge(&mut self, batches: &[Vec<Deposit>], strategy: MergeStrategy);
    /// Apply staged deposits → grid (the `t+1` snapshot, R17), regenerate the conserved source (returns
    /// total injected), diffuse the conserved field (integer flux), and blur+decay the signal field.
    fn solve(&mut self) -> i64;
}

/// Per-creature controller seam. Real neuro-inference is M3; unused in Ф0/M2.
pub trait Brain: Send + Sync {
    fn decide(&self, sensors: &[i64], out: &mut [i64]);
}

/// Boxed world backend, injected by `cli` (keeps R1).
#[derive(Resource)]
pub struct WorldRes(pub Box<dyn WorldView>);

/// Boxed field backend, injected by `cli` (keeps R1).
#[derive(Resource)]
pub struct FieldRes(pub Box<dyn FieldStore>);
