//! E-F/V-1 differentiation emergence: conservation, determinism, and drift-controlled
//! cell-type specialization guard. Arch-independent integer invariants — run on BOTH CI jobs
//! (x86 + arm64).
//!
//! Phase-2 cell-type differentiation (A vs B) emerges when agents specialize on feeding from
//! different layers under frequency-dependent selection. The GRN spec (V-1 weaker bistable
//! matrix) supports heritable point-mutation of cell-type fate. The emergence guard verifies
//! that when layer 1 is FED (regen_rate=3, world_cap_mult=2), the B-guild (layer-1 feeders)
//! reaches a stable ~39% equilibrium (population ~2700–3000); when layer 1 is BARREN
//! (regen_rate=0, world_cap_mult=0, flat_cap=0), B collapses to ~7–10% (mutation-supply floor).
//! The gap is robust (≥2.5×) across natural drift variance and holds on both x86 and arm64.

use cli::{build_sim, differentiation_config, run};
use sim_core::SimConfig;

const SEED: u64 = 0xBE_EF_5EED;
const TICKS: u64 = 512;

/// R15: energy residual = 0 every tick on differentiation_config — the 2-layer economy
/// conserves energy exactly (substrate feed + fed layer 1 regen, no leaks).
#[test]
fn differentiation_r15_conservation_exact() {
    if cfg!(debug_assertions) {
        return;
    }
    let mut sim = build_sim(differentiation_config(SEED));
    for _ in 0..TICKS {
        sim.step();
        assert_eq!(
            sim.conservation_residual(),
            0,
            "energy not conserved at tick {} on differentiation_config (field regen or uptake leaked)",
            sim.tick()
        );
    }
}

/// E-F acceptance: population does not collapse when layer 1 is fed.
/// Verifies that the 2-layer economy with ontogenesis supports stable coexistence.
#[test]
fn differentiation_no_collapse() {
    if cfg!(debug_assertions) {
        return;
    }
    let mut sim = build_sim(differentiation_config(SEED));
    let mut pop_min = u64::MAX;
    let mut pop_max = 0u64;
    for _ in 0..TICKS {
        sim.step();
        let pop = sim.population();
        pop_min = pop_min.min(pop);
        pop_max = pop_max.max(pop);
    }
    // Viability floor: population stays above a survivable minimum.
    const POP_FLOOR: u64 = 10;
    assert!(
        pop_min >= POP_FLOOR,
        "population collapsed below {POP_FLOOR} on differentiation_config — 2-layer economy broke ecosystem stability"
    );
    // Sanity: population hasn't exploded (catches infinite energy bugs).
    const POP_CEIL: u64 = 100_000;
    assert!(
        pop_max <= POP_CEIL,
        "population exploded to {pop_max} on differentiation_config — conservation or regen logic is broken"
    );
}

/// R14: determinism on differentiation_config (repeated same-seed runs match tick-by-tick).
#[test]
fn differentiation_r14_determinism() {
    if cfg!(debug_assertions) {
        return;
    }
    let a = run(differentiation_config(SEED), TICKS);
    let b = run(differentiation_config(SEED), TICKS);
    for t in 0..TICKS as usize {
        assert_eq!(
            a[t], b[t],
            "differentiation_config non-deterministic at tick {t} — state_hash depends on RNG or thread-order"
        );
    }
}

/// E-F/V-1 emergence: drift-controlled guard that cell-type differentiation is genuinely
/// SELECTED under frequency-dependent niche partitioning, not just drifting. Runs BOTH
/// active (layer 1 FED at regen_rate=3) and inert (layer 1 BARREN at regen_rate=0) twins
/// to t=8000, measures the B-guild fraction via `uptake_layer_histogram(2)`, and asserts
/// fed B-fraction exceeds barren baseline by a robust margin (≥2.5×).
///
/// Calibration (PM local arm64 probes, n=3 seeds, t=8000):
/// - fed_config: B-fraction ≈ 0.39 (seeds a11a2a11/deadbeef/cafebabe: 39.2/39.9/39.2%)
///   population ≈ 2700–3000.
/// - barren_config: B-fraction ≈ 0.08 (seeds same: 7.1/9.5/8.3%)
///   population ≈ 1700 (starved B's, fewer niches).
/// - Gap: ~30 points (0.39 - 0.08). Margin: 2.5× on 0.39 ≈ 0.10 floor ensures the ~5×
///   population difference (fed/barren) and the strong niche-selection signal persist across
///   arch-dependent f32 drift on both x86 and arm64.
#[test]
fn differentiation_measure_emergence() {
    if cfg!(debug_assertions) {
        return;
    }
    const HORIZON: u64 = 8000;
    const MARGIN: f64 = 2.5; // fed_b_frac must exceed barren_b_frac * MARGIN

    // Fed config: layer 1 regenerating at regen_rate=3, world_cap_mult=2.
    let fed_config = differentiation_config(SEED);
    let mut sim_fed = build_sim(fed_config);
    for _ in 0..HORIZON {
        sim_fed.step();
    }
    let hist_fed = sim_fed.uptake_layer_histogram(2);
    let pop_fed = sim_fed.population();
    // hist_fed[0] = count of agents with uptake_layer=0 (type A)
    // hist_fed[1] = count of agents with uptake_layer=1 (type B)
    let b_frac_fed = if pop_fed > 0 {
        hist_fed.get(1).copied().unwrap_or(0) as f64 / pop_fed as f64
    } else {
        0.0
    };

    // Barren config: layer 1 drained (regen_rate=0, world_cap_mult=0, flat_cap=0).
    let mut barren_config = differentiation_config(SEED);
    barren_config.layer_specs[1] = sim_core::LayerSpec {
        regen_rate: 0,
        flux_alpha_num: 1,
        flux_alpha_den: 16, // kept same; irrelevant since no regen
        flat_cap: 0,
        world_cap_mult: 0, // no world-derived caps
    };
    let mut sim_barren = build_sim(barren_config);
    for _ in 0..HORIZON {
        sim_barren.step();
    }
    let hist_barren = sim_barren.uptake_layer_histogram(2);
    let pop_barren = sim_barren.population();
    let b_frac_barren = if pop_barren > 0 {
        hist_barren.get(1).copied().unwrap_or(0) as f64 / pop_barren as f64
    } else {
        0.0
    };

    eprintln!(
        "E-F emergence (t={HORIZON}): fed_b_frac={:.2}% (pop={}, A={}/B={}), barren_b_frac={:.2}% (pop={}, A={}/B={}) → ratio={:.2}× (margin {MARGIN}×)",
        b_frac_fed * 100.0, pop_fed, hist_fed.get(0).copied().unwrap_or(0), hist_fed.get(1).copied().unwrap_or(0),
        b_frac_barren * 100.0, pop_barren, hist_barren.get(0).copied().unwrap_or(0), hist_barren.get(1).copied().unwrap_or(0),
        if b_frac_barren > 0.0 { b_frac_fed / b_frac_barren } else { f64::INFINITY }
    );
    assert!(
        b_frac_fed >= b_frac_barren * MARGIN,
        "cell-type differentiation selection signal lost: fed_b_frac={:.2}% < barren_b_frac={:.2}% × margin={MARGIN}. \
         Niche specialization may have regressed or GRN mutation may be broken. fed: pop={}, B-count={}, barren: pop={}, B-count={}",
        b_frac_fed * 100.0, b_frac_barren * 100.0,
        pop_fed, hist_fed.get(1).copied().unwrap_or(0),
        pop_barren, hist_barren.get(1).copied().unwrap_or(0)
    );
}
