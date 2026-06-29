//! C′-2 emergence and viability acceptance gate (issue #175).
//!
//! PRE-DECLARED FALSIFIABLE NUMERICS (anti-self-certification, Slice-C F3 — declared BEFORE
//! measuring; do NOT weaken post-hoc to force green):
//!
//!   (a) Viability:  N̄ ≥ VIAB_FLOOR at t=TICKS (both seeds).
//!       Anchor: analytic N* without reducers (substrate-only P; detritus traps all recycle):
//!         P = regen × n_cells = 6 × 4096 = 24 576 eu/tick (layer 0 only; detritus regen=0)
//!         Without detritus return, economy ≈ Slice-B3: N*_B3 ≈ 134 (economy_corridor.rs doc).
//!         VIAB_FLOOR = 60 ≈ 0.45 × N*_B3 (doc54 §2 bracket ±45%; clustering lowers spatial N̄
//!         below mean-field; conservative floor to distinguish viability from near-extinction).
//!
//!   (b) Reducer guild: guild_pop[Reducer] / total ≥ REDUCER_FRAC at t=TICKS (≥1 seed).
//!       Pre-declared threshold: 10% (issue #175 criterion b).
//!       Justified: detritus accumulates to ≈40 eu/cell by t=4000; U(40) ≈ 75 eu/tick — marginal
//!       but reachable via the 2-mutation path (uptake_layer: 0→2, excrete_layer: 1→0) under
//!       mutation_rate=32/256 over 4000 ticks. 10% indicates the niche is selected, not drift.
//!
//!   (c) Detritus plateau: |det_late − det_early| / det_early < PLATEAU_EPS over the last K ticks.
//!       K=500, ε=0.25 (25%). A plateau signals steady-state consumption (loop closed). Monotonic
//!       accumulation (>25% growth in last 500 ticks) is the §8 wash-out / unclosed-loop failure.
//!
//! Honest-null path (research/14 §8): if the loop does NOT close at detritus_frac=1.0 (gate b or
//! c fails), that is a FINDING — calibrate detritus_frac_num DOWN (hybrid), re-test, re-pin golden.
//! Do NOT weaken the gate to force green.

use cli::{build_sim, cprime_config};
use telemetry::{compute_with_census, Guild};

// ── Horizon ─────────────────────────────────────────────────────────────────────────────────────
const TICKS: u64 = 4_000;
const SEED_A: u64 = 0xA11A_2A11;
const SEED_B: u64 = 0x1234_5678;

// ── Pre-declared gate constants (analytic, declared before measuring) ────────────────────────────
/// Viability floor: N̄ at t=TICKS ≥ VIAB_FLOOR (both seeds). Anchored at 0.45 × N*_B3 ≈ 60.
const VIAB_FLOOR: u64 = 60;
/// Reducer fraction floor: guild_pop[Reducer]/total ≥ 10% (at least one seed must pass).
const REDUCER_FRAC_PCT: usize = 10;
/// Plateau check: last K ticks, relative change ε < 25%.
const PLATEAU_K: u64 = 500;
const PLATEAU_EPS: f64 = 0.25;

// ── Helper ────────────────────────────────────────────────────────────────────────────────────────

struct Snapshot {
    pop: u64,
    reducer_frac: f64,
    detritus_early: i64,
    detritus_late: i64,
}

fn run_cprime_seed(seed: u64) -> Snapshot {
    let mut sim = build_sim(cprime_config(seed));
    // Advance to (TICKS − PLATEAU_K): sample detritus here for plateau check.
    for _ in 0..(TICKS - PLATEAU_K) {
        sim.step();
    }
    let detritus_early = sim.field_layer_total(2);
    // Advance the final PLATEAU_K ticks.
    for _ in 0..PLATEAU_K {
        sim.step();
    }
    let pop = sim.population();
    let detritus_late = sim.field_layer_total(2);
    let tele = sim.telemetry();
    let rep = compute_with_census(&tele.samples, &tele.species_census, sim.econ().detritus_layer);
    let reducer_count = rep.guild_pop[Guild::Reducer as usize];
    let reducer_frac = if rep.population > 0 {
        reducer_count as f64 / rep.population as f64
    } else {
        0.0
    };
    Snapshot { pop, reducer_frac, detritus_early, detritus_late }
}

fn check_plateau(snap: &Snapshot) -> bool {
    if snap.detritus_early == 0 {
        // No detritus early → plateau only if also no detritus late (trivially stable / no deaths).
        return snap.detritus_late == 0;
    }
    let change = (snap.detritus_late - snap.detritus_early).abs() as f64;
    change / (snap.detritus_early as f64) < PLATEAU_EPS
}

// ── Gate tests ────────────────────────────────────────────────────────────────────────────────────

/// (a) Viability: N̄ ≥ VIAB_FLOOR at t=TICKS on BOTH seeds.
/// Anchored to the analytic B3-regime N* (substrate-only economy, no abiotic recycle) ± 45% doc54.
#[test]
fn cprime_viability_both_seeds() {
    if cfg!(debug_assertions) {
        return;
    }
    for seed in [SEED_A, SEED_B] {
        let snap = run_cprime_seed(seed);
        assert!(
            snap.pop >= VIAB_FLOOR,
            "cprime viability FAILED: seed={seed:#x} N̄={} < VIAB_FLOOR={VIAB_FLOOR} at t={TICKS} \
             (economy collapsed before reducers could evolve; calibrate detritus_frac_num DOWN)",
            snap.pop
        );
    }
}

/// (b) Reducer guild: ≥10% reducers on AT LEAST one seed at t=TICKS.
/// A reducer has uptake_layer == detritus_layer (=2). Presence above the floor confirms the niche
/// emerged by evolution (not seeded), closing the biotic loop (research/14 §8).
/// If BOTH seeds fail, the loop does NOT close at frac=1.0 → honest-null finding → calibrate.
#[test]
fn cprime_reducer_guild_emerges() {
    if cfg!(debug_assertions) {
        return;
    }
    let snap_a = run_cprime_seed(SEED_A);
    let snap_b = run_cprime_seed(SEED_B);
    let pass_a = snap_a.reducer_frac * 100.0 >= REDUCER_FRAC_PCT as f64;
    let pass_b = snap_b.reducer_frac * 100.0 >= REDUCER_FRAC_PCT as f64;
    assert!(
        pass_a || pass_b,
        "cprime reducer guild FAILED on BOTH seeds at t={TICKS}: \
         seed_a reducer_frac={:.1}%, seed_b reducer_frac={:.1}% (both < {REDUCER_FRAC_PCT}%) \
         → biotic loop does NOT close at detritus_frac=1.0; calibrate detritus_frac_num DOWN",
        snap_a.reducer_frac * 100.0,
        snap_b.reducer_frac * 100.0,
    );
}

/// (c) Detritus plateau: |det_late − det_early| / det_early < ε over the last PLATEAU_K ticks.
/// A plateau = detritus is in steady state (reducers consuming as fast as deaths deposit).
/// Monotonic accumulation (>ε) = unclosed loop / §8 wash-out failure.
/// Checked on BOTH seeds: both must plateau (a plateau on one seed + explosion on the other is not closure).
#[test]
fn cprime_detritus_plateaus() {
    if cfg!(debug_assertions) {
        return;
    }
    for seed in [SEED_A, SEED_B] {
        let snap = run_cprime_seed(seed);
        let plateau_ok = check_plateau(&snap);
        let change_pct = if snap.detritus_early > 0 {
            (snap.detritus_late - snap.detritus_early).abs() as f64
                / snap.detritus_early as f64
                * 100.0
        } else {
            0.0
        };
        assert!(
            plateau_ok,
            "cprime detritus plateau FAILED: seed={seed:#x} detritus change={change_pct:.1}% \
             over last {PLATEAU_K} ticks (> {:.0}% ε) — monotonic accumulation = unclosed loop \
             (research/14 §8); calibrate detritus_frac_num DOWN",
            PLATEAU_EPS * 100.0,
        );
    }
}
