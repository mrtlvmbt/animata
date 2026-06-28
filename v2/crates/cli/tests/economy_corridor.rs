//! B-4 economy corridor gate (issue #157): long-horizon L=2 production run must land in the
//! measured spatial equilibrium band (range assert, NOT a golden — arch-independent, both CI jobs).
//!
//! IMPORTANT: default_config uses world_dim=64 (4 096 cells). The v2-perf sim-run scenario uses
//! world_dim=128 — a 4× larger world with different dynamics. This test runs default_config, NOT
//! the perf scenario. Calibrate exclusively from `build_sim(default_config(S))` runs.
//!
//! Population dynamics (default_config, seed=0xa11a2a11, **B-3+C+D** on feat/v2-sim-169-d-grn-seed):
//!   D-slice adds: evolvable sensing regulation (`reg_gain`, `reg_setpoint`). Agents that evolve
//!   negative `reg_gain` sense farther in low-substrate patches (lr < setpoint ≈ 80), improving
//!   spatial foraging (preferential patch selection → higher effective uptake per agent). Result:
//!   the equilibrium population RISES and the equilibrium substrate FALLS compared to C-slice alone.
//!
//!   Analytic carrying capacity (economy/01 §5, field balance: production = net field drain):
//!     P          = regen_rate × n_cells = 6 × 4096 = 24 576 eu/tick
//!     excrete    = 8 eu/tick per agent (conserved agent→field return)
//!     recycle·d0·e_cell = (77/256)×(1049/1048576)×1000 ≈ 0.301 eu/tick per agent
//!     N*_D       = P / (U(R*_D) − excrete − recycle·d0·e_cell)
//!
//!   D-slice equilibrium substrate R*_D: regulation-improved foraging depletes substrate further
//!   than C-slice. At N*_D≈302 the field balance gives:
//!     U(R*_D) = P/N*_D + excrete + recycle·d0·e_cell = 24576/302 + 8 + 0.301 ≈ 89.8 eu/tick
//!     R*_D = U⁻¹(89.8) = 89.8×km/(u_max − 89.8) = 89.8×74/(220−89.8) ≈ 6655/130 ≈ 51
//!     N*_D = 24 576 / (89.8 − 8 − 0.301) = 24 576 / 81.5 ≈ 302
//!
//!   C-slice calibration anchor (now subsumed by D): N*_C at R*_C=79 = 24576/(113.6−8−0.301) ≈ 234,
//!   measured 233 (within rounding). D-slice shifts equilibrium to R*_D≈51, N*_D≈302: regulation
//!   increases foraging efficiency → larger population depletes substrate further (R* falls from 79
//!   to 51). At t=4000 the system is near the D-slice equilibrium (pop=300 ≈ N*_D=302, within 1%).
//!
//!   Measured at t=4000 (arm64 + x86, both seeds=0xA11A_2A11; integer-dominated, arch-stable):
//!     N̄ = 300, R̄ = 77 (approaching R*_D=51; reg_gain evolved to [−2,+2]).
//!
//! Band: analytic N*_D ≈ 302 ± 30% → [211, 392]. Measured 300 ∈ [211, 392] ✓ (99% of N*_D).

use cli::{build_sim, default_config};

/// Baked seed — same as the speciation gate; both measure the same trajectory.
const S: u64 = 0xA11A_2A11;

/// Horizon: pre-speciation plateau.
const TICKS: u64 = 4_000;

/// Population floor: analytic N*_D × 0.70 = 302 × 0.70 ≈ 211.
/// Below this → near-extinction or economy collapse (D-slice raises equilibrium from C-slice's N*_C≈234).
const POP_FLOOR: u64 = 211;

/// Population ceiling: analytic N*_D × 1.30 = 302 × 1.30 ≈ 392.
/// Measured 300 lands inside [211, 392] (within 1% of N*_D=302 — system near D-slice equilibrium).
const POP_CEIL: u64 = 392;

/// R̄ floor: R̄_D(t=TICKS) × 0.70 = 77 × 0.70 ≈ 53.
/// field_total below n_layers×n_cells×R_FLOOR → substrate severely depleted.
const R_FLOOR: i64 = 53;

/// R̄ ceiling: R̄_D(t=TICKS) × 1.30 = 77 × 1.30 ≈ 100.
/// Above this → population too low to deplete substrate (economy stalled or too few agents).
const R_CEIL: i64 = 100;

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

    // Population corridor anchored to analytic N*_D≈302 (D-slice equilibrium at R*_D≈51).
    // ±30% band: [211, 392]. Measured N̄=300 (arm64+x86) is 99% of N*_D — near equilibrium.
    assert!(
        pop >= POP_FLOOR,
        "population {pop} < floor {POP_FLOOR} at t={TICKS} — extinction or economy collapse \
         (D-slice: N*_D≈302 at R*_D≈51; regulation-enhanced foraging; P=24576 eu/tick)"
    );
    assert!(
        pop <= POP_CEIL,
        "population {pop} > ceiling {POP_CEIL} at t={TICKS} — unexpected bloom \
         (D-slice: N*_D≈302 at R*_D≈51; band ±30%; measured N̄=300 at t=4000)"
    );

    // Resource corridor (D-slice equilibrium R*_D≈51; at t=4000 R̄=77 approaching R*_D from above).
    assert!(
        r_bar >= R_FLOOR,
        "R̄={r_bar} < floor {R_FLOOR} at t={TICKS} — substrate severely depleted \
         (D-slice: R̄_D=77 arm64 at t={TICKS}; field_total={field_total}, n_layers={n_layers}, n_cells={n_cells})"
    );
    assert!(
        r_bar <= R_CEIL,
        "R̄={r_bar} > ceiling {R_CEIL} at t={TICKS} — resource not consumed (economy stalled?) \
         (D-slice: R̄_D=77 arm64; field_total={field_total}; world_dim=64, n_cells={n_cells})"
    );
}
