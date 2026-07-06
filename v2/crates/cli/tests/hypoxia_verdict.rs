//! P1-2b faithful-verdict (a-d criterion c): does the O₂-diffusion hypoxia cost GROUND settling —
//! i.e. does it selectively penalise LARGER multicellular bodies, so smaller bodies win in an
//! O₂-scarce aerobic environment (R35), rather than being a generic size-knob?
//!
//! Two arms on the SAME `phase2_oxygen_config` (identical morphogen + O₂ economy + seeds), differing
//! ONLY in `econ.ablate_hypoxia`:
//!   WITH     (ablate_hypoxia=false): the shipped P1-2b behaviour — hypoxia scales inner-cell yield down.
//!   ABLATION (ablate_hypoxia=true):  hypoxia≡0, everything else identical (the control).
//!
//! FAITHFUL (crit. c) iff, across a seed-majority: mean body size / multicellular fraction is
//! MEASURABLY LOWER in WITH than in ABLATION (hypoxia pushes bodies down), AND the two arms differ
//! by a real margin (not noise). If the differential is absent → NULL, reported honestly. The margin
//! comes from O₂-diffusion physics alone (no explicit size-cost), so a positive result means settling
//! is grounded emergently.
//!
//! Heavy (2 arms × N seeds × long horizon) — `#[ignore]`d in CI; run via the `hypoxia-verdict`
//! sim-run scenario (`cargo test --release -p cli -- hypoxia_verdict --nocapture --ignored`).
//! Horizon via env `HYPOXIA_VERDICT_TICKS` (default 400 for fast local; cloud overrides ~4000).

use cli::{build_sim, phase2_oxygen_config};

const VERDICT_SEEDS: [u64; 5] = [1, 2, 3, 4, 5];
const POP_FLOOR: i64 = 5; // below this the arm is a collapse, not a measurement
/// Minimum relative gap (WITH body size below ABLATION) to count a seed as "hypoxia pressed bodies
/// down". 5% — small but real; below this is noise. The physics gives ~18-37% inner-fraction penalty
/// on N=4-64 at ~50% scarcity, so a genuine effect should clear this comfortably.
const REL_MARGIN: f64 = 0.05;
const SEED_MAJORITY: usize = 3; // ≥3/5 seeds must show the differential

struct ArmResult {
    mean_body_size: f64,
    mean_frac: f64,
    mean_pop: f64,
    collapsed: bool,
}

fn run_hypoxia_arm(seed: u64, ticks: u64, window_start: u64, ablate: bool) -> ArmResult {
    let mut cfg = phase2_oxygen_config(seed);
    cfg.econ.ablate_hypoxia = ablate;
    // phase2_oxygen_config already ships evolve_body_size=true + hypoxia_base_x1000=543 (the calibrated
    // faithful config). Allow an env override of the calibration knob for sweeps (default = config value).
    if let Some(hb) = std::env::var("HYPOXIA_BASE_X1000").ok().and_then(|s| s.parse().ok()) {
        cfg.econ.hypoxia_base_x1000 = hb;
    }
    let mut sim = build_sim(cfg);
    let mut body_sum = 0.0;
    let mut frac_sum: i64 = 0;
    let mut valid: i64 = 0;
    let mut pop_sum: i64 = 0;
    let mut pop_ticks: i64 = 0;
    for t in 0..ticks {
        sim.step();
        if t >= window_start {
            let tel = sim.telemetry();
            pop_sum += tel.population;
            pop_ticks += 1;
            if tel.population >= POP_FLOOR {
                body_sum += tel.mean_body_size as f64;
                frac_sum += tel.multicellular_frac;
                valid += 1;
            }
        }
    }
    let mean_pop = if pop_ticks > 0 { pop_sum as f64 / pop_ticks as f64 } else { 0.0 };
    if valid == 0 {
        ArmResult { mean_body_size: 0.0, mean_frac: 0.0, mean_pop, collapsed: true }
    } else {
        ArmResult {
            mean_body_size: body_sum / valid as f64,
            mean_frac: frac_sum as f64 / valid as f64,
            mean_pop,
            collapsed: false,
        }
    }
}

#[test]
#[ignore]
fn hypoxia_verdict() {
    let ticks: u64 = std::env::var("HYPOXIA_VERDICT_TICKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(400);
    let window_len = ticks.min(1000);
    let window_start = ticks - window_len;

    println!("\nP1-2b hypoxia faithful-verdict (crit. c): O₂-diffusion grounds settling?");
    println!(
        "PRE-DECLARED: WITH body size < ABLATION by ≥{:.0}% in ≥{SEED_MAJORITY}/{} seeds (else NULL)",
        REL_MARGIN * 100.0,
        VERDICT_SEEDS.len()
    );
    println!("ticks={ticks}  late-window=[{window_start},{ticks}]  config=phase2_oxygen_config");
    println!("\n seed |  WITH body |  ABL body | Δ%(WITH↓) | WITH frac | ABL frac | WITH pop | ABL pop | verdict");
    println!("------+------------+-----------+-----------+-----------+----------+----------+---------+--------");

    let mut seeds_showing = 0usize;
    let mut both_alive = 0usize;
    for &seed in &VERDICT_SEEDS {
        let with = run_hypoxia_arm(seed, ticks, window_start, false);
        let abl = run_hypoxia_arm(seed, ticks, window_start, true);
        let rel_gap = if abl.mean_body_size > 0.0 {
            (abl.mean_body_size - with.mean_body_size) / abl.mean_body_size
        } else {
            0.0
        };
        let alive = !with.collapsed && !abl.collapsed;
        let shows = alive && rel_gap >= REL_MARGIN;
        if alive {
            both_alive += 1;
        }
        if shows {
            seeds_showing += 1;
        }
        println!(
            "  {seed}   |   {:8.3} |  {:8.3} |  {:+7.1}% |  {:8.1} | {:8.1} | {:8.0} | {:7.0} | {}",
            with.mean_body_size,
            abl.mean_body_size,
            rel_gap * 100.0,
            with.mean_frac,
            abl.mean_frac,
            with.mean_pop,
            abl.mean_pop,
            if shows { "hypoxia↓" } else if !alive { "collapse" } else { "flat" }
        );
    }

    println!(
        "\nRESULT: {seeds_showing}/{} seeds show WITH body size < ABLATION by ≥{:.0}% (both-alive={both_alive})",
        VERDICT_SEEDS.len(),
        REL_MARGIN * 100.0
    );
    let faithful = seeds_showing >= SEED_MAJORITY;
    println!(
        "VERDICT: {}",
        if faithful {
            "FAITHFUL — O₂-diffusion hypoxia selectively suppresses larger bodies (settling grounded, R35 crit. c)"
        } else {
            "NULL — no reproducible size-differential from hypoxia (reported honestly, not knob-cranked)"
        }
    );

    // This is an OBSERVATIONAL verdict: the run always completes and PRINTS the verdict for the PM to
    // read (like driver_emergence). It asserts only the harness sanity (arms ran, not universal
    // collapse) — the FAITHFUL/NULL call is a readout, not a hard CI gate.
    assert!(
        both_alive >= SEED_MAJORITY,
        "harness failure: {both_alive}/{} seeds had BOTH arms viable — cannot read a verdict (raise ticks / check config)",
        VERDICT_SEEDS.len()
    );
}
