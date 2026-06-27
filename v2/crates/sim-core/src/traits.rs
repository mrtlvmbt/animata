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
    fn conserved_at(&self, pos: Vec2Fixed, layer: usize) -> i64;
    fn conserved_gradient(&self, pos: Vec2Fixed, range: i64, layer: usize) -> (i64, i64);
    /// Remove up to `amount` from the cell; returns the EXACT amount removed.
    fn conserved_take(&mut self, pos: Vec2Fixed, amount: i64, layer: usize) -> i64;
    fn conserved_total(&self, layer: usize) -> i64;
    /// Sum of `conserved_total` across ALL layers (the energy-balance quantity).
    fn conserved_total_all(&self) -> i64;
    /// Deterministic hash of the conserved grid (integer, canonical cell order) — the R14 subject.
    /// At L=1 the fold is byte-identical to the pre-A1 flat fold (no layer index mixed in).
    fn conserved_hash(&self) -> u64;

    // ── signal field (f32, NOT in the balance) ──────────────────────────────────────────────────────
    // `signal_at` (bilinear sample) removed (M2/F3): had no real consumer in the tick loop.
    // `signal_gradient` removed (M3/F3): the integer brain never read it; dead per-tick f32 work.
    // Both may be re-added when a real consumer lands; the data (signal grid) is still maintained.
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

// ── Brain (M3) — fixed topology, evolved weights, INTEGER fixed-point inference. ──────────────────
// Network shape is a fixed const (D-Brain-1): I inputs → H recurrent hidden → O motor outputs.
// Per-creature weights are an `int8` vector resident in the genome. Activations/hidden are `FixedI16`
// (Q8.8). The rescale is an integer SHIFT (`acc >> BRAIN_SHIFT`), NOT a float multiplier (F11/D-Brain-3).

/// Sensor inputs fed to the net (quantized at the Sense→Brain boundary).
pub const BRAIN_INPUTS: usize = 6;
/// Recurrent hidden units (the `BrainState`).
pub const BRAIN_HIDDEN: usize = 8;
/// Motor outputs (`BrainOutput` → Act).
pub const BRAIN_OUTPUTS: usize = 2;
/// Per-creature weight count: W_ih (H·I) + W_hh (H·H, recurrent) + W_ho (O·H).
pub const BRAIN_WEIGHTS: usize =
    BRAIN_HIDDEN * BRAIN_INPUTS + BRAIN_HIDDEN * BRAIN_HIDDEN + BRAIN_OUTPUTS * BRAIN_HIDDEN;
/// Accumulator rescale: `value = acc >> BRAIN_SHIFT`. int8 weight (Q1.7) × FixedI16 input (Q8.8) →
/// product scale 2^15; shifting by 7 returns Q8.8. Pure-integer ⇒ associative & arch-invariant.
pub const BRAIN_SHIFT: u32 = 7;

// Weight-vector layout — the SINGLE source of truth shared by the genome (founder/mutation, which
// write into the flat `[i8; BRAIN_WEIGHTS]`) and the `brain` crate (which reads it during inference),
// so the two can never drift. Three dense blocks packed in this order: W_ih, W_hh, W_ho.

/// Flat index of `W_ih[j][i]` — input `i` → hidden `j`.
#[inline]
pub const fn brain_w_ih(j: usize, i: usize) -> usize {
    j * BRAIN_INPUTS + i
}
/// Flat index of `W_hh[j][k]` — hidden `k` → hidden `j` (the recurrent block).
#[inline]
pub const fn brain_w_hh(j: usize, k: usize) -> usize {
    BRAIN_HIDDEN * BRAIN_INPUTS + j * BRAIN_HIDDEN + k
}
/// Flat index of `W_ho[o][j]` — hidden `j` → output `o`.
#[inline]
pub const fn brain_w_ho(o: usize, j: usize) -> usize {
    BRAIN_HIDDEN * BRAIN_INPUTS + BRAIN_HIDDEN * BRAIN_HIDDEN + o * BRAIN_HIDDEN + j
}

/// Per-creature controller seam (declared as a type since M0, implemented by the `brain` crate at
/// M3). One `infer` call reads inputs + recurrent `h_old` + the creature's weights, writes the new
/// hidden `h_new` and the motor `out`. PURE INTEGER — no float anywhere in an implementor.
pub trait Brain: Send + Sync {
    fn infer(
        &self,
        inputs: &[i16; BRAIN_INPUTS],
        h_old: &[i16; BRAIN_HIDDEN],
        weights: &[i8; BRAIN_WEIGHTS],
        h_new: &mut [i16; BRAIN_HIDDEN],
        out: &mut [i16; BRAIN_OUTPUTS],
    );
}

/// Boxed brain backend, injected by `cli` (keeps R1).
#[derive(Resource)]
pub struct BrainRes(pub Box<dyn Brain>);

/// Boxed world backend, injected by `cli` (keeps R1).
#[derive(Resource)]
pub struct WorldRes(pub Box<dyn WorldView>);

/// Boxed field backend, injected by `cli` (keeps R1).
#[derive(Resource)]
pub struct FieldRes(pub Box<dyn FieldStore>);
