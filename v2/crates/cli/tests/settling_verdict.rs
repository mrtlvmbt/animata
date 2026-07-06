//! P4/SL-3 settling faithful-verdict (a-d criteria): does the settling mechanic produce an
//! emergent STABLE size intermediate that is reversible, persistent, and costed?
//!
//! Two arms on the SAME `settling_config(seed)` (SL-2: static-O₂ deficit, evolve_body_size, g_dev=1
//! unicellular founder), differing ONLY in the settling toggle:
//!   WITH     (settling=Some(...)): the shipped P4/SL-1 behaviour — settling-selection pulse active.
//!   ABLATION (settling=None):      settling off (stage_settling no-op via the existing SL-1 gate).
//!
//! FAITHFUL (crits. a/d/b) iff, across a seed-majority: mean body size / multicellular fraction is
//! MEASURABLY HIGHER in WITH than in ABLATION (settling pulls bodies up), AND the two arms differ
//! by a real margin (not noise). Further, the WITH arm shows an INTERMEDIATE equilibrium in the
//! late window (body size > 1, below a ceiling, drift < epsilon). If the differential is absent OR
//! the WITH arm maxes out or still-climbs → NULL, reported honestly. The intermediate-persistence
//! claim is: evolved equilibrium (many generations in late window), not instantaneous survivorship,
//! measured via drift across first-half vs second-half of the late window.
//!
//! Heavy (2 arms × N seeds × long horizon) — `#[ignore]`d in CI; run via the `settling-verdict`
//! sim-run scenario (`cargo test --release -p cli -- settling_verdict --nocapture --ignored`).
//! Horizon via env `SETTLING_VERDICT_TICKS` (default 400 for fast local; cloud overrides ~4000).

use cli::{build_sim, settling_config};

const VERDICT_SEEDS: [u64; 5] = [1, 2, 3, 4, 5];
const POP_FLOOR: i64 = 5; // below this the arm is a collapse, not a measurement
/// Minimum relative gap (WITH body size above ABLATION) to count a seed as "settling lifted bodies".
/// 5% — small but real; below this is noise.
const REL_MARGIN: f64 = 0.05;
const SEED_MAJORITY: usize = 3; // ≥3/5 seeds must show the differential
/// Floor (scaled units, BODY_SIZE_SCALE=256) for the WITH arm's mean body size to claim it is
/// genuinely multicellular (>1 cell), not unicellular. 1 cell = 256 in scaled units.
/// TODO PM pass-2
const INTERMEDIATE_FLOOR: i64 = 256; // >1 cell = genuinely multicellular (BODY_SIZE_SCALE=256)
/// Ceiling (scaled units, BODY_SIZE_SCALE=256) for the WITH arm's mean body size to claim it is a
/// genuine INTERMEDIATE (not maxed to the morphogen cap). <12 of the g_dev=4 cap (4096 = 16 cells).
/// TODO PM pass-2
const INTERMEDIATE_CEIL: i64 = 3072; // <12 of the 16-cell g_dev=4 cap (4096) → genuine intermediate, not maxed
/// Late-window drift as a fraction of equilibrium body size. 5% — small stable variation, not still-climbing.
/// TODO PM pass-2
const DRIFT_FRAC: f64 = 0.05; // late-window drift / mean_body_size < 5%

struct ArmResult {
    mean_body_size: f64,
    mean_frac: f64,
    mean_pop: f64,
    collapsed: bool,
    drift: f64, // |first-half mean − second-half mean| for (b-iii) equilibrium
}

fn run_settling_arm(seed: u64, ticks: u64, window_start: u64, ablate: bool) -> ArmResult {
    let mut cfg = settling_config(seed);
    // Ablation: turn off settling via the existing Option gate (NOT a new field).
    if ablate {
        cfg.econ.settling = None;
    }
    let mut sim = build_sim(cfg);
    let mut body_sum = 0.0;
    let mut frac_sum: i64 = 0;
    let mut valid: i64 = 0;
    let mut pop_sum: i64 = 0;
    let mut pop_ticks: i64 = 0;

    // For drift calculation: split the window into first and second halves.
    let window_len = (ticks - window_start).max(1);
    let half_len = (window_len + 1) / 2;
    let window_mid = window_start + half_len;

    let mut first_half_body_sum = 0.0;
    let mut first_half_valid = 0i64;
    let mut second_half_body_sum = 0.0;
    let mut second_half_valid = 0i64;

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

                // Track first/second half for drift calculation.
                if t < window_mid {
                    first_half_body_sum += tel.mean_body_size as f64;
                    first_half_valid += 1;
                } else {
                    second_half_body_sum += tel.mean_body_size as f64;
                    second_half_valid += 1;
                }
            }
        }
    }

    let mean_pop = if pop_ticks > 0 { pop_sum as f64 / pop_ticks as f64 } else { 0.0 };

    if valid == 0 {
        ArmResult {
            mean_body_size: 0.0,
            mean_frac: 0.0,
            mean_pop,
            collapsed: true,
            drift: 0.0,
        }
    } else {
        let mean_body_size = body_sum / valid as f64;
        let first_half_mean =
            if first_half_valid > 0 { first_half_body_sum / first_half_valid as f64 } else { 0.0 };
        let second_half_mean =
            if second_half_valid > 0 { second_half_body_sum / second_half_valid as f64 } else { 0.0 };
        let drift = (first_half_mean - second_half_mean).abs();

        ArmResult {
            mean_body_size,
            mean_frac: frac_sum as f64 / valid as f64,
            mean_pop,
            collapsed: false,
            drift,
        }
    }
}

#[test]
#[ignore]
fn settling_verdict() {
    let ticks: u64 = std::env::var("SETTLING_VERDICT_TICKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(400);
    let window_len = ticks.min(1000);
    let window_start = ticks - window_len;

    println!(
        "\nP4/SL-3 settling faithful-verdict (crits. a/d/b): evolved size intermediate from settling?"
    );
    println!(
        "EVOLVED-EQUILIBRIUM FRAMING: the (b) intermediate measurement is a LATE-WINDOW heritable mean"
    );
    println!(
        "  (many generations past the founder), NOT an instantaneous survivorship snapshot. Body size"
    );
    println!("  is heritable (evolve_body_size=true + g_dev), so a persistent late-window shift means");
    println!(
        "  larger-bodied lineages survived settling rounds and reproduced (group-level selection)."
    );
    println!("PRE-DECLARED THRESHOLDS (F1/F3 falsifiability, BODY_SIZE_SCALE=256, 1 cell=256):");
    println!("  (a/d) reversible/conditional: WITH body size > ABLATION by ≥{:.0}% in ≥{SEED_MAJORITY}/{} seeds", REL_MARGIN * 100.0, VERDICT_SEEDS.len());
    println!(
        "  (b-i) intermediate-genuine: WITH late-window mean body size > {INTERMEDIATE_FLOOR} (>1 cell, multicellular)"
    );
    println!(
        "  (b-ii) intermediate-bounded: WITH late-window mean body size < {INTERMEDIATE_CEIL} (<12 cells, not maxed to g_dev=4 cap)"
    );
    println!(
        "  (b-iii) intermediate-stable: WITH late-window drift / mean_body_size < {:.0}%", DRIFT_FRAC * 100.0
    );
    println!("  (c) measurable-cost: hypoxia structural cost persists in ABLATION (proven SL-2)");
    println!("ticks={ticks}  late-window=[{window_start},{ticks}]  config=settling_config");
    println!("\n seed |  WITH body |  ABL body | Δ%(WITH↑) | WITH frac | ABL frac | WITH pop | ABL pop | WITH drift | verdict");
    println!("------+------------+-----------+-----------+-----------+----------+----------+---------+------------+--------");

    let mut seeds_showing = 0usize;
    let mut both_alive = 0usize;
    let mut intermediates = 0usize;

    for &seed in &VERDICT_SEEDS {
        let with = run_settling_arm(seed, ticks, window_start, false);
        let abl = run_settling_arm(seed, ticks, window_start, true);

        let rel_gap = if abl.mean_body_size > 0.0 {
            (with.mean_body_size - abl.mean_body_size) / abl.mean_body_size
        } else {
            0.0
        };

        let alive = !with.collapsed && !abl.collapsed;
        let shows_differential = alive && rel_gap >= REL_MARGIN;
        let is_intermediate = alive
            && (with.mean_body_size as i64) > INTERMEDIATE_FLOOR
            && (with.mean_body_size as i64) < INTERMEDIATE_CEIL
            && with.mean_body_size > 0.0
            && with.drift / with.mean_body_size < DRIFT_FRAC;

        if alive {
            both_alive += 1;
        }
        if shows_differential {
            seeds_showing += 1;
        }
        if is_intermediate {
            intermediates += 1;
        }

        println!(
            "  {seed}   |   {:8.3} |  {:8.3} |  {:+7.1}% |  {:8.1} | {:8.1} | {:8.0} | {:7.0} |     {:7.3} | {}",
            with.mean_body_size,
            abl.mean_body_size,
            rel_gap * 100.0,
            with.mean_frac,
            abl.mean_frac,
            with.mean_pop,
            abl.mean_pop,
            with.drift,
            if is_intermediate {
                "intermediate"
            } else if !alive {
                "collapse"
            } else if shows_differential {
                "differential"
            } else {
                "flat"
            }
        );
    }

    println!(
        "\nRESULT (a/d): {seeds_showing}/{} seeds show WITH body size > ABLATION by ≥{:.0}%",
        VERDICT_SEEDS.len(),
        REL_MARGIN * 100.0
    );
    println!(
        "RESULT (b): {intermediates}/{} seeds show WITH intermediate (>{INTERMEDIATE_FLOOR}, <{INTERMEDIATE_CEIL}, drift<{:.0}%)",
        VERDICT_SEEDS.len(),
        DRIFT_FRAC * 100.0
    );
    println!("(both-alive={both_alive}, POP_FLOOR={POP_FLOOR})");

    let faithful = seeds_showing >= SEED_MAJORITY && intermediates >= SEED_MAJORITY;
    println!(
        "VERDICT: {}",
        if faithful {
            "FAITHFUL — settling mechanic produces emergent stable size intermediate (P4 Phase-2 close on 2 drivers)"
        } else {
            "NULL — no reproducible intermediate from settling (reported honestly, not knob-cranked)"
        }
    );

    // This is an OBSERVATIONAL verdict: the run always completes and PRINTS the verdict for the PM to
    // read. It asserts only the harness sanity (arms ran, not universal collapse) — the FAITHFUL/NULL
    // call is a readout, not a hard CI gate. Golden-neutral: `#[ignore]` means this never runs in CI.
    assert!(
        both_alive >= SEED_MAJORITY,
        "harness failure: {both_alive}/{} seeds had BOTH arms viable — cannot read a verdict (raise ticks / check config)",
        VERDICT_SEEDS.len()
    );
}
