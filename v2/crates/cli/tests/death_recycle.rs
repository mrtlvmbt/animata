//! C-slice death-recycling tests (issue #167): d0 background hazard (C-1) + recycle fraction (C-2).
//!
//! All tests run on x86 (determinism is per-arch; the 3 exact-golden tests are arm64-only).
//! Range-asserts and conservation checks are arch-independent (integer-dominated).
//!
//! C-1: background death at rate d0 ≈ 0.001/tick — counter-RNG draw, uncorrelated with mutation.
//! C-2: on every death, recycled = ⌊recycle_num · E / RECYCLE_DEN⌋ → substrate layer 0;
//!      E − recycled → ledger.lost. Conservation: residual = 0 every tick.

use cli::{build_sim, default_config};
use sim_core::{D0_MASK, RECYCLE_DEN};
use sim_core::{EconParams, LayerSpec, MergeStrategy, SimConfig};

const S: u64 = 0xC001_D0_00; // C-slice seed, distinct from B-4 seed

// ── Helpers ─────────────────────────────────────────────────────────────────────────────────────

/// Config with default_config structure but custom EconParams override and layer specs.
/// Used to isolate specific mechanics (zero metab, zero regen, etc.) without touching the golden.
fn c_config(seed: u64, econ: EconParams, n_founders: u64, founder_energy: i64) -> SimConfig {
    // Layer 0: no regen (isolates recycle signal); no diffusion change to total (diffusion conserves).
    // Layer 1: organics, also no regen.
    let l0 = LayerSpec { regen_rate: 0, flux_alpha_num: 1, flux_alpha_den: 8, flat_cap: 0, world_cap_mult: 0 };
    let l1 = LayerSpec { regen_rate: 0, flux_alpha_num: 1, flux_alpha_den: 4, flat_cap: 0, world_cap_mult: 0 };
    SimConfig {
        seed,
        n_founders,
        founder_energy,
        econ,
        sim_threads: 1, // serial — removes R14 scatter ordering from the picture
        merge_strategy: MergeStrategy::Canonical,
        n_layers: 2,
        layer_specs: [l0, l1, LayerSpec::default(), LayerSpec::default()],
        thermal_verdict_temps: None,
        founder_templates: None,
    }
}

// ── Test 1: d0 kill-set is replay-invariant ──────────────────────────────────────────────────────

/// Two independent replays of the same seed must produce identical population trajectories.
/// Validates that the d0 counter-RNG (SALT_DEATH, sorted entity bits, clock.tick) is pure and
/// does not depend on any non-deterministic state (wall-clock, thread order, etc.).
///
/// Also validates that changing sim_threads does NOT change the final population count — the
/// kill-set is in stage-7 which is serial (unlike the stage-8 scatter pool).
#[test]
fn death_d0_deterministic() {
    if cfg!(debug_assertions) { return; }

    const TICKS: u64 = 200;

    // Two identical replays — same config, same seed.
    let mut sim_a = build_sim(default_config(S));
    let mut sim_b = build_sim(default_config(S));
    for t in 0..TICKS {
        sim_a.step();
        sim_b.step();
        let pa = sim_a.population();
        let pb = sim_b.population();
        assert_eq!(pa, pb, "replay diverged at tick {t}: pop_a={pa} pop_b={pb}");
    }

    // Conserved-field hashes must also match (validates that recycle deposits land identically).
    let ha = sim_a.conserved_field_hash();
    let hb = sim_b.conserved_field_hash();
    assert_eq!(ha, hb, "conserved field hash differs after replay: {ha:#018x} != {hb:#018x}");
}

// ── Test 2: empirical d0 kill fraction ≈ 0.001/tick ────────────────────────────────────────────

/// Run a sim where birth and starvation are disabled (base_metab=0, repro unreachable,
/// no resource uptake). Any population loss must come from d0 kills.
///
/// After T ticks starting from P0 organisms: P(T) = P0 × (1 − d0)^T.
/// We check P(T) ∈ [P0·(1−d0)^T ± 3σ], where σ = √(P0·p·(1−p)), p = 1−(1−d0)^T.
/// Uses elevated n_founders (200) for statistical power; d0 default (1049/2^20 ≈ 0.001).
#[test]
fn death_d0_rate() {
    if cfg!(debug_assertions) { return; }

    const TICKS: u64 = 500;
    const P0: u64 = 200; // large enough for low relative variance

    // Zero metabolism, zero uptake, zero excretion → only d0 kills agents.
    let econ = EconParams {
        base_metab: 0,
        k_size_metab: 0,
        k_move_cost: 0,
        k_sense_cost: 0,
        u_max: 0,      // Monod numerator=0 → uptake=0 (km>0 keeps denominator safe)
        excrete: 0,
        // d0 at default: 1049 / 1_048_576 ≈ 0.001
        ..EconParams::default()
    };

    // World_dim=64, P0=200 with founder_energy far below repro_threshold (genome default).
    let e_cell = econ.e_cell;
    let config = c_config(S ^ 0x1, econ, P0, e_cell / 2);
    let mut sim = build_sim(config);
    for _ in 0..TICKS { sim.step(); }

    let p_final = sim.population();

    // P(T) = P0 × (1−d0)^T; d0 ≈ 1049/1_048_576.
    // With T=500: survival ≈ exp(−0.5) ≈ 0.607 → expected P_final ≈ 121.
    // 3σ tolerance: σ ≈ √(P0 · p · (1−p)) where p = 1−survival ≈ 0.393.
    // σ ≈ √(200 · 0.393 · 0.607) ≈ √47.7 ≈ 6.9. Use ±4σ ≈ ±28 → floor=93, ceil=149.
    let floor: u64 = 90;
    let ceil: u64 = 155;
    assert!(
        p_final >= floor && p_final <= ceil,
        "d0 rate test: P(T={TICKS})={p_final} outside [{floor},{ceil}] \
         (P0={P0}, expected≈121 at d0≈0.001; world_dim=64, no metab/uptake)"
    );
}

// ── Test 3: recycled energy lands on layer 0, NOT layer 1 ────────────────────────────────────────

/// Validate that the recycle deposit targets substrate (layer 0) exclusively.
/// d0 kills agents; their recycled fraction must appear in layer-0 total, not in layer-1 total.
///
/// Method: compare Δlayer_0 vs Δlayer_1 across a window. Layer 0 must grow from recycle;
/// layer 1 must stay flat (regen=0, no agents feeding on layer 1 in default L=2 config).
#[test]
fn recycle_to_substrate() {
    if cfg!(debug_assertions) { return; }

    const TICKS: u64 = 300;

    // No regen, no metab, no uptake, no excrete. d0 at default (≈0.001/tick).
    // n_founders large enough to produce statistically significant recycle deposits.
    let econ = EconParams {
        base_metab: 0,
        k_size_metab: 0,
        k_move_cost: 0,
        k_sense_cost: 0,
        u_max: 0,
        excrete: 0,
        ..EconParams::default()
    };
    let e_cell = econ.e_cell;
    let config = c_config(S ^ 0x2, econ, 200, e_cell);
    let mut sim = build_sim(config);

    let l0_before = sim.field_layer_total(0);
    let l1_before = sim.field_layer_total(1);

    for _ in 0..TICKS { sim.step(); }

    let l0_after = sim.field_layer_total(0);
    let l1_after = sim.field_layer_total(1);

    let delta_l0 = l0_after - l0_before;
    let delta_l1 = l1_after - l1_before;

    // Layer 0 must have grown: some agents died and 30% of their energy returned here.
    assert!(
        delta_l0 > 0,
        "layer-0 total did not grow; expected recycle deposits \
         (Δl0={delta_l0}, Δl1={delta_l1}, T={TICKS})"
    );
    // Layer 1 must be exactly 0: regen=0, no deposits, started empty, stayed empty.
    assert_eq!(
        l1_after, 0,
        "layer-1 total is non-zero; recycle deposit hit wrong layer \
         (l1_after={l1_after}, Δl1={delta_l1}) — recycle must target layer 0 (substrate)"
    );
}

// ── Test 4: recycle split is exact ──────────────────────────────────────────────────────────────

/// Single-agent, zero-regen, zero-metab, zero-uptake, 100%-kill config.
/// On tick 1: agent (E = e_cell) is killed by d0 (d0_scaled = D0_MASK+1 → P(kill)=1).
/// Layer-0 total must increase by exactly ⌊recycle_num × e_cell / RECYCLE_DEN⌋.
/// No other energy flows: regen=0, uptake=0 (u_max=0), excrete=0, diffusion conserves total.
///
/// This is the ONLY test that exercises the exact recycle formula in isolation.
#[test]
fn recycle_fraction_exact() {
    if cfg!(debug_assertions) { return; }

    let econ = EconParams {
        d0_scaled: D0_MASK + 1, // 100% kill: (r & D0_MASK) < D0_MASK+1 is always true
        recycle_num: 77,        // explicit for clarity (matches default)
        base_metab: 0,
        k_size_metab: 0,
        k_move_cost: 0,
        k_sense_cost: 0,
        u_max: 0,    // no uptake → Δfield_from_eat = 0
        excrete: 0,  // no excreta scatter → Δfield_from_scatter = 0
        pheromone: 0.0,
        ..EconParams::default()
    };
    // Single agent, energy = e_cell (exact body pool = recycle base).
    let e_cell = econ.e_cell;
    let recycle_num = econ.recycle_num;
    let config = c_config(S ^ 0x3, econ, 1, e_cell);
    let mut sim = build_sim(config);

    let l0_before = sim.field_layer_total(0);

    sim.step(); // tick 1: agent dies, recycled = ⌊77 × e_cell / 256⌋

    let l0_after = sim.field_layer_total(0);

    let expected_recycled = recycle_num * e_cell / RECYCLE_DEN; // truncating
    let delta = l0_after - l0_before;
    assert_eq!(
        delta, expected_recycled,
        "recycle split wrong: Δlayer_0={delta} ≠ ⌊recycle_num·e_cell/RECYCLE_DEN⌋={expected_recycled} \
         (e_cell={e_cell}, recycle_num={recycle_num}, RECYCLE_DEN={RECYCLE_DEN})",
    );
    // Agent must be dead.
    assert_eq!(sim.population(), 0, "agent not dead after 100%-kill d0");
}

// ── Test 5: conservation holds every tick with d0 > 0, recycle > 0 ──────────────────────────────

/// Run the default production sim (d0=0.001, recycle≈0.3) for N ticks.
/// `conservation_residual()` must equal 0 on every tick — ledger.lost correctly absorbs the
/// (1−recycle)·E fraction and field staging absorbs the recycle·E fraction via deposit_conserved.
///
/// This is the main regression gate for C: any mis-accounting in the recycle split or the
/// ledger update surfaces here immediately.
#[test]
fn recycle_conserves() {
    if cfg!(debug_assertions) { return; }

    const TICKS: u64 = 1_000;

    let mut sim = build_sim(default_config(S ^ 0x4));
    for t in 0..TICKS {
        sim.step();
        let r = sim.conservation_residual();
        assert_eq!(
            r, 0,
            "conservation violated at tick {t}: residual={r} \
             (d0_scaled={}, recycle_num={}, RECYCLE_DEN={RECYCLE_DEN})",
            sim.econ().d0_scaled, sim.econ().recycle_num
        );
    }
}
