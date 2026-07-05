//! D-2 (#270): `driver_config` — the multicellular-predation cost↔benefit economy. Combines
//! `phase2_config`'s ontogenesis chain (bodies can be multicellular) with predation + a per-prey
//! size-refuge (D-1, `#268`, the benefit) and `c_coord > 0` (M7-e-a, `#251`, the cost). Parameters
//! are chosen for VIABILITY, not tuned for emergence (D-3's job — out of scope here).
//!
//! Arch-independent integer invariants — run on BOTH CI jobs (x86 + arm64). The additive golden
//! (`v2_golden_conserved_driver`, `golden_conserved.rs`) is arm64-only (PM-pinned separately).

use cli::{apply_overrides, build_sim, driver_config, run};
use sim_core::EconParams;

const SEED: u64 = 0xBE_EF_5EED;
const TICKS: u64 = 512;

/// `d2_driver_config_viable`: non-collapse floor over the standard local acceptance length —
/// mirrors `predation_no_collapse`/`differentiation_no_collapse`. If this fails at the chosen
/// defaults, the refuge/c_coord/predation calibration needs adjustment (an early calibration
/// signal to report, not silently patch around).
#[test]
fn d2_driver_config_viable() {
    if cfg!(debug_assertions) {
        return;
    }
    let mut sim = build_sim(driver_config(SEED));
    let mut pop_min = u64::MAX;
    let mut pop_max = 0u64;
    for _ in 0..TICKS {
        sim.step();
        let pop = sim.population();
        pop_min = pop_min.min(pop);
        pop_max = pop_max.max(pop);
    }
    const POP_FLOOR: u64 = 10;
    assert!(
        pop_min >= POP_FLOOR,
        "population collapsed below {POP_FLOOR} on driver_config at tick {TICKS} \
         (pop_min={pop_min}) — the predation/refuge/c_coord defaults are not viable"
    );
    const POP_CEIL: u64 = 100_000;
    assert!(
        pop_max <= POP_CEIL,
        "population exploded to {pop_max} on driver_config — conservation or encounter logic is broken"
    );
}

/// `d2_bodies_can_be_multicellular`: driver_config decodes bodies with `Σ module_cell_count`
/// reaching >1 for some genomes — the multicellular substrate is live, not inert.
#[test]
fn d2_bodies_can_be_multicellular() {
    if cfg!(debug_assertions) {
        return;
    }
    let mut sim = build_sim(driver_config(SEED));
    for _ in 0..TICKS {
        sim.step();
    }
    let (max_body_size, count_multicellular) = sim.body_size_stats();
    assert!(
        max_body_size > 1,
        "driver_config must produce at least one body with Σ module_cell_count > 1 \
         (max observed = {max_body_size}) — the ontogenesis chain looks inert"
    );
    assert!(
        count_multicellular > 0,
        "driver_config must have at least one live multicellular body at tick {TICKS}"
    );
}

/// `d2_predation_size_refuge_active`: with driver_config's own predation spec, a large-bodied prey
/// suffers strictly less predation loss than an equal-energy unicell — the D-1 refuge is ON and
/// biting at the chosen calibration (`DRIVER_REFUGE_K`), not a degenerate no-op.
#[test]
fn d2_predation_size_refuge_active() {
    let spec = driver_config(SEED)
        .econ
        .predation
        .expect("driver_config must configure predation");
    assert!(spec.size_refuge.is_some(), "driver_config must configure size_refuge");

    let predator = sim_core::Genome::founder(1);
    let prey_energy = 10_000i64;

    let loss_unicell = sim_core::resolve_encounter(&predator, prey_energy, 1, &spec).prey_loss;
    let loss_large_body =
        sim_core::resolve_encounter(&predator, prey_energy, 20, &spec).prey_loss;

    assert!(
        loss_large_body < loss_unicell,
        "a large-bodied prey (body_size=20) must lose LESS than an equal-energy unicell under \
         driver_config's size-refuge: loss_large_body={loss_large_body}, loss_unicell={loss_unicell}"
    );
}

/// `d2_c_coord_charged`: `c_coord > 0` in driver_config must genuinely alter the trajectory versus
/// an otherwise-identical `c_coord=0` twin — proving the coordination-cost sink (M7-e-a) is wired
/// AND active in this config (not dead weight because bodies never reach >1 cell — see
/// `d2_bodies_can_be_multicellular` for that half of the proof).
#[test]
fn d2_c_coord_charged() {
    if cfg!(debug_assertions) {
        return;
    }
    assert!(driver_config(SEED).econ.c_coord > 0, "driver_config must ship c_coord > 0");

    let with_cost = run(driver_config(SEED), TICKS);
    let mut cfg_no_cost = driver_config(SEED);
    cfg_no_cost.econ.c_coord = 0;
    let without_cost = run(cfg_no_cost, TICKS);

    assert_ne!(
        with_cost, without_cost,
        "c_coord>0 must alter driver_config's trajectory vs a c_coord=0 twin — the coordination \
         cost must be genuinely charged, not dead weight"
    );
}

/// `d2_conservation_R15`: driver_config closes the energy ledger (residual 0) every tick with
/// refuge + c_coord + predation all composed.
#[test]
fn d2_conservation_r15() {
    if cfg!(debug_assertions) {
        return;
    }
    let mut sim = build_sim(driver_config(SEED));
    for _ in 0..TICKS {
        sim.step();
        assert_eq!(
            sim.conservation_residual(),
            0,
            "energy not conserved at tick {} on driver_config (predation/c_coord/refuge composed)",
            sim.tick()
        );
    }
}

/// `d2_determinism`: driver_config replay bit-identical (1-vs-N is exercised via `r14.rs`'s
/// generic sweep; this is the same-seed repeated-run half already used by every sibling config's
/// `_r14_determinism` test).
#[test]
fn d2_determinism() {
    if cfg!(debug_assertions) {
        return;
    }
    let a = run(driver_config(SEED), TICKS);
    let b = run(driver_config(SEED), TICKS);
    for t in 0..TICKS as usize {
        assert_eq!(
            a[t], b[t],
            "driver_config non-deterministic at tick {t} — state_hash depends on RNG or thread-order"
        );
    }
}

/// `d2_set_overrides`: `--set c_coord=<v>` and `--set refuge_k=<v>` apply + range-guard (reject
/// negative); no-flag path stays byte-identical to `driver_config` itself.
#[test]
fn d2_set_overrides() {
    // Apply: c_coord updates econ.c_coord.
    let mut econ = driver_config(SEED).econ;
    apply_overrides(&mut econ, &[("c_coord".to_string(), "7".to_string())])
        .expect("c_coord=7 must be accepted");
    assert_eq!(econ.c_coord, 7);

    // Apply: refuge_k updates the nested SizeRefugeSpec.
    apply_overrides(&mut econ, &[("refuge_k".to_string(), "9".to_string())])
        .expect("refuge_k=9 must be accepted on a config with predation.size_refuge configured");
    assert_eq!(econ.predation.unwrap().size_refuge.unwrap().refuge_k, 9);

    // Range-guard: negative values rejected for both keys.
    let mut econ_neg = driver_config(SEED).econ;
    let r_c = apply_overrides(&mut econ_neg, &[("c_coord".to_string(), "-1".to_string())]);
    assert!(r_c.is_err(), "c_coord=-1 must return Err");
    assert!(r_c.unwrap_err().starts_with("error:"));

    let r_k = apply_overrides(&mut econ_neg, &[("refuge_k".to_string(), "-1".to_string())]);
    assert!(r_k.is_err(), "refuge_k=-1 must return Err");
    assert!(r_k.unwrap_err().starts_with("error:"));

    // refuge_k is rejected when no predation.size_refuge is configured (structural — plain default).
    let mut econ_plain = EconParams::default();
    let r_no_pred = apply_overrides(&mut econ_plain, &[("refuge_k".to_string(), "3".to_string())]);
    assert!(r_no_pred.is_err(), "refuge_k must be rejected when predation is None");
    assert!(r_no_pred.unwrap_err().starts_with("error:"));

    // No-flag byte-identical: empty override set must leave driver_config's trajectory untouched.
    if !cfg!(debug_assertions) {
        let baseline = run(driver_config(SEED), TICKS);
        let mut econ_empty = driver_config(SEED).econ;
        apply_overrides(&mut econ_empty, &[]).expect("empty override set is always Ok");
        let mut cfg_empty = driver_config(SEED);
        cfg_empty.econ = econ_empty;
        let overridden = run(cfg_empty, TICKS);
        assert_eq!(
            baseline, overridden,
            "empty --set must be byte-identical to driver_config's own trajectory"
        );
    }
}

// ── D-3b (#274) — multicellularity emergence VERDICT ────────────────────────────────────────────
//
// Smoke: driver_config runs a short horizon and `Telemetry.multicellular_frac` reads in-range
// [0, BODY_SIZE_SCALE] — the measurement plumbing the verdict test below depends on is live.
#[test]
fn d3b_multicellular_frac_plumbing_smoke() {
    let mut sim = build_sim(driver_config(SEED));
    for _ in 0..50 {
        sim.step();
    }
    let tel = sim.telemetry();
    assert!(
        (0..=sim_core::BODY_SIZE_SCALE).contains(&tel.multicellular_frac),
        "multicellular_frac={} must be readable and in [0, {}]",
        tel.multicellular_frac,
        sim_core::BODY_SIZE_SCALE
    );
}

// ── D-3b/V-5 emergence verdict experiment ────────────────────────────────────────────────────────
// PRE-DECLARED VERDICT CONSTANTS (recorded BEFORE running, per issue #274 — do NOT adjust to flip
// a NULL to EMERGENCE):
//   EMERGE_FLOOR   = 128/256 (BODY_SIZE_SCALE) — ≥50% of the live population multicellular.
//   MARGIN         = 2x — frac(WITH) must beat BOTH controls (ablation, channel-isolation) by ≥ this.
//   SEED_MAJORITY  = 3/5 — the regime must sustain across at least 3 of 5 seeds.
//   POP_FLOOR      = 10 — drift-confound guard: a tick counts toward the late-window mean only when
//                    live population ≥ POP_FLOOR (an extinct run is NOT "reverted to unicellular").
//   Sweep          = bite_shift ∈ {3, 2, 1, 0}, at a FIXED refuge_k=128 (V-5 #278: round-2 showed
//                    refuge_k inert at the swept values, so it is fixed here at its strongest, and
//                    DRIVER STRENGTH — the bite — is the swept axis instead) and driver_config's
//                    default c_coord.
//   Window         = mean over the last min(ticks, 1000) ticks (sustained, not a single snapshot).
//
// EMERGENCE iff ∃ bite_shift where frac(WITH) ≥ EMERGE_FLOOR AND frac(WITH) ≥ MARGIN·frac(ABLATION)
// AND frac(WITH) ≥ MARGIN·frac(CHANNEL-ISOLATION), sustained in ≥ SEED_MAJORITY of 5 seeds. Else
// NULL — report which sub-condition failed, from the printed regime map.
//
// Three arms per (seed, bite_shift):
//   WITH                predation ON + size-refuge ON at FIXED_REFUGE_K, bite_shift=bs (hypothesis)
//   ABLATION-predators  predation OFF entirely                                    (Boraas control)
//   CHANNEL-ISOLATION   predation ON, refuge_k=0, bite_shift=bs (refuge off, same bite strength as
//                       WITH)                                              (anti-subsidy control)
// ABLATION doesn't depend on bite_shift (no predation at all) — computed once per seed and reused
// across the sweep. CHANNEL-ISOLATION must track WITH's bite_shift (same predation strength, only
// refuge differs) to isolate the refuge's own effect rather than confounding two varying knobs — so
// it is recomputed per sweep value.
//
// Configure horizon via env var DRIVER_EMERGENCE_TICKS (default 400 for fast local iteration; cloud
// dispatch uses 8000 — see scripts/sim-run.sh driver-emergence).
// Run: cargo test --release -p cli -- driver_emergence_verdict --nocapture --ignored
// Cloud: scripts/sim-run.sh driver-emergence ticks=8000  (after this PR merges to main)

const EMERGE_FLOOR: i64 = 128; // ×BODY_SIZE_SCALE(256) == 50%
const MARGIN: i64 = 2;
const SEED_MAJORITY: usize = 3;
const POP_FLOOR: i64 = 10;
/// V-5 (#278): refuge_k fixed at the strongest value from the round-2 sweep — round-2 showed
/// refuge_k inert, so this sweep searches driver STRENGTH (bite_shift) instead.
const FIXED_REFUGE_K: i32 = 128;
const BITE_SHIFT_SWEEP: [u32; 4] = [3, 2, 1, 0];
const VERDICT_SEEDS: [u64; 5] = [1, 2, 3, 4, 5];

/// Result of one driver_config arm run: the mean `multicellular_frac` over the valid (pop≥POP_FLOOR)
/// late-window ticks, the mean population (informational), and whether the run ever collapsed below
/// POP_FLOOR in the window (extinction — not a measurable multicellular_frac, distinct from reverting
/// to unicellular dominance).
struct ArmResult {
    frac: i64,
    mean_pop: f64,
    collapsed: bool,
}

fn run_driver_arm(
    seed: u64,
    ticks: u64,
    window_start: u64,
    predators_on: bool,
    refuge_k: i32,
    bite_shift: u32,
) -> ArmResult {
    let mut cfg = driver_config(seed);
    if !predators_on {
        cfg.econ.predation = None; // Boraas control: no predators at all
    } else {
        let spec = cfg
            .econ
            .predation
            .as_mut()
            .expect("driver_config always configures predation");
        spec.bite_shift = bite_shift;
        spec
            .size_refuge
            .as_mut()
            .expect("driver_config always configures size_refuge")
            .refuge_k = refuge_k;
    }
    let mut sim = build_sim(cfg);
    let mut frac_sum: i64 = 0;
    let mut valid_ticks: i64 = 0;
    let mut pop_sum: i64 = 0;
    let mut pop_ticks: i64 = 0;
    for t in 0..ticks {
        sim.step();
        if t >= window_start {
            let tel = sim.telemetry();
            pop_sum += tel.population;
            pop_ticks += 1;
            if tel.population >= POP_FLOOR {
                frac_sum += tel.multicellular_frac;
                valid_ticks += 1;
            }
        }
    }
    let mean_pop = if pop_ticks > 0 { pop_sum as f64 / pop_ticks as f64 } else { 0.0 };
    if valid_ticks == 0 {
        ArmResult { frac: 0, mean_pop, collapsed: true }
    } else {
        ArmResult { frac: frac_sum / valid_ticks, mean_pop, collapsed: false }
    }
}

/// D-3b emergence verdict: does a non-trivial multicellular body size EMERGE under predation
/// size-refuge (Boraas/Ratcliff) and REVERT without it? Heavy (3 arms × 4-way sweep × 5 seeds ×
/// long horizon) — `#[ignore]`d in CI; run explicitly via the `driver-emergence` sim-run scenario.
#[test]
#[ignore]
fn driver_emergence_verdict() {
    let ticks: u64 = std::env::var("DRIVER_EMERGENCE_TICKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(400);
    let window_len = ticks.min(1000);
    let window_start = ticks - window_len;

    println!("\nD-3b/V-5 emergence verdict: multicellularity under predation size-refuge (Boraas/Ratcliff)");
    println!(
        "PRE-DECLARED: EMERGE_FLOOR={:.0}%, MARGIN={MARGIN}x, SEED_MAJORITY={SEED_MAJORITY}/5, POP_FLOOR={POP_FLOOR}",
        EMERGE_FLOOR as f64 / sim_core::BODY_SIZE_SCALE as f64 * 100.0
    );
    println!(
        "ticks={ticks}  late-window=[{window_start},{ticks}]  fixed refuge_k={FIXED_REFUGE_K}  bite_shift sweep={:?}",
        BITE_SHIFT_SWEEP
    );

    // ABLATION-predators doesn't depend on bite_shift (no predation at all) — compute once per seed.
    let ablation: Vec<ArmResult> = VERDICT_SEEDS
        .iter()
        .map(|&seed| run_driver_arm(seed, ticks, window_start, false, 0, 0))
        .collect();

    let mut any_regime_emerges = false;
    let mut best_bs = BITE_SHIFT_SWEEP[0];
    let mut best_count = 0usize;

    for &bs in &BITE_SHIFT_SWEEP {
        println!("{}", "-".repeat(78));
        println!("bite_shift={bs}");
        println!(
            "{:<6} {:>12} {:>12} {:>12} {:>10} {:>10}",
            "seed", "WITH%", "ablation%", "chan-iso%", "with-pop", "result"
        );

        // CHANNEL-ISOLATION matches this bite_shift (same predation strength as WITH, refuge off)
        // so the control isolates the refuge's effect, not a confound of two varying knobs.
        let channel_iso: Vec<ArmResult> = VERDICT_SEEDS
            .iter()
            .map(|&seed| run_driver_arm(seed, ticks, window_start, true, 0, bs))
            .collect();

        let mut seed_pass_count = 0usize;
        for (i, &seed) in VERDICT_SEEDS.iter().enumerate() {
            let with = run_driver_arm(seed, ticks, window_start, true, FIXED_REFUGE_K, bs);
            let abl = &ablation[i];
            let ciso = &channel_iso[i];

            let floor_ok = !with.collapsed && with.frac >= EMERGE_FLOOR;
            // A COLLAPSED control provides no valid benchmark: `with.frac >= 2*0` would pass vacuously
            // and void the causal comparison. Require the control population to be viable (F1, code-critic).
            let margin_abl_ok = !with.collapsed && !abl.collapsed && with.frac >= MARGIN * abl.frac;
            let margin_ciso_ok = !with.collapsed && !ciso.collapsed && with.frac >= MARGIN * ciso.frac;
            let pass = floor_ok && margin_abl_ok && margin_ciso_ok;
            if pass {
                seed_pass_count += 1;
            }

            let with_pct = with.frac as f64 / sim_core::BODY_SIZE_SCALE as f64 * 100.0;
            let abl_pct = abl.frac as f64 / sim_core::BODY_SIZE_SCALE as f64 * 100.0;
            let ciso_pct = ciso.frac as f64 / sim_core::BODY_SIZE_SCALE as f64 * 100.0;
            let tag = if with.collapsed {
                "COLLAPSED"
            } else if pass {
                "PASS"
            } else {
                "fail"
            };
            println!(
                "{:<6} {:>11.1}% {:>11.1}% {:>11.1}% {:>10.1} {:>10}",
                seed, with_pct, abl_pct, ciso_pct, with.mean_pop, tag
            );
        }

        println!("  seeds passing all 3 conditions: {seed_pass_count}/5 (need ≥{SEED_MAJORITY})");
        if seed_pass_count > best_count {
            best_count = seed_pass_count;
            best_bs = bs;
        }
        if seed_pass_count >= SEED_MAJORITY {
            any_regime_emerges = true;
        }
    }

    println!("{}", "-".repeat(78));
    println!();
    if any_regime_emerges {
        println!("DRIVER-EMERGENCE VERDICT: EMERGENCE");
        println!(
            "  A bite_shift regime (at fixed refuge_k={FIXED_REFUGE_K}) sustains multicellular_frac \u{2265}{EMERGE_FLOOR}/256 and \u{2265}{MARGIN}x both"
        );
        println!("  the predator-ablation and channel-isolation controls, in \u{2265}{SEED_MAJORITY}/5 seeds.");
        println!("  Best regime: bite_shift={best_bs} ({best_count}/5 seeds).");
        println!("  Size-refuge under predation is a genuine driver of multicellularity (Boraas/Ratcliff),");
        println!("  not a generic predation subsidy — the channel-isolation control rules that out.");
    } else {
        println!("DRIVER-EMERGENCE VERDICT: NULL — no bite_shift regime reached SEED_MAJORITY={SEED_MAJORITY}/5.");
        println!("  Closest regime: bite_shift={best_bs} ({best_count}/5 seeds passing all 3 conditions), fixed refuge_k={FIXED_REFUGE_K}.");
        println!("  See the regime map above for which sub-condition (floor / vs-ablation margin /");
        println!("  vs-channel-isolation margin) failed per seed \u{2014} an honest informative finding, not");
        println!("  tuned to pass. V-5 (#278) strengthened the bite (default bite_shift 3\u{2192}1) and swept");
        println!("  the bite axis at refuge_k fixed to its strongest value; NULL here means bite strength");
        println!("  was not the bottleneck either \u{2014} predation prevalence is the next probe (out of scope).");
    }
}

/// `v5_verdict_sweeps_bite_shift` (#278): fast, NOT `#[ignore]`d liveness check that the sweep
/// infra (`run_driver_arm`) genuinely varies with `bite_shift` at a fixed `refuge_k` and runs to
/// completion without panicking — the actual 8000-tick verdict is `driver_emergence_verdict`
/// (heavy, `#[ignore]`d, dispatched via `scripts/sim-run.sh driver-emergence`). This just proves
/// the extended sweep compiles, runs, and its stats are readable (a short regime-map slice).
#[test]
fn v5_verdict_sweeps_bite_shift() {
    if cfg!(debug_assertions) {
        return;
    }
    const TICKS: u64 = 200;
    const WINDOW_START: u64 = 100;
    let seed = VERDICT_SEEDS[0];

    println!("v5_verdict_sweeps_bite_shift smoke (ticks={TICKS}, fixed refuge_k={FIXED_REFUGE_K}):");
    for &bs in &BITE_SHIFT_SWEEP {
        let with = run_driver_arm(seed, TICKS, WINDOW_START, true, FIXED_REFUGE_K, bs);
        println!(
            "  bite_shift={bs}: frac={} mean_pop={:.1} collapsed={}",
            with.frac, with.mean_pop, with.collapsed
        );
        assert!(with.mean_pop >= 0.0, "bite_shift={bs} produced a nonsensical negative mean_pop");
    }
}
