//! animata v2 `sim-core` — the deterministic simulation core. **M1: first life (Ф0 economy)** —
//! the empty M0 stage skeleton is now filled with the minimal closed ecological loop
//! genome → energy balance → division/death → emergent selection.
//!
//! Determinism contract (held mechanically):
//! * The CONSERVED layer (energy ledger + resource field) is **pure integer fixed-point** — exact
//!   `== 0` conservation (R13/R15), associative integer merge (R14). The no-float guard test keeps
//!   `energy.rs`/`genome.rs`/the ledger float-free.
//! * Float enters only the SPATIAL layer via the `world` heightmap noise (behind a feature), which
//!   makes the trajectory arch-dependent → the golden is arm64-pinned (`v2_golden_*`, arm64-only CI),
//!   while the energy invariant + two-run-same-seed (integer, arch-independent) run on both arches.
//! * One reduction point ([`deterministic_fold`]): collect → sort by `Entity` → fold. Never natural
//!   query order. Core state maps are [`DetMap`]/`BTreeSet`, never a randomly-hashed std map.
//! * `Sim::step` is `&mut self` only — no clock, no render, no IO (R1). Backends (`world`/`fields`)
//!   are injected as boxed trait objects, so the core depends on no backend crate.

mod components;
mod det_map;
mod energy;
mod genome;
mod grid;
mod grn;
mod grn_lut;
mod hash;
mod homology;
mod input;
mod morphogen;
mod params;
mod pool;
mod predation;
mod rng;
mod stages;
mod traits;

pub use components::{
    BrainOutput, BrainState, Energy, Intent, MineralQuota, PendingSpeciation, Sensors, SpeciesId,
    Velocity, VelocityNext,
};
pub use det_map::DetMap;
pub use energy::EnergyLedger;
pub use genome::{isqrt, size_pow_three_quarters, CellGraph, Genome, Phenotype, RespiratoryPathway};
pub use grid::{morton2, NeighborGrid};
pub use hash::{deterministic_fold, fnv_mix, FNV_OFFSET};
pub use homology::genome_distance;
pub use input::{sort_tick_events, InputEvent, InputKind};
pub use grn::{grn, grn_resolve, sigma as grn_sigma, CellType, GrnSpec};
pub use grn_lut::{
    SIGMA_LUT, SIGMA_LUT_SHA256, EXPR_MAX as GRN_EXPR_MAX, LUT_BIN as GRN_LUT_BIN,
    PREACT_MAX as GRN_PREACT_MAX, PREACT_MIN as GRN_PREACT_MIN,
};
pub use morphogen::{morphogen, morphogen_steps, Boundary, Gradient, MorphogenSpec};
pub use params::{AmbientToleranceSpec, EconParams, FieldId, LayerSpec, LightSpec, SettlingSpec, SimConfig, D0_MASK, RECYCLE_DEN, light_at_tick, tolerance_penalty};
pub use predation::{resolve_encounter, refuge_attenuate, Outcome, PredationMode, PredationSpec, SizeRefugeSpec};
pub use stages::expressed_capacity;
pub use pool::{ScatterParams, SimPool};
pub use rng::{seed_fold, splitmix64};
pub use traits::{
    brain_w_hh, brain_w_ho, brain_w_ih, Brain, BrainRes, Deposit, FieldRes, FieldStore,
    MergeStrategy, WorldRes, WorldView, BRAIN_HIDDEN, BRAIN_INPUTS, BRAIN_OUTPUTS, BRAIN_SHIFT,
    BRAIN_WEIGHTS,
};

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::Schedule;
use std::sync::Arc;

/// Integer 2-vector — the fixed-point spatial domain (`i64`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Vec2Fixed(pub i64, pub i64);

/// Hot position component, version `t` (read). Double-buffered with [`PositionNext`]; stage 10 Swap.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Position(pub Vec2Fixed);

/// Write-side of the [`Position`] double buffer.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PositionNext(pub Vec2Fixed);

/// Core clock + run seed. `tick` is the only time the core knows.
#[derive(Resource, Debug)]
pub struct SimClock {
    pub seed: u64,
    pub tick: u64,
}

/// Tick-stamped input stream (R18). Empty in Ф0.
#[derive(Resource, Default)]
pub struct InputLog {
    pub events: Vec<InputEvent>,
}

/// Parents that divided this tick (Entity bits) — the reproduction signal for the Price covariance.
/// `BTreeSet` ⇒ deterministic iteration.
///
/// `stillbirths` (E-5b): cumulative count of REAL, criterion-triggered stillbirths — unlike
/// `parents`, this is a run-lifetime total, NEVER cleared per-tick. Incremented ONLY when
/// `Genome::is_stillbirth_by_size_criterion` attributes the `None` to the size-viability gate, NOT
/// to the `#[cfg(test)]` `force_decode_none` injection — a test mixing both in one run would
/// double-count (documented on the predicate; probes must run "clean"). Folded into this EXISTING
/// resource rather than a new `Resource` type/insertion: `bevy_ecs` 0.19 allocates a fresh `Entity`
/// slot per `insert_resource` call, so one more `w.insert_resource` in `Sim::new` would shift every
/// subsequent entity's bits — and therefore every `seed_fold(seed, [.., bits, ..])` RNG draw — for
/// every config, silently breaking all six goldens (the same pitfall `SpeciationState` is kept
/// outside the ECS `World` to avoid — see its doc comment on the `Sim` struct).
#[derive(Resource, Default)]
pub struct ReproEvents {
    pub parents: std::collections::BTreeSet<u64>,
    pub stillbirths: u64,
}

/// One organism's traits + offspring-this-tick, snapshotted by Observe for the telemetry crate.
/// Slots 0–5: six Ф0 traits (metabolism_eff, move_speed, sense_range, size, repro_threshold,
/// mutation_rate). Slots 6–7: B-2 layer traits (uptake_layer, excrete_layer) — extended so that
/// layer-targeting selection is observable through the Price covariance path.
///
/// D′-3b: `photo_in` and `chem_in` carry the per-cell realized income split recorded at the
/// booking sites (stage_interactions). These are EXACT copies of the booked integers — never
/// re-derived — so they match the tick that credited the energy. Purely observational; never
/// fed to state_hash or any conserved value.
#[derive(Clone, Copy, Debug, Default)]
pub struct TraitSample {
    pub traits: [i32; 8],
    pub offspring: u32,
    /// D′-3b: realized per-cell photo energy income this tick (exact booked integer).
    /// 0 for non-dprime configs (photo_gain ≡ 0 → photo_demand returns 0 always).
    pub photo_in: i64,
    /// D′-3b: realized per-cell chemical (field) energy income this tick (after metabolism_eff).
    /// 0 if field was empty or cell received no grant.
    pub chem_in: i64,
}

/// Speciation state (M5/criterion 2). Tracks the founder genome of each species and the
/// parent-child species tree. Stored on `Sim` (NOT as an ECS Resource) so that inserting it
/// does not allocate a bevy entity, which would shift the fresh-entity counter and break the
/// deterministic golden (entity ids feed `deterministic_fold` via `e.to_bits()`).
pub struct SpeciationState {
    /// Founder genome of each species, keyed by SpeciesId. Grows monotonically — no GC of
    /// extinct entries, so `parent_of` references stay valid. Bounded by total divisions.
    pub refs: DetMap<SpeciesId, Genome>,
    /// Parent species of each non-root species (needed for the 5a separation gate in tests).
    pub parent_of: DetMap<SpeciesId, SpeciesId>,
    /// Monotone allocator: the next SpeciesId to hand out. Species 0 is the root (all founders).
    pub next_id: u32,
}

impl Default for SpeciationState {
    fn default() -> Self {
        SpeciationState {
            refs: DetMap::default(),
            parent_of: DetMap::default(),
            next_id: 1,
        }
    }
}

/// D-3a: fixed-point scale for the body-size telemetry (`mean_body_size`/`multicellular_frac`).
/// Same scale as `RECYCLE_DEN`/`metabolism_eff` — one integer multiply-then-divide, no float.
pub const BODY_SIZE_SCALE: i64 = 256;

/// D-3a (#272): pure integer aggregate over a population's per-entity `CellGraph::body_size()`
/// values — `(mean_body_size, max_body_size, multicellular_frac)`, mean/frac fixed-point ×
/// [`BODY_SIZE_SCALE`], max a raw count. `(0, 0, 0)` when `sizes` is empty (population 0). Order-
/// independent (sum/max/count are commutative), so entity-id ordering upstream is for the OTHER
/// telemetry this shares a pass with (genome_diversity), not required by this fn itself. Extracted
/// as a standalone fn so `stage_observe`'s wiring and the arithmetic itself are both directly
/// unit-testable (`cli` crate's `d3a_body_size.rs`).
pub fn body_size_aggregate(sizes: &[i64]) -> (i64, i64, i64) {
    if sizes.is_empty() {
        return (0, 0, 0);
    }
    let n = sizes.len() as i64;
    let sum: i64 = sizes.iter().sum();
    let max = *sizes.iter().max().unwrap();
    let count_multicellular = sizes.iter().filter(|&&s| s > 1).count() as i64;
    (sum * BODY_SIZE_SCALE / n, max, count_multicellular * BODY_SIZE_SCALE / n)
}

/// Read-only telemetry sink (stage 9). Overwritten each tick. The `telemetry` crate derives Price
/// covariance / diversity from `samples` — keeping that statistics code OUT of the core (R1).
#[derive(Resource, Default)]
pub struct Telemetry {
    pub population: i64,
    pub field_total: i64,
    /// Signal-field total concentration (R25 metric) — read-only, never feeds the tick.
    pub signal_total: f32,
    pub samples: Vec<TraitSample>,
    /// Live species count (label-based, from stage_observe). Integer → safe for CI assertions.
    pub species_count: u64,
    /// Per-species live member count: `(species_id, count)` sorted by id. Observational; use
    /// for Shannon/Simpson diversity in the CLI (never fed to the tick or state hash).
    pub species_census: Vec<(u32, u32)>,
    /// D′-1: total light energy credited to agents this tick (Σᵢ photo_gain_i·L(t)/(km+L(t))).
    /// Written by stage_interactions each tick; 0 for non-dprime configs or when L(t)=0 (night).
    /// Observational only — never fed to the tick or state hash.
    pub photo_produced: i64,
    /// D′-2a: cumulative photo-machinery expression cost dissipated across the ENTIRE run (eu).
    /// Accumulates monotonically (`+=`) each metab tick; never reset. 0 for non-dprime configs
    /// (photo_gain ≡ 0 there → cost inert). Checked by the non-inertness tooth (must be > 0 after
    /// ≥6000 ticks on dprime seed 0xA11A_2A11 where photo sweeps). Observational only.
    pub photo_cost_total: i64,

    // ── D′-2c: reg-activity telemetry ────────────────────────────────────────────────────────────
    /// D′-2c: count of live agents with `reg_gain ≠ 0` (regulation ACTIVE) this tick.
    /// Computed in stage_observe from all live genomes. Divide by `population` to get the fraction.
    /// 0 for non-dprime configs (reg_gain stays 0 there — has_light=false blocks mutation).
    /// Observational only — never fed to tick or state hash. No golden is re-pinned.
    pub reg_active_count: i64,
    /// D′-2c: count of live agents with `reg_gain > 0` (day-phase expression gate active).
    /// A sub-count of `reg_active_count`; agents with `reg_gain < 0` are night-phase regulators.
    /// Observational only — never fed to tick or state hash.
    pub reg_active_day_count: i64,

    // ── D′-3b: per-entity realized income record ──────────────────────────────────────────────────
    /// D′-3b: entity_bits → (photo_in, chem_in) for the current tick.
    /// Populated in stage_interactions with the EXACT booked integers (same values credited to Energy).
    /// Cleared at the start of each stage_interactions call; consumed (via std::mem::take) in
    /// stage_observe to build per-sample income split in TraitSample. Purely observational —
    /// never fed to state_hash or any conserved value; non-dprime entities always have (0, 0).
    pub income_record: DetMap<u64, (i64, i64)>,

    /// V-3-e: population diversity observable — mean [`genome_distance`] over CONSECUTIVE valid
    /// (`Some(grn_spec)`) genomes, entity-id order (computed by `stages::stage_observe`). `0` when
    /// fewer than 2 valid genomes exist (all non-phase2 configs; single-survivor populations).
    /// Read-only, never fed to the tick or folded into `state_hash` — the speciation/reproductive-
    /// barrier consumer is deferred to a later phase.
    pub genome_diversity: i64,

    // ── D-3a: body-size telemetry (multicellularity emergence measurable, #272) ─────────────────
    /// D-3a: mean body size (`Σ Phenotype.graph.module_cell_count`, clamped ≥1 per entity) over the
    /// live population, entity-id order, fixed-point ×[`BODY_SIZE_SCALE`]. `0` when population is 0.
    /// Read-only, never fed to the tick or folded into `state_hash`.
    pub mean_body_size: i64,
    /// D-3a: max body size over the live population — a raw integer count (NOT fixed-point). `0`
    /// when population is 0. Read-only, never fed to the tick or folded into `state_hash`.
    pub max_body_size: i64,
    /// D-3a: fraction of live entities with body_size > 1 (multicellular), fixed-point
    /// ×[`BODY_SIZE_SCALE`]. `0` when population is 0. Every non-phase2 config decodes an empty
    /// `CellGraph` (body_size 1 for all) so this stays 0 there — byte-identical to before D-3a.
    /// Read-only, never fed to the tick or folded into `state_hash`.
    pub multicellular_frac: i64,
}

// ── R-1: render seam (RnD 02 §det-orthogonal, R26/R17/R19/R21) ─────────────────────────────────────

/// One live creature's render-relevant state — an OWNED, `Copy` snapshot (critic F5): never a borrow
/// into the ECS, so it safely outlives the query and can cross to the render thread. Mirrors v1
/// `render_snapshot.rs::CreatureDot`, adapted to v2 components.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CreatureDot {
    pub id: u64,
    pub pos: Vec2Fixed,
    pub energy: i64,
    pub species: u32,
    /// Compact appearance token (E-4a ontogenesis attractor); `None` for the Ф0 / non-morphogen
    /// configs. `CellType` is `Copy` — an owned value, not a borrow (critic F5).
    pub cell_type: Option<CellType>,
    /// Body size (R-4): read from `Genome::size` in `observe_render`. Range [1, 32]. Scales morphology
    /// rendering and energy-visual representation. Copy snapshot, not a borrow (critic F5).
    pub size: i32,
    /// Uptake layer (R-4): read from `Phenotype::uptake_layer` in `observe_render`. Which conserved
    /// layer the creature feeds from. Copy snapshot for render consistency. May support future biome/
    /// layer-driven coloring.
    pub uptake_layer: i32,
}

/// T-2: Snapshot of life statistics — population-level metrics collected from live entities.
/// Integer-only, observational (never fed to state_hash or tick). Golden-NEUTRAL.
#[derive(Clone, Debug, Default)]
pub struct LifeStats {
    /// Tick at which this snapshot was captured.
    pub tick: u64,
    /// Total number of living entities (matches RenderSnapshot.population type).
    pub population: i64,
    /// Average energy of all living entities. 0 if population is 0.
    pub avg_energy: i64,
    /// Average body size (biomass) of all living entities. 0 if population is 0.
    pub avg_biomass: i64,
    /// Count of distinct species (live organisms with unique SpeciesId).
    pub species_count: u64,
    /// Trophic layer distribution — count of entities feeding from each layer.
    /// Index = uptake_layer, value = count. Empty if no entities.
    pub trophic_fractions: Vec<u64>,
}

/// R-1 render seam: an OWNED, read-only copy of per-entity render state, produced by
/// [`Sim::observe_render`]. Holds no borrow into the `Sim` — the render thread can read it while the
/// worker steps the next tick. Terrain is read separately via `WorldView` (R-2).
#[derive(Clone, Debug, Default)]
pub struct RenderSnapshot {
    pub tick: u64,
    pub population: i64,
    /// Live species count — the other aggregate the R-1 HUD shows (from [`Telemetry::species_count`]).
    pub species_count: u64,
    pub creatures: Vec<CreatureDot>,
    /// T-2: Population-level life statistics (golden-NEUTRAL, observational).
    pub life: Option<LifeStats>,
}

#[cfg(feature = "perf")]
mod perf {
    use crate::DetMap;
    use bevy_ecs::prelude::Resource;

    /// Deterministic work-counters (R26 / D1a-c): per-entity operation counts on the real hot paths.
    /// Accumulated monotonically across ticks — never reset, never fed into the tick or state hash.
    /// Locked in the `work_counter` gate test via `counter ≤ C · N_peak · ticks` (O(N) bound).
    #[derive(Resource, Default, Clone, Copy, Debug)]
    pub struct WorkCounters {
        /// integer-brain `infer` calls (stage 2, runs every K ticks only).
        pub brain_infer: u64,
        /// `conserved_take` calls in stage 6 interactions (every entity, every tick).
        pub field_takes: u64,
        /// entity iterations in stage 7 birth/death (every entity, every tick).
        pub birth_death_iters: u64,
        /// scatter deposits in stage 8 serial gather (every entity, every tick).
        pub scatter_deposits: u64,
    }

    /// Per-stage instrumentation (R26). Timing is non-deterministic → never feeds the tick or hash.
    #[derive(Default)]
    pub struct PerfReport {
        stages: DetMap<&'static str, (u128, u128)>,
        /// Snapshot of accumulated work counters at the end of the last `step()`.
        pub work: WorkCounters,
    }
    impl PerfReport {
        pub fn record(&mut self, name: &'static str, ns: u128, entities: u128) {
            let e = self.stages.entry(name).or_insert((0, 0));
            e.0 += ns;
            e.1 = ns / entities.max(1);
        }
        pub fn stages(&self) -> &DetMap<&'static str, (u128, u128)> {
            &self.stages
        }
    }
}
#[cfg(feature = "perf")]
pub use perf::{PerfReport, WorkCounters};

/// The deterministic core. Build with [`Sim::new`] (backends injected), drive with [`Sim::step`].
pub struct Sim {
    world: World,
    stages: Vec<(&'static str, Schedule)>,
    /// Speciation state lives here (NOT in the ECS world) to avoid allocating an extra bevy entity,
    /// which would shift the fresh-entity counter and break the golden (see SpeciationState doc).
    speciation: SpeciationState,
    #[cfg(feature = "perf")]
    perf: PerfReport,
}

impl Sim {
    /// Build the world, spawn `n_founders`, wire the 11-stage pipeline. `world`/`field`/`brain` are the
    /// injected backends (keeps R1 — `sim-core` names only the traits).
    pub fn new(
        config: SimConfig,
        world: Box<dyn WorldView>,
        field: Box<dyn FieldStore>,
        brain: Box<dyn Brain>,
    ) -> Self {
        let econ = config.econ;
        // R8: grids are integer and consistent — validated at construction (no save/load until M5,
        // so this constructor guard is the "checked on load" invariant for now).
        assert!(econ.m_sim > 0 && econ.world_dim % econ.m_sim == 0, "world_dim % m_sim != 0 (R8)");
        // Pass econ.m_field (the INDEPENDENT expected value) — not field.m_field() (which would
        // compare the field to itself, a tautology that can never fail). Fix for M1/F1.
        field.check_meta(econ.m_field).expect("field M_field meta check (R8)");
        // D′-2b R20 alignment guard: when a light field is present, day-night phase boundaries
        // MUST align with the metabolism period so every n-tick metab window falls wholly within
        // one phase. `stage_metabolism` samples L(t) ONCE at the start of each n-tick lump and
        // charges (eff·n)/den — this is R20-invariant ONLY when L(t) is constant across the window.
        // If a window straddles a day↔night boundary, eff is unrepresentative of the full period
        // and the cost is deterministic-but-wrong (trajectory-corrupting). Hard assert (not
        // debug_assert): CI runs --release; a debug_assert would be invisible in the green gate.
        // dprime_config satisfies: day_ticks=50, period_ticks=100, metab_period=2 → 50%2=0, 100%2=0.
        if let Some(ls) = econ.light {
            let n = econ.metab_period.max(1);
            assert!(
                ls.day_ticks % n == 0 && ls.period_ticks % n == 0,
                "R20 alignment violated: light phase boundaries must align with \
                 metab_period={n} so every n-tick cost window is wholly within one phase. \
                 day_ticks={dt} % {n} = {dr}, period_ticks={pt} % {n} = {pr}. \
                 Fix: ensure day_ticks and period_ticks are exact multiples of metab_period.",
                dt = ls.day_ticks, dr = ls.day_ticks % n,
                pt = ls.period_ticks, pr = ls.period_ticks % n,
            );
        }

        let mut w = World::new();
        w.insert_resource(SimClock { seed: config.seed, tick: 0 });
        w.insert_resource(InputLog::default());
        w.insert_resource(Telemetry::default());
        w.insert_resource(econ.clone());
        // NeighborGrid is intentionally NOT inserted here (M1/F2): it was rebuilt every tick by
        // stage_spatial_rebuild but never queried by any stage → dead per-tick work. Removed until
        // a real neighbour-coupled consumer lands (M4+ nearest-neighbour interactions).
        w.insert_resource(ReproEvents::default());
        // The sim's OWN scatter pool with an explicit N (F5) + the merge strategy.
        w.insert_resource(SimPool::new(config.sim_threads));
        w.insert_resource(ScatterParams {
            threads: config.sim_threads,
            strategy: config.merge_strategy,
        });

        // B-2 layer-count guard: the genome clamp uses econ.n_layers; the field must match.
        // build_sim sets econ.n_layers = config.n_layers — this catches any direct Sim::new callers.
        debug_assert_eq!(
            econ.n_layers, config.n_layers,
            "econ.n_layers ({}) != config.n_layers ({}): set econ.n_layers = config.n_layers before \
             calling Sim::new (build_sim does this automatically)",
            econ.n_layers, config.n_layers,
        );
        // V-1: seed the founder's heritable GRN/morphogen spec from `EconParams`'s founder
        // template — the ONLY place `econ.grn`/`econ.morphogen` are consulted; `decode` reads
        // `self.grn_spec`/`self.morphogen_spec` from here on (never `econ.grn`/`econ.morphogen`
        // directly). `GrnSpec` is `Arc`-wrapped for CoW; `MorphogenSpec` is `Copy`, no `Arc` needed.
        // P3-1: initialize ambient-tolerance genes from gate (founder=0 inert until gate is Some).
        let founder = Genome::founder(config.n_layers)
            .with_specs(econ.grn.clone().map(Arc::new), econ.morphogen)
            .with_ambient_tolerance(econ.ambient_tolerance);
        let has_mineral = econ.mineral_layer.is_some();
        for i in 0..config.n_founders {
            // Deterministic scatter across the domain (co-prime strides → spread, no float).
            let x = ((i.wrapping_mul(7).wrapping_add(3)) % econ.world_dim as u64) as i64;
            let z = ((i.wrapping_mul(13).wrapping_add(5)) % econ.world_dim as u64) as i64;
            let p = Vec2Fixed(x, z);
            // D′-3a: founders get MineralQuota(0) when mineral is active. The quota(0) does NOT
            // change EnergyLedger.initial: quota=0 contributes nothing to the conserved sum.
            // Non-dprime configs do NOT spawn MineralQuota → their archetype is unchanged →
            // byte-identical goldens (no extra entity column, no hash perturbation).
            // E-1: decode the founder genome once at birth; Ф0 always returns Some.
            let founder_phenotype = founder.decode(&econ).expect("Ф0 founder decode must succeed");
            // V-1: `Genome` is no longer `Copy` (heritable `Arc<GrnSpec>`) — each spawn needs its
            // own clone; the last iteration's clone is cheap (CoW: an `Arc` refcount bump, not a
            // deep copy — critic F14).
            if has_mineral {
                w.spawn((
                    Position(p),
                    PositionNext(p),
                    Velocity::default(),
                    VelocityNext::default(),
                    Energy(config.founder_energy),
                    founder.clone(),
                    founder_phenotype, // E-1: cached cold phenotype
                    SpeciesId(0),
                    Sensors::default(),
                    Intent::default(),
                    BrainState::zeroed(),
                    BrainOutput::zeroed(),
                    MineralQuota(0),
                ));
            } else {
                w.spawn((
                    Position(p),
                    PositionNext(p),
                    Velocity::default(),
                    VelocityNext::default(),
                    Energy(config.founder_energy),
                    founder.clone(),
                    founder_phenotype, // E-1: cached cold phenotype
                    SpeciesId(0),
                    Sensors::default(),
                    Intent::default(),
                    // Spawn contract (D-Brain-2a): brain buffers start zeroed — same path as a newborn.
                    BrainState::zeroed(),
                    BrainOutput::zeroed(),
                ));
            }
        }

        // P2: the energy ledger excludes ONLY the O₂ layer — it is a separate OPEN quantity (produced by
        // photosynthesis, consumed by respiration) with no ledger counterpart (its own R30-P2 balance),
        // so its dynamic mass would drift R15. Pre-P2 this was masked because O₂ was STATIC (a constant
        // on both `initial` and the running total → it cancelled); dynamic O₂ broke the constant.
        // NOTE: `n_energy_layers` is the MUTATION range, NOT conservation membership — the MINERAL layer
        // is also `>= n_energy_layers` but IS conserved (agents hold `MineralQuota`), so we must keep it.
        // Only O₂ is excluded. Non-oxygen configs: unchanged (byte-identical).
        let mut field_total = field.conserved_total_all();
        if econ.enable_oxygen {
            field_total -= field.conserved_total(crate::FieldId::Oxygen.as_usize());
        }
        let agents_total = config.n_founders as i64 * config.founder_energy;
        w.insert_resource(EnergyLedger {
            initial: field_total + agents_total,
            produced: 0,
            dissipated: 0,
            lost: 0,
        });
        w.insert_resource(WorldRes(world));
        w.insert_resource(FieldRes(field));
        w.insert_resource(BrainRes(brain));
        #[cfg(feature = "perf")]
        w.insert_resource(WorkCounters::default());

        // SpeciationState lives on the Sim struct (NOT in the ECS world) to avoid allocating an
        // extra bevy entity that would shift the fresh-entity counter and break the golden.
        let mut speciation = SpeciationState::default();
        speciation.refs.insert(SpeciesId(0), founder);

        Self {
            world: w,
            stages: build_stages(),
            speciation,
            #[cfg(feature = "perf")]
            perf: PerfReport::default(),
        }
    }

    /// Advance one fixed tick: 11 stages in fixed order (barrier between each), then bump the tick.
    pub fn step(&mut self) {
        #[cfg(feature = "perf")]
        let n = self.population() as u128;
        for (_name, sched) in &mut self.stages {
            #[cfg(feature = "perf")]
            let start = std::time::Instant::now();
            sched.run(&mut self.world);
            #[cfg(feature = "perf")]
            self.perf.record(_name, start.elapsed().as_nanos(), n);
        }
        // Sync accumulated work counters into PerfReport (monotonic — never reset).
        #[cfg(feature = "perf")]
        {
            self.perf.work = *self.world.resource::<WorkCounters>();
        }
        // M5: assign final SpeciesId to newly-born entities (marked with PendingSpeciation by
        // stage_birth_death). Runs after all stages so children are live in the world.
        self.process_pending_speciation();
        self.world.resource_mut::<SimClock>().tick += 1;
    }

    /// Canonical golden state hash (R19, arch-bound → arm64): folds Position + Energy + Genome (incl.
    /// the evolved brain weights) + the recurrent `BrainState` (`h_old`/`h_new`) + the motor
    /// `BrainOutput` + the current `Velocity` per entity (via the single [`deterministic_fold`] point)
    /// plus the signal field (f32 bits). Folding `Velocity` closes the M1/F6 gap: two states differing
    /// only in velocity now hash differently. The conserved field is NOT here — it has its own
    /// [`Sim::conserved_field_hash`] for R14.
    ///
    /// **SpeciesId is intentionally excluded** (M5/criterion 4/F7): it is a deterministic observational
    /// label, not a behavioural or energy-state driver. Including it would make the golden depend on
    /// label allocation order without adding physical information. The separate [`Sim::species_hash`]
    /// covers the SpeciesId layer in the two-run-identical CI check.
    pub fn state_hash(&mut self) -> u64 {
        let mut q = self
            .world
            .query::<(Entity, &Position, &Energy, &Genome, &BrainState, &BrainOutput, &Velocity, Option<&MineralQuota>)>();
        let items: Vec<(Entity, u64)> = q
            .iter(&self.world)
            .map(|(e, p, en, g, bs, bo, v, mq)| {
                let mut h = fnv_mix(FNV_OFFSET, p.0 .0 as u64);
                h = fnv_mix(h, p.0 .1 as u64);
                h = fnv_mix(h, en.0 as u64);
                h = g.hash_contribution(h);
                for &iv in bs.h_old.iter().chain(bs.h_new.iter()) {
                    h = fnv_mix(h, iv as u64);
                }
                for &iv in &bo.out {
                    h = fnv_mix(h, iv as u64);
                }
                h = fnv_mix(h, v.0 .0 as u64);
                h = fnv_mix(h, v.0 .1 as u64);
                // D′-3a: fold mineral quota only when non-zero (same gating as photo_gain in
                // Genome::hash_contribution). Non-dprime entities have no MineralQuota → mq=None
                // → sum=0 → byte-identical for all non-dprime goldens. Dprime: quota is folded
                // when it differs from 0 (immediately after first mineral uptake tick).
                if let Some(m) = mq {
                    if m.0 != 0 {
                        h = fnv_mix(h, m.0 as u64);
                    }
                }
                (e, h)
            })
            .collect();
        let entities = deterministic_fold(items);
        let signal = self.world.resource::<FieldRes>().0.signal_hash();
        fnv_mix(entities, signal)
    }

    /// Hash of the CONSERVED field only (integer, canonical order). The R14 subject: identical across
    /// thread counts. Kept SEPARATE from [`Sim::state_hash`] so the float signal can never make the
    /// 1-vs-N conserved assert flaky.
    pub fn conserved_field_hash(&self) -> u64 {
        self.world.resource::<FieldRes>().0.conserved_hash()
    }

    /// Total conserved-field energy on a single layer (C tests, not fed to state hash).
    /// Panics if `layer >= n_layers`.
    pub fn field_layer_total(&self, layer: usize) -> i64 {
        self.world.resource::<FieldRes>().0.conserved_total(layer)
    }

    /// Total signal concentration (serial reduction).
    pub fn signal_total(&self) -> f32 {
        self.world.resource::<FieldRes>().0.signal_total()
    }

    /// NaN/Inf guard on the signal field — every cell finite (always-on in the release harness).
    pub fn signal_finite(&self) -> bool {
        self.world.resource::<FieldRes>().0.signal_all_finite()
    }

    /// Energy-conservation residual (R15) — MUST be 0 every tick. Sums live conserved field + agent
    /// energy (+ mineral quota when D′-3a active) and the ledger buckets. The signal field is float,
    /// NOT in the balance. The mineral layer (when `mineral_layer.is_some()`) is part of
    /// `conserved_total_all()` and mineral quotas are added to `agents`, making this one unified
    /// identity: `(field_E + field_M + Σ energy + Σ quota + dissipated + lost) − produced − initial = 0`.
    /// When `mineral_layer` is None: no entities have `MineralQuota` → quota sum is 0 → backwards-compatible.
    pub fn conservation_residual(&mut self) -> i64 {
        // P2: energy conservation excludes ONLY the O₂ layer (open quantity, R30-P2) — NOT the mineral
        // layer, which IS conserved (agents hold MineralQuota; see the identity in the doc above). Matches
        // the `initial` baseline in Sim::new. Non-oxygen configs: unchanged (byte-identical).
        let enable_oxygen = self.world.resource::<EconParams>().enable_oxygen;
        let field = self.world.resource::<FieldRes>();
        let mut field_total = field.0.conserved_total_all();
        if enable_oxygen {
            field_total -= field.0.conserved_total(crate::FieldId::Oxygen.as_usize());
        }
        let mut q = self.world.query::<(&Energy, Option<&MineralQuota>)>();
        let agents: i64 = q.iter(&self.world)
            .map(|(e, mq)| e.0 + mq.map(|m| m.0).unwrap_or(0))
            .sum();
        let ledger = *self.world.resource::<EnergyLedger>();
        ledger.residual(field_total, agents)
    }

    /// Read-only per-creature brain snapshot `(entity bits, BrainOutput, BrainState)`, sorted by
    /// entity — for the spawn-contract / multi-rate tests (never feeds the tick). Lets a test assert a
    /// newborn born off-phase is frozen (`h = 0`, `out = 0`) until its first global Brain tick.
    pub fn brain_snapshot(&mut self) -> Vec<(u64, BrainOutput, BrainState)> {
        let mut q = self.world.query::<(Entity, &BrainOutput, &BrainState)>();
        let mut v: Vec<(u64, BrainOutput, BrainState)> =
            q.iter(&self.world).map(|(e, bo, bs)| (e.to_bits(), *bo, *bs)).collect();
        v.sort_unstable_by_key(|x| x.0);
        v
    }

    /// Current population.
    pub fn population(&mut self) -> u64 {
        let mut q = self.world.query::<&Energy>();
        q.iter(&self.world).count() as u64
    }

    /// (min_l1, max_l1) L1 brain-weight distance from a reference weight vector across all living
    /// creatures. Probe/calibration helper — not used in the deterministic tick loop or state hash.
    pub fn weight_l1_stats(&mut self, reference: &[i8; BRAIN_WEIGHTS]) -> (i64, i64) {
        let mut q = self.world.query::<&Genome>();
        let mut min_l1 = i64::MAX;
        let mut max_l1 = 0i64;
        for g in q.iter(&self.world) {
            let l1: i64 = g.weights.iter().zip(reference.iter())
                .map(|(a, b)| (*a as i64 - *b as i64).abs())
                .sum();
            min_l1 = min_l1.min(l1);
            max_l1 = max_l1.max(l1);
        }
        if min_l1 == i64::MAX { (0, 0) } else { (min_l1, max_l1) }
    }

    /// E-4b-i: per-layer count of live entities' `Phenotype.uptake_layer` (index = layer, up to
    /// `n_layers`). Probe/test helper — read-only, not used in the deterministic tick loop or state
    /// hash. The direct per-entity liveness proof: comparing this histogram between a Phase-2 config
    /// and its specs-`None` twin is the authoritative "the chain is live" test (critic F2/F10) — the
    /// conserved golden hash moves only transitively through field sinks and could stay silent.
    pub fn uptake_layer_histogram(&mut self, n_layers: usize) -> Vec<u64> {
        let mut hist = vec![0u64; n_layers.max(1)];
        let mut q = self.world.query::<&Phenotype>();
        for ph in q.iter(&self.world) {
            let l = (ph.uptake_layer as usize).min(hist.len() - 1);
            hist[l] += 1;
        }
        hist
    }

    /// E-5b: cumulative count of REAL, criterion-triggered stillbirths (the size-viability gate —
    /// never `#[cfg(test)]` `force_decode_none` injections). Probe/test helper — read-only, not used
    /// in the deterministic tick loop or state hash. Mirrors [`Sim::uptake_layer_histogram`]'s
    /// telemetry-probe pattern: a stillbirth leaves no entity behind to query, so this is a run-
    /// lifetime counter incremented at the attribution site in `stage_birth_death`, not a snapshot.
    pub fn stillbirth_count(&mut self) -> u64 {
        self.world.resource::<ReproEvents>().stillbirths
    }

    /// P-2a: combat_trait population statistics. Returns (max, count_positive, sum) for computing
    /// mean = sum / population. Golden-NEUTRAL: read-only query, no state mutation.
    pub fn combat_trait_stats(&mut self) -> (i32, u64, i64) {
        let mut max = 0i32;
        let mut count_positive = 0u64;
        let mut sum = 0i64;
        let mut q = self.world.query::<&Genome>();
        for g in q.iter(&self.world) {
            max = max.max(g.combat_trait);
            if g.combat_trait > 0 {
                count_positive += 1;
                sum += g.combat_trait as i64;
            }
        }
        (max, count_positive, sum)
    }

    /// D-2 (#270): multicellular body-size population statistics — `Σ Phenotype.graph.
    /// module_cell_count` per entity. Returns `(max_body_size, count_multicellular)`. Probe/test
    /// helper — read-only, no state mutation, not used in the deterministic tick loop or state hash.
    pub fn body_size_stats(&mut self) -> (i64, u64) {
        let mut max_size = 0i64;
        let mut count_multicellular = 0u64;
        let mut q = self.world.query::<&Phenotype>();
        for ph in q.iter(&self.world) {
            let n: i64 = ph.graph.module_cell_count.iter().map(|&c| c as i64).sum();
            max_size = max_size.max(n);
            if n > 1 {
                count_multicellular += 1;
            }
        }
        (max_size, count_multicellular)
    }

    /// D-3a (#272): independent cross-check probe for `Telemetry::{mean,max}_body_size`/
    /// `multicellular_frac` — a fresh query returning every live entity's `CellGraph::body_size()`
    /// (unordered; `body_size_aggregate` is order-independent). Unlike `body_size_stats` (D-2,
    /// unclamped — built to detect ">1 ever happens", where a non-phase2 empty graph correctly reads
    /// 0), this clamps to match D-3a's telemetry definition (empty graph → body_size 1). Test-only
    /// verification helper: read-only, no state mutation, not used in the deterministic tick loop.
    pub fn body_size_probe(&mut self) -> Vec<i64> {
        let mut q = self.world.query::<&Phenotype>();
        q.iter(&self.world).map(|ph| ph.graph.body_size()).collect()
    }

    pub fn tick(&self) -> u64 {
        self.world.resource::<SimClock>().tick
    }

    /// Telemetry snapshot (samples for Price covariance, population, field total, species census).
    pub fn telemetry(&self) -> &Telemetry {
        self.world.resource::<Telemetry>()
    }

    /// R-1: read-only per-entity render snapshot (RnD 02 R26/R17/R19/R21 — render never enters the
    /// tick). `&self` ONLY — never `&mut`, and no render-side args (selection/viewport are R-3/R-4
    /// render concerns; this returns ALL live creatures, the render side culls/selects later).
    /// Allocates its own owned output and mutates nothing, so the tick trajectory is byte-identical
    /// whether or not this is ever called — pinned by `v2_observe_render_is_golden_neutral` (cli
    /// crate). Uses `iter_entities` + `EntityRef::get` (read-only) rather than `World::query`, which
    /// in bevy_ecs 0.19 requires `&mut World` to build the `QueryState` — the `&self` signature is a
    /// hard acceptance criterion, not a style choice.
    pub fn observe_render(&self) -> RenderSnapshot {
        let mut creatures: Vec<CreatureDot> = self
            .world
            .iter_entities()
            .filter_map(|e| {
                let pos = e.get::<Position>()?;
                let energy = e.get::<Energy>()?;
                let species = e.get::<SpeciesId>()?;
                let phenotype = e.get::<Phenotype>()?;
                let genome = e.get::<Genome>()?;
                let cell_type = phenotype.cell_type;
                // R-4: read size and uptake_layer from the genome/phenotype for LOD rendering.
                let size = genome.size;
                let uptake_layer = phenotype.uptake_layer;
                Some(CreatureDot {
                    id: e.id().to_bits(),
                    pos: pos.0,
                    energy: energy.0,
                    species: species.0,
                    cell_type,
                    size,
                    uptake_layer,
                })
            })
            .collect();
        creatures.sort_unstable_by_key(|c| c.id);
        let tel = self.world.resource::<Telemetry>();
        let tick = self.world.resource::<SimClock>().tick;
        let population = tel.population;

        // T-2: Collect life statistics (golden-NEUTRAL, observational).
        let life = if population > 0 {
            Some(LifeStats {
                tick,
                population,
                avg_energy: self.avg_energy(),
                avg_biomass: self.avg_biomass(),
                species_count: self.species_count(),
                trophic_fractions: self.trophic_fractions(),
            })
        } else {
            None
        };

        RenderSnapshot {
            tick,
            population,
            species_count: tel.species_count,
            creatures,
            life,
        }
    }

    /// Hash of the live species assignment: fold of sorted live SpeciesId values plus the
    /// monotone `next_id` allocator state. Deterministic and integer-only. Included in the
    /// two-run-identical CI check (M5/criterion 4). Must be called AFTER a step that produces
    /// live SpeciesId diversity (i.e., at a tick where species_count > 1 is expected).
    pub fn species_hash(&mut self) -> u64 {
        let mut q = self.world.query::<&SpeciesId>();
        let mut ids: Vec<u32> = q.iter(&self.world).map(|s| s.0).collect();
        ids.sort_unstable();
        let next_id = self.speciation.next_id;
        let mut h = FNV_OFFSET;
        for id in ids {
            h = fnv_mix(h, id as u64);
        }
        fnv_mix(h, next_id as u64)
    }

    /// Read-only access to the speciation state (for CI separation-gate assertions).
    pub fn speciation_state(&self) -> &SpeciationState {
        &self.speciation
    }

    /// Economy parameters (for CI threshold assertions).
    pub fn econ(&self) -> &EconParams {
        self.world.resource::<EconParams>()
    }

    #[cfg(feature = "perf")]
    pub fn perf(&self) -> &PerfReport {
        &self.perf
    }

    /// T-1: Average energy of all living entities. Returns 0 if population is 0.
    /// Read-only query; not fed to state hash or tick. Golden-NEUTRAL.
    pub fn avg_energy(&self) -> i64 {
        let mut sum: i64 = 0;
        let mut count: u64 = 0;
        for e in self.world.iter_entities() {
            if let Some(energy) = e.get::<Energy>() {
                sum += energy.0;
                count += 1;
            }
        }
        if count == 0 { 0 } else { sum / count as i64 }
    }

    /// T-1: Average biomass (body size) of all living entities. Returns 0 if population is 0.
    /// Read-only query; not fed to state hash or tick. Golden-NEUTRAL.
    pub fn avg_biomass(&self) -> i64 {
        let mut sum: i64 = 0;
        let mut count: u64 = 0;
        for e in self.world.iter_entities() {
            if let Some(genome) = e.get::<Genome>() {
                sum += genome.size as i64;
                count += 1;
            }
        }
        if count == 0 { 0 } else { sum / count as i64 }
    }

    /// T-1: Count of distinct species (live organisms with unique SpeciesId).
    /// Read-only query; not fed to state hash or tick. Golden-NEUTRAL.
    pub fn species_count(&self) -> u64 {
        let mut species_set = std::collections::BTreeSet::new();
        for e in self.world.iter_entities() {
            if let Some(species) = e.get::<SpeciesId>() {
                species_set.insert(species.0);
            }
        }
        species_set.len() as u64
    }

    /// T-1: Trophic layer distribution — histogram of uptake_layer indices across all living entities.
    /// Index = uptake_layer, value = count of entities eating from that layer.
    /// Returns a vector sized to accommodate the highest uptake_layer found (or empty if no entities).
    /// Read-only query; not fed to state hash or tick. Golden-NEUTRAL.
    pub fn trophic_fractions(&self) -> Vec<u64> {
        let mut histogram = std::collections::BTreeMap::<i32, u64>::new();
        for e in self.world.iter_entities() {
            if let Some(phenotype) = e.get::<Phenotype>() {
                *histogram.entry(phenotype.uptake_layer).or_insert(0) += 1;
            }
        }
        if histogram.is_empty() {
            vec![]
        } else {
            let max_layer = *histogram.keys().max().unwrap_or(&-1) as usize;
            let mut result = vec![0u64; max_layer + 1];
            for (layer, count) in histogram {
                if layer >= 0 && (layer as usize) < result.len() {
                    result[layer as usize] = count;
                }
            }
            result
        }
    }

    /// Assign final SpeciesId to entities born this tick (marked `PendingSpeciation`).
    /// Runs after all stages so children are fully live in the world. Processes in entity-id
    /// order (matching stage_birth_death's iteration order) for a deterministic next_id sequence.
    fn process_pending_speciation(&mut self) {
        use bevy_ecs::query::With;

        let threshold = self.world.resource::<EconParams>().speciation_threshold;

        // Collect pending (newly born) entities — sort by entity id for determinism.
        let pending: Vec<(Entity, Genome, SpeciesId)> = {
            let mut q = self
                .world
                .query_filtered::<(Entity, &Genome, &SpeciesId), With<PendingSpeciation>>();
            let mut v: Vec<(Entity, Genome, SpeciesId)> =
                q.iter(&self.world).map(|(e, g, s)| (e, g.clone(), *s)).collect();
            v.sort_unstable_by_key(|(e, _, _)| e.to_bits());
            v
        };

        // Determine final SpeciesId for each child — may found a new species.
        let mut updates: Vec<(Entity, SpeciesId)> = Vec::with_capacity(pending.len());
        for (e, genome, parent_species) in pending {
            let parent_ref =
                self.speciation.refs.get(&parent_species).cloned().unwrap_or_else(|| genome.clone());
            let d = genome.brain_weight_l1(&parent_ref);
            let species_c = if d > threshold {
                let new_id = SpeciesId(self.speciation.next_id);
                self.speciation.next_id += 1;
                self.speciation.refs.insert(new_id, genome);
                self.speciation.parent_of.insert(new_id, parent_species);
                new_id
            } else {
                parent_species
            };
            updates.push((e, species_c));
        }

        // Apply: update SpeciesId, remove marker.
        for (e, species_c) in updates {
            let mut em = self.world.entity_mut(e);
            em.insert(species_c);
            em.remove::<PendingSpeciation>();
        }

        // Recompute live species census (reflects extinctions + new births this tick).
        let census: std::collections::BTreeMap<u32, u32> = {
            let mut q = self.world.query::<&SpeciesId>();
            let mut map = std::collections::BTreeMap::new();
            for s in q.iter(&self.world) {
                *map.entry(s.0).or_insert(0) += 1;
            }
            map
        };
        let species_count = census.len() as u64;
        let mut tel = self.world.resource_mut::<Telemetry>();
        tel.species_count = species_count;
        tel.species_census = census.into_iter().collect();
    }
}

/// Build the 11 stages, each its own `Schedule` → explicit fork-join barrier + Commands sync point
/// at every boundary. Serial within a stage at M1 (true intra-stage parallelism is M2).
fn build_stages() -> Vec<(&'static str, Schedule)> {
    use stages::*;
    macro_rules! stage {
        ($name:expr, $sys:expr) => {{
            let mut s = Schedule::default();
            s.add_systems($sys);
            ($name, s)
        }};
    }
    vec![
        stage!("1_sense", stage_sense),
        stage!("2_brain", stage_brain),
        stage!("3_act", stage_act),
        stage!("4_move", stage_move),
        stage!("5_metabolism", stage_metabolism),
        stage!("6_interactions", stage_interactions),
        stage!("6b_mineral_feed", stage_mineral_feed),
        stage!("6c_predation", stage_predation),
        stage!("6d_settling", stage_settling),
        stage!("7_birth_death", stage_birth_death),
        stage!("8_field_scatter", stage_field_scatter),
        stage!("9_observe", stage_observe),
        stage!("10_swap", stage_swap),
    ]
}

// ── E-1 decode-gate integration test ──────────────────────────────────────────────────────────────
// Stub backends live here so sim-core has no dev-dep on cli/fields/world (they all dep on sim-core).
#[cfg(test)]
mod e1_gate_tests {
    use super::*;
    use crate::traits::{
        Brain, Deposit, FieldStore, MergeStrategy, WorldView,
        BRAIN_HIDDEN, BRAIN_INPUTS, BRAIN_OUTPUTS, BRAIN_WEIGHTS,
    };
    use crate::params::{EconParams, LayerSpec, SimConfig};

    const WORLD_DIM: i64 = 64;
    const N_CELLS: usize = (WORLD_DIM * WORLD_DIM) as usize;
    const SEED_E1: u64 = 0x_E1_5EED;

    // ── Minimal stub WorldView — no solid terrain, uniform resource reading. ─────────────────
    struct StubWorld;
    impl WorldView for StubWorld {
        fn is_solid(&self, _p: Vec2Fixed) -> bool { false }
        fn height(&self, _x: i64, _z: i64) -> i64 { 0 }
        fn biome(&self, _p: Vec2Fixed) -> u8 { 0 }
        fn resource(&self, _p: Vec2Fixed) -> i64 { 100 }
        fn temp_at(&self, _p: Vec2Fixed) -> i32 { 1500 } // P3-1: stub returns mesophile (15°C)
    }

    // ── Minimal stub Brain — outputs zeros (entities stay put). ─────────────────────────────
    struct StubBrain;
    impl Brain for StubBrain {
        fn infer(
            &self, _in: &[i16; BRAIN_INPUTS], _hold: &[i16; BRAIN_HIDDEN],
            _w: &[i8; BRAIN_WEIGHTS], hnew: &mut [i16; BRAIN_HIDDEN], out: &mut [i16; BRAIN_OUTPUTS],
        ) {
            hnew.iter_mut().for_each(|x| *x = 0);
            out.iter_mut().for_each(|x| *x = 0);
        }
    }

    // ── Minimal stub FieldStore — two layers, all cells start at `initial` eu. ──────────────
    // Conservation is NOT tracked (deposit_conserved is a no-op) — this is intentional: the
    // test checks entity count, not ledger residuals. Energy conservation is tested elsewhere.
    struct StubField {
        layers: Vec<Vec<i64>>, // [layer][cell]
    }
    impl StubField {
        fn new(n_layers: usize, initial: i64) -> Self {
            Self { layers: vec![vec![initial; N_CELLS]; n_layers] }
        }
        fn cell(&self, pos: Vec2Fixed) -> usize {
            let x = (pos.0.rem_euclid(WORLD_DIM)) as usize;
            let z = (pos.1.rem_euclid(WORLD_DIM)) as usize;
            x + z * WORLD_DIM as usize
        }
    }
    impl FieldStore for StubField {
        fn m_field(&self) -> i64 { 1 }
        fn cell_index(&self, pos: Vec2Fixed) -> usize { self.cell(pos) }
        fn cell_morton(&self, _p: Vec2Fixed) -> u32 { 0 }
        fn check_meta(&self, m: i64) -> Result<(), String> {
            if m == 1 { Ok(()) } else { Err(format!("expected m=1, got {m}")) }
        }
        fn conserved_at(&self, pos: Vec2Fixed, layer: usize) -> i64 {
            let c = self.cell(pos);
            self.layers.get(layer).and_then(|l| l.get(c)).copied().unwrap_or(0)
        }
        fn conserved_gradient(&self, _p: Vec2Fixed, _r: i64, _l: usize) -> (i64, i64) { (0, 0) }
        fn conserved_take(&mut self, pos: Vec2Fixed, amount: i64, layer: usize) -> i64 {
            let c = self.cell(pos);
            if let Some(l) = self.layers.get_mut(layer) {
                if let Some(v) = l.get_mut(c) {
                    let taken = (*v).min(amount);
                    *v -= taken;
                    return taken;
                }
            }
            0
        }
        fn deposit_conserved(&mut self, _c: usize, _a: i64, _l: usize) {}
        fn conserved_total(&self, layer: usize) -> i64 {
            self.layers.get(layer).map(|l| l.iter().sum()).unwrap_or(0)
        }
        fn conserved_total_all(&self) -> i64 { self.layers.iter().flat_map(|l| l.iter()).sum() }
        fn conserved_hash(&self) -> u64 { 0 }
        fn signal_total(&self) -> f32 { 0.0 }
        fn signal_hash(&self) -> u64 { 0 }
        fn signal_all_finite(&self) -> bool { true }
        fn commit_merge(&mut self, _b: &[Vec<Deposit>], _s: MergeStrategy) {}
        fn solve(&mut self) -> i64 { 0 }
    }

    /// Build a minimal Sim where founders reproduce on tick 1:
    /// `founder_energy=2000 > e_cell+c_div=1100`, `repro_threshold=1000`.
    fn make_quick_repro_sim(seed: u64, n_founders: u64) -> Sim {
        let config = SimConfig {
            seed,
            n_founders,
            // 2000 >> e_cell+c_div=1100 and >> genome.repro_threshold=1500 →
            // after tick-0 metabolism (~12 eu cost), founders still have 1988 eu ≥ 1500 → reproduce.
            founder_energy: 2000,
            // n_layers=2 must equal econ.n_layers (debug_assert in Sim::new).
            n_layers: 2,
            econ: EconParams {
                n_layers: 2,      // explicit — must match SimConfig::n_layers
                n_energy_layers: 2,
                ..EconParams::default()
            },
            sim_threads: 1,
            merge_strategy: MergeStrategy::Canonical,
            layer_specs: [
                LayerSpec { regen_rate: 6, flux_alpha_num: 1, flux_alpha_den: 8,
                            flat_cap: 0, world_cap_mult: 0 },
                LayerSpec { regen_rate: 0, flux_alpha_num: 1, flux_alpha_den: 4,
                            flat_cap: 0, world_cap_mult: 0 },
                LayerSpec::default(),
                LayerSpec::default(),
            ],
            thermal_verdict_temps: None,
        };
        Sim::new(config, Box::new(StubWorld), Box::new(StubField::new(2, 100_000)), Box::new(StubBrain))
    }

    /// Control: normal sim (force_decode_none=false everywhere) grows after 1 tick.
    /// Proves the test config actually triggers reproduction — the test is not vacuous.
    #[test]
    fn control_quick_repro_grows() {
        let mut sim = make_quick_repro_sim(SEED_E1, 8);
        let initial = sim.population();
        sim.step();
        let after = sim.population();
        assert!(
            after > initial,
            "control: population must grow when decode() returns Some; initial={initial} after={after}"
        );
    }

    /// E-1 None-gate end-to-end: when `force_decode_none=true` on all founder genomes,
    /// `child_genome.decode()` in `stage_birth_death` returns `None`, the `let Some(...) else
    /// { continue; }` gate fires at BOTH spawn sites, and no entity is ever materialized.
    /// Population can only decrease (deaths) or stay at initial — never grow.
    ///
    /// This exercises THE REAL BirthDeath code path (not a separate wrapper function):
    /// - `stage_birth_death` calls `child_genome.decode()` (the production function, unchanged)
    /// - `force_decode_none=true` makes that same `decode()` return `None`
    /// - `mutate()` copies `*self` → children inherit the flag → lineage stays stillborn
    #[test]
    fn e1_none_gate_suppresses_births_end_to_end() {
        let mut sim = make_quick_repro_sim(SEED_E1, 8);
        let initial = sim.population();

        // Poison all founders: force_decode_none propagates to children via mutate()'s `*self` copy.
        // Direct world access is safe here (same module as Sim).
        {
            let mut q = sim.world.query::<&mut genome::Genome>();
            for mut g in q.iter_mut(&mut sim.world) {
                g.force_decode_none = true;
            }
        }

        sim.step();
        let after = sim.population();

        assert!(
            after <= initial,
            "E-1 None-gate: population must NOT grow when decode() returns None for all children; \
             initial={initial} after={after} — births occurred despite gate → gate not wired at \
             one or both child spawn sites (stages.rs mineral:{} / non-mineral:{})",
            "stages.rs:~642", "stages.rs:~660"
        );
    }

    // ── E-5a: stillbirth conservation fix ────────────────────────────────────────────────────────
    //
    // `d0_scaled: 0` in BOTH configs below: `StubField::deposit_conserved` is a NO-OP (documented
    // above — the stub was built for entity-count checks, not ledger residuals), so a background-
    // death (C-1) recycle deposit on the SAME tick would silently vanish from the field and falsely
    // trip the residual-0 assertion below with an unrelated failure. Disabling d0 isolates the
    // conservation math to exactly the stillbirth path under test.

    /// Non-mineral variant of `make_quick_repro_sim`, with background death OFF (see above).
    fn make_quick_repro_sim_no_d0(seed: u64, n_founders: u64) -> Sim {
        let config = SimConfig {
            seed,
            n_founders,
            founder_energy: 2000,
            n_layers: 2,
            econ: EconParams {
                n_layers: 2, n_energy_layers: 2, d0_scaled: 0,
                excrete: 0, // StubField::deposit_conserved is a no-op — excretion would silently
                // vanish from the field, leaking the residual independent of the stillbirth path.
                ..EconParams::default()
            },
            sim_threads: 1,
            merge_strategy: MergeStrategy::Canonical,
            layer_specs: [
                LayerSpec { regen_rate: 6, flux_alpha_num: 1, flux_alpha_den: 8, flat_cap: 0, world_cap_mult: 0 },
                LayerSpec { regen_rate: 0, flux_alpha_num: 1, flux_alpha_den: 4, flat_cap: 0, world_cap_mult: 0 },
                LayerSpec::default(),
                LayerSpec::default(),
            ],
            thermal_verdict_temps: None,
        };
        Sim::new(config, Box::new(StubWorld), Box::new(StubField::new(2, 100_000)), Box::new(StubBrain))
    }

    /// Mineral-active variant (critic F5/F6): `mineral_layer: Some(2)`, `n_layers: 3`, background
    /// death OFF (see above), and **`q_mineral: 0`** — clears the Liebig AND-gate (`stages.rs:589`)
    /// so a founder spawned with the production default `MineralQuota(0)` (`lib.rs` founder spawn,
    /// `has_mineral` branch) still satisfies `quota_ready = q_val >= q_mineral = 0 >= 0`. Without
    /// this the division block is skipped entirely BEFORE the stillbirth gate is ever reached and
    /// the test would pass green while exercising nothing (critic F6).
    fn make_quick_repro_sim_mineral(seed: u64, n_founders: u64) -> Sim {
        let config = SimConfig {
            seed,
            n_founders,
            founder_energy: 2000,
            n_layers: 3,
            econ: EconParams {
                n_layers: 3,
                n_energy_layers: 2, // mineral (layer 2) excluded from energy-uptake targeting
                mineral_layer: Some(2),
                q_mineral: 0, // clears the Liebig gate for MineralQuota(0) founders
                d0_scaled: 0,
                excrete: 0, // see make_quick_repro_sim_no_d0 — StubField's deposit_conserved no-op
                ..EconParams::default()
            },
            sim_threads: 1,
            merge_strategy: MergeStrategy::Canonical,
            layer_specs: [
                LayerSpec { regen_rate: 6, flux_alpha_num: 1, flux_alpha_den: 8, flat_cap: 0, world_cap_mult: 0 },
                LayerSpec { regen_rate: 0, flux_alpha_num: 1, flux_alpha_den: 4, flat_cap: 0, world_cap_mult: 0 },
                LayerSpec { regen_rate: 0, flux_alpha_num: 1, flux_alpha_den: 4, flat_cap: 0, world_cap_mult: 0 },
                LayerSpec::default(),
            ],
            thermal_verdict_temps: None,
        };
        Sim::new(config, Box::new(StubWorld), Box::new(StubField::new(3, 100_000)), Box::new(StubBrain))
    }

    /// Anti-vacuity control (mirrors `control_quick_repro_grows`): the mineral config, with the
    /// Liebig gate cleared and `force_decode_none=false`, actually grows the population — proving
    /// the gate-clearing itself is not what silently prevented division (critic F6's own concern
    /// applied to the negative/control direction too).
    #[test]
    fn control_quick_repro_grows_mineral() {
        let mut sim = make_quick_repro_sim_mineral(SEED_E1, 8);
        let initial = sim.population();
        sim.step();
        let after = sim.population();
        assert!(
            after > initial,
            "mineral control: population must grow (Liebig gate cleared, decode()=Some); \
             initial={initial} after={after}"
        );
    }

    /// The E-5a fix, non-mineral: on a `force_decode_none`-injected stillbirth, (1) conservation
    /// residual stays EXACTLY 0, (2) population does not grow (no child materialized), and (3) no
    /// offspring flag is set (`ReproEvents.parents` stays empty — `born_total` would not inflate).
    #[test]
    fn stillbirth_conserves_energy_and_sets_no_offspring_flag() {
        let mut sim = make_quick_repro_sim_no_d0(SEED_E1, 8);
        let initial = sim.population();

        {
            let mut q = sim.world.query::<&mut genome::Genome>();
            for mut g in q.iter_mut(&mut sim.world) {
                g.force_decode_none = true;
            }
        }

        sim.step();

        let after = sim.population();
        assert_eq!(after, initial, "stillbirth: population must be UNCHANGED (no death enabled, no child spawned)");

        let residual = sim.conservation_residual();
        assert_eq!(residual, 0, "stillbirth: conservation residual must be EXACTLY 0, got {residual}");

        let repro = sim.world.resource::<ReproEvents>();
        assert!(
            repro.parents.is_empty(),
            "stillbirth: ReproEvents.parents must be EMPTY (no offspring flag on a miscarried division) — \
             got {} entries; born_total would be inflated",
            repro.parents.len()
        );
    }

    /// The E-5a fix, mineral-active (critic F3): same three assertions, with the Liebig gate cleared
    /// so the division block — and therefore the stillbirth branch — is genuinely reached, exercising
    /// the `q_mineral` debit/dissipate path that runs BEFORE the decode gate.
    #[test]
    fn stillbirth_conserves_energy_mineral_active() {
        let mut sim = make_quick_repro_sim_mineral(SEED_E1, 8);
        let initial = sim.population();

        {
            let mut q = sim.world.query::<&mut genome::Genome>();
            for mut g in q.iter_mut(&mut sim.world) {
                g.force_decode_none = true;
            }
        }

        sim.step();

        let after = sim.population();
        assert_eq!(after, initial, "mineral stillbirth: population must be UNCHANGED");

        let residual = sim.conservation_residual();
        assert_eq!(residual, 0, "mineral stillbirth: conservation residual must be EXACTLY 0, got {residual}");

        let repro = sim.world.resource::<ReproEvents>();
        assert!(
            repro.parents.is_empty(),
            "mineral stillbirth: ReproEvents.parents must be EMPTY — got {} entries",
            repro.parents.len()
        );
    }

    // ── E-5b: REAL (non-injected) criterion-triggered stillbirth ────────────────────────────────
    //
    // No new conservation code here (per the issue's explicit "out of scope") — this reuses the
    // EXACT E-5a booking/ordering, just reaches it via the real `size`-viability gate instead of
    // `force_decode_none`. `phase2_shaped_econ` mirrors `cli::phase2_config`'s morphogen/grn
    // fixtures (same values) so the chain is prod-shaped, not a stand-in.
    //
    // `n_founders=1` + directly setting that founder's `size` to `SIZE_VIABILITY_FLOOR` (the exact
    // boundary — a real production field, no `#[cfg(test)]` flag needed) makes its first division
    // attempt deterministically likely to produce a real stillbirth: mutation fires only ~12.5% of
    // the time, so the child's `size` stays at the floor (inviable) unless that rare draw both
    // fires AND rolls +1. A single founder keeps the tick's ReproEvents/population trace
    // attributable to exactly one lineage — the same clean-isolation shape as `make_quick_repro_sim`.

    fn phase2_shaped_econ() -> EconParams {
        use crate::{Boundary, GrnSpec, MorphogenSpec};
        let mspec = MorphogenSpec {
            g_dev: 4, n_dev: 8, boundary: Boundary::Reflecting,
            diffuse_shift: 3, decay_num: 1, decay_shift: 4, seed_scale: 4096, stop_threshold: 0,
            apoptosis_threshold: None,
            germ_threshold: None,
            supply_source: None,
            adhesion_threshold: None,
        };
        let gspec = GrnSpec::new(2, vec![64, -64, -64, 64], vec![0, 0], vec![0, 0], 3, 12, 0, 0, vec![256, 0]);
        EconParams {
            n_layers: 2, n_energy_layers: 2, d0_scaled: 0,
            excrete: 0, // see make_quick_repro_sim_no_d0 — StubField's deposit_conserved no-op
            morphogen: Some(mspec), grn: Some(gspec),
            ..EconParams::default()
        }
    }

    fn make_phase2_shaped_sim_no_d0(seed: u64, n_founders: u64) -> Sim {
        let config = SimConfig {
            seed, n_founders, founder_energy: 2000, n_layers: 2,
            econ: phase2_shaped_econ(),
            sim_threads: 1,
            merge_strategy: MergeStrategy::Canonical,
            layer_specs: [
                LayerSpec { regen_rate: 6, flux_alpha_num: 1, flux_alpha_den: 8, flat_cap: 0, world_cap_mult: 0 },
                LayerSpec { regen_rate: 0, flux_alpha_num: 1, flux_alpha_den: 4, flat_cap: 0, world_cap_mult: 0 },
                LayerSpec::default(),
                LayerSpec::default(),
            ],
            thermal_verdict_temps: None,
        };
        Sim::new(config, Box::new(StubWorld), Box::new(StubField::new(2, 100_000)), Box::new(StubBrain))
    }

    /// Control: the phase2-shaped chain, with founder `size` left at its default (4, viable) — the
    /// division block is reached and succeeds. Proves the boundary-poisoning test below isn't
    /// vacuous (the config itself divides fine absent the size-floor poison).
    #[test]
    fn control_phase2_shaped_sim_grows_at_default_size() {
        let mut sim = make_phase2_shaped_sim_no_d0(SEED_E1, 1);
        let initial = sim.population();
        sim.step();
        let after = sim.population();
        assert!(after > initial, "control: phase2-shaped sim must grow at default founder size; initial={initial} after={after}");
    }

    /// The direct proof (critic's "no new conservation code" + "real, non-injected" requirements):
    /// a REAL size-criterion stillbirth — reached via `Genome::decode`'s `(Some, Some)` chain arm,
    /// not `force_decode_none` — conserves energy exactly, leaves the population unchanged by that
    /// birth, sets NO offspring flag, AND is attributed by `ReproEvents.stillbirths` (not left as an
    /// unattributed `None`).
    #[test]
    fn real_criterion_stillbirth_conserves_energy_and_sets_no_offspring_flag() {
        let mut sim = make_phase2_shaped_sim_no_d0(SEED_E1, 1);
        {
            let mut q = sim.world.query::<&mut genome::Genome>();
            for mut g in q.iter_mut(&mut sim.world) {
                g.size = genome::SIZE_VIABILITY_FLOOR;
            }
        }
        let initial = sim.population();
        let stillbirths_before = sim.stillbirth_count();

        sim.step();

        let after = sim.population();
        assert_eq!(after, initial, "real stillbirth: population must be UNCHANGED (no death enabled, no child spawned)");

        let stillbirths_after = sim.stillbirth_count();
        assert_eq!(
            stillbirths_after, stillbirths_before + 1,
            "real stillbirth: ReproEvents.stillbirths must have incremented by exactly 1 \
             (test setup drifted from calibration — the boundary-size founder did not miscarry as expected)"
        );

        let residual = sim.conservation_residual();
        assert_eq!(residual, 0, "real stillbirth: conservation residual must be EXACTLY 0, got {residual}");

        let repro = sim.world.resource::<ReproEvents>();
        assert!(
            repro.parents.is_empty(),
            "real stillbirth: ReproEvents.parents must be EMPTY (no offspring flag on a miscarried \
             division) — got {} entries; born_total would be inflated",
            repro.parents.len()
        );
    }

    // ── T-1: Aggregate metrics (telemetry foundation) ──────────────────────────────────────────────

    /// T-1: Determinism check — avg_energy, avg_biomass, species_count, trophic_fractions
    /// must be identical on the same seed across 300 ticks (1 thread vs N threads produce the same
    /// metric values). Golden-NEUTRAL: the methods are read-only and do not feed the tick.
    #[test]
    fn t1_metrics_deterministic_1v1() {
        let seed = 0xDEAD_BEEF_u64;
        let mut sim1 = make_quick_repro_sim(seed, 4);
        let mut sim2 = make_quick_repro_sim(seed, 4);

        for _tick in 0..100 {
            sim1.step();
            sim2.step();

            // Check metrics are identical
            let e1 = sim1.avg_energy();
            let e2 = sim2.avg_energy();
            assert_eq!(e1, e2, "avg_energy mismatch at tick {_tick}: {e1} vs {e2}");

            let b1 = sim1.avg_biomass();
            let b2 = sim2.avg_biomass();
            assert_eq!(b1, b2, "avg_biomass mismatch at tick {_tick}: {b1} vs {b2}");

            let s1 = sim1.species_count();
            let s2 = sim2.species_count();
            assert_eq!(s1, s2, "species_count mismatch at tick {_tick}: {s1} vs {s2}");

            let t1 = sim1.trophic_fractions();
            let t2 = sim2.trophic_fractions();
            assert_eq!(t1, t2, "trophic_fractions mismatch at tick {_tick}: {t1:?} vs {t2:?}");
        }
    }

    /// T-1: Read-only check — calling avg_energy, avg_biomass, species_count, trophic_fractions
    /// must NOT change the state hash. Proves the methods are pure read-only queries.
    #[test]
    fn t1_metrics_readonly() {
        let mut sim = make_quick_repro_sim(0xCAFE_BABE_u64, 4);
        sim.step();
        sim.step();

        // Snapshot state hash before metrics
        let hash_before = sim.state_hash();

        // Call all metrics
        let _e = sim.avg_energy();
        let _b = sim.avg_biomass();
        let _s = sim.species_count();
        let _t = sim.trophic_fractions();

        // State hash must be unchanged
        let hash_after = sim.state_hash();
        assert_eq!(
            hash_before, hash_after,
            "metrics must be read-only: state_hash changed from {hash_before:016x} to {hash_after:016x}"
        );
    }

    /// T-1: Sanity check — avg_energy on a controlled population must match manual calculation.
    /// Create a minimal scenario where we know exact energy values, then verify avg_energy agrees.
    #[test]
    fn t1_avg_energy_sane() {
        let mut sim = make_quick_repro_sim(0x1234_5678_u64, 2);

        // At spawn, founders have known energy (2000 eu per founder_energy).
        // Let them run one tick (metabolism will deduct ~12 eu each).
        sim.step();

        let avg = sim.avg_energy();
        let pop = sim.population();

        // Collect all energies for manual verification
        let mut q = sim.world.query::<&Energy>();
        let mut energies: Vec<i64> = q.iter(&sim.world).map(|e| e.0).collect();
        energies.sort();

        let manual_sum: i64 = energies.iter().sum();
        let manual_avg = if pop > 0 { manual_sum / pop as i64 } else { 0 };

        assert_eq!(
            avg, manual_avg,
            "avg_energy sanity: computed {avg} but manual calculation gives {manual_avg} \
             (sum={manual_sum}, population={pop})"
        );

        // Also verify that avg falls in a reasonable range: not 0 (entities live) and not huge
        assert!(avg > 0, "avg_energy sanity: should be positive, got {avg}");
        assert!(
            avg < 2000,
            "avg_energy sanity: should be < 2000 eu (started at 2000, metabolism cost paid), got {avg}"
        );
    }

    /// T-1: trophic_fractions histogram correctly maps uptake_layer distribution.
    /// Controlled test on a known population shape.
    #[test]
    fn t1_trophic_fractions_histogram() {
        let mut sim = make_quick_repro_sim(0x7777_7777_u64, 3);
        sim.step();
        sim.step();

        let pop = sim.population();
        assert!(pop > 0, "test population must be > 0");

        let trophic = sim.trophic_fractions();

        // Histogram must not be empty if population > 0
        assert!(
            !trophic.is_empty(),
            "trophic_fractions must not be empty when population={pop}"
        );

        // Sum of histogram counts must equal population
        let total: u64 = trophic.iter().sum();
        assert_eq!(
            total, pop,
            "trophic_fractions histogram sum must equal population: sum={total}, pop={pop}"
        );

        // Manual verification: count Phenotype.uptake_layer myself and compare
        let mut q = sim.world.query::<&Phenotype>();
        let mut manual_hist: std::collections::BTreeMap<i32, u64> = std::collections::BTreeMap::new();
        for ph in q.iter(&sim.world) {
            *manual_hist.entry(ph.uptake_layer).or_insert(0) += 1;
        }

        // Verify that trophic histogram entries match manual counts
        for (layer, count) in manual_hist.iter() {
            if *layer >= 0 && (*layer as usize) < trophic.len() {
                assert_eq!(
                    trophic[*layer as usize], *count,
                    "trophic_fractions layer {layer}: expected {count}, got {}",
                    trophic[*layer as usize]
                );
            }
        }
    }

    #[test]
    fn t2_lifestats_matches_metrics() {
        let mut sim = make_quick_repro_sim(0x1234_5678_u64, 3);
        sim.step();
        sim.step();

        let pop = sim.population();
        assert!(pop > 0, "test population must be > 0");

        // Collect metrics directly
        let avg_e = sim.avg_energy();
        let avg_b = sim.avg_biomass();
        let sp_count = sim.species_count();
        let trophic = sim.trophic_fractions();

        // Get snapshot and verify life stats match
        let snapshot = sim.observe_render();
        assert!(snapshot.life.is_some(), "life stats must be present when population > 0");

        let life = snapshot.life.unwrap();
        assert_eq!(life.population, pop as i64, "life.population must match sim.population()");
        assert_eq!(life.avg_energy, avg_e, "life.avg_energy must match sim.avg_energy()");
        assert_eq!(life.avg_biomass, avg_b, "life.avg_biomass must match sim.avg_biomass()");
        assert_eq!(life.species_count, sp_count, "life.species_count must match sim.species_count()");
        assert_eq!(life.trophic_fractions, trophic, "life.trophic_fractions must match sim.trophic_fractions()");
    }

    #[test]
    fn t2_lifestats_deterministic() {
        // Same seed should produce identical LifeStats across runs.
        let mut sim1 = make_quick_repro_sim(0xABCD_1234_u64, 3);
        let mut sim2 = make_quick_repro_sim(0xABCD_1234_u64, 3);

        for _ in 0..5 {
            sim1.step();
            sim2.step();
        }

        let snap1 = sim1.observe_render();
        let snap2 = sim2.observe_render();

        assert_eq!(snap1.tick, snap2.tick, "ticks must match");
        assert_eq!(snap1.population, snap2.population, "populations must match");

        match (&snap1.life, &snap2.life) {
            (Some(l1), Some(l2)) => {
                assert_eq!(l1.tick, l2.tick, "life.tick must match");
                assert_eq!(l1.population, l2.population, "life.population must match");
                assert_eq!(l1.avg_energy, l2.avg_energy, "life.avg_energy must be deterministic");
                assert_eq!(l1.avg_biomass, l2.avg_biomass, "life.avg_biomass must be deterministic");
                assert_eq!(l1.species_count, l2.species_count, "life.species_count must be deterministic");
                assert_eq!(l1.trophic_fractions, l2.trophic_fractions, "life.trophic_fractions must be deterministic");
            }
            _ => panic!("both snapshots must have life stats when population > 0"),
        }
    }

    #[test]
    fn t2_lifestats_empty_pop() {
        let mut sim = make_quick_repro_sim(0x9876_5432_u64, 0); // 0 founders
        sim.step();
        sim.step();

        let snapshot = sim.observe_render();
        assert_eq!(snapshot.population, 0, "population must be 0");
        assert!(snapshot.life.is_none(), "life stats must be None when population = 0");
    }

    #[test]
    fn t2_observe_render_readonly() {
        let mut sim = make_quick_repro_sim(0xFEDC_BA98_u64, 3);
        sim.step();

        // Capture state before observe_render
        let pop_before = sim.population();
        let tick_before = sim.tick();

        // Call observe_render (it is &self, not &mut self)
        let snapshot = sim.observe_render();

        // Verify that observe_render did not mutate the sim
        assert_eq!(sim.population(), pop_before, "observe_render must not mutate population");
        assert_eq!(sim.tick(), tick_before, "observe_render must not mutate tick");
        assert_eq!(snapshot.tick, tick_before, "snapshot.tick must match sim.tick()");
        assert_eq!(snapshot.population, pop_before as i64, "snapshot.population must match sim.population()");
    }
}
