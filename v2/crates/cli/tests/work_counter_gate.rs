//! Perf-regression gate (R26 / D1a–c): asserts O(N) complexity bounds on per-entity work counters.
//!
//! Only compiled and run with `--features perf` (the CI step is a dedicated `--features perf` step
//! in `v2-sim-x86`, D1b). Never in `v2-golden-arm64` (R19).
//!
//! # Gate design (D1a)
//! Each counter is bounded by `counter ≤ C · N_sustain · ticks` where `N_sustain` is the
//! **minimum population observed over the last `SUSTAIN_WINDOW` ticks** — the steady-state
//! population after the ecosystem has stabilised — and `C` is the committed per-counter
//! complexity constant (with headroom).
//!
//! Using `min_pop` (not `max_pop`) over the sustain window closes the "founder-dominated phantom
//! bound" gap: with 200 founders alive at tick 0, `max_pop ≈ 200` always, so an O(N²) loop on
//! a real population of 5 that collapsed by tick 50 would never breach a bound scaled to 200.
//! With `min_pop` the bound tracks the actual sustained scale, so an O(N²) regression on N_sustain
//! entities provably yields `counter ≈ N_sustain² · ticks ≫ C · N_sustain · ticks`.
//!
//! # Sustained-population floor (F1 fix)
//! The test asserts `sustain_pop ≥ SUSTAIN_FLOOR` before any bound check. If the bench scenario
//! collapses below the floor, the bench itself is broken — not the bound check.
//!
//! # Liveness lower-bound
//! Each counter must also be ≥ `FLOOR · N_sustain`, guarding against a refactor that silently
//! zeroes a counter's increment (a stuck-at-zero counter would otherwise pass green forever).
//!
//! # Synthetic negative test
//! `v2_work_counter_negative_synthetic` proves `check_bound`/`check_liveness` actually bite —
//! a counter one over the bound fails, and a zero counter fails the liveness check.
//!
//! # Bench scale (F8)
//! `BENCH_N_FOUNDERS=200` on a `world_dim=128` world (carrying capacity ≈450+) yields a
//! sustained N_sustain ≫ headroom: an injected O(N²) nested loop provably drives
//! `counter/(N_sustain·ticks)` → O(N) and breaches `C`. The default 64×64 world at 40 founders
//! would NOT reliably trigger this (D1a).

#![cfg(feature = "perf")]

// ── Committed bench constants ──────────────────────────────────────────────────────────────────────
const BENCH_SEED: u64 = 0xBEEF_F00D_C0DE_1111;
const BENCH_TICKS: u64 = 400;
const BENCH_N_FOUNDERS: u64 = 200; // → steady-state pop typically 300–450+

// Sustain window: last quarter of the run (after ecosystem has stabilised post-ramp-up).
const SUSTAIN_WINDOW: u64 = BENCH_TICKS / 4; // last 100 ticks
// Min population required in the sustain window — ensures the scenario is self-sustaining.
const SUSTAIN_FLOOR: u64 = 50;

// ── Committed complexity bounds (C per counter, D1a) ──────────────────────────────────────────────
// brain_infer: stage 2 fires only every K=4 ticks → max ratio ≈ 1/K = 0.25; C=0.5 gives 2× slack.
const C_BRAIN: f64 = 0.5;
// field_takes, birth_death_iters, scatter_deposits: one op per entity per tick → max ratio = 1.0;
// C=2.0 gives 2× slack against the gap between avg_pop and sustain_pop during the run.
const C_FIELD_TAKES: f64 = 2.0;
const C_BIRTH_DEATH: f64 = 2.0;
const C_SCATTER: f64 = 2.0;

// ── Committed liveness floor (counter ≥ FLOOR · N_sustain) ────────────────────────────────────────
const FLOOR: f64 = 1.0; // detects a stuck-at-zero counter; actual counters are orders of magnitude higher

// ─────────────────────────────────────────────────────────────────────────────────────────────────

/// Returns true iff `counter` is within the O(N) complexity bound.
/// The negative test proves it returns `false` above the bound.
fn check_bound(counter: u64, c: f64, n_sustain: u64, ticks: u64) -> bool {
    (counter as f64) <= c * n_sustain as f64 * ticks as f64
}

/// Returns true iff `counter` satisfies the liveness lower-bound.
fn check_liveness(counter: u64, n_sustain: u64) -> bool {
    (counter as f64) >= FLOOR * n_sustain as f64
}

// ── Main gate test ─────────────────────────────────────────────────────────────────────────────────
#[test]
fn v2_work_counter_complexity_gate() {
    let mut sim = cli::build_sim_bench(BENCH_SEED, BENCH_N_FOUNDERS);
    let mut pops: Vec<u64> = Vec::with_capacity(BENCH_TICKS as usize);
    for _ in 0..BENCH_TICKS {
        sim.step();
        pops.push(sim.population());
    }

    // ── Sustained-population floor ─────────────────────────────────────────────────────────────────
    // Use min population over the last SUSTAIN_WINDOW ticks (steady state after ramp-up).
    // This is the representative scale: it is NOT founder-dominated (founders are the initial
    // count at tick 0; by tick 300 the ecosystem is self-sustaining at its carrying capacity).
    let window_start = (BENCH_TICKS - SUSTAIN_WINDOW) as usize;
    let sustain_pop = pops[window_start..].iter().copied().min().unwrap_or(0);
    assert!(
        sustain_pop >= SUSTAIN_FLOOR,
        "bench sustain_pop={sustain_pop} < SUSTAIN_FLOOR={SUSTAIN_FLOOR} \
         (min over last {SUSTAIN_WINDOW} of {BENCH_TICKS} ticks). \
         The bench scenario is not self-sustaining — check bench_config/world scale."
    );

    let wc = sim.perf().work;
    let ns = sustain_pop; // N_sustain: the denominator for all bound checks
    let t = BENCH_TICKS;

    // ── Upper bounds (O(N) complexity gate) ───────────────────────────────────────────────────────
    assert!(
        check_bound(wc.brain_infer, C_BRAIN, ns, t),
        "brain_infer={} exceeds O(N) bound {:.0} \
         (C={C_BRAIN} * sustain_pop={ns} * ticks={t}). \
         A super-linear regression was introduced in stage_brain.",
        wc.brain_infer,
        C_BRAIN * ns as f64 * t as f64,
    );
    assert!(
        check_bound(wc.field_takes, C_FIELD_TAKES, ns, t),
        "field_takes={} exceeds O(N) bound {:.0} \
         (C={C_FIELD_TAKES} * sustain_pop={ns} * ticks={t}). \
         A super-linear regression was introduced in stage_interactions.",
        wc.field_takes,
        C_FIELD_TAKES * ns as f64 * t as f64,
    );
    assert!(
        check_bound(wc.birth_death_iters, C_BIRTH_DEATH, ns, t),
        "birth_death_iters={} exceeds O(N) bound {:.0} \
         (C={C_BIRTH_DEATH} * sustain_pop={ns} * ticks={t}). \
         A super-linear regression was introduced in stage_birth_death.",
        wc.birth_death_iters,
        C_BIRTH_DEATH * ns as f64 * t as f64,
    );
    assert!(
        check_bound(wc.scatter_deposits, C_SCATTER, ns, t),
        "scatter_deposits={} exceeds O(N) bound {:.0} \
         (C={C_SCATTER} * sustain_pop={ns} * ticks={t}). \
         A super-linear regression was introduced in stage_field_scatter.",
        wc.scatter_deposits,
        C_SCATTER * ns as f64 * t as f64,
    );

    // ── Liveness lower-bounds ─────────────────────────────────────────────────────────────────────
    assert!(
        check_liveness(wc.brain_infer, ns),
        "brain_infer={} < liveness floor {:.0} (FLOOR={FLOOR} * sustain_pop={ns}). \
         Counter increment may have been zeroed by a refactor.",
        wc.brain_infer, FLOOR * ns as f64,
    );
    assert!(
        check_liveness(wc.field_takes, ns),
        "field_takes={} < liveness floor {:.0}. Counter increment may have been zeroed.",
        wc.field_takes, FLOOR * ns as f64,
    );
    assert!(
        check_liveness(wc.birth_death_iters, ns),
        "birth_death_iters={} < liveness floor {:.0}. Counter increment may have been zeroed.",
        wc.birth_death_iters, FLOOR * ns as f64,
    );
    assert!(
        check_liveness(wc.scatter_deposits, ns),
        "scatter_deposits={} < liveness floor {:.0}. Counter increment may have been zeroed.",
        wc.scatter_deposits, FLOOR * ns as f64,
    );
}

// ── Synthetic negative test (F9): proves check_bound and check_liveness actually bite ────────────
#[test]
fn v2_work_counter_negative_synthetic() {
    // check_bound must reject counter = bound + 1.
    let ns: u64 = 100; // synthetic sustain_pop
    let t: u64 = 400;
    let bound = C_BRAIN * ns as f64 * t as f64; // 0.5 * 100 * 400 = 20000
    let over = bound as u64 + 1;
    assert!(
        !check_bound(over, C_BRAIN, ns, t),
        "check_bound must return false when counter={over} > bound={bound}"
    );
    // check_bound must accept counter exactly at the bound.
    assert!(
        check_bound(bound as u64, C_BRAIN, ns, t),
        "check_bound must return true when counter={} == bound={bound}", bound as u64
    );

    // check_liveness must reject a zero counter.
    assert!(
        !check_liveness(0, ns),
        "check_liveness must return false for counter=0 (stuck-at-zero counter detection)"
    );
    // check_liveness must accept the liveness floor itself.
    let floor_val = (FLOOR * ns as f64) as u64;
    assert!(
        check_liveness(floor_val, ns),
        "check_liveness must return true when counter={floor_val} >= floor"
    );
}
