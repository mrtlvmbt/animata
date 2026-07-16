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
    /// P-2b (#448): cumulative deaths per [`DeathChannel`], bumped by [`count_death`] at the
    /// EXACT alive→dead transition of every death-owning system. Golden-neutral (`EnergyLedger`
    /// isn't in `state_hash`); P-3's survival-to-maturity denominator (with `deaths_growing_by_channel`).
    pub deaths_by_channel: [u64; 4],
    /// P-2b (#448): the subset of `deaths_by_channel` where the body was still growing
    /// (`grown.0 < growth_cells.len()`) at death — P-3's during-growth mortality numerator.
    pub deaths_growing_by_channel: [u64; 4],
    /// P-2b (#448): cumulative `eu` transferred parent→child by `5a_provision` — the ON-arm DOSE
    /// (critic F107). Lets P-3 distinguish "mechanism didn't fire" (≈0) from "fired, didn't help".
    pub provision_granted_total: u64,
    /// P-2b (#448, critic F143/F148): cumulative `eu` released by [`release_provisioned`] (the FIVE
    /// zero-energy death sites) AND the d0 inline recycle-fold. MUST stay `== 0` under all-or-nothing
    /// grants (same-tick-drain, F131/F133) — `debug_assert!` is compiled OUT in `--release`, so this
    /// counter is the falsifiability backstop: a broken invariant would otherwise route a bank into
    /// `lost` silently with `conservation_residual()` staying 0 (nothing else would observe it).
    pub provisioned_released_total: u64,
}

/// P-2b (#448, critic F111): a 4-variant death-channel enum — d0 (background death) is a FOURTH
/// death mode alongside predation/starvation/hazard-adjacent branches, so `deaths_by_channel`/
/// `deaths_growing_by_channel` count ALL deaths, not just the three that pre-existed this slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeathChannel {
    Hazard,
    Predation,
    Starvation,
    Background,
}

/// P-2b (#448, critic F1/F2): the LIVE, P-3-load-bearing death counter — called EXACTLY at the
/// alive→dead transition of every death-owning system, with `growing` computed AT the call site
/// (the ONE source of truth for "was this body still growing" — never re-derived here, critic F64).
/// Deliberately NOT welded to [`release_provisioned`] (critic F1/F2 — welding a live counter to a
/// defensive energy release behind one predicate is the exact mechanism of an F1 miscount: a body
/// with no `Provisioned` component, or a `Provisioned(0)` bank, must still be counted as a death).
pub fn count_death(ledger: &mut EnergyLedger, channel: DeathChannel, growing: bool) {
    ledger.deaths_by_channel[channel as usize] += 1;
    if growing {
        ledger.deaths_growing_by_channel[channel as usize] += 1;
    }
}

/// P-2b (#448, critic F105): the DEFENSIVE, provably-zero energy release at a death site — under
/// all-or-nothing grants (5a) the bank is drained to 0 same-tick by `stage_grow`, so this books 0 in
/// production; it exists as the R15 tripwire (`provisioned_released_total`) that would catch a
/// broken invariant in `--release` (where the `debug_assert!` in `stage_grow`/death sites is
/// compiled out). `&mut` + `mem::take` ⇒ IDEMPOTENT under re-entry (critic F105 — a second call on
/// the same entity/tick releases 0, never double-books). `channel` is for symmetry/logging only —
/// it does NOT bump `deaths_by_channel` (that is [`count_death`]'s job, called separately).
pub fn release_provisioned(ledger: &mut EnergyLedger, prov: Option<&mut crate::Provisioned>, _channel: DeathChannel) {
    let taken = prov.map_or(0, |p| std::mem::take(&mut p.0));
    ledger.lost += taken;
    debug_assert!(taken >= 0, "release_provisioned: Provisioned bank must never go negative");
    ledger.provisioned_released_total += taken as u64;
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
    /// P-2b (#448): cumulative deaths per [`DeathChannel`] (`[Hazard, Predation, Starvation,
    /// Background]`), and the still-growing subset — P-3's survival-to-maturity denominators.
    pub deaths_by_channel: [u64; 4],
    pub deaths_growing_by_channel: [u64; 4],
    /// P-2b (#448): cumulative `eu` transferred parent→child by `5a_provision` (the ON-arm dose).
    pub provision_granted_total: u64,
    /// P-2b (#448): cumulative `eu` released by a death-site defensive drain — MUST stay 0 under
    /// all-or-nothing grants (the R15 falsifiability tripwire, since `debug_assert!` is compiled
    /// out in `--release`).
    pub provisioned_released_total: u64,
}
