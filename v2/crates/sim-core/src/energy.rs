//! Exact integer energy conservation (R15, F3). Every `eu` lives in exactly one bucket; every stage
//! moves `eu` between buckets with exact integer add/sub. The audited TOTAL is therefore invariant:
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
}

impl EnergyLedger {
    /// Residual of the conservation identity. MUST be 0 every tick.
    pub fn residual(&self, field_total: i64, agents_total: i64) -> i64 {
        (field_total + agents_total + self.dissipated + self.lost) - self.produced - self.initial
    }
}
