//! Exact integer energy conservation (R15, F3). Every `eu` lives in exactly one bucket; every stage
//! moves `eu` between buckets with exact integer add/sub. The audited TOTAL is therefore invariant:
// Guard: no float arithmetic in the conserved layer (M0/F2). Complements the token-grep in
// no_float_guard.rs: `float_arithmetic` catches operations on inferred-float types that the grep
// misses (e.g. `let x = 1.5; x + 1.0` where no `f32`/`f64` keyword appears).
#![deny(clippy::float_arithmetic)]
//!
//! ```text
//! TOTAL = Σ(field) + Σ(agent energy) + dissipated + lost − produced  ==  initial   (∀ tick)
//! ```
//!
//! so the residual `(field + agents + dissipated + lost) − produced − initial` is **EXACTLY 0** —
//! not `±ε` (that was a float legacy; the integer ledger has no rounding leak). The check is run in
//! the `cli` golden-harness, always-on, **active in `--release`** (F8 — CI runs release).

use bevy_ecs::prelude::Resource;

/// Cumulative energy accounting (the sink/source buckets). Live agent + field energy are summed at
/// audit time from the ECS / field, not stored here.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct EnergyLedger {
    /// Σ(field) + Σ(agents) at construction — the conserved constant.
    pub initial: i64,
    /// Cumulative regeneration injected (the explicit SOURCE).
    pub produced: i64,
    /// Cumulative energy dissipated as heat: base metabolism, movement/sensing cost, feeding
    /// inefficiency, division overhead.
    pub dissipated: i64,
    /// Cumulative unrecycled body energy at death (0 in Ф0 — death only at energy 0 — but tracked so
    /// the bucket exists when recycling lands).
    pub lost: i64,
    /// P-2a (#442): grow-step diagnostic buckets, indexed by `GrowGate` discriminant
    /// (`Grow`/`BlockedLump`/`BlockedCell`), bumped inside `stage_grow` off the SAME `grow_gate`
    /// call that decides growth — so each still-growing metab tick hits exactly one slot and the
    /// buckets can never drift from the decision (critic F74/F81). NOT in `state_hash`/conservation
    /// (golden-neutral, not folded here — see `conservation_residual`).
    pub grow_step_counts: [u64; 3],
    /// P-2a (#442): cumulative maturation count, bumped inside `stage_grow`'s growth branch right
    /// after `grown.0 += 1`, iff the body just reached its decoded target (critic F126 — NOT at the
    /// maturity early-`continue`, which fires every metab tick for already-mature bodies).
    pub maturations_total: u64,
}

impl EnergyLedger {
    /// Residual of the conservation identity. MUST be 0 every tick.
    pub fn residual(&self, field_total: i64, agents_total: i64) -> i64 {
        (field_total + agents_total + self.dissipated + self.lost) - self.produced - self.initial
    }
}

/// P-2a (#442): read-only grow-step diagnostics snapshot, exposed via `Sim::ledger_snapshot()` (cf.
/// `Sim::conservation_residual()`). `grow_steps_total` is DERIVED (`Σ grow_step_counts`), NOT bumped
/// independently, so the denominator cannot drift from the buckets (critic F81). This slice's fields
/// are the P-2a set; P-2b EXTENDS the struct with the death-channel + provision fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LedgerSnapshot {
    pub blocked_lump: u64,
    pub blocked_cell: u64,
    pub grow_steps_total: u64,
    pub maturations_total: u64,
}
