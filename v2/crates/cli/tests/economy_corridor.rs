//! B-4 economy corridor gate (issue #157): long-horizon L=2 production run must land in the
//! measured spatial equilibrium band (range assert, NOT a golden — arch-independent, x86 CI job).
//!
//! Two-phase population dynamics (B-3 on main, seed=0xa11a2a11):
//!   Phase 1 — pre-speciation equilibrium  [t≈1 850 – t≈11 932]
//!     pop-bloom at t≈1 850 (layer-1 consumers appear); single-species plateau N̄≈662–669, R̄≈65.
//!   Phase 2 — multi-species expansion  [t≥11 932]
//!     K first ≥ 3 at tick 11 932; K(16000)=378, pop=4103 (speciation gate calibration).
//!
//! This corridor tests Phase-1 (pre-speciation economy): TICKS=4 000 is firmly in the
//! single-species equilibrium — 2 150 ticks post-bloom, 7 900 ticks before speciation onset.
//! The bounds are arch-independent because Phase-1 dynamics are identical in perf and non-perf
//! modes (speciation has not started yet, so the feature gate has no observable effect).
//!
//! Calibration source: x86 CI sim-run v2-perf (seed=0xa11a2a11, ticks=20000, B-3 main,
//! run #28320700397):  t=4 000: pop=662, field=536295, R̄=65  (perf≡non-perf at t<11 932).
//!
//! Mean-field reference (economy/01 §5): N*≈172, R*≈21.9 at P=100.
//! Spatial N̄=662 ≈ 3.9×N*, R̄=65 ≈ 3.0×R* — cross-feeding and clustering raise capacity.
//! Mean-field numbers are recorded for cross-check only; corridor bounds are x86 spatial values.

use cli::{build_sim, default_config};

/// Baked seed — same as the speciation gate; both measure the same trajectory.
const S: u64 = 0xA11A_2A11;

/// Horizon: post-bloom, pre-speciation equilibrium (Phase 1).
/// Bloom at t≈1 850; speciation onset t≈11 932; TICKS=4 000 is solidly in the plateau.
/// Faster than the 16 000-tick speciation gate — no new large CI-time block added.
const TICKS: u64 = 4_000;

/// Population floor: N̄(x86, t=TICKS) × 0.75 = 662 × 0.75 ≈ 496 → 500.
/// Below this → near-extinction or economy collapse before carrying capacity is reached.
const POP_FLOOR: u64 = 500;

/// Population ceiling: N̄(x86, t=TICKS) × 1.25 = 662 × 1.25 ≈ 827 → 825.
/// Above this → runaway growth regression; single-species phase must be bounded by R.
const POP_CEIL: u64 = 825;

/// R̄ floor: R̄(x86, t=TICKS) × 0.75 = 65 × 0.75 ≈ 48.
/// field_total below n_layers×n_cells×R_FLOOR → substrate nearly depleted (Km regression).
const R_FLOOR: i64 = 48;

/// R̄ ceiling: R̄(x86, t=TICKS) × 1.25 = 65 × 1.25 ≈ 81 → 82.
/// field_total above n_layers×n_cells×R_CEIL → economy not consuming resource.
const R_CEIL: i64 = 82;

/// Phase-1 L=2 economy corridor (B-4 / issue #157).
///
/// Asserts that the L=2 production economy (B-0 generalized build_sim, B-1 Monod uptake,
/// B-2 per-genome metabolic profile + cross-feeding, B-3 proportional rationing) reaches the
/// measured single-species equilibrium: population bounded and non-extinct, substrate R̄ in
/// the measured band. Catches Km drift, regressions that collapse or explode the population,
/// or substrate depletion before the cross-feeding ecosystem has time to stabilise.
///
/// Range assert (no golden constant) → arch-independent, runs on the x86 CI job.
/// No arm64 golden re-pin produced by this PR.
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

    // Population corridor (x86 Phase-1 equilibrium, seed S, t=TICKS, B-3 main, run #28320700397).
    assert!(
        pop >= POP_FLOOR,
        "population {pop} < floor {POP_FLOOR} at t={TICKS} — near-extinction or pre-equilibrium \
         collapse (measured N̄=662 at t={TICKS} x86; mean-field N*≈172 for sanity reference)"
    );
    assert!(
        pop <= POP_CEIL,
        "population {pop} > ceiling {POP_CEIL} at t={TICKS} — runaway growth regression \
         (single-species phase bounded by R; measured N̄=662 at calibration)"
    );

    // Resource corridor (spatial R̄=65 at calibration; mean-field R*≈21.9 for sanity reference).
    assert!(
        r_bar >= R_FLOOR,
        "R̄={r_bar} < floor {R_FLOOR} at t={TICKS} — substrate depleted below minimum \
         (field_total={field_total}, n_layers={n_layers}, n_cells={n_cells}; measured R̄=65)"
    );
    assert!(
        r_bar <= R_CEIL,
        "R̄={r_bar} > ceiling {R_CEIL} at t={TICKS} — resource not consumed (economy broken?) \
         (field_total={field_total}; measured R̄=65 at calibration, run #28320700397)"
    );
}
