//! B-4 economy corridor gate (issue #157): long-horizon L=2 production run must land in the
//! measured spatial equilibrium band (range assert, NOT a golden — arch-independent, both CI jobs).
//!
//! IMPORTANT: default_config uses world_dim=64 (4 096 cells). The v2-perf sim-run scenario uses
//! world_dim=128 — a 4× larger world with different dynamics. This test runs default_config, NOT
//! the perf scenario. Calibrate exclusively from `build_sim(default_config(S))` runs.
//!
//! Population dynamics (default_config, seed=0xa11a2a11, B-3 on main):
//!   pre-speciation plateau  [t=40 founders → t≈11 932 speciation onset]:
//!     N̄(t=4000) = 122 (arm64 + x86 probe; integer-dominated, arch-independent).
//!     R̄(t=4000) = 379 (field=3 109 683 / n_layers=2 / n_cells=4096; field still accumulating).
//!   post-speciation equilibrium  [t≥11 932]:
//!     K first ≥ 3 at tick 11 932 → K(16000)=378, pop=4103, R̄=23 (speciation gate calibration).
//!
//! This corridor tests the PRE-SPECIATION plateau (t=4 000): population must be bounded and
//! alive, substrate R̄ must be in the measured accumulation band. Catches economy regressions
//! that collapse population before the speciation bloom can start.
//!
//! Calibration: b4_calibration_probe_4000 (arm64 local + x86 CI branch probe):
//!   pop=122, field=3 109 683, R̄=379 (world_dim=64, n_cells=4096).
//! Band: ±30% of measured plateau (generous, tightenable once stable across more seeds).
//!
//! Mean-field reference (economy/01 §5): N*≈172, R*≈21.9 at P=100 (chemostat, single-layer).
//! Not used for calibration; recorded for cross-check only.

use cli::{build_sim, default_config};

/// Baked seed — same as the speciation gate; both measure the same trajectory.
const S: u64 = 0xA11A_2A11;

/// Horizon: pre-speciation plateau. Speciation onset t≈11 932; t=4 000 is well inside the
/// stable pre-bloom regime. Test runs in seconds (vs 16 000-tick speciation gate).
const TICKS: u64 = 4_000;

/// Population floor: N̄(t=TICKS) × 0.70 = 122 × 0.70 ≈ 85.
/// Below this → near-extinction or economy collapse before the speciation bloom can start.
const POP_FLOOR: u64 = 85;

/// Population ceiling: N̄(t=TICKS) × 1.31 = 122 × 1.31 ≈ 160.
/// Above this → early bloom regression; pre-speciation plateau must stay bounded.
/// Matches B-3 corridor CEIL=160 (same plateau, different horizon).
const POP_CEIL: u64 = 160;

/// R̄ floor: R̄(t=TICKS) × 0.70 = 379 × 0.70 ≈ 265.
/// field_total below n_layers×n_cells×R_FLOOR → substrate severely depleted (Km regression).
const R_FLOOR: i64 = 265;

/// R̄ ceiling: R̄(t=TICKS) × 1.30 = 379 × 1.30 ≈ 492 → 495.
/// field_total above this → no resource consumption detected (economy stalled).
const R_CEIL: i64 = 495;

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

    // Population corridor (measured pre-speciation plateau, seed S, t=TICKS, B-3 main, arm64+x86).
    assert!(
        pop >= POP_FLOOR,
        "population {pop} < floor {POP_FLOOR} at t={TICKS} — extinction or collapse before speciation \
         (measured N̄=122 at t={TICKS}; founders=40; mean-field N*≈172 for sanity reference)"
    );
    assert!(
        pop <= POP_CEIL,
        "population {pop} > ceiling {POP_CEIL} at t={TICKS} — early-bloom regression \
         (pre-speciation plateau bounded; measured N̄=122; speciation onset expected t≈11 932)"
    );

    // Resource corridor (pre-bloom accumulation phase; R̄ drops to ≈23 post-speciation at t=16000).
    assert!(
        r_bar >= R_FLOOR,
        "R̄={r_bar} < floor {R_FLOOR} at t={TICKS} — substrate severely depleted in pre-bloom phase \
         (field_total={field_total}, n_layers={n_layers}, n_cells={n_cells}; measured R̄=379)"
    );
    assert!(
        r_bar <= R_CEIL,
        "R̄={r_bar} > ceiling {R_CEIL} at t={TICKS} — no resource consumed (economy stalled?) \
         (field_total={field_total}; measured R̄=379 at calibration, world_dim=64)"
    );
}
