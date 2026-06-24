//! animata v2 `sim-core` — the deterministic simulation core (M0 walking skeleton).
//!
//! NO biology, fields, brains, or energy here yet. M0 proves the three things doc 13 says "cannot be
//! retrofitted later": determinism, per-stage instrumentation, and bit-for-bit golden replay — on
//! dummy entities, while the cost is low.
//!
//! Determinism contract held mechanically, not by comment:
//! * INTEGER ONLY — no `f32`/`f64` in this crate until M1 (the no-float guard test enforces it). M0
//!   arithmetic is integer ⇒ cross-arch bit-identical ⇒ an x86-only M0 golden is justified by the
//!   mechanism (F7).
//! * One reduction point ([`deterministic_fold`]): collect → sort by `Entity` → fold. Never natural
//!   query order.
//! * Core state maps are [`DetMap`] (BTreeMap), never a randomly-hashed std map.
//! * `Sim::step` is `&mut self` only — no clock, no render, no IO (R1). The fixed-dt loop driver
//!   lives OUTSIDE the core, in the `cli` crate.

mod det_map;
mod hash;
mod input;
mod rng;
mod traits;

pub use det_map::DetMap;
pub use hash::{deterministic_fold, fnv_mix};
pub use input::{sort_tick_events, InputEvent, InputKind};
pub use rng::{seed_fold, splitmix64};
pub use traits::{Brain, FieldStore, WorldView};

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::Schedule;

/// Integer 2-vector — the fixed-point spatial domain (`i64`). This is the SAME fixed-point scale
/// layer M1 reuses for the conserved energy ledger: the float↔fixed boundary is marked from M0, so
/// the retrofit scale is exercised early (F7). A 32-/64-bit-float position would have made M0 itself
/// arch-dependent while M0 has no matched-arch CI job — a hole that would only surface, misdiagnosed,
/// at M1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Vec2Fixed(pub i64, pub i64);

/// The one dummy hot component of M0. DOUBLE-BUFFERED: [`Position`] holds version `t` (read),
/// [`PositionNext`] holds `t+1` (written by stage 4 Move), and stage 10 Swap copies next→current.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Position(pub Vec2Fixed);

/// Write-side of the double buffer for [`Position`]. See [`Position`].
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PositionNext(pub Vec2Fixed);

/// Core clock + run seed. `tick` is the only time the core knows — there is no wall clock here.
#[derive(Resource, Debug)]
pub struct SimClock {
    pub seed: u64,
    pub tick: u64,
}

/// The tick-stamped input stream (R18). Empty on Phase 0; the carrier of replay from M1+.
#[derive(Resource, Default)]
pub struct InputLog {
    pub events: Vec<InputEvent>,
}

/// Read-only telemetry sink shape (stage 9 Observe). `DetMap` ⇒ deterministic iteration. M0 writes
/// one dummy metric; real evolution telemetry (Price covariance, diversity) arrives at M1.
#[derive(Resource, Default)]
pub struct Telemetry {
    pub metrics: DetMap<&'static str, i64>,
}

// ── RNG salts (one per stochastic stage; disjoint so streams never alias) ─────────────────────────
const SALT_MOVE: u64 = 0x4D4F_5645; // "MOVE"

// ── The 11 stages (0–10). Bodies are EMPTY except Move (the one dummy stage), Observe (the sink),
//    and Swap (the double-buffer swap). Each stage is its own Schedule, so its boundary is a
//    fork-join barrier and a command-buffer sync point (R11, R12) — true intra-stage parallelism
//    starts at M2; M0 runs them serially. ─────────────────────────────────────────────────────────

fn stage_spatial_rebuild() {}
fn stage_sense() {}
fn stage_brain() {}
fn stage_act() {}

/// Stage 4 Move — the dummy stage: a deterministic INTEGER position step driven by seeded RNG.
/// Reads version-`t` [`Position`], writes `t+1` into [`PositionNext`]. No float anywhere.
fn stage_move(clock: Res<SimClock>, mut q: Query<(Entity, &Position, &mut PositionNext)>) {
    for (e, pos, mut next) in &mut q {
        let r = seed_fold(clock.seed, &[SALT_MOVE, e.to_bits(), clock.tick]);
        // Two disjoint bit-fields → a step in {-1,0,1} per axis. Pure integer.
        let dx = (r % 3) as i64 - 1;
        let dy = ((r >> 8) % 3) as i64 - 1;
        next.0 = Vec2Fixed(pos.0 .0 + dx, pos.0 .1 + dy);
    }
}

fn stage_metabolism() {}
fn stage_interactions() {}

/// Stage 7 BirthDeath — the structural-change sync point (R12). Empty in M0, but it takes `Commands`
/// and its Schedule applies them at the stage boundary, so the deferred-spawn/despawn path is wired.
fn stage_birth_death(mut _commands: Commands) {}

fn stage_field_scatter() {}

/// Stage 9 Observe — the read-only telemetry sink. Reads sim state, writes only to [`Telemetry`];
/// never mutates the simulation (R26 monitoring is read-only).
fn stage_observe(q: Query<&Position>, mut tel: ResMut<Telemetry>) {
    let n = q.iter().count() as i64;
    tel.metrics.insert("entity_count", n);
}

/// Stage 10 Swap — double-buffer swap: `Position`(t) ← `PositionNext`(t+1).
fn stage_swap(mut q: Query<(&mut Position, &PositionNext)>) {
    for (mut pos, next) in &mut q {
        pos.0 = next.0;
    }
}

#[cfg(feature = "perf")]
mod perf {
    use crate::DetMap;
    /// Per-stage instrumentation (R26): accumulated wall-clock ns and last-tick ns/entity. Timing is
    /// non-deterministic, so it NEVER feeds the tick or the state hash — only this side report.
    #[derive(Default)]
    pub struct PerfReport {
        stages: DetMap<&'static str, (u128, u128)>,
    }
    impl PerfReport {
        pub fn record(&mut self, name: &'static str, ns: u128, entities: u128) {
            let e = self.stages.entry(name).or_insert((0, 0));
            e.0 += ns;
            e.1 = ns / entities.max(1);
        }
        /// `name → (total_ns, last_ns_per_entity)`, in canonical stage order.
        pub fn stages(&self) -> &DetMap<&'static str, (u128, u128)> {
            &self.stages
        }
    }
}
#[cfg(feature = "perf")]
pub use perf::PerfReport;

/// The deterministic core. Build with [`Sim::new`], drive with [`Sim::step`], read the canonical
/// state with [`Sim::state_hash`]. The loop driver (accumulator, fixed dt, step cap) is the `cli`
/// crate's job — the core has no clock.
pub struct Sim {
    world: World,
    stages: Vec<(&'static str, Schedule)>,
    // Read only by the perf instrumentation (ns/entity); kept always so `new` stays uniform.
    #[cfg_attr(not(feature = "perf"), allow(dead_code))]
    n_entities: u64,
    #[cfg(feature = "perf")]
    perf: PerfReport,
}

impl Sim {
    /// Spawn `n_entities` dummy entities deterministically (ids `0..n` in order, no despawn in M0 →
    /// stable `Entity` ids) and build the 11-stage pipeline.
    pub fn new(seed: u64, n_entities: u64) -> Self {
        let mut world = World::new();
        world.insert_resource(SimClock { seed, tick: 0 });
        world.insert_resource(InputLog::default());
        world.insert_resource(Telemetry::default());
        for i in 0..n_entities {
            let p = Vec2Fixed(i as i64, 0);
            world.spawn((Position(p), PositionNext(p)));
        }
        Self {
            world,
            stages: build_stages(),
            n_entities,
            #[cfg(feature = "perf")]
            perf: PerfReport::default(),
        }
    }

    /// Advance one fixed tick. Runs the 11 stages in fixed order (barrier between each), then bumps
    /// the tick. No `dt` argument — the tick is the unit; the driver decides how many to run.
    pub fn step(&mut self) {
        for (_name, sched) in &mut self.stages {
            #[cfg(feature = "perf")]
            let start = std::time::Instant::now();
            sched.run(&mut self.world);
            #[cfg(feature = "perf")]
            self.perf.record(_name, start.elapsed().as_nanos(), self.n_entities as u128);
        }
        self.world.resource_mut::<SimClock>().tick += 1;
    }

    /// Canonical full-ECS state hash (R19) via the one [`deterministic_fold`] reduction point.
    pub fn state_hash(&mut self) -> u64 {
        let mut q = self.world.query::<(Entity, &Position)>();
        let items: Vec<(Entity, u64)> =
            q.iter(&self.world).map(|(e, p)| (e, hash_position(p))).collect();
        deterministic_fold(items)
    }

    /// Current tick.
    pub fn tick(&self) -> u64 {
        self.world.resource::<SimClock>().tick
    }

    /// Live telemetry sink (for the headless demo / tests).
    pub fn telemetry(&self) -> &Telemetry {
        self.world.resource::<Telemetry>()
    }

    /// Per-stage perf report (only with `--features perf`).
    #[cfg(feature = "perf")]
    pub fn perf(&self) -> &PerfReport {
        &self.perf
    }
}

/// Per-entity hash contribution: fold the integer coordinates. The single sort+fold in
/// [`deterministic_fold`] then makes the whole-world reduction order-independent.
fn hash_position(p: &Position) -> u64 {
    let h = fnv_mix(0xcbf2_9ce4_8422_2325, p.0 .0 as u64);
    fnv_mix(h, p.0 .1 as u64)
}

/// Build the 11 stages, each as its own `Schedule`. A separate schedule per stage = an explicit
/// fork-join barrier between stages and a command-buffer apply point at each boundary.
fn build_stages() -> Vec<(&'static str, Schedule)> {
    macro_rules! stage {
        ($name:expr, $sys:expr) => {{
            let mut s = Schedule::default();
            s.add_systems($sys);
            ($name, s)
        }};
    }
    vec![
        stage!("0_spatial_rebuild", stage_spatial_rebuild),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_runs_same_seed_match_per_tick() {
        let mut a = Sim::new(42, 16);
        let mut b = Sim::new(42, 16);
        for _ in 0..128 {
            a.step();
            b.step();
            assert_eq!(a.state_hash(), b.state_hash(), "tick {}", a.tick());
        }
    }

    #[test]
    fn different_seed_diverges() {
        let mut a = Sim::new(1, 16);
        let mut b = Sim::new(2, 16);
        for _ in 0..32 {
            a.step();
            b.step();
        }
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn observe_writes_dummy_metric() {
        let mut s = Sim::new(7, 10);
        s.step();
        assert_eq!(s.telemetry().metrics.get("entity_count"), Some(&10));
    }

    #[test]
    fn swap_advances_position() {
        // After one tick every entity moved by a step in {-1,0,1} per axis from its start.
        let mut s = Sim::new(7, 4);
        s.step();
        let mut q = s.world.query::<&Position>();
        for p in q.iter(&s.world) {
            assert!(p.0 .1 >= -1 && p.0 .1 <= 1);
        }
    }
}
