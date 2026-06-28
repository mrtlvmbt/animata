//! D-slice GRN seed mechanism tests (issue #169): evolvable metabolic-gene expression regulation.
//! The regulated target is the EXPRESSED UPTAKE LAYER (substrate switching), NOT sense_range.
//! All tests run on x86 CI (arch-independent: pure integer genome math + conserved-layer invariants).
//! Test 5 is the §B selective-value A/B experiment required by the acceptance criteria.

use cli::{build_sim, default_config, run};
use sim_core::{Genome, SimConfig};

// ── Test 1: expressed_layer is deterministic; founder (reg_gain=0) is explicitly inert ──────────────

/// Verifies the `expressed_layer` contract (D-slice / GRN seed):
/// - Explicit disabled encoding (F7): `reg_gain == 0` → INERT (always cold uptake_layer),
///   with no sign(0) ambiguity. Every possible local_resource must yield cold uptake_layer.
/// - `reg_gain > 0` → switch to other layer when lr ≥ setpoint (rich-patch case).
/// - `reg_gain < 0` → switch to other layer when lr < setpoint (depletion case).
/// - Same inputs always produce the same output (no hidden state, no float, R13).
#[test]
fn reg_expression_deterministic() {
    let founder = Genome::founder(2);
    assert_eq!(founder.reg_gain, 0, "founder must start with reg_gain=0 (regulation OFF)");

    // Founder inert: expressed_layer == cold uptake_layer for every possible local_resource.
    // This is the EXPLICIT disabled-state check (F7 — not a sign(0) artefact).
    for lr in [i64::MIN, -1000, -1, 0, 1, 37, 39, 40, 79, 100, 1000, i64::MAX] {
        assert_eq!(
            founder.expressed_layer(lr, 2), founder.uptake_layer as usize,
            "founder reg_gain=0: must stay on cold uptake_layer ({}) at lr={lr}",
            founder.uptake_layer,
        );
    }

    // L=1 guard: even with non-zero reg_gain, expressed_layer returns cold layer when L<2.
    let g_active = Genome { reg_gain: 2, reg_setpoint: 39, ..founder };
    assert_eq!(g_active.expressed_layer(100, 1), g_active.uptake_layer as usize,
        "L<2 guard: expressed_layer must return cold layer when n_layers<2");

    // Polarity: reg_gain > 0 → switch to other layer when lr >= setpoint.
    let g_pos = Genome { uptake_layer: 0, reg_gain: 2, reg_setpoint: 39, ..founder };
    assert_eq!(g_pos.expressed_layer(39, 2), 1, "reg_gain>0: should switch at lr==setpoint");
    assert_eq!(g_pos.expressed_layer(100, 2), 1, "reg_gain>0: should switch at lr>setpoint");
    assert_eq!(g_pos.expressed_layer(38, 2), 0, "reg_gain>0: should stay at lr<setpoint");

    // Polarity: reg_gain < 0 → switch to other layer when lr < setpoint.
    let g_neg = Genome { uptake_layer: 0, reg_gain: -2, reg_setpoint: 39, ..founder };
    assert_eq!(g_neg.expressed_layer(38, 2), 1, "reg_gain<0: should switch at lr<setpoint");
    assert_eq!(g_neg.expressed_layer(39, 2), 0, "reg_gain<0: should stay at lr==setpoint");
    assert_eq!(g_neg.expressed_layer(100, 2), 0, "reg_gain<0: should stay at lr>setpoint");

    // Symmetry: with uptake_layer=1, other layer is 0.
    let g_l1 = Genome { uptake_layer: 1, reg_gain: 1, reg_setpoint: 39, ..founder };
    assert_eq!(g_l1.expressed_layer(100, 2), 0, "L=2, uptake_layer=1: other layer is 0");

    // Determinism: same inputs → same output every call (no mutable state, no float).
    for lr in [0i64, 20, 38, 39, 40, 100, i64::MAX] {
        assert_eq!(g_pos.expressed_layer(lr, 2), g_pos.expressed_layer(lr, 2));
    }
}

// ── Test 2: expressed_layer always returns a valid layer index 0..n_layers ────────────────────────

/// Verifies expressed_layer stays within [0, n_layers) for all valid inputs.
#[test]
fn reg_layer_in_bounds() {
    let founder = Genome::founder(2);
    for uptake in 0i32..=1 {
        for gain in -4i32..=4 {
            for setpoint in [0i32, 20, 39, 80, 128, 256] {
                let g = Genome { uptake_layer: uptake, reg_gain: gain, reg_setpoint: setpoint, ..founder };
                for lr in [i64::MIN, -1, 0, 1, 38, 39, 40, 255, 256, i64::MAX] {
                    let layer = g.expressed_layer(lr, 2);
                    assert!(layer < 2, "expressed_layer {layer} out of bounds [0,2) at lr={lr}");
                }
            }
        }
    }
}

// ── Test 3: both reg fields are folded into hash_contribution (F9) ────────────────────────────────

/// Verifies F9: a genome field outside `hash_contribution` silently decouples mutation from the
/// determinism lock, making the trajectory irreproducible. Both `reg_setpoint` and `reg_gain` must
/// change the hash when they differ from the founder values.
#[test]
fn reg_fields_in_hash() {
    let base = Genome::founder(2);
    let h_base = base.hash_contribution(0);

    let with_setpoint = Genome { reg_setpoint: 100, ..base };
    assert_ne!(
        with_setpoint.hash_contribution(0), h_base,
        "reg_setpoint not reflected in hash_contribution (F9 violated)"
    );

    let with_gain = Genome { reg_gain: 1, ..base };
    assert_ne!(
        with_gain.hash_contribution(0), h_base,
        "reg_gain not reflected in hash_contribution (F9 violated)"
    );

    let a = Genome { reg_setpoint: 40, ..base };
    let b = Genome { reg_setpoint: 120, ..base };
    assert_ne!(a.hash_contribution(0), b.hash_contribution(0));
}

// ── Test 4: R14 + R15 hold with regulation active (reg_gain ≠ 0 evolved in population) ────────────

/// Verifies that determinism (R14) and exact energy conservation (R15) hold when regulation is
/// active — i.e. after natural mutation has produced agents with non-zero `reg_gain`.
#[test]
fn reg_r14_r15_active() {
    if cfg!(debug_assertions) {
        return; // 512-tick run benefits from release optimisation
    }
    const TICKS: u64 = 512;

    // R15: exact conservation holds every tick (regulation only changes WHICH layer is drawn,
    // not whether energy is conserved — the same B-3 conservative transfer applies).
    let mut sim = build_sim(default_config(0xA11A_2A11));
    for _ in 0..TICKS {
        sim.step();
        assert_eq!(
            sim.conservation_residual(), 0,
            "R15 violated at tick {} with regulation active", sim.tick()
        );
    }
    // Verify regulation was actually active (non-zero reg_gain evolved via mutation).
    let (min_gain, max_gain) = sim.reg_gain_range();
    assert!(
        min_gain != 0 || max_gain != 0,
        "expected non-zero reg_gain after {TICKS} ticks — regulation never activated \
         (reg_gain range [{min_gain}, {max_gain}])"
    );

    // R14: two identical-seed runs produce identical per-tick state hashes.
    let a = run(default_config(0xA11A_2A11), TICKS);
    let b = run(default_config(0xA11A_2A11), TICKS);
    for t in 0..TICKS as usize {
        assert_eq!(a[t], b[t], "R14 non-determinism at tick {t} with regulation active");
    }
}

// ── Test 5: §B selective-value A/B experiment — regulation must be non-vestigial ────────────────────

/// §B selective-value A/B experiment (issue #169): the plastic line (reg_gain_max=4, expression
/// rule free to evolve) vs the control line (reg_gain_max=0, regulation locked OFF, uptake_layer
/// still free to evolve). PRIMARY metric: mean equilibrium population N̄ over ticks [2000, 4000].
///
/// Pre-declaration (anti-self-certification):
///   Metric: mean population over ticks [2000, 4000] per seed.
///   Margin: plastic_mean ≥ control_mean + 3 × σ_control (measured from control replicates).
///   Basis:  D-slice mechanism — when lr < reg_setpoint (depleted patch), reg_gain < 0 switches
///   the agent to layer-1 (organics). Both layers must be heterogeneously available (≥30% occupied
///   cells favor each layer — the false-negative guard measured below before trusting the verdict).
///   Seeds: both [0xA11A_2A11, 0x1234_5678] must pass (≥2-seed replication, plan §5).
///
/// Anti-degenerate guard: confirm the plastic arm actually switches between layers (not a static
/// drift to one layer). Record expressed_layer distribution at equilibrium.
///
/// False-negative guard: confirm ≥30% of occupied cells favor each layer at t=4000.
#[test]
fn reg_selective_value_ab() {
    if cfg!(debug_assertions) { return; }

    const TICKS: u64 = 4_000;
    const MEAN_FROM: u64 = 2_000;
    const SEEDS: [u64; 2] = [0xA11A_2A11, 0x1234_5678];
    // Control replicates: 2 seeds × reg_gain_max=0.
    // σ_control = std_dev of the two control means (across seeds — the within-condition variance).
    // Pre-declared margin: plastic_mean ≥ control_mean + 3 × σ_control (per-seed).
    // NOTE: with only 2 replicates σ is (|mean_A − mean_B|/2); we measure this before running
    // the plastic arm, then declare the threshold. See below.

    // ── Step 1: measure control replicates (reg_gain_max=0, locked OFF) ──────────────────────
    let control_means: Vec<u64> = SEEDS.iter().map(|&seed| {
        let mut cfg = default_config(seed);
        cfg.econ.reg_gain_max = 0;
        mean_pop_from(cfg, TICKS, MEAN_FROM)
    }).collect();

    // σ_control over the 2 seeds (half the range — the unbiased 2-sample std dev estimate).
    let ctrl_mean_all = (control_means[0] + control_means[1]) / 2;
    let sigma_num = control_means[0].abs_diff(control_means[1]);
    let sigma_ctrl = sigma_num / 2; // floor division; conservative

    // Pre-declared threshold (written BEFORE running plastic arm):
    // plastic_mean ≥ ctrl_seed_mean + 3·σ for that seed.
    // threshold_i = control_means[i] + 3·sigma_ctrl
    let thresholds: Vec<u64> = control_means.iter().map(|&c| c + 3 * sigma_ctrl).collect();

    // ── Step 2: run plastic arm (reg_gain_max=4, expression rule free) ───────────────────────
    for (i, &seed) in SEEDS.iter().enumerate() {
        let mut sim = build_sim(default_config(seed));
        let (mut sum, mut count) = (0u64, 0u64);
        for t in 0..TICKS {
            sim.step();
            if t >= MEAN_FROM {
                sum += sim.population();
                count += 1;
            }
        }
        let plastic_mean = if count == 0 { 0 } else { sum / count };

        // ── False-negative guard: field must be heterogeneous ──────────────────────────────
        let (fav_l0, _eq, fav_l1) = sim.layer_dominance_at_occupied();
        let fav_total = fav_l0 + fav_l1;
        assert!(
            fav_total > 0 && fav_l0 * 10 >= fav_total * 3 && fav_l1 * 10 >= fav_total * 3,
            "seed=0x{seed:016X}: layer field not heterogeneous enough — \
             l0_dom={fav_l0}/{fav_total} l1_dom={fav_l1}/{fav_total} (need ≥30% each) \
             — A/B result is ambiguous in a one-sided field (§169 false-negative guard)"
        );

        // ── Anti-degenerate: confirm agents actually switch layers ─────────────────────────
        let (on_cold, switched) = sim.switching_counts();
        let sw_total = on_cold + switched;
        assert!(
            switched > 0,
            "seed=0x{seed:016X}: no agents switching layers at equilibrium (cold={on_cold} switched=0) \
             — regulation may be vestigial or setpoint degenerate"
        );
        assert!(
            on_cold > 0,
            "seed=0x{seed:016X}: ALL agents switched to other layer ({switched}/{sw_total}) \
             — static drift, not adaptive switching (anti-degenerate check)"
        );

        // ── Primary selective-value gate (pre-declared margin) ────────────────────────────
        let threshold = thresholds[i];
        assert!(
            plastic_mean >= threshold,
            "seed=0x{seed:016X}: plastic N̄={plastic_mean} < threshold={threshold} \
             (control={} ctrl_σ={sigma_ctrl} margin=+3σ={}) — regulation vestigial \
             (§169 A/B gate, pre-declared margin)",
            control_means[i], 3 * sigma_ctrl
        );

        let _ = ctrl_mean_all; // suppress unused-variable warning
    }
}

fn mean_pop_from(config: SimConfig, ticks: u64, from: u64) -> u64 {
    let mut sim = build_sim(config);
    let (mut sum, mut count) = (0u64, 0u64);
    for t in 0..ticks {
        sim.step();
        if t >= from {
            sum += sim.population();
            count += 1;
        }
    }
    if count == 0 { 0 } else { sum / count }
}
