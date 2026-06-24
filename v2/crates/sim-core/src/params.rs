//! Run parameters. `EconParams` are the on-the-shore economy numbers (economy/01); they are a
//! documented cargo-tunable contract (re-pinning the golden after a change is cheap). All integer.

use bevy_ecs::prelude::Resource;

/// Energy/space economy constants (integer `eu`). The energy SCALE is 1 eu = 1 integer unit here;
/// raising it (a documented cargo parameter) only rescales the ledger — conservation is unaffected.
#[derive(Resource, Clone, Copy, Debug)]
pub struct EconParams {
    /// Energy capacity of a body = stock handed to an offspring = recycle pool (one number — the
    /// single-pool invariant; splitting it into inconsistent values leaks energy, economy/01 §2).
    pub e_cell: i64,
    /// Division overhead (dissipated on each split).
    pub c_div: i64,
    /// Base metabolic floor per tick.
    pub base_metab: i64,
    /// Metabolic cost per `size^(3/4)` unit per tick.
    pub k_size_metab: i64,
    /// Movement cost per `move_speed` unit per tick.
    pub k_move_cost: i64,
    /// Sensing cost per `sense_range` unit per tick.
    pub k_sense_cost: i64,
    /// Max resource a cell can feed one agent per tick (Interactions).
    pub u_max: i64,
    /// Square world side length, in cells.
    pub world_dim: i64,
    /// Sim-neighbor grid scale `M` (cells per neighbor bucket) — integer, immutable, checked (R8).
    pub m_sim: i64,
    /// Metabolism sub-tick period N (R20). Ф0 = 1 (every tick); a system meta-constant, not dynamic.
    pub metab_period: u64,
}

impl Default for EconParams {
    fn default() -> Self {
        EconParams {
            e_cell: 1000,
            c_div: 100,
            base_metab: 2,
            k_size_metab: 1,
            k_move_cost: 1,
            k_sense_cost: 1,
            u_max: 220,
            world_dim: 64,
            m_sim: 4,
            metab_period: 1,
        }
    }
}

/// Construction-time configuration handed to `Sim::new`.
#[derive(Clone, Copy, Debug)]
pub struct SimConfig {
    pub seed: u64,
    pub n_founders: u64,
    pub founder_energy: i64,
    pub econ: EconParams,
}
