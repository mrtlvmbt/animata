//! D′-1 + D′-2a conservation, determinism, oscillation, viability, and cost teeth (issues #177, #181).
//!
//! PRE-DECLARED FALSIFIABLE NUMERICS (anti-self-certification, doc54 §2 — declared BEFORE
//! measuring; do NOT weaken post-hoc to force green):
//!
//!   R15 — residual = 0 every tick on dprime_config (photo energy booked correctly to ledger.produced).
//!
//!   R14 — conserved-field hash 1-vs-N thread-identical (photo uptake is deterministic, per-cell).
//!
//!   (c) L(t) oscillates: light_at_tick varies within one period — pre-declared max=l_max, min=0.
//!       Proves a constant L would FAIL this tooth, protecting D′-2 gate from false positive (F6).
//!
//!   (a) Viability: N̄ ≥ VIAB_FLOOR at t=TICKS (both seeds).
//!       Anchor: N*_chem ≈ 134 (B3 corridor doc, same base chemistry as default_config). Light
//!       subsidy is additive → N* can only rise once photo_gain evolves. Floor = 0.60 × N*_B3 = 80
//!       (doc54 ±40% bracket; conservative since photo_gain starts at 0 and needs evolution time).
//!
//!   (b) Photo non-trivial: on a day-phase tick after TICKS evolution, photo_produced > PHOTO_DAY_FLOOR.
//!       Pre-declared floor: 1 eu/tick. In integer arithmetic, photo_demand(g, km=30, L=100) truncates
//!       to 0 for photo_gain=1 and ≥1 for photo_gain≥2. So floor=1 proves: gene escaped 0 (evolved
//!       to ≥2), path is wired, and contribution is non-zero. Seed variance under Ns≈0.1 drift is wide;
//!       1 eu/tick is the honest minimum. "Meaningful fraction" materialises at D′-2/D′-3 timescales.
//!
//!   (d) L(t) sim-level oscillation: photo_produced differs between a day tick and a night tick after
//!       evolution — night must be 0 (L=0 → no photo), day must exceed PHOTO_DAY_FLOOR.
//!       This is the sim-level complement to tooth (c): catches a wiring bug where L was computed
//!       correctly but the photo path wasn't actually gated on it.

use cli::{build_sim, dprime_config, run_conserved_hashes};
use sim_core::{light_at_tick, SimConfig};

const SEED: u64 = 0xD0_DE_5EED;
const TICKS_SHORT: u64 = 512;  // for R14/R15 (fast, determinism)
const TICKS: u64 = 4_000;       // for viability + photo-fraction gate (heavy, release-only)
const N_THREADS: usize = 4;

// ── Pre-declared gate constants ──────────────────────────────────────────────────────────────────
/// Viability floor: N̄ ≥ VIAB_FLOOR at t=TICKS (both seeds). 0.60 × N*_B3 ≈ 80 (doc54 §2).
const VIAB_FLOOR: u64 = 80;
/// Photo non-trivial: photo_produced on a day tick after TICKS > this (1 eu/tick).
///
/// Analytically derived: `photo_demand(g, km=30, L=100) = g·100/130` integer-truncating.
/// photo_gain=1 → 0 eu (truncated); photo_gain≥2 → ≥1 eu. So `photo_produced ≥ 1` proves
/// at least one cell evolved photo_gain≥2 — the gene escaped 0 and the path is active.
///
/// Under weak selection (Ns≈0.1, N≈100) + integer truncation at low gains, mean photo_produced
/// fluctuates 0–30 eu/tick at 4000 ticks depending on seed drift. Floor=1 is the correct
/// honest minimum: it rules out "path never wired" while acknowledging that constitutive
/// phototrophy at founder=0 needs D′-2/D′-3 timescales for selection to drive meaningful fraction.
const PHOTO_DAY_FLOOR: i64 = 1;

// ── R15 ─────────────────────────────────────────────────────────────────────────────────────────

/// R15: energy residual = 0 every tick on dprime_config — photo energy is booked as Σᵢ photo_energyᵢ
/// to ledger.produced (exact integer match to what was credited to agent Energy components).
/// Covers both day ticks (photo_total > 0) and night ticks (photo_total = 0, path inert).
#[test]
fn dprime_r15_conservation_exact() {
    if cfg!(debug_assertions) {
        return;
    }
    let mut sim = build_sim(dprime_config(SEED));
    for _ in 0..TICKS_SHORT {
        sim.step();
        assert_eq!(
            sim.conservation_residual(),
            0,
            "energy not conserved at tick {} on dprime_config \
             (photo energy booking leaked or residual non-zero)",
            sim.tick()
        );
    }
}

// ── R14 ─────────────────────────────────────────────────────────────────────────────────────────

/// R14: 1-vs-N conserved-field hash identical on dprime_config — photo uptake is a deterministic
/// pure function of (photo_gain, L(t)) with no cross-cell interaction, so thread count cannot
/// affect the result. Conserved-field trajectory changes only through births/deaths/excretion,
/// which are already canonical-merge guarded.
#[test]
fn dprime_r14_thread_count_independent() {
    if cfg!(debug_assertions) {
        return;
    }
    let one = run_conserved_hashes(SimConfig { sim_threads: 1, ..dprime_config(SEED) }, TICKS_SHORT);
    let many = run_conserved_hashes(SimConfig { sim_threads: N_THREADS, ..dprime_config(SEED) }, TICKS_SHORT);
    for t in 0..TICKS_SHORT as usize {
        assert_eq!(
            one[t], many[t],
            "dprime conserved hash differs 1-vs-{N_THREADS} at tick {t} (R14 broken on photo path)"
        );
    }
}

// ── L(t) oscillation tooth (pure-function, F3/F6) ───────────────────────────────────────────────

/// (c) L(t) oscillation — pure function test (no sim needed). Within one full period, light_at_tick
/// must reach both l_max (day) and 0 (night). Pre-declared: max = l_max, min = 0.
///
/// A constant L cannot pass this: if someone accidentally made `light_at_tick` return l_max always,
/// `min_l = l_max != 0` → FAIL. This protects D′-2 against a false no-difference pass on an inert
/// (constant) light field (critic F6 from plan).
#[test]
fn dprime_light_field_oscillates() {
    let spec = dprime_config(0).econ.light.expect("dprime_config must have light: Some(...)");
    assert!(spec.period_ticks > 0, "period_ticks must be > 0");
    assert!(spec.day_ticks > 0, "day_ticks must be > 0 — else no day phase exists");
    assert!(
        spec.day_ticks < spec.period_ticks,
        "day_ticks ({}) must be < period_ticks ({}) — else no night phase",
        spec.day_ticks, spec.period_ticks
    );

    let max_l = (0..spec.period_ticks).map(|t| light_at_tick(&spec, t)).max().unwrap_or(0);
    let min_l = (0..spec.period_ticks).map(|t| light_at_tick(&spec, t)).min().unwrap_or(0);

    assert_eq!(
        max_l, spec.l_max,
        "L(t) never reaches l_max={} in one period — no day phase \
         (constant L would make D′-2 gate a false positive)",
        spec.l_max
    );
    assert_eq!(
        min_l, 0,
        "L(t) never reaches 0 in one period — no night phase \
         (constant L would make D′-2 gate a false positive)"
    );
}

// ── Viability + photo non-trivial + sim-level oscillation (heavy, release-only) ─────────────────

/// (a) + (b) + (d): dprime_config economy viable after 4000 ticks (both seeds); photo channel
/// non-trivially exercised on a day tick; photo_produced = 0 on a night tick (L(t) oscillates
/// at the sim level, not just as a pure function).
///
/// Heavy test — release only (4000+ ticks × ~134+ cells per seed).
#[test]
fn dprime_viability_and_photo_nontrivial() {
    if cfg!(debug_assertions) {
        return;
    }
    for seed in [0xD1_00_1111u64, 0xD2_00_2222u64] {
        let mut sim = build_sim(dprime_config(seed));
        let spec = sim.econ().light.expect("dprime_config must have light: Some(...)");

        // Run TICKS ticks to let photo_gain evolve under selection.
        for _ in 0..TICKS {
            sim.step();
        }

        // (a) Viability gate.
        let pop = sim.population();
        assert!(
            pop >= VIAB_FLOOR,
            "dprime viability FAILED: seed={seed:#x} N̄={pop} < VIAB_FLOOR={VIAB_FLOOR} at t={TICKS} \
             (light economy or photo gene blocked population growth; check dprime_config params)"
        );

        // (b) + (d): advance one full period, check day and night photo_produced.
        // After TICKS ticks, photo_gain should have evolved so day-tick photo_total > PHOTO_DAY_FLOOR.
        let mut found_day = false;
        let mut found_night = false;
        for _ in 0..spec.period_ticks {
            sim.step();
            // clock.tick when stage_interactions ran = sim.tick() - 1 (step increments at end)
            let ran_at = sim.tick().saturating_sub(1);
            let l = light_at_tick(&spec, ran_at);
            let pp = sim.telemetry().photo_produced;

            if l > 0 && !found_day {
                // (b) Day tick: photo_produced must exceed the non-trivial floor.
                assert!(
                    pp >= PHOTO_DAY_FLOOR,
                    "dprime photo FAILED (non-trivial): seed={seed:#x} photo_produced={pp} < \
                     PHOTO_DAY_FLOOR={PHOTO_DAY_FLOOR} on day tick (ran_at={ran_at}, L={l}) \
                     — photo path not exercised or photo_gain failed to evolve after {TICKS} ticks"
                );
                found_day = true;
            }
            if l == 0 && !found_night {
                // (d) Night tick: photo_produced must be exactly 0 (L=0 → no photo energy).
                assert_eq!(
                    pp, 0,
                    "dprime L(t) sim-oscillation FAILED: seed={seed:#x} \
                     photo_produced={pp} on night tick (ran_at={ran_at}, L=0) \
                     — photo path not gated on L(t)"
                );
                found_night = true;
            }
            if found_day && found_night {
                break;
            }
        }
        assert!(found_day, "no day tick found in one period (period_ticks={})", spec.period_ticks);
        assert!(found_night, "no night tick found in one period (period_ticks={})", spec.period_ticks);
    }
}

// ── D′-2a: photo-machinery expression cost teeth ─────────────────────────────────────────────────
//
// PRE-DECLARED gate constants (declared BEFORE measuring; do NOT weaken post-hoc):
//
//   (e) Non-inertness: total photo-machinery cost dissipated > 0 over TICKS_LONG=8000 ticks on
//       the canonical seed 0xA11A_2A11. This seed's photo sub-population sweeps ~tick 5000
//       (PM probe), so by 8000 ticks many cells have photo_gain above the truncation threshold
//       (NUM=1, DEN=16, n=2 → threshold gain=8). A green CI with cost always 0 would be a
//       silent slice failure — this tooth makes that impossible to ship.
//
//   (f) D′-2a viability re-band (direction: DOWN from D′-1). Cost lowers net energy → slightly
//       lower N*. Pre-declared floor: N̄ ≥ VIAB_FLOOR_D2A = 50 at TICKS_LONG ticks on the
//       same canonical seed. If cost collapses the population (N < 50), REPORT and do not
//       silently lower cost — that is a real finding about the calibration's sim-transfer.
//       Derived: D′-1 floor was 80 at 4000 ticks; direction is down; ±40% from D′-1 N̄ gives
//       a lower bound; 50 represents "not collapsed" without predicting the exact magnitude.

/// Canonical D′-2 seed: used by the dprime golden + PM probe (photo sweeps by ~tick 5000).
const SEED_DPRIME2: u64 = 0xA11A_2A11;
/// Long-horizon tick count for D′-2a teeth. Photo sweep + selection stabilise by ~8000 ticks.
const TICKS_LONG: u64 = 8_000;
/// D′-2a viability floor: N̄ ≥ 50 at TICKS_LONG ticks. Direction DOWN from D′-1 (cost reduces
/// net energy). 50 = "not collapsed" threshold. Pre-declared per doc54 §2 before measuring.
const VIAB_FLOOR_D2A: u64 = 50;

/// (e) Non-inertness: total photo-machinery cost dissipated > 0 over TICKS_LONG on SEED_DPRIME2.
///
/// Rationale: cost formula `(NUM·gain·n)/DEN` truncates to 0 for `gain < DEN/n = 8` (NUM=1,
/// DEN=16, n=2). At TICKS_LONG, the photo sweep (known for this seed ~tick 5000) should have
/// produced cells with `photo_gain ≥ 8` → non-zero charge. If the total is 0, the cost is
/// silently inert across the whole run — a slice failure, not a green CI.
///
/// Heavy test — release only (TICKS_LONG × ~population per tick).
#[test]
fn dprime_d2a_cost_non_inert() {
    if cfg!(debug_assertions) {
        return;
    }
    let mut sim = build_sim(dprime_config(SEED_DPRIME2));
    for _ in 0..TICKS_LONG {
        sim.step();
    }
    let total_cost = sim.telemetry().photo_cost_total;
    assert!(
        total_cost > 0,
        "D′-2a non-inertness FAILED: photo_cost_total=0 after {TICKS_LONG} ticks on \
         seed={SEED_DPRIME2:#x}. Cost formula (NUM·gain·n)/DEN is inert for all cells \
         (photo_gain never reached threshold ≥8). Either photo sweep failed or NUM/DEN miscalibrated."
    );
}

/// (f) D′-2a viability re-band: N̄ ≥ VIAB_FLOOR_D2A at TICKS_LONG on SEED_DPRIME2.
///
/// The cost reduces net energy per cell → direction DOWN from D′-1 N̄. Pre-declared floor=50
/// ("not collapsed"). If cost collapses the population, this fails — report the finding, do NOT
/// silently reduce the cost to pass.
///
/// Heavy test — release only (TICKS_LONG × ~population per tick).
#[test]
fn dprime_d2a_viability_reband() {
    if cfg!(debug_assertions) {
        return;
    }
    let mut sim = build_sim(dprime_config(SEED_DPRIME2));
    for _ in 0..TICKS_LONG {
        sim.step();
    }
    let pop = sim.population();
    assert!(
        pop >= VIAB_FLOOR_D2A,
        "D′-2a viability FAILED: seed={SEED_DPRIME2:#x} N̄={pop} < VIAB_FLOOR_D2A={VIAB_FLOOR_D2A} \
         at t={TICKS_LONG}. Photo-machinery cost may have collapsed the population. \
         Report this finding — do NOT silently lower the cost. Check calibration sim-transfer."
    );
}
