//! Perf-regression gate (R26 / D1a–c): asserts O(N) complexity bounds on per-entity work counters.
//!
//! Only compiled and run with `--features perf` (the CI step is a dedicated `--features perf` step
//! in `v2-sim-x86`, D1b). Never in `v2-golden-arm64` (R19).
//!
//! # Gate design (D1a)
//! Each counter is bounded by `counter ≤ C · N_peak · ticks` where `N_peak` is the emergent peak
//! population observed during the bench run, `ticks` is the committed run length, and `C` is the
//! committed per-counter complexity constant (with headroom). This catches algorithmic regressions
//! from O(N) → O(N²) but does NOT redden on population drift (a linear counter keeps the ratio stable
//! under `C` regardless of how many more/fewer creatures survive).
//!
//! # Liveness lower-bound
//! Each counter must also be ≥ `FLOOR · N_peak`, guarding against a refactor that silently zeroes
//! a counter's increment (a stuck-at-zero counter would otherwise pass green forever).
//!
//! # Synthetic negative test
//! `v2_work_counter_negative_synthetic` proves `check_bound` itself bites — a counter one over the
//! bound fails, and a zero counter fails the liveness check. No manual hack required.
//!
//! # Bench scale (F8)
//! `BENCH_N_FOUNDERS=200` on a `world_dim=128` world yields a sustained N_peak ≫ headroom (typically
//! 200-450+), so an injected O(N²) nested loop provably drives `counter/(N_peak·ticks)` → O(N) and
//! breaches `C`. The default 64³ world at 40 founders would NOT reliably trigger this (D1a).

#![cfg(feature = "perf")]

// ── Committed bench constants ──────────────────────────────────────────────────────────────────────
const BENCH_SEED: u64 = 0xBEEF_F00D_C0DE_1111;
const BENCH_TICKS: u64 = 400;
const BENCH_N_FOUNDERS: u64 = 200; // → peak pop typically 200–450+

// ── Committed complexity bounds (C per counter, D1a) ──────────────────────────────────────────────
// brain_infer: stage 2 fires only every K=4 ticks → max ratio ≈ 1/(K) = 0.25; C=0.5 gives 2× slack.
const C_BRAIN: f64 = 0.5;
// field_takes, birth_death_iters, scatter_deposits: one op per entity per tick → max ratio = 1.0;
// C=2.0 gives 2× slack against emergent population variance (the ratio is invariant to pop drift).
const C_FIELD_TAKES: f64 = 2.0;
const C_BIRTH_DEATH: f64 = 2.0;
const C_SCATTER: f64 = 2.0;

// ── Committed liveness floors (floor · N_peak, cumulative over all ticks) ─────────────────────────
// Even if just one brain tick and one entity fired, brain_infer ≥ 1. N_peak is far larger; this
// just detects a zero counter. A per-tick run over 400 ticks gives counters in the thousands.
const FLOOR: f64 = 1.0; // counter ≥ FLOOR * N_peak (all counters share this conservative floor)

// ─────────────────────────────────────────────────────────────────────────────────────────────────

/// Returns true iff `counter` is within the O(N) complexity bound.
/// The test calls this for every counter; the negative test proves it returns `false` above the bound.
fn check_bound(counter: u64, c: f64, n_peak: u64, ticks: u64) -> bool {
    (counter as f64) <= c * n_peak as f64 * ticks as f64
}

/// Returns true iff `counter` satisfies the liveness lower-bound.
fn check_liveness(counter: u64, n_peak: u64) -> bool {
    (counter as f64) >= FLOOR * n_peak as f64
}

// ── Main gate test ─────────────────────────────────────────────────────────────────────────────────
#[test]
fn v2_work_counter_complexity_gate() {
    let mut sim = cli::build_sim_bench(BENCH_SEED, BENCH_N_FOUNDERS);
    let mut peak_pop: u64 = 0;
    for _ in 0..BENCH_TICKS {
        sim.step();
        peak_pop = peak_pop.max(sim.population());
    }
    assert!(peak_pop > 0, "population went extinct — bench scenario is broken");

    let wc = sim.perf().work;
    let np = peak_pop;
    let t = BENCH_TICKS;

    // ── Upper bounds (O(N) complexity gate) ───────────────────────────────────────────────────────
    assert!(
        check_bound(wc.brain_infer, C_BRAIN, np, t),
        "brain_infer={} exceeds O(N) bound {:.0} (C={C_BRAIN} * peak_pop={np} * ticks={t}). \
         A super-linear regression was introduced in stage_brain.",
        wc.brain_infer,
        C_BRAIN * np as f64 * t as f64,
    );
    assert!(
        check_bound(wc.field_takes, C_FIELD_TAKES, np, t),
        "field_takes={} exceeds O(N) bound {:.0} (C={C_FIELD_TAKES} * peak_pop={np} * ticks={t}). \
         A super-linear regression was introduced in stage_interactions.",
        wc.field_takes,
        C_FIELD_TAKES * np as f64 * t as f64,
    );
    assert!(
        check_bound(wc.birth_death_iters, C_BIRTH_DEATH, np, t),
        "birth_death_iters={} exceeds O(N) bound {:.0} (C={C_BIRTH_DEATH} * peak_pop={np} * ticks={t}). \
         A super-linear regression was introduced in stage_birth_death.",
        wc.birth_death_iters,
        C_BIRTH_DEATH * np as f64 * t as f64,
    );
    assert!(
        check_bound(wc.scatter_deposits, C_SCATTER, np, t),
        "scatter_deposits={} exceeds O(N) bound {:.0} (C={C_SCATTER} * peak_pop={np} * ticks={t}). \
         A super-linear regression was introduced in stage_field_scatter.",
        wc.scatter_deposits,
        C_SCATTER * np as f64 * t as f64,
    );

    // ── Liveness lower-bounds (counter must be non-trivially > 0) ─────────────────────────────────
    assert!(
        check_liveness(wc.brain_infer, np),
        "brain_infer={} < liveness floor {:.0} (FLOOR={FLOOR} * peak_pop={np}). \
         Counter increment may have been zeroed by a refactor.",
        wc.brain_infer, FLOOR * np as f64,
    );
    assert!(
        check_liveness(wc.field_takes, np),
        "field_takes={} < liveness floor {:.0}. Counter increment may have been zeroed.",
        wc.field_takes, FLOOR * np as f64,
    );
    assert!(
        check_liveness(wc.birth_death_iters, np),
        "birth_death_iters={} < liveness floor {:.0}. Counter increment may have been zeroed.",
        wc.birth_death_iters, FLOOR * np as f64,
    );
    assert!(
        check_liveness(wc.scatter_deposits, np),
        "scatter_deposits={} < liveness floor {:.0}. Counter increment may have been zeroed.",
        wc.scatter_deposits, FLOOR * np as f64,
    );
}

// ── Synthetic negative test (F9): proves check_bound and check_liveness actually bite ────────────
#[test]
fn v2_work_counter_negative_synthetic() {
    // check_bound must reject counter = bound + 1.
    let np: u64 = 100;
    let t: u64 = 400;
    let bound = C_BRAIN * np as f64 * t as f64; // e.g. 0.5 * 100 * 400 = 20000
    let over = bound as u64 + 1;
    assert!(
        !check_bound(over, C_BRAIN, np, t),
        "check_bound must return false when counter={over} > bound={bound}"
    );
    // check_bound must accept counter = bound (exact).
    assert!(
        check_bound(bound as u64, C_BRAIN, np, t),
        "check_bound must return true when counter={} == bound={bound}", bound as u64
    );

    // check_liveness must reject a zero counter.
    assert!(
        !check_liveness(0, np),
        "check_liveness must return false for counter=0 (stuck-at-zero counter detection)"
    );
    // check_liveness must accept the liveness floor itself.
    let floor_val = (FLOOR * np as f64) as u64;
    assert!(
        check_liveness(floor_val, np),
        "check_liveness must return true when counter={floor_val} >= floor"
    );
}
