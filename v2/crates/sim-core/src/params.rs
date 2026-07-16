//! Run parameters. `EconParams` are the on-the-shore economy numbers (economy/01); they are a
//! documented cargo-tunable contract (re-pinning the golden after a change is cheap). All integer.

use crate::{Genome, GrnSpec, MergeStrategy, MorphogenSpec, PredationSpec};
use bevy_ecs::prelude::Resource;

// ── Conserved field layer identifiers (P1-0 ШВ-1) ────────────────────────────────────────────────

/// Conserved field layer IDs — type-safe layer semantics for redox-tower (ШВ-1). Used as a bridge
/// between semantic enum and usize array indices at the FieldStore boundary (P1-0: `layer: usize` →
/// `FieldId` enums for O₂/NO₃/SO₄). Internal implementation stays i64-indexed for speed.
/// - Layer 0: **Substrate** — primary energy source (ProcgenWorld-derived per-cell caps, all configs).
/// - Layer 1: **Organics/Excreta** — metabolic waste (energy-layer, all configs with L≥2).
/// - Layer 2: **Oxygen** — first conserved redox-acceptor (P1-0+, ШВ-1). Biom-derived O₂-caps.
///   Excluded from n_energy_layers to prevent contamination by agent excretion (non-energy layer).
/// - Layer 3: **Nitrate** — secondary acceptor (P5+, reserved).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FieldId {
    /// Layer 0: substrate (primary energy source, all configs).
    Substrate = 0,
    /// Layer 1: organics/excreta (metabolic byproducts, energy-layer when present).
    Organics = 1,
    /// Layer 2: O₂ (first conserved redox-acceptor, P1-0+, non-energy layer excluded from mutation).
    Oxygen = 2,
    /// Layer 3: NO₃⁻ (secondary acceptor, P5+, reserved).
    Nitrate = 3,
}

impl FieldId {
    /// Convert to layer array index for FieldStore internal buffers.
    #[inline]
    pub const fn as_usize(self) -> usize {
        self as u8 as usize
    }
}

// ── ENV-0a'-a1: spatial monopolization configuration (frontier-D5) ─────────────────────────────

/// ENV-0a'-a1: environment frontier spatial monopolization config. Composes the D-5 hazard refuge
/// with a patch-grain resource field to enable spatial bonded-monopolization mechanic (frequency-
/// dependent bistability). When `Some`, enables priority-ration in `stage_interactions` phase 3:
/// bonded multicellular bodies pre-empt resource uptake in contested cells before unbonded unicells.
/// When `None` (default, all shipped configs), uptake is proportional (byte-identical trajectories).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnvFrontierConfig {
    /// Spatial correlation length (cells) for patch-grain resource heterogeneity.
    /// Default ~4 (small patch), tuned per-scenario in a2 sweep; fixed for a1.
    pub patch_grain: i64,
}

// ── R30-1.1 (#408): income-harvest mode selector ────────────────────────────────────────────────

/// Selects how the footprint income harvest (`stage_interactions`) generates contestants for a
/// body. Replaces the former boolean `body_footprint` (critic F7): the three lanes are MUTUALLY
/// EXCLUSIVE by construction (both-on was unrepresentable with a bool). Pure selector, no RNG.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IncomeMode {
    /// Single contestant at the entity's own anchor position (`Position`). The pre-EXT-0a path.
    Anchor,
    /// `side²` contestants over a FILLED square footprint (`side = g_dev.max(1)`), one per grid
    /// cell regardless of live/dead — the EXT-0a path. Debug-asserts `body_size == side²`.
    Footprint,
    /// R30-1.1: one contestant per LIVE cell in `CellGraph.cell_positions` — the actual live
    /// shape, not a filled square. A dead cell contributes no contestant and thus no income.
    Extent,
}

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
///
/// **`Clone`, NOT `Copy`** (E-4a): `GrnSpec` carries `Vec<i32>` regulatory-matrix fields, so
/// `Option<GrnSpec>` cannot be `Copy`. Every prior implicit-copy call site (`let econ = config.econ;`
/// etc.) was audited and converted to an explicit `.clone()` — the identical value, just no longer
/// silently duplicated.
#[derive(Resource, Clone, Debug)]
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
    /// World resource base: rescale cap from [0,CAP_MAX=300] into [1, resource_base+1] magnitude.
    /// Carried-capacity knob for per-config balance (W-6b: bloom-prone @ 91, starve-prone @ 120).
    /// Default 120 (NoiseWorld calibration). Set per-config to avoid population overshoot/collapse.
    pub resource_base: i64,
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
    /// Number of layers available to genome layer-targeting traits (`uptake_layer`/`excrete_layer`).
    /// Normally equal to `n_layers`. Set LOWER than `n_layers` when a non-energy special layer
    /// (e.g. the D′-3a mineral layer) is present and must NOT be reachable as an energy food source.
    /// dprime_config: `n_energy_layers=2` (agents eat layers 0–1 only; mineral on layer 2 is
    /// exclusively accessed by `stage_mineral_feed`). Default=`n_layers` for backward compat.
    pub n_energy_layers: usize,

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

    // ── D′-2b: photo-GRN regulation gene ─────────────────────────────────────────────────────────
    /// Maximum absolute value of the evolvable `reg_gain` field (D′-2b).
    /// Reg-gene mutations clamp to `[−reg_gain_max, +reg_gain_max]`.
    ///   `reg_gain_max = 0`: regulation locked OFF (the D′-2c constitutive control line) —
    ///     reg_gain stays at the founder value (0) forever, dprime behaves as D′-2a.
    ///   `reg_gain_max > 0`: regulation can evolve; non-zero gain enables day/night gating.
    /// Default 4 (regulation enabled, light-gating can evolve). Non-dprime configs are
    /// unaffected: reg_gain only mutates when `has_light=true` (same gate as photo_gain).
    pub reg_gain_max: i32,

    // ── D′-3a: mineral nutrient economy ──────────────────────────────────────────────────────────
    /// Mineral conserved-layer index (D′-3a). `Some(l)` enables the mineral economy: contested
    /// Monod uptake from layer `l` into per-entity `MineralQuota`, Liebig AND-gate on division,
    /// overflow-heat when energy-ready but mineral-poor. `None` → mineral inert, byte-identical.
    ///
    /// Option-gated like `detritus_layer` and `light` so default/l3/cprime stay byte-identical.
    /// In `dprime_config`: `Some(2)` (layer 0 = substrate, layer 1 = organics, layer 2 = mineral).
    pub mineral_layer: Option<usize>,
    /// Monod half-saturation constant for mineral uptake (D′-3a, eu-mineral).
    /// Calibration mapping: Km_mineral=20 model units → `km_mineral=200` (×10 scale).
    /// Must be > 0. At `km_mineral=200`, mineral concentration 22 eu ≈ 11% Km → oligotrophic.
    pub km_mineral: i64,
    /// Monod U_max for mineral uptake (D′-3a, eu-mineral per tick per entity).
    /// Calibration mapping: U_max_mineral=2.5 model units × (world_dim²/N_calibration) scale.
    /// With world_dim=64 (4096 cells), regen_rate=1 and N*≈583 at field depletion:
    ///   N × U_max × M*/(M*+Km) = regen × n_cells → U_max ≈ 70 gives M*≈22 eu at N*=583.
    pub u_max_mineral: i64,
    /// Mineral quota required per division event (D′-3a, eu-mineral). Parent spends this from
    /// its quota when dividing; child inherits 0. Liebig gate: `quota ≥ q_mineral` required.
    /// Calibration mapping: q_mineral=0.10 model units × 10 = 1 → T_accumulate ≈ 1–2 ticks.
    /// Set to `q_mineral=4000` so T_mineral ≈ T_energy at equilibrium N* (Liebig binds).
    pub q_mineral: i64,
    /// Mineral recycle fraction numerator (D′-3a). `recycle_mineral = recycle_mineral_num / 256`.
    /// On death: `recycled = recycle_mineral_num × quota / 256` → mineral field; remainder → lost.
    /// Calibration: recycle_mineral=0.4 → `round(0.4 × 256) = 102`.
    pub recycle_mineral_num: i64,
    /// Energy burned per tick as overflow-heat when a cell is energy-ready but mineral-poor
    /// (D′-3a). Trigger: `energy ≥ e_cell+c_div && quota < q_mineral`. Deducted from agent energy
    /// → `ledger.lost`. Calibrated to neutralise the photo-subsidy at mineral-limited N*, limiting
    /// the standing crop below the energy-only ceiling (the Liebig cap signature).
    pub overflow_delta: i64,

    // ── D′-2a: photo-machinery expression cost ────────────────────────────────────────────────────
    /// Photo-machinery expression cost numerator (D′-2a). Per-tick rate:
    /// `r = (photo_cost_num · photo_gain) / photo_cost_den` eu/tick.
    ///
    /// Charged EVERY tick (day AND night) whenever `photo_gain > 0` — the constitutive cell pays
    /// around the clock. That asymmetry (pays at night with zero income) is the lever D′-2b exploits.
    ///
    /// To avoid premature integer truncation at small `photo_gain`, the per-event charge is computed
    /// as `(photo_cost_num · photo_gain · n) / photo_cost_den` (n = `metab_period`), which delays
    /// the division. This scales linearly with n → R20 N-invariance holds.
    ///
    /// Calibration: `photo_cost_num=1`, `photo_cost_den=8` targets ≈17% of day photo income at
    /// the effective threshold (`photo_gain=4`, n=2): charge = (1·4·2)/8 = 1 eu/event,
    /// day income = 4·100/130·2 = 6 eu/event → 16.7%.
    ///
    /// This is within the model band [0%, 27%] from `phase1_photocost_model.py` (band ∈ [0, 0.75]
    /// eu/tick against model day income 2.77 eu/tick = 27% max). The suggested den∈[15,22] from
    /// the issue (§acceptance) assumed cells evolve to gain≥8, but empirically post-sweep
    /// (tick 5000) photo_gain concentrates at 2-7 under weak selection. DEN=8 (threshold=4) is
    /// calibrated to engage as soon as the photo sweep occurs (gain≥4 reachable within ~1000 ticks
    /// post-sweep), while the 17% fraction is close to the issue's 15% upper guide value.
    ///
    /// Fraction at threshold scales as `130 / (gain_threshold × 100 × 2)`:
    ///   threshold=4 → 130/800 = 16.3%  ← DEN=8, n=2 (chosen)
    ///   threshold=7 → 130/1400 = 9.3%  ← DEN=14 (issue suggestion; inert in 8000-tick window)
    ///
    /// Inert for non-dprime configs: `photo_gain ≡ 0` (mutation gate in `genome.rs` ensures this
    /// when `light: None`) → cost is 0 for all non-dprime trajectories → byte-identical goldens.
    pub photo_cost_num: i64,
    /// Photo-machinery expression cost denominator (D′-2a). Must be > 0. See `photo_cost_num`.
    pub photo_cost_den: i64,

    // ── P-2a: predation economy (heritable combat_trait + encounter resolution) ────────────────
    /// Predation configuration (P-2a). `Some` enables predation encounters: heritable `combat_trait`
    /// mutation active, deterministic mean-field predation in `stage_predation`. `None` (default) →
    /// predation inert, `combat_trait` stays 0 in all genomes, stage is a no-op → default_config/l3/
    /// cprime/dprime trajectories remain byte-identical (the isolation gate; un-re-pinned existing
    /// goldens are the test). Option-gated exactly like `light`/`mineral_layer` above.
    pub predation: Option<PredationSpec>,

    // ── P1-0: O₂ conserved-field infrastructure (no respiration yet) ────────────────────────────
    /// O₂-field infrastructure (P1-0, ШВ-1). `true` enables the conserved O₂ resource layer
    /// (respiration sink P1-1, photosynthesis source P2+), biom-derived O₂-caps from world-gen.
    /// `false` (default, all 5 existing production configs + l3_config) → O₂ layer inert,
    /// trajectories byte-identical (the isolation gate; un-re-pinned existing goldens are the test).
    /// When `true`, `n_layers` and `layer_specs` must include O₂-layer entry (via `oxygen_config`).
    /// Option-gated exactly like `light`/`mineral_layer` above.
    pub enable_oxygen: bool,

    // ── P5-0: NO₃ conserved-field infrastructure (inert substrate) ────────────────────────────────
    /// NO₃-field infrastructure (P5-0, ШВ-1). `true` enables the conserved NO₃ resource layer
    /// (static, inert in P5-0; consumption P5-2+), biom-derived NO₃-caps (INVERSE of O₂) from world-gen.
    /// `false` (default, all 6 existing production configs) → NO₃ layer inert, trajectories byte-identical
    /// (the isolation gate; un-re-pinned existing goldens are the test). When `true`, `n_layers` and
    /// `layer_specs` must include NO₃-layer entry (via `nitrate_config`). Option-gated exactly like
    /// `enable_oxygen` above.
    pub enable_nitrate: bool,

    // ── P5-D: spatial (depth-attenuated) light economy ────────────────────────────────────────────
    /// P5-D: when true, photosynthesis light is depth-attenuated by cell height (`light_at`);
    /// default false → uniform light, byte-identical to P2. Isolation gate like `enable_oxygen`.
    pub enable_photic: bool,

    // ── P1-2b: O₂-diffusion cost scaling (hypoxia self-shading) ───────────────────────────────────
    /// O₂-field cap value for hypoxia calculation (P1-2b). The scarcity factor normalizes
    /// ambient O₂ against this cap: `scarcity = 1000 − (field_o2 × 1000 / cap_o2)`.
    /// Derived from `layer_specs[FieldId::Oxygen.as_usize()].flat_cap` at `build_sim` time.
    /// Must be > 0 when `enable_oxygen=true` (guarded by bounds-check in compute_hypoxia_factor).
    /// Non-oxygen configs: o2_cap=0 (hypoxia returns 0 immediately in the bounds-guard).
    pub o2_cap: i64,

    /// P1-2b faithful-verdict ABLATION knob: when `true`, hypoxia is forced to 0 (the diffusion cost
    /// is switched off) while EVERYTHING else — aerobic yield, respiratory strategy, O₂ field —
    /// stays identical. This is the control arm for the a-d verdict (crit. c): the size-selection
    /// differential must be PRESENT with hypoxia and VANISH under ablation to be faithful (else NULL).
    /// Default `false` → hypoxia active → byte-identical to the shipped P1-2b behaviour (golden-neutral).
    pub ablate_hypoxia: bool,

    /// P1-2b hypoxia calibration knob (dive #70 §4.1 `hypoxia_base_x1000`): scales the raw physical
    /// hypoxia factor so the penalty can be anchored to the Ratcliffe −10% size-cost regime instead of
    /// the artefactually harsh raw value. Applied at the stage_interactions call site:
    /// `hypoxia = compute_hypoxia_factor(...) × hypoxia_base_x1000 / 1000`. Default `1000` → ×1.0 →
    /// byte-identical to raw physics (golden-neutral). The faithful verdict config lowers it (~543) so
    /// N=4 clusters pay ≈−10% (calibrated empirically via the hypoxia-verdict run, NOT a taste knob:
    /// the target is the external Ratcliffe measurement, so lowering it grounds — it does not chase
    /// emergence). The raw-physics unit tests on `compute_hypoxia_factor_x1000` are unaffected.
    pub hypoxia_base_x1000: i64,

    // ── E-4a: ontogenesis chain opt-in (morphogen → GRN → cell fate) ────────────────────────────
    /// Morphogen reaction-diffusion spec (E-2). `Some` together with `grn` enables the full
    /// `decode` ontogenesis chain; `None` (default, all 5 existing configs) → `decode` stays the
    /// E-1 trivial Ф0 projection, byte-identical to every existing golden. Option-gated exactly
    /// like `light`/`mineral_layer` above. NO production config sets this in E-4a (E-4b does).
    pub morphogen: Option<MorphogenSpec>,
    /// GRN dynamics spec (E-3). See `morphogen` — both must be `Some` for the chain to run.
    pub grn: Option<GrnSpec>,

    // ── V-3-b: variable-length genome operators (duplication, indel, translocation) ──────────────
    /// Enable the variable-length genome operators (V-3-b duplication, V-3-c indel, V-3-d translocation).
    /// `false` (default, all 6 existing production configs) → the operators are inert, draw zero from
    /// the stream, and n_genes stays constant → trajectories are byte-identical, existing goldens
    /// un-re-pinned. `true` only on test/research configs with dedicated genome fixtures designed
    /// to exercise the operators. Determinism: when false, mutate() reads zero values from the
    /// operator stream → backward-compatible draw positions preserved (§5.2).
    pub enable_variable_length: bool,

    // ── M7-e: multicellular coordination-cost sink ────────────────────────────────────────────────
    /// Coordination cost per live body cell per tick (M7-e-a). Charged as `c_coord · N` inside the
    /// metabolism bracket (`N = Σ module_cell_count`, the total live body cell count from
    /// `Phenotype.graph`), same per-tick lump as `base_metab`/`k_size_metab`/etc. Default `0`
    /// (all 6 existing production configs) → the term adds nothing → byte-identical goldens; the
    /// `Phenotype.graph` read is live (wired) but inert. `c_coord > 0` (calibration + viability
    /// verification + re-pin) is M7-e-b, not this slice.
    pub c_coord: i64,

    // ── V-4: evolvable developmental grid (body-size axis) ──────────────────────────────────────
    /// Enable heritable mutation of `morphogen_spec.g_dev` (V-4, #276) — the unicellular↔
    /// multicellular body-size axis. `false` (default, all existing production configs) → `mutate()`
    /// draws zero values from the `SALT_GDEV` stream → g_dev never changes → byte-identical goldens.
    /// `true` only on `driver_config` (the emergence testbed), gated additionally on
    /// `morphogen_spec.is_some()` in `Genome::mutate`.
    pub evolve_body_size: bool,

    // ── EXT-0b: morphogen cap and step-count as config parameters ────────────────────────────
    /// Maximum g_dev (developmental grid side length) — the morphogen cap.
    /// Default 4 (ships with g_dev ≤ 4, byte-identical to main). Swept to 5, 6 in diagnostic scenarios
    /// (EXT-0b) to test whether body size saturates or settles below the cap.
    /// Used at the g_dev mutate clamp site (genome.rs:888) — `clamp(1, econ.gdev_cap)`.
    pub gdev_cap: usize,

    /// Maximum reaction-diffusion steps (morphogen decode compute budget).
    /// Default 8 (ships with n_dev = 8, byte-identical to main). Paired with g_dev in diagnostic
    /// scenarios: (4→8, 5→10, 6→12) to satisfy the constraint `n_dev ≥ 2·g_dev − 2`.
    /// Used in founder specs (cli/src/lib.rs) — replaces hardcoded `n_dev: 8`.
    pub morphogen_steps: u32,

    // ── P3-1: ambient-tolerance thermal niche (heritable tol_optimum, tol_breadth) ───────────
    /// Ambient-tolerance specification (P3-1). `Some` enables thermal-tolerance genes
    /// (`tol_optimum`, `tol_breadth`) with mutation active and metabolic penalty applied.
    /// `None` (default, all 6 existing production configs) → tolerance genes stay 0 forever,
    /// mutation gate prevents draw, hash gate prevents state inclusion → byte-identical goldens
    /// (the isolation gate; un-re-pinned existing goldens are the test). Option-gated exactly
    /// like `light`, `predation`, `morphogen`, `mineral_layer` above.
    pub ambient_tolerance: Option<AmbientToleranceSpec>,

    // ── P4/SL-1: settling-selection (size²-attenuated mortality pulse) ──────────────────────
    /// Settling-selection specification (P4/SL-1). `Some` enables settling-mechanic: every
    /// `period` ticks, a size²-attenuated mortality pulse drains energy (dissipated via
    /// `ledger.dissipated`, death at ≤0 in stage 7). `None` (default, all 6 existing
    /// production configs) → `stage_settling` is a no-op → byte-identical goldens (the isolation
    /// gate; un-re-pinned existing goldens are the test). Option-gated exactly like `light`,
    /// `predation`, `morphogen`, `mineral_layer` above. SL-1 settling-mechanic ONLY; diffusion
    /// cost (criterion c) = SL-2; verdict (a-d gates) = SL-3.
    pub settling: Option<SettlingSpec>,

    // ── DL-M: division-of-labor mechanic (germ/soma specialization) ─────────────────────────
    /// Division-of-labor economy (DL-M). `true` enables germ/soma specialization:
    /// predation refuge scales with soma_mass (not total body), reproduction rate scales with
    /// germ investment (all-soma is sterile). `false` (default, all 6 existing production configs)
    /// → mechanics inert, byte-identical goldens (the isolation gate; un-re-pinned existing
    /// goldens are the test). Option-gated exactly like `enable_oxygen`/`enable_photic` above.
    pub division_of_labor: bool,
    /// Germ-throughput reproduction gate (DL-V verdict arm isolation). When `division_of_labor=true`
    /// and `dol_germ_repro=true`, reproduction rate scales with germ investment (germ=0 → sterile).
    /// When `division_of_labor=true` and `dol_germ_repro=false`, soma-refuge is active but repro is
    /// scalar (decoupled from germ:soma ratio). `false` (default) keeps all shipped configs byte-identical.
    pub dol_germ_repro: bool,
    /// Economy-coupled division-of-labor redesign (DR-0+). When `true`, enables the redesigned
    /// economy-coupled mechanic: soma cells scale resource uptake (demand ∝ soma), germ provides
    /// a flat fertility gate (germ=0 → sterile; germ≥1 → flat threshold). `false` (default, all
    /// existing production configs) → mechanics inert, byte-identical goldens (the isolation gate).
    pub dol_economy: bool,
    // ── GA-LOAD: deleterious-mutation load substrate ────────────────────────────────────
    /// Enable the deleterious-mutation-load mechanic (GA-LOAD, genetic-architecture enrichment).
    /// `false` (default, all 6 existing production configs) → load mechanic inert, genetic_load
    /// stays 0 forever, mutation gate prevents draw, hash gate prevents state inclusion →
    /// byte-identical goldens (the isolation gate). `true` only on `ga_load_config`.
    pub enable_mutation_load: bool,
    /// Deleterious mutation probability numerator (per mutation EVENT). `del_num/del_den`
    /// is the fraction of mutations that are deleterious. Anchored to give f_del≈1/16 at
    /// founder mutation rate (so U ≈ 1 for L-shaped DFE; biological range). Default 0 when
    /// enable_mutation_load=false (inert).
    pub mut_load_del_num: i32,
    /// Deleterious mutation probability denominator. Default 0 when enable_mutation_load=false.
    pub mut_load_del_den: i32,
    /// Beneficial (compensatory) mutation probability numerator. Smaller than del_num.
    /// Default 0 when enable_mutation_load=false (inert).
    pub mut_load_ben_num: i32,
    /// Beneficial mutation probability denominator. Default 0 when enable_mutation_load=false.
    pub mut_load_ben_den: i32,
    /// Energy cost per load unit per tick (burden_cost = genetic_load * burden_cost_k).
    /// Anchored to ≈2–3% of ~80 eu income → s≈0.025. Default 0 when enable_mutation_load=false (inert).
    pub burden_cost_k: i64,

    // ── ENV-0a'-a1: spatial monopolization (frontier-D5) ────────────────────────────────────
    /// Environment frontier spatial monopolization config (ENV-0a'-a1). `Some` enables the
    /// priority-ration mechanism in `stage_interactions` phase 3: bonded multicellular bodies
    /// pre-empt resource before unbonded unicells, enabling frequency-dependent bistability.
    /// `None` (default, all existing production configs) → uptake remains proportional (all
    /// contested-cell grants scale equally) → byte-identical goldens (the isolation gate).
    pub env_frontier_config: Option<EnvFrontierConfig>,

    // ── EXT-0a/R30-1.1: income-harvest mode (spatial extent income) ─────────────────────────
    /// Income-harvest mode (EXT-0a + R30-1.1, #408). Replaces the former boolean `body_footprint`
    /// with a mutually-exclusive [`IncomeMode`] selector. `Anchor` (default, all existing
    /// production configs) → single contestant at the entity's own position, income flat in N
    /// regardless of body size → byte-identical goldens (the isolation gate). `Footprint` → the
    /// EXT-0a filled-square `side²` path (unchanged). `Extent` → R30-1.1's live-shape path over
    /// `CellGraph.cell_positions`.
    /// Determinism: each lane is a pure function of (Position, phenotype, world_dim) — no RNG/HashMap.
    pub income_mode: IncomeMode,

    // ── Rung 1 (topo-diff §3 SLOT 1, #401): heritable body-plan mutation gate ───────────────────
    /// Enable heritable mutation of `morphogen_spec.body_plan` (Square↔Ring↔Filament). `false`
    /// (default, all existing production configs) → `mutate()` draws zero values from the
    /// `SALT_BODY_PLAN` stream → `body_plan` never changes → stays `Square` forever → byte-identical
    /// goldens (mirrors `evolve_body_size`/`SALT_GDEV`). Gated additionally on
    /// `morphogen_spec.is_some()` in `Genome::mutate`.
    pub enable_body_plan: bool,

    // ── R30-1.1a (#412): Kleiber metabolic cost welded to live cell count ────────────────────────
    /// When `true`, the Kleiber metabolic term (`stage_metabolism`) reads the body's LIVE cell
    /// count (`Σ ph.graph.module_cell_count`) instead of the `Genome::size` gene. `false` (default,
    /// all existing production configs) → term stays `size_pow_three_quarters(g.size)`, byte-
    /// identical goldens (the isolation gate). `Genome::size` stays wired to viability, mutation,
    /// and `state_hash` regardless of this flag — only the metabolic consumer moves.
    pub metab_reads_n_cells: bool,

    // ── R30-1.1b (#414): newborn endowment welded to live body size, transferred from parent ────
    /// When `true`, a newborn's starting energy is `e_cell · body_size(child)` (a 1-cell child
    /// still gets the flat `e_cell` baseline; an N-cell child gets N times it) instead of the flat
    /// `e_cell`, and is TRANSFERRED from the parent (parent debits `endowment + c_div`) under a
    /// STRUCTURAL affordability gate (`energy ≥ endowment + c_div`, alongside `repro_bar`) — no
    /// clamp, no forced death: a parent that cannot yet afford the N-scaled endowment simply does
    /// not divide this tick and keeps accumulating. `false` (default, all existing production
    /// configs) → the flat `e_cell` endowment, byte-identical goldens (the isolation gate).
    pub newborn_energy_per_cell: bool,

    // ── P-1: propagule growth primitive (#429) ────────────────────────────────────────────────
    /// Enable the propagule-growth subsystem (P-1, #429). Gates the WHOLE subsystem: the
    /// `n_propagule` mutation, the birth-seam truncation, the grow step (`stage_grow`), AND the
    /// `Grown` hash-fold. `false` (default, all existing production configs) → nothing changes,
    /// byte-identical goldens (the isolation gate). `true` requires the `Sim::new` hard config
    /// assert to pass (`income_mode != Footprint`, no light, no O₂, no morphogen
    /// supply_source/adhesion_threshold) — see `Sim::new`.
    pub enable_propagule: bool,

    // ── P-2b: provisioning (#448) ──────────────────────────────────────────────────────────────
    /// Enable the provisioning subsystem (P-2b, #448): the `Parent`/`Provisioned` components, the
    /// `5a_provision` stage, the `provision_rate` mutation, and the `Provisioned` hash-fold. `false`
    /// (default) → nothing changes, byte-identical goldens (the isolation gate). `Sim::new` asserts
    /// `enable_provision ⇒ enable_propagule` (provisioning a body that cannot grow is meaningless).
    pub enable_provision: bool,
    /// Config/founder seed for `Genome::provision_rate` (`--set`-able, default `0` ⇒ golden-neutral
    /// — the founder is unmodified from `Genome::founder`'s `0`). Critic F163/F165: from the `0`
    /// sentinel the locus sits on a phenotypically-invisible neutral plateau (all-or-nothing grants
    /// need `rate ≳ 60–250` at default econ) the ±1 kernel drifts across ~10 steps/run — a dose
    /// ladder needs a SEEDED rate, not one reached by drift, to test a rung at all.
    pub provision_rate_init: i32,
    /// When `true`, `Genome::mutate` SKIPS the `provision_rate` locus (seed-and-lock, critic
    /// F163/F165) — else the seeded rate mutates ±1 under direct fecundity selection (draining the
    /// parent's liquid before stage 7) and the dose-ladder rungs decay toward the `0` plateau,
    /// conflating "dose ineffective" with "dose eroded". Default `false` (golden-neutral).
    pub provision_rate_locked: bool,
    /// Config/founder seed for `Genome::n_propagule` (`--set`-able, default `0` ⇒ golden-neutral).
    /// Critic F172: the founder is born `n_propagule=0` (full/instant sentinel) ⇒ every body is
    /// born at full target ⇒ zero still-growing children ⇒ `5a_provision` has zero eligible
    /// targets ⇒ the dose ladder is structurally incapable of firing at seed. A nonzero seed makes
    /// the P-3 arms born as a real bootstrap propagule (must GROW to target).
    pub n_propagule_init: i32,
    /// When `true`, `Genome::mutate` SKIPS the `n_propagule` locus (seed-and-lock, same F165
    /// argument as `provision_rate_locked`) — else the seeded propagule size relaxes toward the `0`
    /// sentinel mid-run and the growth substrate the dose ladder measures evaporates. Default
    /// `false` (golden-neutral).
    pub n_propagule_locked: bool,
}

/// P-2b (#448, critic F6): the named, empirically-validated `provision_rate` dose rungs — an
/// unnamed "rung P-3 intends to use" would be unfalsifiable (the coder could rubber-stamp
/// `rate=256`). Every element here is verified (test (B), `propagule_growth.rs`) to grant a
/// NONZERO `provision_granted_total` under `driver_config`-like econ magnitudes — a rung that
/// grants structural zero (`rate/256·grant_pool < typical need`) is dropped from this list, not
/// left in to be misread as a "dose ineffective" NULL in P-3.
pub const PROVISION_RATE_LADDER: &[i64] = &[64, 128, 192, 256];

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

// ── P3-1: ambient-tolerance thermal niche ─────────────────────────────────────────────────────

/// Ambient-tolerance thermal niche specification for `EconParams.ambient_tolerance` (P3-1+).
/// When `Some`, enables heritable tolerance genes (`tol_optimum`, `tol_breadth`) with mutation active.
/// When `None` (default) → tolerance genes inert, `tol_optimum`/`tol_breadth` stay 0 forever,
/// existing goldens stay byte-identical (the isolation gate). Option-gated exactly like `light`
/// and `predation` above.
#[derive(Clone, Copy, Debug)]
pub struct AmbientToleranceSpec {
    /// P3-2: Breadth-cost coefficient (linear scaling). Specialist/generalist tradeoff:
    /// wider `tol_breadth` incurs `breadth_cost_k * tol_breadth / BREADTH_COST_SCALE` metabolic cost.
    /// Integer, zero-float. Gated on `is_some()` (byte-identical when None).
    /// CALIBRATION-PROVISIONAL: final value from P3-3 neutral-run measurement.
    pub breadth_cost_k: i64,
}

// ── P4/SL-1: settling-selection (size²-attenuated mortality pulse) ────────────────────────────

/// Settling-selection specification for `EconParams.settling` (P4/SL-1).
/// When `Some`, enables settling-mechanic: every `period` ticks, size²-attenuated mortality pulse.
/// When `None` (default, all 6 existing production configs) → `stage_settling` is a no-op,
/// byte-identical goldens (the isolation gate). Option-gated exactly like `light`, `predation`,
/// `morphogen`, `mineral_layer` above. SL-1 settling-mechanic ONLY; diffusion cost (criterion c)
/// = SL-2; verdict (a-d gates) = SL-3.
#[derive(Clone, Copy, Debug)]
pub struct SettlingSpec {
    /// Settling-pulse period (ticks). `period=0` → stage_settling is a no-op (byte-identical).
    /// Every `period` ticks where `SimClock.tick % period == 0`, trigger a settling-pulse.
    /// Must be > 0 when active (gate: if `period==0` treat as None for compatibility).
    pub period: u64,
    /// Settling-pulse strength (base drain before size²-attenuation). Integer, non-negative.
    /// Q-format numerator: `drain = (strength << SHIFT) / ((1 << SHIFT) + settling_k · size²)`.
    pub strength: i64,
    /// Settling-attenuation factor (size² scaling in Q-format denominator).
    /// `settling_k=0` → drain unchanged (inert), larger k → stronger attenuation by size².
    /// Calibrated empirically for settling-velocity emergence (ТЗ: PROVISIONAL).
    pub settling_k: i32,
    /// Q-format shift for size²-attenuation denominator. Standard Q-format (mirrors refuge_attenuate).
    /// Must be > 0 and ≤ 32 (defensively clamped to prevent overflow). Typical: 16.
    pub shift: u32,
}

// ── P3-1: Gaussian thermal-penalty kernel (constant LUT, zero float) ──────────────────────────

/// P3-1 (B4): Gaussian kernel exp(−x²/2) precomputed to integer Q8.8 fixed-point.
/// Indexed [0..=256]; entry at index i corresponds to normalized deviation i/256.
/// Precomputed offline via Python: `exp(-norm_x²/2)` scaled to [0, 256].
/// Zero float arithmetic; deterministic cross-architecture.
/// Used by `tolerance_penalty()` to compute metabolic penalty from thermal deviation.
pub const THERMAL_KERNEL_Q256: [i32; 257] = [
    256, 255, 254, 252, 251, 250, 248, 247, 246, 244,  // x ∈ [0, 9]
    243, 242, 240, 239, 238, 236, 235, 234, 232, 231,  // x ∈ [10, 19]
    230, 228, 227, 226, 224, 223, 222, 220, 219, 218,  // x ∈ [20, 29]
    216, 215, 214, 212, 211, 210, 208, 207, 206, 204,  // x ∈ [30, 39]
    203, 202, 200, 199, 198, 196, 195, 194, 192, 191,  // x ∈ [40, 49]
    190, 188, 187, 186, 184, 183, 182, 180, 179, 178,  // x ∈ [50, 59]
    176, 175, 174, 172, 171, 170, 169, 167, 166, 165,  // x ∈ [60, 69]
    163, 162, 161, 159, 158, 157, 156, 154, 153, 152,  // x ∈ [70, 79]
    150, 149, 148, 147, 145, 144, 143, 141, 140, 139,  // x ∈ [80, 89]
    138, 136, 135, 134, 132, 131, 130, 129, 127, 126,  // x ∈ [90, 99]
    125, 124, 122, 121, 120, 119, 117, 116, 115, 114,  // x ∈ [100, 109]
    112, 111, 110, 109, 107, 106, 105, 104, 103, 101,  // x ∈ [110, 119]
    100, 99, 98, 97, 95, 94, 93, 92, 91, 89,           // x ∈ [120, 129]
    88, 87, 86, 85, 83, 82, 81, 80, 79, 77,            // x ∈ [130, 139]
    76, 75, 74, 73, 72, 70, 69, 68, 67, 66,            // x ∈ [140, 149]
    65, 63, 62, 61, 60, 59, 58, 57, 55, 54,            // x ∈ [150, 159]
    53, 52, 51, 50, 49, 48, 46, 45, 44, 43,            // x ∈ [160, 169]
    42, 41, 40, 39, 37, 36, 35, 34, 33, 32,            // x ∈ [170, 179]
    31, 30, 29, 28, 26, 25, 24, 23, 22, 21,            // x ∈ [180, 189]
    20, 19, 18, 17, 16, 15, 13, 12, 11, 10,            // x ∈ [190, 199]
    9, 8, 7, 6, 5, 5, 4, 3, 2, 1,                      // x ∈ [200, 209]
    1, 0, 0, 0, 0, 0, 0, 0, 0, 0,                      // x ∈ [210, 219]
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,                      // x ∈ [220, 229]
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,                      // x ∈ [230, 239]
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,                      // x ∈ [240, 249]
    0, 0, 0, 0, 0, 0, 0,                                // x ∈ [250, 256]
];

// ── P3-2: Breadth-cost scaling ──────────────────────────────────────────────────────────────────
/// P3-2 (B5): Breadth-cost denominator for the specialist/generalist tradeoff.
/// Cost = `breadth_cost_k * tol_breadth / BREADTH_COST_SCALE`.
/// Integer scaling: `tol_breadth ∈ [100, 2000]`, base_metab ~ 2, so cost is several units.
/// CALIBRATION-PROVISIONAL: final value from P3-3 neutral-run measurement.
pub const BREADTH_COST_SCALE: i64 = 1000;

/// P3-1 (B4): Thermal tolerance penalty (Gaussian decay, integer LUT).
/// Computes `exp(−(deviation / breadth)²/2)` using a precomputed integer lookup table.
/// All integer; no float; deterministic cross-arch.
///
/// **Parameters:**
/// - `world_temp`: ambient temperature at entity position (centidegrees)
/// - `tol_optimum`: entity's tolerance optimum (centidegrees)
/// - `tol_breadth`: entity's tolerance breadth, half-width σ (centidegrees)
///
/// **Returns:** penalty factor ∈ [0, 256] (Q8.8 fixed-point).
/// - At `world_temp == tol_optimum`: penalty = 256 (no penalty; 100% metabolic capacity)
/// - At `world_temp == tol_optimum ± tol_breadth`: penalty ≈ 96 (exp(−1) ≈ 0.37; ~37% retained)
/// - At `world_temp == tol_optimum ± 2×tol_breadth`: penalty ≈ 7 (exp(−4) ≈ 0.018; ~2% retained)
///
/// **Application in stage_interactions (P3-2):**
/// `thermal_x256 = tolerance_penalty(...)` applied as a multiplicative factor on energy income:
/// `gained = ((got * eff / 256 * kept / 1000) * thermal_x256) / 256`
/// Penalty reduces uptake at suboptimal T (stress → slow uptake kinetics), driving selection toward T_optimum.
#[inline]
pub fn tolerance_penalty(world_temp: i32, tol_optimum: i32, tol_breadth: i32) -> i32 {
    let dev = (world_temp - tol_optimum).abs() as i64;
    let breadth = (tol_breadth as i64) * 2;  // full width = 2σ
    if breadth == 0 {
        return 256;  // guard: no breadth → no penalty (shouldn't happen at founder=500)
    }
    let normalized = (dev * 256 / breadth).min(256) as usize;
    THERMAL_KERNEL_Q256[normalized]
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

// ── P5-D: spatial (depth-attenuated) light ────────────────────────────────────────────────────────

/// P5-D: Critical depth (in world height units) below which photosynthesis is fully attenuated.
/// STRUCTURAL CONSTANT — calibrated ONCE at P5-D-1 so both an oxic and an anoxic band exist; do NOT tune per verdict.
pub const PHOTIC_H: i64 = 200;

/// P5-D: Q-scale denominator for photic-attenuation ramp (e.g., 256 for 8-bit fixed-point).
/// Used in: `light_attenuated = base_light * photic_atten(height) / PHOTIC_SCALE`.
pub const PHOTIC_SCALE: i64 = 256;

/// P5-D: Depth-attenuation ramp. Maps height ∈ [0, PHOTIC_H] → [0, PHOTIC_SCALE].
/// `0` at h=0 (deepest/dark) → `PHOTIC_SCALE` at h≥PHOTIC_H (shallow/lit).
fn photic_atten(height: i64) -> i64 {
    height.clamp(0, PHOTIC_H) * PHOTIC_SCALE / PHOTIC_H
}

/// P5-D: Spatial (depth-attenuated) light intensity. Per-cell function of tick + height.
///
/// When `enable_photic=false` (the default, gate OFF): returns `light_at_tick(ls, tick)` with ZERO
/// attenuation arithmetic — the exact same integer expression as before the patch. This guarantees
/// byte-identical output for every shipped config (all have `enable_photic=false`).
///
/// When `enable_photic=true` (gate ON, P5-D-1+): attenuates by depth via `photic_atten(height) / PHOTIC_SCALE`.
/// Cells with height ≥ PHOTIC_H get full light; shallow cells get full light; deep cells get diminished light
/// or zero. This creates a spatial oxic↔anoxic gradient.
///
/// Pure integer, deterministic (height is static per world-gen, light_at_tick is pure).
pub fn light_at(light: Option<&LightSpec>, tick: u64, enable_photic: bool, height: i64) -> i64 {
    match light {
        None => 0,
        Some(ls) => {
            let base = light_at_tick(ls, tick);
            if !enable_photic {
                base  // ★ F2: early return, NO arithmetic — byte-identical to today's l_now
            } else {
                base * photic_atten(height) / PHOTIC_SCALE  // gate ON: attenuate by height
            }
        }
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
            resource_base: 120, // NoiseWorld-calibrated magnitude [1, resource_base+1]; per-config tuning
            world_dim: 64,
            m_sim: 4,
            brain_period: 4, // K — behaviour at 16 Hz (64/4)
            metab_period: 2, // N — metabolism at 32 Hz, charged ×2 per tick (economy ≈invariant)
            excrete: 8,
            pheromone: 1.0,
            m_field: 1, // one field cell per world cell (the CLI default / doc 14 §1)
            speciation_threshold: 80,
            n_layers: 2,
            n_energy_layers: 2, // same as n_layers by default; dprime overrides to 2 (mineral layer excluded)
            d0_scaled: 1049, // round(0.001 × 1_048_576); mean lifetime ≈ 1000 ticks (economy/01)
            recycle_num: 77,  // round(0.3 × 256) = 76.8 → 77; recycle ≈ 30.1% (economy/01 §3)
            detritus_layer: None,    // C′-1: None → byte-identical Slice-C behavior (→ layer 0)
            detritus_frac_num: RECYCLE_DEN, // = 256; dormant (only active when detritus_layer is Some)
            light: None,             // D′-1: None → light economy inert, photo_gain stays 0
            // D′-2b: regulation gene. reg_gain_max=4 enables regulation (range [-4,+4]).
            // Non-dprime unaffected: reg_gain mutates only when has_light=true.
            // Set to 0 for the D′-2c constitutive-control experiment.
            reg_gain_max: 4,
            // D′-3a: mineral economy. None → inert; non-dprime configs are byte-identical.
            // MineralQuota only spawned when Some; queries return empty on non-dprime → safe.
            mineral_layer: None,
            km_mineral: 200,          // Km=20 model units × 10 scale
            u_max_mineral: 70,        // calibrated so N×U(M*)=regen×4096 at N*≈583 (local tuning)
            q_mineral: 4000,          // T_mineral≈T_energy at equilibrium (Liebig-binds condition)
            recycle_mineral_num: 102, // ≈0.4 × 256 (calibration recycle_mineral=0.4)
            overflow_delta: 50,       // energy drain when energy-ready but mineral-poor; calibrated
            // D′-2a: photo-machinery cost. Applies only when photo_gain > 0 (non-dprime configs
            // have photo_gain ≡ 0 → cost is inert → byte-identical isolation from existing goldens).
            // Calibrated at ≈9% of day income at threshold gain (NUM=1, DEN=16, n=2 → gain≥8).
            photo_cost_num: 1,
            photo_cost_den: 8,
            // P-2a: predation OFF by default — None for all 6 existing configs.
            predation: None,
            // P1-0: O₂-field economy OFF by default — false for all 5 existing configs + l3 (byte-identical).
            enable_oxygen: false,
            // P5-0: NO₃-field economy OFF by default — false for all 6 existing configs (byte-identical).
            enable_nitrate: false,
            // P5-D: spatial light economy OFF by default — false for all 6 existing configs (byte-identical).
            enable_photic: false,
            // P1-2b: O₂-cap OFF by default (0 → hypoxia disabled when enable_oxygen=false).
            o2_cap: 0,
            ablate_hypoxia: false,
            hypoxia_base_x1000: 1000,
            // E-4a: ontogenesis chain OFF by default — None for all 5 existing configs.
            morphogen: None,
            grn: None,
            // V-3-b: variable-length operators OFF by default — false for all 6 existing configs.
            enable_variable_length: false,
            // M7-e: coordination cost OFF by default — 0 for all 6 existing configs (neutral wire).
            c_coord: 0,
            // V-4: body-size axis OFF by default — false for all existing configs (driver_config
            // opts in explicitly).
            evolve_body_size: false,
            // EXT-0b: morphogen cap and step-count. Defaults are shipped values, byte-identical to main.
            gdev_cap: 4,           // default morphogen cap; swept to 5, 6 in EXT-0b diagnostic
            morphogen_steps: 8,    // default reaction-diffusion steps; paired (4→8, 5→10, 6→12) in EXT-0b
            // P3-1: ambient-tolerance thermal niche OFF by default — None for all 6 existing configs.
            ambient_tolerance: None,
            // P4/SL-1: settling-selection OFF by default — None for all 6 existing configs.
            settling: None,
            // DL-M: division-of-labor OFF by default — false for all 6 existing configs (byte-identical).
            division_of_labor: false,
            // DL-V: germ-throughput repro gate OFF by default — false for all 6 existing configs (byte-identical).
            dol_germ_repro: false,
            // DR-0: economy-coupled division-of-labor OFF by default — false for all existing configs (byte-identical).
            dol_economy: false,
            // GA-LOAD: mutation-load economy OFF by default — false for all 6 existing configs (byte-identical).
            enable_mutation_load: false,
            // GA-LOAD defaults (meaningful when enable_mutation_load=true, diagnostics-swept otherwise).
            // Anchored to U ≈ 1: f_del ≈ 1/16 at founder μ. Values below assume mutation_rate=32/256=0.125, L=136.
            mut_load_del_num: 16,    // del_den=8 → f_del = 16*256 / (8*256) = 16/8 = 2 (not 1/16; recalibrate if needed)
            mut_load_del_den: 256,   // placeholder; swept in ga-load diagnostic
            mut_load_ben_num: 2,     // beneficial rate ≪ deleterious (stub)
            mut_load_ben_den: 256,   // placeholder; swept in ga-load diagnostic
            burden_cost_k: 2,        // energy cost per load unit; anchored to ~2% of income (swept in diagnostic)
            // ENV-0a'-a1: spatial monopolization OFF by default — None for all existing configs (byte-identical).
            env_frontier_config: None,
            // EXT-0a/R30-1.1: income-harvest mode — Anchor (the shipped path) for all existing
            // configs (byte-identical). Preserves the pre-#408 discriminant: body_footprint=false.
            income_mode: IncomeMode::Anchor,
            // Rung 1 (#401): body-plan mutation OFF by default — false for all existing production
            // configs (byte-identical goldens; body_plan stays Square forever).
            enable_body_plan: false,
            // R30-1.1a (#412): Kleiber-reads-n_cells OFF by default — false for all existing
            // production configs (byte-identical; metab term stays gene-based).
            metab_reads_n_cells: false,
            // R30-1.1b (#414): newborn-energy-per-cell OFF by default — false for all existing
            // production configs (byte-identical; endowment stays flat e_cell).
            newborn_energy_per_cell: false,
            // P-1 (#429): propagule growth OFF by default — false for all existing production
            // configs (byte-identical; no truncation, no grow, no Grown/growth_cells hash-fold).
            enable_propagule: false,
            // P-2b (#448): provisioning OFF by default — false/0 for all existing production
            // configs (byte-identical; no Parent/Provisioned, no 5a_provision, no mutation, no fold).
            enable_provision: false,
            provision_rate_init: 0,
            provision_rate_locked: false,
            n_propagule_init: 0,
            n_propagule_locked: false,
        }
    }
}

/// Construction-time configuration handed to `Sim::new`.
///
/// **`Clone`, NOT `Copy`** (E-4a): carries `EconParams`, which lost `Copy` when `GrnSpec`'s
/// `Vec<i32>` fields entered it (see `EconParams` docs). Callers that reused a `SimConfig` value
/// twice now `.clone()` explicitly.
#[derive(Clone, Debug)]
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
    /// P3-3 thermal-verdict world-temp override. `Some([i32; 14])` injects custom biome temperatures
    /// (ONLY for verdict harness); `None` (default) uses stock BIOME_TEMP from world-gen. Widened
    /// from 13 to 14 entries alongside W-SIM-7 (#423)'s `FinalBiome::Ocean` (index 13).
    /// Passed to ProcgenWorld at construction; byte-identical when `None` (F1, golden-neutral gate).
    pub thermal_verdict_temps: Option<[i32; 14]>,
    /// ENV-0a'-a0: Multi-genotype founder seeding (optional, golden-neutral gate).
    /// When `Some(templates)`, founders are seeded from the caller-specified genome templates
    /// with a deterministic integer count split. Each template's founders carry a distinct
    /// SpeciesId (0, 1, 2, ...). When `None` (default, all existing production configs),
    /// the legacy single-founder path is used: one template from `econ.grn`/`econ.morphogen`
    /// with all `n_founders` assigned `SpeciesId(0)` — byte-identical to pre-ENV-0a' runs.
    ///
    /// Template seeds must sum to `n_founders` (strict integer accounting, no clamp).
    /// Each template is spawned deterministically in index order with co-prime position strides.
    pub founder_templates: Option<Vec<(Genome, u64)>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tolerance_penalty() {
        // At optimum temperature: penalty should be maximal (256 = no penalty applied).
        let p_at_optimum = tolerance_penalty(1500, 1500, 500);
        assert_eq!(p_at_optimum, 256, "penalty at optimum temp should be 256");

        // At ±1σ (±tol_breadth/2): large penalty (exp(-1) ≈ 0.37).
        let p_one_sigma = tolerance_penalty(2000, 1500, 500);
        assert!(p_one_sigma < 256, "penalty at +1σ should be less than 256");
        assert!(p_one_sigma > 0, "penalty should remain positive");

        // At ±1.5σ (±3/4 of tol_breadth): severe penalty (exp(-2.25) ≈ 0.105).
        let p_one_half_sigma = tolerance_penalty(2375, 1500, 500); // dev=875, breadth=1000, norm=224
        assert!(p_one_half_sigma < p_one_sigma, "penalty at +1.5σ should be less than +1σ");
        assert!(p_one_half_sigma >= 0, "penalty should be non-negative");

        // Symmetric: negative deviation should equal positive.
        let p_neg_one_sigma = tolerance_penalty(1000, 1500, 500);
        assert_eq!(p_neg_one_sigma, p_one_sigma, "penalty symmetric around optimum");

        // Guard: zero breadth returns 256 (degenerate, no penalty).
        let p_zero_breadth = tolerance_penalty(1500, 1500, 0);
        assert_eq!(p_zero_breadth, 256, "zero breadth should return 256");

        // Extreme temperature: penalty saturates at index 256 (return value 0).
        let p_extreme = tolerance_penalty(5000, 1500, 500);  // normalized >= 256 → kernel[256]=0
        let p_more_extreme = tolerance_penalty(10000, 1500, 500);
        assert_eq!(p_extreme, p_more_extreme, "penalty saturates at normalized=256");
        assert_eq!(p_extreme, 0, "penalty at extreme deviation is 0 (total stress)");
    }

    #[test]
    fn test_light_at_gate_off_byte_identity() {
        // F2: Gate OFF (enable_photic=false) returns light_at_tick with ZERO attenuation arithmetic.
        // Proof: output is independent of height.
        let spec = LightSpec {
            l_max: 100,
            period_ticks: 100,
            day_ticks: 50,
            km_photo: 30,
        };

        // Day tick (within day_ticks).
        let tick_day = 25;
        let l_base_day = light_at_tick(&spec, tick_day);
        assert_eq!(l_base_day, 100, "day tick should return l_max");

        // Gate OFF: all heights should return the same value as light_at_tick.
        for height in [0, 50, 100, PHOTIC_H, 1000].iter() {
            let l_gated_off = light_at(Some(&spec), tick_day, false, *height);
            assert_eq!(
                l_gated_off, l_base_day,
                "gate OFF at height {} should match light_at_tick",
                height
            );
        }

        // Night tick (outside day_ticks).
        let tick_night = 75;
        let l_base_night = light_at_tick(&spec, tick_night);
        assert_eq!(l_base_night, 0, "night tick should return 0");

        // Gate OFF at night: all heights should return 0.
        for height in [0, 50, 100, PHOTIC_H, 1000].iter() {
            let l_gated_off = light_at(Some(&spec), tick_night, false, *height);
            assert_eq!(
                l_gated_off, 0,
                "gate OFF at night (height {}) should return 0",
                height
            );
        }
    }

    #[test]
    fn test_light_at_gate_off_none_returns_zero() {
        // When light=None, light_at always returns 0 (no spec to attenuate).
        for height in [0, 100, 1000].iter() {
            let l = light_at(None, 12345, false, *height);
            assert_eq!(l, 0, "light=None at height {} should return 0", height);

            let l = light_at(None, 12345, true, *height);
            assert_eq!(l, 0, "light=None at height {} (gate ON) should return 0", height);
        }
    }

    #[test]
    fn test_light_at_gate_on_depth_ramp() {
        // F2: Gate ON (enable_photic=true) applies attenuation.
        let spec = LightSpec {
            l_max: 100,
            period_ticks: 100,
            day_ticks: 50,
            km_photo: 30,
        };

        // Day tick: attenuation applies.
        let tick_day = 25;
        let l_base_day = light_at_tick(&spec, tick_day);
        assert_eq!(l_base_day, 100, "day tick should return l_max");

        // Deepest cell (height=0): fully attenuated → 0.
        let l_deep = light_at(Some(&spec), tick_day, true, 0);
        assert_eq!(l_deep, 0, "height=0 should return 0 (fully attenuated)");

        // At PHOTIC_H: full light (no attenuation).
        let l_photic = light_at(Some(&spec), tick_day, true, PHOTIC_H);
        assert_eq!(
            l_photic, l_base_day,
            "height=PHOTIC_H should return full light (no attenuation)"
        );

        // Above PHOTIC_H: clamps to full light (no overshoot).
        let l_above = light_at(Some(&spec), tick_day, true, PHOTIC_H + 100);
        assert_eq!(
            l_above, l_base_day,
            "height > PHOTIC_H should clamp to full light"
        );

        // Intermediate heights: monotonically increasing (0 < h < PHOTIC_H).
        let l_quarter = light_at(Some(&spec), tick_day, true, PHOTIC_H / 4);
        let l_half = light_at(Some(&spec), tick_day, true, PHOTIC_H / 2);
        let l_three_quarter = light_at(Some(&spec), tick_day, true, 3 * PHOTIC_H / 4);

        assert!(l_quarter > 0, "light at 1/4 depth should be > 0");
        assert!(l_quarter < l_half, "light strictly increases with height");
        assert!(l_half < l_three_quarter, "light strictly increases with height");
        assert!(l_three_quarter < l_photic, "light strictly increases with height");
    }

    #[test]
    fn test_light_at_gate_on_night_zero() {
        // At night, even with gate ON, light is 0 (nothing to attenuate).
        let spec = LightSpec {
            l_max: 100,
            period_ticks: 100,
            day_ticks: 50,
            km_photo: 30,
        };

        let tick_night = 75;
        let l_base = light_at_tick(&spec, tick_night);
        assert_eq!(l_base, 0, "night tick should return 0");

        // Gate ON: should still return 0 (0 * anything = 0).
        for height in [0, PHOTIC_H / 2, PHOTIC_H, 1000].iter() {
            let l = light_at(Some(&spec), tick_night, true, *height);
            assert_eq!(
                l, 0,
                "gate ON at night (height {}) should return 0",
                height
            );
        }
    }

    #[test]
    fn test_light_at_determinism_integer() {
        // Pure integer, no float/rand — same inputs yield same output.
        let spec = LightSpec {
            l_max: 256,
            period_ticks: 200,
            day_ticks: 100,
            km_photo: 40,
        };

        let tick = 50;
        let height = 120;

        // Call twice, expect identical results.
        let l1 = light_at(Some(&spec), tick, true, height);
        let l2 = light_at(Some(&spec), tick, true, height);
        assert_eq!(l1, l2, "deterministic: same inputs → same output");

        // Calling with different gates/heights produces different results (as expected).
        let l_off = light_at(Some(&spec), tick, false, height);
        let l_on = light_at(Some(&spec), tick, true, height);
        assert_eq!(l_off, 256, "gate OFF should return full light at day");
        // At height=120, atten = 120 * 256 / 200 = 153.6 → 153 (truncated).
        // light_at = 256 * 153 / 256 = 153.
        assert_eq!(l_on, 153, "gate ON at height 120 should return attenuated value");
    }
}
