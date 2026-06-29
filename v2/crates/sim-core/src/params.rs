//! Run parameters. `EconParams` are the on-the-shore economy numbers (economy/01); they are a
//! documented cargo-tunable contract (re-pinning the golden after a change is cheap). All integer.

use crate::MergeStrategy;
use bevy_ecs::prelude::Resource;

// ── C-slice death-recycling constants ────────────────────────────────────────────────────────────

/// Bit-mask for the `d0` background-death RNG draw. `D0_MASK = 2^20 − 1`.
/// Kill condition: `(r & D0_MASK) < d0_scaled` — probability = d0_scaled / (D0_MASK+1).
/// At `d0_scaled=1049`: kill-prob ≈ 1049/1048576 ≈ 0.001/tick (mean lifetime ≈ 1000 ticks).
/// Pure integer compare — no float in the decision path (R13).
pub const D0_MASK: u64 = 0xF_FFFF; // 2^20 − 1 = 1_048_575

/// Denominator for the `recycle` fixed-point fraction. `recycle = recycle_num / RECYCLE_DEN`.
/// `RECYCLE_DEN = 256` (same scale as `metabolism_eff`) — single integer multiply + shift.
/// Valid range: `recycle_num ∈ [0, RECYCLE_DEN]`.
pub const RECYCLE_DEN: i64 = 256;

/// Per-layer field construction parameters carried by `SimConfig`.
/// `build_sim` reads the first `n_layers` entries; unused slots are ignored and may be zeroed.
///
/// Layer 0 always uses world-noise-derived per-cell caps (`WorldView::resource`).
/// Layers 1+ use `flat_cap` unless `world_cap_mult > 0`, in which case caps = world·mult.
#[derive(Clone, Copy, Debug, Default)]
pub struct LayerSpec {
    pub regen_rate: i64,
    pub flux_alpha_num: i64,
    pub flux_alpha_den: i64,
    /// Per-cell cap for layers 1+. `0` → empty start (initial mass = cap/2 = 0).
    /// Ignored for layer 0 (which always uses world-noise caps).
    pub flat_cap: i64,
    /// If > 0, use world-derived cap × this multiplier for layers 1+ (overrides `flat_cap`).
    pub world_cap_mult: i64,
}

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
    /// Monod half-saturation constant for substrate uptake (economy/01 §2). Uptake demand is
    /// `U(R) = u_max·R / (R+km)` (integer, truncating). Must be `> 0` (km=0 → 0/0 at R=0).
    /// Calibrated from the measured spatial equilibrium field value R̄: `km ≈ 2.3·R̄`
    /// (oligotrophic linear regime — economy/01 §2). Arch-dependent trajectory → fitted to
    /// the x86 runner (CI arch: ubuntu, the corridor measurement arch).
    pub km: i64,
    /// Asymptotic per-tick uptake capacity (the Monod U_max). At R≫Km, uptake → u_max.
    pub u_max: i64,
    /// Square world side length, in cells.
    pub world_dim: i64,
    /// Sim-neighbor grid scale `M` (cells per neighbor bucket) — integer, immutable, checked (R8).
    pub m_sim: i64,
    /// Brain (behaviour) period K (R20 / D-Brain-4) — inference runs on ticks where `tick % K == 0`
    /// (GLOBAL phase, not per-creature-from-birth, F5). K∈4..=6 ⇒ 10–30 Hz at the 64 Hz base. A
    /// per-system meta-constant, not adaptive (adaptive K/N is M4).
    pub brain_period: u64,
    /// Metabolism sub-tick period N (R20). M1 was N=1; M3 generalises to N∈2..=4. On a metabolism tick
    /// the per-tick cost is charged ×N (a lump for the N ticks it stands in for), so the energy economy
    /// stays ≈invariant to N and conservation is exact. A meta-constant with a GLOBAL phase, not dynamic.
    pub metab_period: u64,
    /// Conserved excretion per tick (agent→field, exact integer transfer — exercises the conserved
    /// multithreaded scatter / R14). Detritus returned to the resource pool.
    pub excrete: i64,
    /// Signal (pheromone) deposited per agent per tick (f32, NOT in the energy balance).
    pub pheromone: f32,
    /// Field cell size `M_field` (world cells per field bucket, ≥ 1). This is the INDEPENDENT
    /// expected value for the `check_meta(R8)` load-check in `Sim::new` — passing `field.m_field()`
    /// would compare the field to itself (a tautology); this provides the external reference (M1/F1).
    pub m_field: i64,
    /// Genetic distance threshold for speciation (M5/criterion 2): a child whose L1 brain-weight
    /// distance from its parent species' founder genome exceeds this value founds a new species.
    /// Integer. Calibrated via probe (issue #130): max_L1≈180–242 at tick 8000, T=80 gives
    /// ≈7.5 divisions per speciation at the observed mutation cadence (avg ≈10.7 L1/division).
    pub speciation_threshold: i64,
    /// Active conserved-layer count — mirrors `SimConfig::n_layers` so it is reachable in ECS
    /// stages (e.g. `stage_birth_death` needs it to clamp layer-trait mutations). Kept in sync by
    /// `build_sim` (`config.econ.n_layers = config.n_layers`). Default 2 (L=2 production).
    pub n_layers: usize,

    // ── C-slice: background death + abiotic recycling (economy/01 §3) ────────────────────────────
    /// Background death hazard (C-1). Integer probability over `D0_MASK` (see constant above).
    /// `d0_scaled = round(d0 × (D0_MASK+1))`. Default: `round(0.001 × 1_048_576) = 1049`.
    /// Mean lifetime ≈ 1_048_576 / 1049 ≈ 999.6 ticks ≈ 1000 ticks (economy/01 §3).
    /// Set to 0 to disable background death. Re-pins the arm64 golden when changed.
    pub d0_scaled: u64,
    /// Recycle fraction numerator (C-2). `recycle = recycle_num / RECYCLE_DEN`.
    /// Default `recycle_num = 77` → `recycle ≈ 77/256 ≈ 0.301` (economy/01 §3: recycle = 0.3).
    /// On every death: `recycled = recycle_num · E / RECYCLE_DEN` (truncating) → substrate layer 0;
    /// `E − recycled` → `ledger.lost`. Truncation remainder lands in `lost`, never created.
    pub recycle_num: i64,

    // ── C′-slice: biotic detritus redirect (C′-1) ────────────────────────────────────────────────
    /// Detritus conserved layer index (C′-1). When `Some(l)`, the C-2 death-recycle deposit is
    /// REDIRECTED to layer `l` (weighted by `detritus_frac_num`); when `None` (default), the deposit
    /// keeps the exact Slice-C behavior (`deposit_conserved(cell, recycled, 0)`) — byte-identical,
    /// so `default_config` and `l3_config` trajectories/goldens are unchanged.
    pub detritus_layer: Option<usize>,
    /// Detritus fraction numerator (C′-1). `detritus_frac = detritus_frac_num / RECYCLE_DEN`.
    /// Active only when `detritus_layer` is `Some`. Bootstrap = `RECYCLE_DEN` (1.0, full-replace):
    /// ALL recycled body energy → detritus layer on death; none abiotic. C′-3 calibrates down for
    /// a hybrid if the biotic loop needs a partial abiotic shortcut to close before reducers evolve.
    pub detritus_frac_num: i64,

    // ── D′-slice: light economy (D′-1) ───────────────────────────────────────────────────────────
    /// Light field specification (D′-1). `Some(spec)` enables the light economy: `photo_gain` gene
    /// mutation active, per-cell `U_photo(L(t))` credited each tick as an external source.
    /// `None` (default) → light economy fully inert, `photo_gain` stays 0 in all genomes, and the
    /// photo code path is never entered → `default_config`/`l3_config`/`cprime_config` trajectories
    /// remain byte-identical (the isolation gate; un-re-pinned existing goldens ARE the test).
    pub light: Option<LightSpec>,
}

// ── D′-1 light field ─────────────────────────────────────────────────────────────────────────────

/// Light field specification for `EconParams.light` (D′-1).
/// Light is a NON-conserved external flux — top-injected, per-cell, non-rival. It does NOT enter
/// the conserved-layer ledger as a stock and does NOT bump `n_layers`. Instead it is credited to
/// each cell's energy as `U_photo(L(t)) = photo_gain · L(t) / (km_photo + L(t))` and booked via
/// `ledger.produced` (same bucket as field regen) so R15 closes to residual 0.
#[derive(Clone, Copy, Debug)]
pub struct LightSpec {
    /// Peak light intensity (eu, integer). During day phase: L = l_max. At night: L = 0.
    /// Calibrated at 100 eu (plan §0: `L_max=100`, same scale as substrate km=74).
    pub l_max: i64,
    /// Full day-night period in ticks. Must be > 0. E.g. 100 → 100-tick day-night cycle.
    pub period_ticks: u64,
    /// Day-phase duration per period (ticks where `tick % period_ticks < day_ticks → L = l_max`).
    /// `duty_cycle = day_ticks / period_ticks`. Requires `0 < day_ticks < period_ticks`.
    pub day_ticks: u64,
    /// Photo Monod half-saturation constant (eu). Must be > 0. Km_photo < Km_chem (plan §0:
    /// faster light saturation than substrate — calibrated at 30 vs km_chem=74).
    pub km_photo: i64,
}

/// L(t): deterministic day-night light intensity, pure function of tick + `LightSpec`.
///
/// Day phase (`tick % period_ticks < day_ticks`) → `l_max`. Night → `0`.
/// Pure, integer-only, no RNG — the photo path never introduces randomness.
/// If `period_ticks == 0` (degenerate), returns `l_max` for every tick.
pub fn light_at_tick(spec: &LightSpec, tick: u64) -> i64 {
    if spec.period_ticks == 0 || tick % spec.period_ticks < spec.day_ticks {
        spec.l_max
    } else {
        0
    }
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
            km: 74,   // calibrated: km=50→R̄=32.2→km₁=74→R̄=32.2→fixed (B-1)
            u_max: 220, // Monod asymptote — realized U(R̄) < u_max; km tunes the shape (B-1)
            world_dim: 64,
            m_sim: 4,
            brain_period: 4, // K — behaviour at 16 Hz (64/4)
            metab_period: 2, // N — metabolism at 32 Hz, charged ×2 per tick (economy ≈invariant)
            excrete: 8,
            pheromone: 1.0,
            m_field: 1, // one field cell per world cell (the CLI default / doc 14 §1)
            speciation_threshold: 80,
            n_layers: 2,
            d0_scaled: 1049, // round(0.001 × 1_048_576); mean lifetime ≈ 1000 ticks (economy/01)
            recycle_num: 77,  // round(0.3 × 256) = 76.8 → 77; recycle ≈ 30.1% (economy/01 §3)
            detritus_layer: None,    // C′-1: None → byte-identical Slice-C behavior (→ layer 0)
            detritus_frac_num: RECYCLE_DEN, // = 256; dormant (only active when detritus_layer is Some)
            light: None,             // D′-1: None → light economy inert, photo_gain stays 0
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
    /// Number of sim threads for the scatter pool (F5 — explicit, NOT `num_cpus`/bevy default, so the
    /// R14 1-vs-N test can run both inside one process).
    pub sim_threads: usize,
    /// Scatter merge strategy (`Canonical` in production; `NonAssociative` only for the R14 negative).
    pub merge_strategy: MergeStrategy,
    /// Number of conserved layers. Default 2 (substrate + organics); bench uses 1; L=3 test uses 3.
    pub n_layers: usize,
    /// Per-layer field parameters. Only the first `n_layers` entries are used by `build_sim`.
    /// Unused slots may be zeroed (`LayerSpec::default()`).
    pub layer_specs: [LayerSpec; 4],
}
