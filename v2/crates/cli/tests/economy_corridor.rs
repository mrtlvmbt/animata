//! B-4 economy corridor gate (issue #157): long-horizon L=2 production run must land in the
//! measured spatial equilibrium band (range assert, NOT a golden — arch-independent, both CI jobs).
//!
//! IMPORTANT: default_config uses world_dim=64 (4 096 cells). The v2-perf sim-run scenario uses
//! world_dim=128 — a 4× larger world with different dynamics. This test runs default_config, NOT
//! the perf scenario. Calibrate exclusively from `build_sim(default_config(S))` runs.
//!
//! Population dynamics (default_config, seed=0xa11a2a11, **B-3+C** on feat/v2-sim-167-c-death-recycle):
//!   C-slice adds: d0=0.001/tick background death, recycle≈30% of body energy → layer 0.
//!   Effect: recycle boosts substrate availability → pre-bloom plateau RISES; R̄ FALLS (higher pop).
//!
//!   pre-speciation plateau  [t=40 founders → t≈speciation onset]:
//!     N̄(t=4000) = 233 (arm64 probe; x86 TBD via c_calibration_probe CI run).
//!     R̄(t=4000) = 79  (field=651 263 / n_layers=2 / n_cells=4096).
//!   post-speciation equilibrium predicted analytically (economy/01 §5):
//!     N* = P/(U(R*) − recycle·d0·e_cell) ≈ 24 576/(52 − 0.3) ≈ 473
//!     (upper bound; pre-bloom plateau < N* by definition — not yet in steady state).
//!
//! Calibration (C-slice, arm64 c_calibration_probe):
//!   pop=233, field=651 263, R̄=79 (world_dim=64, n_cells=4096, seed=0xA11A_2A11).
//!   x86 values pending CI run — bands set wide to bracket expected arch variance.
//!
//! Band: ±30% of measured plateau, bounded above by N*≈473 / 2 (pre-bloom cannot exceed steady-state).

use cli::{build_sim, default_config};

/// Baked seed — same as the speciation gate; both measure the same trajectory.
const S: u64 = 0xA11A_2A11;

/// Horizon: pre-speciation plateau.
const TICKS: u64 = 4_000;

/// Population floor: N̄(t=TICKS) × 0.70 = 233 × 0.70 ≈ 163.
/// Below this → near-extinction or economy collapse before the speciation bloom can start.
const POP_FLOOR: u64 = 163;

/// Population ceiling: N̄(t=TICKS) × 1.30 = 233 × 1.30 ≈ 303.
/// Below analytic N*≈473 — pre-bloom plateau cannot exceed steady-state carrying capacity.
const POP_CEIL: u64 = 303;

/// R̄ floor: R̄(t=TICKS) × 0.70 = 79 × 0.70 ≈ 55.
/// field_total below n_layers×n_cells×R_FLOOR → substrate severely depleted.
const R_FLOOR: i64 = 55;

/// R̄ ceiling: R̄(t=TICKS) × 1.30 = 79 × 1.30 ≈ 103.
/// Above this → population too low to deplete substrate (economy stalled or too few agents).
const R_CEIL: i64 = 103;

/// Phase-1 L=2 economy corridor (B-4 / issue #157).
///
/// Asserts the L=2 production economy (B-0 generalized build_sim, B-1 Monod uptake,
/// B-2 per-genome metabolic profile + cross-feeding, B-3 proportional rationing) holds its
/// pre-speciation plateau: population alive and bounded, substrate R̄ in the measured band.
/// Catches Km drift or regressions that collapse or explode the population before speciation.
///
/// Range assert (no golden constant) → arch-independent, runs on BOTH CI jobs (x86 corridors
/// and arm64 golden workspace). No arm64 golden re-pin required.
#[test]
fn phase1_economy_corridor() {
    // Skip in debug — 4 000 ticks benefits from release optimisation.
    if cfg!(debug_assertions) {
        return;
    }

    let mut sim = build_sim(default_config(S));
    for _ in 0..TICKS {
        sim.step();
    }

    let pop = sim.population();
    let tel = sim.telemetry();
    let field_total = tel.field_total;

    // R̄ = field_total / n_layers / (world_dim²).
    let n_layers = sim.econ().n_layers as i64;
    let world_dim = sim.econ().world_dim;
    let n_cells = world_dim * world_dim;
    let r_bar = field_total / n_layers / n_cells;

    // Population corridor (C-slice calibration: arm64 measured N̄=233 at t=4000; x86 pending CI probe).
    // Analytic upper bound: N*≈473 (economy/01 §5, d0+recycle). Pre-bloom pop < N* → CEIL=303 safe.
    assert!(
        pop >= POP_FLOOR,
        "population {pop} < floor {POP_FLOOR} at t={TICKS} — extinction or economy collapse \
         (C-slice: N̄=233 arm64 at t={TICKS}; d0≈0.001, recycle≈0.30; analytic N*≈473)"
    );
    assert!(
        pop <= POP_CEIL,
        "population {pop} > ceiling {POP_CEIL} at t={TICKS} — unexpected early bloom \
         (C-slice: N̄=233 arm64; pre-bloom < analytic N*≈473; speciation still pending)"
    );

    // Resource corridor (pre-bloom phase; field depleted faster with higher pop under C-slice).
    // R̄ drops from 79 → ≈23 post-speciation. At t=4000 expect R̄ near calibrated 79.
    assert!(
        r_bar >= R_FLOOR,
        "R̄={r_bar} < floor {R_FLOOR} at t={TICKS} — substrate severely depleted \
         (C-slice: R̄=79 arm64 at t={TICKS}; field_total={field_total}, n_layers={n_layers}, n_cells={n_cells})"
    );
    assert!(
        r_bar <= R_CEIL,
        "R̄={r_bar} > ceiling {R_CEIL} at t={TICKS} — resource not consumed (economy stalled?) \
         (C-slice: R̄=79 arm64; field_total={field_total}; world_dim=64, n_cells={n_cells})"
    );
}
