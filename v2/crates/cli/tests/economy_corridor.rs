//! B-4 economy corridor gate (issue #157): long-horizon L=2 production run must land in the
//! measured spatial equilibrium band (range assert, NOT a golden — arch-independent, both CI jobs).
//!
//! IMPORTANT: default_config uses world_dim=64 (4 096 cells). The v2-perf sim-run scenario uses
//! world_dim=128 — a 4× larger world with different dynamics. This test runs default_config, NOT
//! the perf scenario. Calibrate exclusively from `build_sim(default_config(S))` runs.
//!
//! Population dynamics (default_config, seed=0xa11a2a11, **B-3+C+D** on feat/v2-sim-169-d-grn-seed):
//!   D-slice PIVOT (2026-06-28, issue #169): regulated target changed from sense_range to
//!   EXPRESSED UPTAKE LAYER (substrate switching). Agents evolve a threshold rule:
//!   when local layer-0 < reg_setpoint (or > reg_setpoint, direction evolvable), switch to layer-1.
//!   The expressed layer is computed in stage_interactions (stage 6) from the cold uptake_layer's
//!   local field value — transient, derived, never hashed/cached (doc50 §3).
//!
//!   Analytic carrying capacity BASELINE (economy/01 §5, same mean-field as C-slice):
//!     P          = regen_rate × n_cells = 6 × 4096 = 24 576 eu/tick
//!     excrete    = 8 eu/tick per agent (conserved agent→field return)
//!     recycle·d0·e_cell = (77/256)×(1049/1048576)×1000 ≈ 0.301 eu/tick per agent
//!   N*_C = 24576/(U(R*_C) − 8 − 0.301) ≈ 234 at R*_C≈79 (C-slice baseline).
//!
//!   D-slice uptake-layer switching effect: agents can access BOTH layers when local conditions
//!   favor switching → higher effective per-agent uptake → potentially higher N* (or similar).
//!   Direction and magnitude are EMPIRICAL — calibration probe pending. Analytic prediction for
//!   the corridor: N*_D in the same order as N*_C≈234 (switching is a second-order foraging
//!   improvement over a balanced field). Band is PRE-CALIBRATION PLACEHOLDER (doc54 §2):
//!   N*_D ≈ 234 ± 50% → [117, 351]; will be tightened after x86 equilibrium measurement.
//!
//! Band (PLACEHOLDER — pre-calibration, will be tightened after CI probe):
//!   pop: [100, 500] — catches extinction or runaway bloom before calibration numbers arrive.
//!   R̄:  [10, 150]  — catches total substrate collapse or stalled consumption.

use cli::{build_sim, default_config};

/// Baked seed — same as the speciation gate; both measure the same trajectory.
const S: u64 = 0xA11A_2A11;

/// Horizon: pre-speciation plateau.
const TICKS: u64 = 4_000;

/// Population floor: pre-calibration placeholder (catches extinction). Will be tightened after
/// x86 equilibrium measurement with D-pivot (uptake-layer switching) mechanism.
const POP_FLOOR: u64 = 100;

/// Population ceiling: pre-calibration placeholder (catches runaway bloom).
const POP_CEIL: u64 = 500;

/// R̄ floor: pre-calibration placeholder (catches total substrate collapse).
const R_FLOOR: i64 = 10;

/// R̄ ceiling: pre-calibration placeholder (catches zero consumption / stalled economy).
const R_CEIL: i64 = 150;

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

    // Population corridor: D-pivot (uptake-layer switching) — pre-calibration placeholder band.
    // Will be tightened to ±30% of measured N*_D after CI probe returns x86 equilibrium values.
    assert!(
        pop >= POP_FLOOR,
        "population {pop} < floor {POP_FLOOR} at t={TICKS} — extinction or economy collapse \
         (D-pivot: uptake-layer switching, N*_baseline≈234; pre-calibration band; P=24576 eu/tick)"
    );
    assert!(
        pop <= POP_CEIL,
        "population {pop} > ceiling {POP_CEIL} at t={TICKS} — unexpected bloom \
         (D-pivot: uptake-layer switching; pre-calibration placeholder band)"
    );

    // Resource corridor: pre-calibration placeholder, catches severe depletion or stalled economy.
    assert!(
        r_bar >= R_FLOOR,
        "R̄={r_bar} < floor {R_FLOOR} at t={TICKS} — substrate severely depleted \
         (D-pivot; field_total={field_total}, n_layers={n_layers}, n_cells={n_cells})"
    );
    assert!(
        r_bar <= R_CEIL,
        "R̄={r_bar} > ceiling {R_CEIL} at t={TICKS} — resource not consumed (economy stalled?) \
         (D-pivot; field_total={field_total}; world_dim=64, n_cells={n_cells})"
    );
}
