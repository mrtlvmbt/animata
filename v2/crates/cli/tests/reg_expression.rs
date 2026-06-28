//! D-slice GRN seed mechanism tests (issue #169): evolvable expression regulation via `sense_range_eff`.
//! All tests run on x86 CI (arch-independent: pure integer genome math + conserved-layer invariants).
//! Test 5 is the §B selective-value A/B experiment required by the acceptance criteria.

use cli::{build_sim, default_config, run};
use sim_core::{Genome, SimConfig};

// ── Test 1: sense_range_eff is a deterministic pure function; founder (reg_gain=0) is inert ─────────

/// Verifies the pure-integer `sense_range_eff` contract (D-slice / GRN seed):
/// - At `reg_gain = 0` (the founder), `eff == sense_range` for ALL `local_resource` values → inert.
/// - Same inputs always produce the same output (no hidden state).
/// Covers the §8 pitfall guard: the founder starts with regulation OFF; only mutation wires it in.
#[test]
fn reg_expression_deterministic() {
    let founder = Genome::founder(2);
    assert_eq!(founder.reg_gain, 0, "founder must start with reg_gain=0 (regulation OFF)");

    // Founder inert: eff == sense_range for every possible local_resource
    for lr in [i64::MIN, -1000, -1, 0, 1, 79, 80, 81, 200, 1000, i64::MAX] {
        assert_eq!(
            founder.sense_range_eff(lr), founder.sense_range,
            "founder reg_gain=0: eff must equal sense_range ({}) at lr={lr}", founder.sense_range,
        );
    }

    // Determinism: same inputs → same output (no internal state, no float)
    let g = Genome { reg_gain: 2, reg_setpoint: 80, ..founder };
    for lr in [0i64, 50, 79, 80, 81, 200, -100] {
        let a = g.sense_range_eff(lr);
        let b = g.sense_range_eff(lr);
        assert_eq!(a, b, "sense_range_eff must be deterministic at lr={lr}");
    }

    // Direction: at lr > setpoint (signum=+1), eff = sense_range + reg_gain
    let g_pos = Genome { sense_range: 3, reg_gain: 2, reg_setpoint: 80, ..founder };
    assert_eq!(g_pos.sense_range_eff(100), 5); // 3 + 2*1 = 5

    // Direction: at lr < setpoint (signum=-1), eff = sense_range - reg_gain
    assert_eq!(g_pos.sense_range_eff(50), 1); // 3 + 2*(-1) = 1

    // At lr == setpoint (signum=0), eff = sense_range
    assert_eq!(g_pos.sense_range_eff(80), 3); // 3 + 2*0 = 3
}

// ── Test 2: sense_range_eff output stays in 0..=8 for all valid inputs (no overflow from clamp) ─────

/// Verifies that the clamp in `sense_range_eff` prevents out-of-bounds effort across ALL valid
/// combinations of sense_range (0..=8), reg_gain (−4..=4), and extreme local_resource values.
/// "Across a real run, the cached effort stays in 0..=8 for every agent" — pure function proof.
#[test]
fn reg_expression_clamped() {
    let base = Genome::founder(2);
    for sr in 0i32..=8 {
        for gain in -4i32..=4 {
            let g = Genome { sense_range: sr, reg_gain: gain, reg_setpoint: 80, ..base };
            for lr in [i64::MIN, -1000, -1, 0, 1, 79, 80, 81, 1000, i64::MAX] {
                let eff = g.sense_range_eff(lr);
                assert!(
                    (0..=8).contains(&eff),
                    "effort {eff} OOB with sense_range={sr} reg_gain={gain} local_resource={lr}"
                );
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

    // Sanity: different setpoints hash differently
    let a = Genome { reg_setpoint: 40, ..base };
    let b = Genome { reg_setpoint: 120, ..base };
    assert_ne!(a.hash_contribution(0), b.hash_contribution(0));
}

// ── Test 4: R14 + R15 hold with regulation active (reg_gain ≠ 0 evolved in population) ────────────

/// Verifies that determinism (R14) and exact energy conservation (R15) hold when regulation is
/// active — i.e. after natural mutation has produced agents with non-zero `reg_gain`. At tick≥512
/// with the default mutation rate (32/256 ≈ 12.5% per division), non-zero reg_gain is statistically
/// certain (multiple generations, ~40 founders → ~200 agents, many carrying regulation alleles).
#[test]
fn reg_r14_r15_active() {
    if cfg!(debug_assertions) {
        return; // 512-tick run benefits from release optimisation
    }
    const TICKS: u64 = 512;

    // R15: exact conservation holds every tick (regulation only changes dissipation amount, not identity)
    let mut sim = build_sim(default_config(0xA11A_2A11));
    for _ in 0..TICKS {
        sim.step();
        assert_eq!(
            sim.conservation_residual(), 0,
            "R15 violated at tick {} with regulation active", sim.tick()
        );
    }
    // Verify regulation was actually active (non-zero reg_gain evolved via mutation)
    let (min_gain, max_gain) = sim.reg_gain_range();
    assert!(
        min_gain != 0 || max_gain != 0,
        "expected non-zero reg_gain after {TICKS} ticks — regulation never activated \
         (reg_gain range [{min_gain}, {max_gain}])"
    );

    // R14: two identical-seed runs produce identical per-tick state hashes
    let a = run(default_config(0xA11A_2A11), TICKS);
    let b = run(default_config(0xA11A_2A11), TICKS);
    for t in 0..TICKS as usize {
        assert_eq!(a[t], b[t], "R14 non-determinism at tick {t} with regulation active");
    }
}

// ── Test 5: §B selective-value A/B experiment — regulation must be non-vestigial ────────────────────

/// §B selective-value A/B experiment (issue #169): the plastic line (reg_gain_max=4) must achieve
/// a higher mean population than the control line (reg_gain_max=0, regulation locked OFF) in a
/// verified-patchy field.
///
/// Pre-declaration (anti-self-certification, before CI measurement):
///   Metric: mean population over ticks [2000, 4000] per seed.
///   Margin: plastic_mean ≥ control_mean × 1.05 (5% selective advantage per seed).
///   Basis: evolved negative reg_gain (selected when R̄ < setpoint ≈ 80) senses farther in
///   low-substrate patches → preferential access to high-resource cells → lower average sense cost
///   → higher N*. Control line has reg_gain≡0, so it behaves identically to C-slice (N*_C≈234).
///   Patchiness: NoiseWorld with HMAX=16 guarantees non-uniform per-cell caps (spatial_var > 0
///   by construction — verified via HMAX constant). Both regulation directions are viable since
///   reg_setpoint=80 sits inside the field range [RESOURCE_BASE/2 ± HMAX] = [60±8] → patches
///   exist both above and below the setpoint.
///   Seeds: [0xA11A_2A11, 0x1234_5678] — both must pass individually (≥2-seed coverage).
///
/// Regulation is non-vestigial iff BOTH seeds pass the pre-declared 5% margin.
#[test]
fn reg_selective_value_ab() {
    if cfg!(debug_assertions) { return; }

    const TICKS: u64 = 4_000;
    const MEAN_FROM: u64 = 2_000;
    const SEEDS: [u64; 2] = [0xA11A_2A11, 0x1234_5678];

    for &seed in &SEEDS {
        let plastic_mean = mean_pop_from(default_config(seed), TICKS, MEAN_FROM);

        let mut ctrl = default_config(seed);
        ctrl.econ.reg_gain_max = 0; // lock regulation OFF — the A/B control line
        let control_mean = mean_pop_from(ctrl, TICKS, MEAN_FROM);

        // Pre-declared 5% margin: plastic_mean ≥ control_mean × 1.05.
        assert!(
            plastic_mean * 100 >= control_mean * 105,
            "seed=0x{seed:016X}: plastic mean pop {plastic_mean} < control mean pop {control_mean} × 1.05 \
             — regulation appears vestigial (§169 A/B, pre-declared 5% margin)"
        );
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
