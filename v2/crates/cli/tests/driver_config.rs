//! D-2 (#270): `driver_config` — the multicellular-predation cost↔benefit economy. Combines
//! `phase2_config`'s ontogenesis chain (bodies can be multicellular) with predation + a per-prey
//! size-refuge (D-1, `#268`, the benefit) and `c_coord > 0` (M7-e-a, `#251`, the cost). Parameters
//! are chosen for VIABILITY, not tuned for emergence (D-3's job — out of scope here).
//!
//! Arch-independent integer invariants — run on BOTH CI jobs (x86 + arm64). The additive golden
//! (`v2_golden_conserved_driver`, `golden_conserved.rs`) is arm64-only (PM-pinned separately).

use cli::{apply_overrides, build_sim, driver_config, dol_probe_config, run};
use sim_core::{EconParams, Genome};

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

/// `d5_hazard_drain_monotone`: with driver_config's D-5 hazard predation, a large-bodied entity
/// suffers strictly less drain than an equal-energy unicell — the refuge attenuates the hazard drain.
#[test]
fn d5_hazard_drain_monotone() {
    let spec = driver_config(SEED)
        .econ
        .predation
        .expect("driver_config must configure predation");
    assert_eq!(spec.mode, sim_core::PredationMode::Hazard, "driver_config must use Hazard mode");
    assert!(spec.size_refuge.is_some(), "driver_config must configure size_refuge");
    assert!(spec.base_hazard > 0, "driver_config must have base_hazard > 0");

    let refuge = spec.size_refuge.unwrap();
    let drain_unicell = sim_core::refuge_attenuate(spec.base_hazard, 1, refuge.shift, refuge.refuge_k);
    let drain_large_body = sim_core::refuge_attenuate(spec.base_hazard, 20, refuge.shift, refuge.refuge_k);

    assert!(
        drain_large_body < drain_unicell,
        "a large-bodied entity (body_size=20) must DRAIN LESS than a unicell under \
         driver_config's hazard refuge: drain_large_body={drain_large_body}, drain_unicell={drain_unicell}"
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

/// `d2_set_overrides`: `--set c_coord=<v>`, `--set refuge_k=<v>`, and D-5 `--set base_hazard=<v>`
/// apply + range-guard (reject negative/out-of-range); no-flag path stays byte-identical to
/// `driver_config` itself.
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

    // D-5: Apply: base_hazard updates the hazard predation spec.
    let mut econ_hazard = driver_config(SEED).econ;
    apply_overrides(&mut econ_hazard, &[("base_hazard".to_string(), "1000".to_string())])
        .expect("base_hazard=1000 must be accepted on driver_config");
    assert_eq!(econ_hazard.predation.unwrap().base_hazard, 1000);

    // Range-guard: negative values rejected for all keys.
    let mut econ_neg = driver_config(SEED).econ;
    let r_c = apply_overrides(&mut econ_neg, &[("c_coord".to_string(), "-1".to_string())]);
    assert!(r_c.is_err(), "c_coord=-1 must return Err");
    assert!(r_c.unwrap_err().starts_with("error:"));

    let r_k = apply_overrides(&mut econ_neg, &[("refuge_k".to_string(), "-1".to_string())]);
    assert!(r_k.is_err(), "refuge_k=-1 must return Err");
    assert!(r_k.unwrap_err().starts_with("error:"));

    let mut econ_bh = driver_config(SEED).econ;
    let r_bh = apply_overrides(&mut econ_bh, &[("base_hazard".to_string(), "-100".to_string())]);
    assert!(r_bh.is_err(), "base_hazard=-100 must return Err");
    assert!(r_bh.unwrap_err().starts_with("error:"));

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

// ── D-4 (#281) — universal size-predation emergence VERDICT ──────────────────────────────────
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

// ── D-5 (#286) emergence verdict experiment — hazard-refuge predation ────────────────────────────
// PRE-DECLARED VERDICT CONSTANTS (recorded BEFORE running, per issue #286 — do NOT adjust to flip
// a NULL to EMERGENCE):
//   EMERGE_FLOOR   = 128/256 (BODY_SIZE_SCALE) — ≥50% of the live population multicellular.
//   MARGIN         = 2x — frac(WITH) must beat BOTH controls (ablation, channel-isolation) by ≥ this.
//   SEED_MAJORITY  = 3/5 — the regime must sustain across at least 3 of 5 seeds.
//   POP_FLOOR      = 10 — drift-confound guard: a tick counts toward the late-window mean only when
//                    live population ≥ POP_FLOOR (an extinct run is NOT "reverted to unicellular").
//   Sweep          = base_hazard ∈ {10, 20, 30, 45} (D-5 #285: hazard mode replaces CombatSplit;
//                    base_hazard is the per-tick drain, attenuated by size-refuge. Sweep brackets
//                    viable→collapse: base_hazard=10 is viable without collapse, =45 approaches
//                    collapse; fixed refuge_k=FIXED_REFUGE_K_HAZARD and driver_config's default c_coord).
//   Window         = mean over the last min(ticks, 1000) ticks (sustained, not a single snapshot).
//
// EMERGENCE iff ∃ base_hazard where frac(WITH) ≥ EMERGE_FLOOR AND frac(WITH) ≥ MARGIN·frac(ABLATION)
// AND frac(WITH) ≥ MARGIN·frac(CHANNEL-ISOLATION), sustained in ≥ SEED_MAJORITY of 5 seeds. Else
// NULL — report which sub-condition failed, from the printed regime map.
//
// Three arms per (seed, base_hazard):
//   WITH                hazard mode + size-refuge ON at FIXED_REFUGE_K_HAZARD, base_hazard=bh (hypothesis)
//   ABLATION-predators  predation OFF entirely                                    (Boraas control)
//   CHANNEL-ISOLATION   hazard mode, refuge_k=0 (refuge off, same base_hazard as WITH),
//                       body-independent drain → selective effect isolated (anti-subsidy control)
// ABLATION doesn't depend on base_hazard (no predation at all) — computed once per seed and reused
// across the sweep. CHANNEL-ISOLATION must track WITH's base_hazard (same drain, only refuge differs)
// to isolate the refuge's own effect rather than confounding two varying knobs — so it is recomputed
// per sweep value.
//
// Intermediate-persistence readout (informational, #288 FIX): late-window mean body size and drift-flatness,
// expressed in cell counts. Body size and drift are stored internally in ×BODY_SIZE_SCALE units; this
// readout converts to cells (divide by BODY_SIZE_SCALE=256) for interpretability. Checks: size ∈ [1, 0.9·MAX_CELLS]
// cells, drift < 0.5 cells (epsilon). Verifies whether intermediate-body selection is stable or drifting.
//
// Configure horizon via env var DRIVER_EMERGENCE_TICKS (default 400 for fast local iteration; cloud
// dispatch uses 8000 — see scripts/sim-run.sh driver-emergence).
// Run: cargo test --release -p cli -- driver_emergence_verdict --nocapture --ignored
// Cloud: scripts/sim-run.sh driver-emergence ticks=8000  (after this PR merges to main)

const EMERGE_FLOOR: i64 = 128; // ×BODY_SIZE_SCALE(256) == 50%
const MARGIN: i64 = 2;
const SEED_MAJORITY: usize = 3;
const POP_FLOOR: i64 = 10;
/// D-5 (#286): refuge_k fixed for hazard-refuge mode — per-entity size-dependent drain attenuation.
const FIXED_REFUGE_K_HAZARD: i32 = 128;
/// D-5 (#286): base_hazard sweep bracketing viable→collapse. Calibration: base_hazard=10 is viable,
/// =50 is collapse; sweep [10,20,30,45] probes the selective band.
const BASE_HAZARD_SWEEP: [i64; 4] = [10, 20, 30, 45];
const VERDICT_SEEDS: [u64; 5] = [1, 2, 3, 4, 5];

/// D-5 (#290): Extended robustness-probe at base_hazard=10 (the passing regime). Seeds beyond the
/// 5-seed verdict gate (VERDICT_SEEDS), for informational robustness readout. The main verdict GATE
/// remains on VERDICT_SEEDS with SEED_MAJORITY=3/5 (never touched). This extended set is a separate
/// robustness measurement, not a re-gating of the verdict.
const EXTENDED_ROBUSTNESS_SEEDS: [u64; 15] = [6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20];
const EXTENDED_ROBUSTNESS_BASE_HAZARD: i64 = 10; // robustness-probe at the verified passing regime

/// Intermediate-persistence readout thresholds (#288): expressed in cell counts.
/// BODY_SIZE_INTERMEDIATE_MIN = 1 cell (below: unicellular refuge).
/// BODY_SIZE_INTERMEDIATE_MAX = 0.9 * MAX_CELLS ≈ 28.8 cells (intermediate-multicellular range).
/// DRIFT_EPSILON_CELLS = 0.5 cells (max permitted drift between 1st & 2nd half of window to be "stable").
const BODY_SIZE_INTERMEDIATE_MIN: f64 = 1.0;
const BODY_SIZE_INTERMEDIATE_MAX: f64 = 0.9 * 32.0; // MAX_CELLS = 32
const DRIFT_EPSILON_CELLS: f64 = 0.5;

/// Result of one driver_config arm run: the mean `multicellular_frac` over the valid (pop≥POP_FLOOR)
/// late-window ticks, the mean population, the mean body size, and whether the run ever collapsed.
struct ArmResult {
    frac: i64,
    mean_pop: f64,
    mean_body_size: f64,
    body_size_drift: f64, // |mean(2nd half) - mean(1st half)| of body size in window
    collapsed: bool,
}

fn run_driver_arm(
    seed: u64,
    ticks: u64,
    window_start: u64,
    predators_on: bool,
    refuge_k: i32,
    base_hazard: i64,
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
        // D-5: switch to Hazard mode and set base_hazard + refuge_k (gated to mode=Hazard).
        spec.mode = sim_core::PredationMode::Hazard;
        spec.base_hazard = base_hazard;
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
    let mut body_size_sum: f64 = 0.0;
    let mut body_size_ticks: usize = 0;
    // Split the window into halves for drift calculation.
    let window_len = (ticks - window_start) as usize;
    let mut body_size_first_half: Vec<f64> = Vec::new();
    let mut body_size_second_half: Vec<f64> = Vec::new();
    let mid_point = window_start + (window_len as u64 / 2);

    for t in 0..ticks {
        sim.step();
        if t >= window_start {
            let tel = sim.telemetry();
            pop_sum += tel.population;
            pop_ticks += 1;
            // Accumulate body size stats (informational, even if collapsed).
            let body_size = tel.mean_body_size as f64;
            if t < mid_point {
                body_size_first_half.push(body_size);
            } else {
                body_size_second_half.push(body_size);
            }
            if tel.population >= POP_FLOOR {
                frac_sum += tel.multicellular_frac;
                valid_ticks += 1;
                body_size_sum += body_size;
                body_size_ticks += 1;
            }
        }
    }
    let mean_pop = if pop_ticks > 0 { pop_sum as f64 / pop_ticks as f64 } else { 0.0 };
    let mean_body_size =
        if body_size_ticks > 0 { body_size_sum / body_size_ticks as f64 } else { 0.0 };

    // Compute drift: |mean(2nd half) - mean(1st half)|.
    let drift = if !body_size_first_half.is_empty() && !body_size_second_half.is_empty() {
        let mean_first = body_size_first_half.iter().sum::<f64>() / body_size_first_half.len() as f64;
        let mean_second =
            body_size_second_half.iter().sum::<f64>() / body_size_second_half.len() as f64;
        (mean_second - mean_first).abs()
    } else {
        0.0
    };

    if valid_ticks == 0 {
        ArmResult {
            frac: 0,
            mean_pop,
            mean_body_size,
            body_size_drift: drift,
            collapsed: true,
        }
    } else {
        ArmResult {
            frac: frac_sum / valid_ticks,
            mean_pop,
            mean_body_size,
            body_size_drift: drift,
            collapsed: false,
        }
    }
}

/// D-5 (#286) emergence verdict: under hazard-refuge predation, does multicellularity EMERGE as a
/// size-defense mechanism (refuge-specific) or as a generic predation side-effect?
/// Heavy (3 arms × 4-way sweep × 5 seeds × long horizon) — `#[ignore]`d in CI; run explicitly
/// via the `driver-emergence` sim-run scenario.
#[test]
#[ignore]
fn driver_emergence_verdict() {
    let ticks: u64 = std::env::var("DRIVER_EMERGENCE_TICKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(400);
    let window_len = ticks.min(1000);
    let window_start = ticks - window_len;

    println!("\nD-5 (#286) emergence verdict: multicellularity under hazard-refuge predation (Boraas/Ratcliff)");
    println!(
        "PRE-DECLARED: EMERGE_FLOOR={:.0}%, MARGIN={MARGIN}x, SEED_MAJORITY={SEED_MAJORITY}/5, POP_FLOOR={POP_FLOOR}",
        EMERGE_FLOOR as f64 / sim_core::BODY_SIZE_SCALE as f64 * 100.0
    );
    println!(
        "ticks={ticks}  late-window=[{window_start},{ticks}]  fixed refuge_k={FIXED_REFUGE_K_HAZARD}  base_hazard sweep={:?}",
        BASE_HAZARD_SWEEP
    );

    // ABLATION-predators doesn't depend on base_hazard (no predation at all) — compute once per seed.
    let ablation: Vec<ArmResult> = VERDICT_SEEDS
        .iter()
        .map(|&seed| run_driver_arm(seed, ticks, window_start, false, 0, 0))
        .collect();

    let mut any_regime_emerges = false;
    let mut best_bh = BASE_HAZARD_SWEEP[0];
    let mut best_count = 0usize;
    // Track which sub-conditions failed in the best regime (for honest NULL diagnosis)
    let mut best_floor_fails = 0usize;
    let mut best_abl_fails = 0usize;
    let mut best_ciso_fails = 0usize;

    for &bh in &BASE_HAZARD_SWEEP {
        println!("{}", "-".repeat(78));
        println!("base_hazard={bh}");
        println!(
            "{:<6} {:>12} {:>12} {:>12} {:>10} {:>11} {:>8}",
            "seed", "WITH%", "ablation%", "chan-iso%", "size", "drift", "result"
        );

        // CHANNEL-ISOLATION matches this base_hazard (same drain, only refuge differs)
        // so the control isolates the refuge's effect, not a confound of two varying knobs.
        let channel_iso: Vec<ArmResult> = VERDICT_SEEDS
            .iter()
            .map(|&seed| run_driver_arm(seed, ticks, window_start, true, 0, bh))
            .collect();

        let mut seed_pass_count = 0usize;
        let mut bs_floor_fails = 0usize;
        let mut bs_abl_fails = 0usize;
        let mut bs_ciso_fails = 0usize;

        for (i, &seed) in VERDICT_SEEDS.iter().enumerate() {
            let with = run_driver_arm(seed, ticks, window_start, true, FIXED_REFUGE_K_HAZARD, bh);
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
            } else {
                if !floor_ok {
                    bs_floor_fails += 1;
                }
                if !margin_abl_ok {
                    bs_abl_fails += 1;
                }
                if !margin_ciso_ok {
                    bs_ciso_fails += 1;
                }
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
            // #288 FIX: intermediate-persistence readout in cell counts.
            // Convert from ×BODY_SIZE_SCALE units to cells by dividing by BODY_SIZE_SCALE.
            let body_size_cells = with.mean_body_size / sim_core::BODY_SIZE_SCALE as f64;
            let drift_cells = with.body_size_drift / sim_core::BODY_SIZE_SCALE as f64;
            
            let size_in_range = (BODY_SIZE_INTERMEDIATE_MIN..=BODY_SIZE_INTERMEDIATE_MAX).contains(&body_size_cells);
            let drift_flat = drift_cells < DRIFT_EPSILON_CELLS;
            let persist_note = if size_in_range && drift_flat { "✓" } else { "·" };
            println!(
                "{:<6} {:>11.1}% {:>11.1}% {:>11.1}% {:>9.1} {:>7.2} {:>8}",
                seed, with_pct, abl_pct, ciso_pct, body_size_cells, drift_cells, tag
            );
            if !persist_note.eq("✓") && !with.collapsed {
                println!("       (body_size={:.2} cells in ({:.1},{:.1})? {}, drift={:.3} <{} cells? {})",
                    body_size_cells, BODY_SIZE_INTERMEDIATE_MIN, BODY_SIZE_INTERMEDIATE_MAX, size_in_range, 
                    drift_cells, DRIFT_EPSILON_CELLS, drift_flat);
            }
        }

        println!("  seeds passing all 3 conditions: {seed_pass_count}/5 (need ≥{SEED_MAJORITY})");
        if seed_pass_count > best_count {
            best_count = seed_pass_count;
            best_bh = bh;
            best_floor_fails = bs_floor_fails;
            best_abl_fails = bs_abl_fails;
            best_ciso_fails = bs_ciso_fails;
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
            "  A base_hazard regime (at fixed refuge_k={FIXED_REFUGE_K_HAZARD}) sustains multicellular_frac \u{2265}{EMERGE_FLOOR}/256 and \u{2265}{MARGIN}x both"
        );
        println!("  the predator-ablation and channel-isolation controls, in \u{2265}{SEED_MAJORITY}/5 seeds.");
        println!("  Best regime: base_hazard={best_bh} ({best_count}/5 seeds).");
        println!("  Size-refuge under hazard-draining predation is a genuine driver of multicellularity (Boraas/Ratcliff),");
        println!("  not a generic predation subsidy — the channel-isolation control rules that out.");
    } else {
        println!("DRIVER-EMERGENCE VERDICT: NULL — no base_hazard regime reached SEED_MAJORITY={SEED_MAJORITY}/5.");
        println!("  Closest regime: base_hazard={best_bh} ({best_count}/5 seeds passing all 3 conditions), fixed refuge_k={FIXED_REFUGE_K_HAZARD}.");

        // Distinguish the failure mode: absolute emergence vs. channel-specificity
        if best_floor_fails > 0 {
            println!("  ✗ Sub-condition failures in best regime (base_hazard={best_bh}):");
            println!("    - Absolute emergence floor (WITH ≥ {:.0}%): {}/{} seeds failed",
                EMERGE_FLOOR as f64 / sim_core::BODY_SIZE_SCALE as f64 * 100.0,
                best_floor_fails, 5);
            println!("    - vs-ablation margin (WITH ≥ {MARGIN}×ablation): {}/{} seeds failed", best_abl_fails, 5);
            println!("  Interpretation: multicellularity does not EMERGE sufficiently under hazard-refuge");
            println!("  predation (WITH stays <{:.0}% or <{MARGIN}×predator-off baseline).",
                EMERGE_FLOOR as f64 / sim_core::BODY_SIZE_SCALE as f64 * 100.0);
        } else if best_abl_fails > 0 {
            println!("  ✗ Sub-condition failures in best regime (base_hazard={best_bh}):");
            println!("    - vs-ablation margin (WITH ≥ {MARGIN}×ablation): {}/{} seeds failed", best_abl_fails, 5);
            println!("  Interpretation: multicellularity effect is NOT specific to predation");
            println!("  (WITH ≤ {MARGIN}×ablation, similar to predator-off baseline).");
        } else if best_ciso_fails > 0 {
            // Emerged via WITH ≥ floor AND ≥ MARGIN×ablation, but failed channel-isolation
            println!("  ⚠ EMERGED-BUT-NOT-CHANNEL-SPECIFIC:");
            println!("    - WITH ≥ {:.0}%: {}/{} seeds passed (emergence floor reached)",
                EMERGE_FLOOR as f64 / sim_core::BODY_SIZE_SCALE as f64 * 100.0,
                5 - best_floor_fails, 5);
            println!("    - WITH ≥ {MARGIN}×ablation: {}/{} seeds passed (vs predator-off)", 5 - best_abl_fails, 5);
            println!("    - WITH ≥ {MARGIN}×channel-iso: {}/{} seeds FAILED", best_ciso_fails, 5);
            println!();
            println!("  The multicellularity transition EMERGES when hazard-draining predators are on (WITH ≈ {:.0}%),",
                EMERGE_FLOOR as f64 / sim_core::BODY_SIZE_SCALE as f64 * 100.0);
            println!("  and the effect is specific vs. predator-ablation (Boraas control). HOWEVER,");
            println!("  it is NOT specific to size-refuge: WITH ≥ {MARGIN}×channel-iso FAILS, meaning");
            println!("  multicellularity also reaches {:.0}% when refuge is DISABLED (refuge_k=0),",
                EMERGE_FLOOR as f64 / sim_core::BODY_SIZE_SCALE as f64 * 100.0);
            println!("  same hazard drain. Under the hazard-refuge model, this indicates the");
            println!("  transition is driven by hazard per se (predation favours larger");
            println!("  bodies in general via refuge), NOT exclusively by the size-refuge mechanism.");
            println!();
            println!("  FLAG: Channel-isolation control (refuge_k=0) under hazard mode DOES isolate");
            println!("  the refuge channel — hazard still acts, but body-independent (no refuge).");
            println!("  The failure to reach SEED_MAJORITY on channel-iso suggests size-refuge is");
            println!("  not the sole driver, or prevalence/parameters need further tuning.");
        } else {
            println!("  See the regime map above for which sub-condition (floor / vs-ablation margin /");
            println!("  vs-channel-isolation margin) failed per seed \u{2014} an honest informative finding, not");
            println!("  tuned to pass. D-5 verdict: hazard prevalence is the next probe (out of scope).");
        }
    }

    // ── D-5 (#290) Extended robustness-probe @ base_hazard=10 ─────────────────────────────────
    // INFORMATIONAL ONLY: extended seed-set (10-15 seeds) to measure robustness of the passing regime.
    // The verdict GATE (VERDICT_SEEDS, SEED_MAJORITY=3/5) is NOT re-gated; this is a separate measurement.
    println!();
    println!("{}", "=".repeat(78));
    println!("D-5 (#290) EXTENDED ROBUSTNESS-PROBE @ base_hazard={EXTENDED_ROBUSTNESS_BASE_HAZARD}");
    println!("(informational; verdict gate remains on VERDICT_SEEDS={:?}, SEED_MAJORITY={SEED_MAJORITY}/5)",
        VERDICT_SEEDS);
    println!("{}", "=".repeat(78));

    // Ablation is the same for extended seeds (predators off, base_hazard irrelevant).
    let ablation_extended: Vec<ArmResult> = EXTENDED_ROBUSTNESS_SEEDS
        .iter()
        .map(|&seed| run_driver_arm(seed, ticks, window_start, false, 0, 0))
        .collect();

    // Channel-isolation at EXTENDED_ROBUSTNESS_BASE_HAZARD.
    let channel_iso_extended: Vec<ArmResult> = EXTENDED_ROBUSTNESS_SEEDS
        .iter()
        .map(|&seed| run_driver_arm(seed, ticks, window_start, true, 0, EXTENDED_ROBUSTNESS_BASE_HAZARD))
        .collect();

    println!(
        "{:<6} {:>12} {:>12} {:>12} {:>10} {:>11} {:>8}",
        "seed", "WITH%", "ablation%", "chan-iso%", "size", "drift", "result"
    );

    let mut robustness_pass_count = 0usize;
    for (i, &seed) in EXTENDED_ROBUSTNESS_SEEDS.iter().enumerate() {
        let with = run_driver_arm(seed, ticks, window_start, true, FIXED_REFUGE_K_HAZARD, EXTENDED_ROBUSTNESS_BASE_HAZARD);
        let abl = &ablation_extended[i];
        let ciso = &channel_iso_extended[i];

        let floor_ok = !with.collapsed && with.frac >= EMERGE_FLOOR;
        let margin_abl_ok = !with.collapsed && !abl.collapsed && with.frac >= MARGIN * abl.frac;
        let margin_ciso_ok = !with.collapsed && !ciso.collapsed && with.frac >= MARGIN * ciso.frac;
        let pass = floor_ok && margin_abl_ok && margin_ciso_ok;
        if pass {
            robustness_pass_count += 1;
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
        let body_size_cells = with.mean_body_size / sim_core::BODY_SIZE_SCALE as f64;
        let drift_cells = with.body_size_drift / sim_core::BODY_SIZE_SCALE as f64;

        println!(
            "{:<6} {:>11.1}% {:>11.1}% {:>11.1}% {:>9.1} {:>7.2} {:>8}",
            seed, with_pct, abl_pct, ciso_pct, body_size_cells, drift_cells, tag
        );
    }

    println!();
    println!("robustness @ base_hazard={EXTENDED_ROBUSTNESS_BASE_HAZARD}: {robustness_pass_count}/{} seeds pass all 3 conditions",
        EXTENDED_ROBUSTNESS_SEEDS.len());
    println!("  (verdict gate: VERDICT_SEEDS, SEED_MAJORITY={SEED_MAJORITY}/5 — UNCHANGED)");
}

/// D-5 (#286) smoke test: fast, NOT `#[ignore]`d liveness check that the base_hazard sweep infra
/// (`run_driver_arm`) compiles, runs, and stats are readable. Verifies the sweep produces DISTINCT
/// regimes (not identical as they were when incorrectly swept over bite_shift in D-4). The actual
/// 8000-tick verdict is `driver_emergence_verdict` (heavy, `#[ignore]`d, dispatched via
/// `scripts/sim-run.sh driver-emergence`).
#[test]
fn d5_verdict_sweeps_base_hazard() {
    if cfg!(debug_assertions) {
        return;
    }
    const TICKS: u64 = 200;
    const WINDOW_START: u64 = 100;
    let seed = VERDICT_SEEDS[0];

    println!("D-5 (#286) base_hazard sweep smoke (ticks={TICKS}, fixed refuge_k={FIXED_REFUGE_K_HAZARD}):");
    let mut last_frac: Option<i64> = None;
    let mut distinct_count = 0;
    for &bh in &BASE_HAZARD_SWEEP {
        let with = run_driver_arm(seed, TICKS, WINDOW_START, true, FIXED_REFUGE_K_HAZARD, bh);
        println!(
            "  base_hazard={bh}: frac={} mean_pop={:.1} size={:.1} collapsed={}",
            with.frac, with.mean_pop, with.mean_body_size, with.collapsed
        );
        assert!(with.mean_pop >= 0.0, "base_hazard={bh} produced a nonsensical negative mean_pop");
        // Verify that sweep produces distinct regimes (not identi cal across all values).
        if let Some(prev) = last_frac {
            if with.frac != prev {
                distinct_count += 1;
            }
        }
        last_frac = Some(with.frac);
    }
    assert!(
        distinct_count > 0,
        "base_hazard sweep must produce at least one distinct regime (not all identical)"
    );
}

/// DL-0.5 D2b: calibration unit test for DOL precondition-probe config.
/// Verifies that dol_probe_config's developmental GRN produces ≥2 modules AND a germ/soma mix,
/// across a size sweep. This is the fast gate (runs locally in CI, no cloud sim-run) proving the
/// config is not degenerate BEFORE the expensive ecological probe.
#[test]
fn dol_probe_config_produces_germ_soma_mix() {
    if cfg!(debug_assertions) {
        return;
    }

    let econ = dol_probe_config(SEED).econ;

    // Size sweep: test a range of founder sizes to confirm mix is not size-dependent fluke
    let sizes = vec![2, 4, 8, 16, 32];
    let mut sizes_with_mix = 0;

    for &size in &sizes {
        let mut g = Genome::founder(2).with_specs(
            econ.grn.clone().map(std::sync::Arc::new),
            econ.morphogen.clone(),
        );
        g.size = size;

        let ph = g.decode(&econ).expect("dol_probe_config genome must decode to Some");
        let n_modules = ph.graph.module_cell_count.len();
        let germ_cells: i64 = ph.graph.module_cell_count.iter()
            .zip(ph.graph.module_is_germ.iter())
            .filter_map(|(&count, &is_germ)| if is_germ { Some(count as i64) } else { None })
            .sum();
        let soma_cells: i64 = ph.graph.module_cell_count.iter()
            .zip(ph.graph.module_is_germ.iter())
            .filter_map(|(&count, &is_germ)| if !is_germ { Some(count as i64) } else { None })
            .sum();

        // Check both conditions: ≥2 modules AND both germ and soma present
        let has_modules = n_modules >= 2;
        let has_germ = germ_cells > 0;
        let has_soma = soma_cells > 0;
        let has_mix = has_germ && has_soma;

        if has_modules && has_mix {
            sizes_with_mix += 1;
        }
    }

    // At least half of the sizes should produce a germ/soma mix
    assert!(
        sizes_with_mix >= 3,  // 3 out of 5 sizes
        "dol_probe_config must produce germ/soma mix for at least 3 sizes in [2,4,8,16,32]; \
         got {sizes_with_mix}/5. Check GRN spec (input_weights must be live-drive [8,0], not [0,0]) \
         and germ_threshold setting."
    );
}

/// DL-0.5 D3: DOL precondition-probe ecological test — `#[ignore]`d, run via sim-run cloud scenario.
/// Measures whether dol_probe_config bodies develop ≥2 modules, split into germ/soma mix, and
/// whether germ_frac is separable from body_size (low correlation = axis exists).
#[test]
#[ignore]
fn dol_precondition_probe() {
    let ticks: u64 = std::env::var("DOL_PROBE_TICKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2000);

    // Measure across a small seed set
    let seeds = vec![1u64, 2, 3];

    for seed in seeds {
        let mut sim = build_sim(dol_probe_config(seed));
        for _ in 0..ticks {
            sim.step();
        }

        // Snapshot the live population
        let snapshot = sim.cellgraph_snapshot();
        let population = snapshot.len() as u64;

        if population == 0 {
            println!("DOL-PROBE-DIAG: seed={} population=0", seed);
            continue;
        }

        // Compute per-body germ_frac and aggregate stats
        let mut n_modules_sum: i64 = 0;
        let mut germ_frac_list: Vec<i64> = Vec::new();  // Stored as ×1000 (fixed-point)
        let mut germ_cells_total: i64 = 0;
        let mut soma_cells_total: i64 = 0;
        let mut total_cells_total: i64 = 0;
        let mut body_sizes: Vec<i64> = Vec::new();

        for &(n_modules, germ_cells, soma_cells, total_cells) in &snapshot {
            n_modules_sum += n_modules as i64;
            germ_cells_total += germ_cells;
            soma_cells_total += soma_cells;
            total_cells_total += total_cells;

            if total_cells > 0 {
                let germ_frac_x1000 = (germ_cells * 1000) / total_cells;
                germ_frac_list.push(germ_frac_x1000);
                body_sizes.push(total_cells);
            }
        }

        let mean_modules = if population > 0 { n_modules_sum as f64 / population as f64 } else { 0.0 };
        let mean_germ = if population > 0 { germ_cells_total as f64 / population as f64 } else { 0.0 };
        let mean_soma = if population > 0 { soma_cells_total as f64 / population as f64 } else { 0.0 };
        let mean_total = if population > 0 { total_cells_total as f64 / population as f64 } else { 0.0 };

        // Compute germ_frac deciles
        germ_frac_list.sort_unstable();
        let mut deciles = String::new();
        for d in 1..=10 {
            let idx = (d as usize * germ_frac_list.len() / 10).saturating_sub(1).min(germ_frac_list.len() - 1);
            let val = germ_frac_list.get(idx).unwrap_or(&0);
            deciles.push_str(&format!("{}", val / 100));  // Convert ×1000 to ×100 for display
            if d < 10 {
                deciles.push(',');
            }
        }

        // Compute correlation(germ_frac, total_cells) as a covariance-sign proxy
        // Simplified: correlation = sign(covariance) + magnitude
        let mut cov_sum: i64 = 0;
        let mean_gf = germ_frac_list.iter().sum::<i64>() as f64 / germ_frac_list.len().max(1) as f64;
        let mean_size = body_sizes.iter().sum::<i64>() as f64 / body_sizes.len().max(1) as f64;

        if germ_frac_list.len() > 0 {
            for i in 0..germ_frac_list.len() {
                let gf = germ_frac_list[i] as f64;
                let sz = body_sizes[i] as f64;
                cov_sum += ((gf - mean_gf) * (sz - mean_size)) as i64;
            }
        }

        let corr_germfrac_size = if germ_frac_list.len() > 0 {
            cov_sum / (germ_frac_list.len() as i64).max(1)
        } else {
            0
        };

        println!(
            "DOL-PROBE-DIAG: seed={} population={} mean_modules={:.2} germ_frac_deciles=[{}] \
             mean_germ={:.1} mean_soma={:.1} mean_total={:.1} corr_germfrac_size={}",
            seed, population, mean_modules, deciles, mean_germ, mean_soma, mean_total, corr_germfrac_size
        );
    }
}
