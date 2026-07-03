//! B-4 economy corridor gate (issue #157): long-horizon L=2 production run must land in the
//! measured spatial equilibrium band (range assert, NOT a golden — arch-independent, both CI jobs).
//!
//! IMPORTANT: default_config uses world_dim=64 (4 096 cells). The v2-perf sim-run scenario uses
//! world_dim=128 — a 4× larger world with different dynamics. This test runs default_config, NOT
//! the perf scenario. Calibrate exclusively from `build_sim(default_config(S))` runs.
//!
//! Population dynamics (default_config, seed=0xa11a2a11, **ProcgenWorld** on feat/v2-sim-221-w6-wire-procgen-world):
//!   W-6b re-banded for ProcgenWorld (2026-07-03): prior NoiseWorld band pop≤282, R̄∈[55,103] was a
//!   smooth-gradient artifact. ProcgenWorld's patchy caps (~0.52× NoiseWorld mean) select cheaper
//!   metabolism → more agents per unit energy → legitimately higher equilibrium.
//!
//!   Analytic carrying capacity at measured R̄≈46 (ProcgenWorld config):
//!     P          = regen_rate × n_cells = 6 × 4096 = 24 576 eu/tick (unchanged)
//!     U(R̄=46)   = u_max × R̄/(R̄+km) = 220×46/(46+74) = 10 120/120 ≈ 84.3 eu/tick per agent
//!     recycle·d0·e_cell = (77/256)×(1049/1048576)×1000 ≈ 0.301 eu/tick per agent (unchanged)
//!     N*_C       = P / (U(R̄) − recycle·d0·e_cell) = 24 576/(84.3−0.301) ≈ 24 576/84 ≈ 1746
//!
//!   Measured at t=4000 (arm64 + x86, both seeds=0xA11A_2A11; integer-dominated, arch-stable):
//!     N̄ = 1790, R̄ = 46 (ProcgenWorld equilibrium, multi-seed robust ±60 agents, all arches).
//!
//! Band: measured N̄ = 1790 ± 30% → [1253, 2327]. Measured R̄ = 46 ± 30% → [32, 60].

use cli::{build_sim, default_config};

/// Baked seed — same as the speciation gate; both measure the same trajectory.
const S: u64 = 0xA11A_2A11;

/// Horizon: pre-speciation plateau.
const TICKS: u64 = 4_000;

/// Population floor: measured equilibrium 1790 × 0.70 ≈ 1253 (ProcgenWorld).
/// Below this → near-extinction or economy collapse before the speciation bloom can start.
const POP_FLOOR: u64 = 1253;

/// Population ceiling: measured equilibrium 1790 × 1.30 ≈ 2327 (ProcgenWorld).
/// Multi-seed robust across arches; field may still accumulate early.
const POP_CEIL: u64 = 2327;

/// R̄ floor: measured equilibrium 46 × 0.70 ≈ 32 (ProcgenWorld).
/// field_total below n_layers×n_cells×R_FLOOR → substrate severely depleted.
const R_FLOOR: i64 = 32;

/// R̄ ceiling: measured equilibrium 46 × 1.30 ≈ 60 (ProcgenWorld).
/// Above this → population too low to deplete substrate (economy stalled or too few agents).
const R_CEIL: i64 = 60;

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

    // Population corridor anchored to measured ProcgenWorld equilibrium N̄=1790 (at R̄≈46).
    // ±30% band: [1253, 2327]. Multi-seed robust; integer-dominated dynamics arch-independent.
    assert!(
        pop >= POP_FLOOR,
        "population {pop} < floor {POP_FLOOR} at t={TICKS} — extinction or economy collapse \
         (ProcgenWorld equilibrium N̄≈1790 at R̄≈46; band ±30%; P=24576 eu/tick)"
    );
    assert!(
        pop <= POP_CEIL,
        "population {pop} > ceiling {POP_CEIL} at t={TICKS} — unexpected early bloom or recycle runaway \
         (ProcgenWorld equilibrium N̄≈1790 at R̄≈46; band ±30%; measured at t=4000)"
    );

    // Resource corridor (pre-bloom phase; field depleted faster with higher pop under ProcgenWorld).
    // R̄ stabilises at ≈46 in equilibrium. At t=4000 expect R̄ near calibrated 46.
    assert!(
        r_bar >= R_FLOOR,
        "R̄={r_bar} < floor {R_FLOOR} at t={TICKS} — substrate severely depleted \
         (ProcgenWorld: R̄≈46 at t={TICKS}; field_total={field_total}, n_layers={n_layers}, n_cells={n_cells})"
    );
    assert!(
        r_bar <= R_CEIL,
        "R̄={r_bar} > ceiling {R_CEIL} at t={TICKS} — resource not consumed (economy stalled?) \
         (ProcgenWorld: R̄≈46; field_total={field_total}; world_dim=64, n_cells={n_cells})"
    );
}
