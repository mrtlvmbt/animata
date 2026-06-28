//! B-4 economy corridor gate (issue #157): long-horizon L=2 production run must land in the
//! measured spatial equilibrium band (range assert, NOT a golden — arch-independent, both CI jobs).
//!
//! IMPORTANT: default_config uses world_dim=64 (4 096 cells). The v2-perf sim-run scenario uses
//! world_dim=128 — a 4× larger world with different dynamics. This test runs default_config, NOT
//! the perf scenario. Calibrate exclusively from `build_sim(default_config(S))` runs.
//!
//! Population dynamics (default_config, seed=0xa11a2a11, **B-3+C** on feat/v2-sim-167-c-death-recycle):
//!   C-slice adds: d0=0.001/tick background death, recycle≈30% of body energy → layer 0.
//!
//!   Analytic carrying capacity at measured R̄=79 (economy/01 §5, mean-field):
//!     P          = regen_rate × n_cells = 6 × 4096 = 24 576 eu/tick
//!     U(R̄=79)   = u_max × R̄/(R̄+km) = 220×79/(79+74) = 17 380/153 ≈ 113.6 eu/tick per agent
//!     recycle·d0·e_cell = (77/256)×(1049/1048576)×1000 ≈ 0.301 eu/tick per agent
//!     N*_C       = P / (U(R̄) − recycle·d0·e_cell) = 24 576/(113.6−0.301) ≈ 24 576/113.3 ≈ 217
//!
//!   B-3 baseline (no C-slice, R̄=379): N*_B3 = 24 576/(220×379/(379+74)) ≈ 24 576/184 ≈ 134.
//!   C-slice +91% jump (122→233): recycle returns substrate → higher effective production → equilibrium
//!   shifts from R̄=379 (N*≈134) to R̄=79 (N*≈217). Measured 233 is ~7% above analytic 217 because
//!   field is still accumulating (sub-ceiling cells present; consistent with pre-bloom transient).
//!
//!   Measured at t=4000 (arm64 + x86, both seeds=0xA11A_2A11; integer-dominated, arch-stable):
//!     N̄ = 233, R̄ = 79 (field=651 263 / n_layers=2 / n_cells=4096).
//!
//! Band: analytic N*_C ≈ 217 ± 30% → [152, 282]. Measured 233 ∈ [152, 282] ✓.

use cli::{build_sim, default_config};

/// Baked seed — same as the speciation gate; both measure the same trajectory.
const S: u64 = 0xA11A_2A11;

/// Horizon: pre-speciation plateau.
const TICKS: u64 = 4_000;

/// Population floor: analytic N*_C × 0.70 = 217 × 0.70 ≈ 152.
/// Below this → near-extinction or economy collapse before the speciation bloom can start.
const POP_FLOOR: u64 = 152;

/// Population ceiling: analytic N*_C × 1.30 = 217 × 1.30 ≈ 282.
/// Measured 233 lands inside [152, 282] (field still accumulating; 7% above N*_C is expected).
const POP_CEIL: u64 = 282;

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

    // Population corridor anchored to analytic N*_C≈217 (at measured R̄=79, C-slice economy).
    // ±30% band: [152, 282]. Measured N̄=233 (arm64+x86) is ~7% above N*_C — expected (pre-bloom).
    assert!(
        pop >= POP_FLOOR,
        "population {pop} < floor {POP_FLOOR} at t={TICKS} — extinction or economy collapse \
         (analytic N*_C≈217 at R̄=79; d0≈0.001, recycle≈0.30; P=24576 eu/tick)"
    );
    assert!(
        pop <= POP_CEIL,
        "population {pop} > ceiling {POP_CEIL} at t={TICKS} — unexpected early bloom or recycle runaway \
         (analytic N*_C≈217 at R̄=79; band ±30%; measured N̄=233 at t=4000)"
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
