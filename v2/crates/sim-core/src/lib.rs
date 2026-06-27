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
mod hash;
mod input;
mod params;
mod pool;
mod rng;
mod stages;
mod traits;

pub use components::{
    BrainOutput, BrainState, Energy, Intent, Sensors, SpeciesId, Velocity, VelocityNext,
};
pub use det_map::DetMap;
pub use energy::EnergyLedger;
pub use genome::{isqrt, size_pow_three_quarters, Genome};
pub use grid::{morton2, NeighborGrid};
pub use hash::{deterministic_fold, fnv_mix, FNV_OFFSET};
pub use input::{sort_tick_events, InputEvent, InputKind};
pub use params::{EconParams, SimConfig};
pub use pool::{ScatterParams, SimPool};
pub use rng::{seed_fold, splitmix64};
pub use traits::{
    brain_w_hh, brain_w_ho, brain_w_ih, Brain, BrainRes, Deposit, FieldRes, FieldStore,
    MergeStrategy, WorldRes, WorldView, BRAIN_HIDDEN, BRAIN_INPUTS, BRAIN_OUTPUTS, BRAIN_SHIFT,
    BRAIN_WEIGHTS,
};

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::Schedule;

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
#[derive(Resource, Default)]
pub struct ReproEvents {
    pub parents: std::collections::BTreeSet<u64>,
}

/// One organism's traits + offspring-this-tick, snapshotted by Observe for the telemetry crate.
#[derive(Clone, Copy, Debug, Default)]
pub struct TraitSample {
    pub traits: [i32; 6],
    pub offspring: u32,
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

        let mut w = World::new();
        w.insert_resource(SimClock { seed: config.seed, tick: 0 });
        w.insert_resource(InputLog::default());
        w.insert_resource(Telemetry::default());
        w.insert_resource(econ);
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

        let founder = Genome::founder();
        for i in 0..config.n_founders {
            // Deterministic scatter across the domain (co-prime strides → spread, no float).
            let x = ((i.wrapping_mul(7).wrapping_add(3)) % econ.world_dim as u64) as i64;
            let z = ((i.wrapping_mul(13).wrapping_add(5)) % econ.world_dim as u64) as i64;
            let p = Vec2Fixed(x, z);
            w.spawn((
                Position(p),
                PositionNext(p),
                Velocity::default(),
                VelocityNext::default(),
                Energy(config.founder_energy),
                founder,
                SpeciesId(0),
                Sensors::default(),
                Intent::default(),
                // Spawn contract (D-Brain-2a): brain buffers start zeroed — same path as a newborn.
                BrainState::zeroed(),
                BrainOutput::zeroed(),
            ));
        }

        let field_total = field.conserved_total();
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

        Self {
            world: w,
            stages: build_stages(),
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
        self.world.resource_mut::<SimClock>().tick += 1;
    }

    /// Canonical golden state hash (R19, arch-bound → arm64): folds Position + Energy + Genome (incl.
    /// the evolved brain weights) + the recurrent `BrainState` (`h_old`/`h_new`) + the motor
    /// `BrainOutput` + the current `Velocity` per entity (via the single [`deterministic_fold`] point)
    /// plus the signal field (f32 bits). Folding `Velocity` closes the M1/F6 gap: two states differing
    /// only in velocity now hash differently. The conserved field is NOT here — it has its own
    /// [`Sim::conserved_field_hash`] for R14.
    pub fn state_hash(&mut self) -> u64 {
        let mut q = self
            .world
            .query::<(Entity, &Position, &Energy, &Genome, &BrainState, &BrainOutput, &Velocity)>();
        let items: Vec<(Entity, u64)> = q
            .iter(&self.world)
            .map(|(e, p, en, g, bs, bo, v)| {
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

    /// Total signal concentration (serial reduction).
    pub fn signal_total(&self) -> f32 {
        self.world.resource::<FieldRes>().0.signal_total()
    }

    /// NaN/Inf guard on the signal field — every cell finite (always-on in the release harness).
    pub fn signal_finite(&self) -> bool {
        self.world.resource::<FieldRes>().0.signal_all_finite()
    }

    /// Energy-conservation residual (R15) — MUST be 0 every tick. Sums live conserved field + agent
    /// energy and the ledger buckets. (The signal field is float, NOT in the balance.)
    pub fn conservation_residual(&mut self) -> i64 {
        let field_total = self.world.resource::<FieldRes>().0.conserved_total();
        let mut q = self.world.query::<&Energy>();
        let agents: i64 = q.iter(&self.world).map(|e| e.0).sum();
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

    pub fn tick(&self) -> u64 {
        self.world.resource::<SimClock>().tick
    }

    /// Telemetry snapshot (samples for Price covariance, population, field total).
    pub fn telemetry(&self) -> &Telemetry {
        self.world.resource::<Telemetry>()
    }

    #[cfg(feature = "perf")]
    pub fn perf(&self) -> &PerfReport {
        &self.perf
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
        stage!("7_birth_death", stage_birth_death),
        stage!("8_field_scatter", stage_field_scatter),
        stage!("9_observe", stage_observe),
        stage!("10_swap", stage_swap),
    ]
}
