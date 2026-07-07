//! P5-1b Eh-gradient precondition gate: verifies the world substrate stratifies spatially
//! (O₂ ↔ NO₃ gradient) BEFORE the P5-4 verdict harness invests in a-d selection testing.
//!
//! **Two-tier NULL attribution (F11 resolution):** This gate separates PRECONDITION-NULL
//! (world substrate is flat, diffusion homogenized the initial gradient) from
//! ARCHITECTURE-NULL (gradient forms, but the mechanic is genuinely marginal). P5-1b is the
//! PRECONDITION gate — if it ever fails, that signals world-params are insufficient for the
//! redox niche and P5 closes as substrate-out-of-scope, distinct from a mechanic fault.
//!
//! **No NO₃ consumption (P5-2) needed:** The gradient is the SUM of:
//!   - O₂ DEPLETION: respiration consumes O₂ in populated cells (P1 proven-faithful).
//!   - NO₃ ANTI-CORRELATION: NO₃ biome-inverse caps + diffusion-only persistence (P5-0 proved
//!     the inverse cap structure; P5-1b adds the **live measurement** that diffusion does NOT
//!     homogenize it).
//!
//! The gradient is **golden-NEUTRAL:** the probe is `&self` read-only (no mutation),
//! reads existing conserved resources, zero behavior change. Same seed ⇒ identical probe report
//! (pure integer read of a deterministic sim). No golden re-pin.
//!
//! **Partition-axis assumption (F2):** The probe partitions by live-O₂ VALUE (median split),
//! not by depth, because animata's redox is biome-horizontal (aerated forest vs anaerobic
//! wetland cells), not a water-column depth structure. Valid because the world is biome-stratified
//! and NO₃ caps are inverse-initialised per biome (P5-0).
//!
//! **Pre-trial guard constant (F3):** The flat-band (5%) is PRE-DECLARED (dive §3.5), not
//! empirically calibrated. Adjustable ONLY on horizon-insufficiency (sim too short), never tuned
//! to force a pass/fail. A genuine flat result is a real finding (honest-NULL discipline).

use cli::{build_sim, redox_precondition_config};

/// Fixed seed verified at authoring to have biome-rich world (both aerated and anaerobic zones).
/// This ensures the precondition test runs on a representative substrate, not an edge case.
/// Verified to produce Wetland/Floodplain (high NO₃) AND Forest/Grassland (high O₂) biomes
/// in the same world, as confirmed in P5-0's nitrate_field test suite.
const TEST_SEED: u64 = 0x2222_2222;

/// Ticks for the live-field measurement. ~500 ticks allows respiration to establish O₂ depletion
/// in populated cells and diffusion to reach a quasi-equilibrium (not homogeneous).
const MEASURE_TICKS: u64 = 500;

/// Pre-declared flat-band guard constant: 5% of the pooled mean of each field.
/// Gradient detection requires both O₂ stratification AND NO₃ anti-correlation to exceed this.
/// Adjustable ONLY on horizon-insufficiency (sim too short), never tuned to force a result.
const FLAT_BAND_FRAC: f64 = 0.05;

#[test]
fn p5_1b_redox_precondition_gate() {
    let cfg = redox_precondition_config(TEST_SEED);
    let mut sim = build_sim(cfg);

    // Run the sim for ~500 ticks to allow O₂ depletion and diffusion to establish gradients.
    for _ in 0..MEASURE_TICKS {
        sim.step();
    }

    // **DIAGNOSTIC DUMP: Replace assertions with raw statistics collection.**
    // This test is a temporary diagnostic to surface the live-field distribution.
    // The panic at the end is intentional — it carries the REDOX-DIAG output to CI.

    let probe = sim.redox_precondition_probe();
    let all_cells = sim.redox_cell_probe();
    let n_livable = all_cells.len();

    // 1. O₂ distribution: min, p10, p25, median, p75, p90, max, mean
    let mut o2_vals: Vec<i64> = all_cells.iter().map(|&(o2, _)| o2).collect();
    o2_vals.sort_unstable();

    let o2_min = o2_vals.first().copied().unwrap_or(0);
    let o2_max = o2_vals.last().copied().unwrap_or(0);
    let o2_mean = if !o2_vals.is_empty() {
        o2_vals.iter().sum::<i64>() / o2_vals.len() as i64
    } else {
        0
    };
    let percentile = |vals: &[i64], p: usize| -> i64 {
        if vals.is_empty() {
            0
        } else {
            let idx = (vals.len() * p) / 100;
            vals[std::cmp::min(idx, vals.len() - 1)]
        }
    };

    let o2_p10 = percentile(&o2_vals, 10);
    let o2_p25 = percentile(&o2_vals, 25);
    let o2_median = percentile(&o2_vals, 50);
    let o2_p75 = percentile(&o2_vals, 75);
    let o2_p90 = percentile(&o2_vals, 90);

    // 2. NO₃ distribution: min, p10, p25, median, p75, p90, max, mean
    let mut no3_vals: Vec<i64> = all_cells.iter().map(|&(_, no3)| no3).collect();
    no3_vals.sort_unstable();

    let no3_min = no3_vals.first().copied().unwrap_or(0);
    let no3_max = no3_vals.last().copied().unwrap_or(0);
    let no3_mean = if !no3_vals.is_empty() {
        no3_vals.iter().sum::<i64>() / no3_vals.len() as i64
    } else {
        0
    };

    let no3_p10 = percentile(&no3_vals, 10);
    let no3_p25 = percentile(&no3_vals, 25);
    let no3_median = percentile(&no3_vals, 50);
    let no3_p75 = percentile(&no3_vals, 75);
    let no3_p90 = percentile(&no3_vals, 90);

    // 3. Count cells with low O₂ thresholds
    let count_o2_lt5 = all_cells.iter().filter(|(o2, _)| *o2 < 5).count();
    let count_o2_lt10 = all_cells.iter().filter(|(o2, _)| *o2 < 10).count();
    let count_o2_lt20 = all_cells.iter().filter(|(o2, _)| *o2 < 20).count();

    let frac_lt5 = if n_livable > 0 { (count_o2_lt5 as f64 / n_livable as f64) * 100.0 } else { 0.0 };
    let frac_lt10 = if n_livable > 0 { (count_o2_lt10 as f64 / n_livable as f64) * 100.0 } else { 0.0 };
    let frac_lt20 = if n_livable > 0 { (count_o2_lt20 as f64 / n_livable as f64) * 100.0 } else { 0.0 };

    // 4. For cells with o2_live < 10, mean and max NO₃
    let low_o2_cells: Vec<(i64, i64)> = all_cells.iter().filter(|&&(o2, _)| o2 < 10).copied().collect();
    let (low_o2_no3_mean, low_o2_no3_max) = if !low_o2_cells.is_empty() {
        let sum: i64 = low_o2_cells.iter().map(|&(_, no3)| no3).sum();
        let mean = sum / low_o2_cells.len() as i64;
        let max = low_o2_cells.iter().map(|&(_, no3)| no3).max().unwrap_or(0);
        (mean, max)
    } else {
        (0, 0)
    };

    // 5. O₂-CAP partition analysis: median split and per-bucket stats
    let mut o2_split: Vec<i64> = all_cells.iter().map(|&(o2, _)| o2).collect();
    o2_split.sort_unstable();
    let o2_cap_median = if !o2_split.is_empty() {
        o2_split[o2_split.len() / 2]
    } else {
        0
    };

    let low_o2_bucket: Vec<(i64, i64)> = all_cells.iter().filter(|&&(o2, _)| o2 < o2_cap_median).copied().collect();
    let high_o2_bucket: Vec<(i64, i64)> = all_cells.iter().filter(|&&(o2, _)| o2 >= o2_cap_median).copied().collect();

    let (mean_o2_low_bkt, mean_no3_low_bkt) = if !low_o2_bucket.is_empty() {
        let o2_sum: i64 = low_o2_bucket.iter().map(|&(o2, _)| o2).sum();
        let no3_sum: i64 = low_o2_bucket.iter().map(|&(_, no3)| no3).sum();
        (o2_sum / low_o2_bucket.len() as i64, no3_sum / low_o2_bucket.len() as i64)
    } else {
        (0, 0)
    };

    let (mean_o2_high_bkt, mean_no3_high_bkt) = if !high_o2_bucket.is_empty() {
        let o2_sum: i64 = high_o2_bucket.iter().map(|&(o2, _)| o2).sum();
        let no3_sum: i64 = high_o2_bucket.iter().map(|&(_, no3)| no3).sum();
        (o2_sum / high_o2_bucket.len() as i64, no3_sum / high_o2_bucket.len() as i64)
    } else {
        (0, 0)
    };

    // Build the diagnostic output message.
    let diag_msg = format!(
        "REDOX-DIAG: Live-field distribution at tick {}\n\
         \n\
         === LIVABLE CELLS ===\n\
         n_livable: {}\n\
         \n\
         === O₂ LIVE DISTRIBUTION (over {} livable cells) ===\n\
         min: {}, p10: {}, p25: {}, median: {}, p75: {}, p90: {}, max: {}, mean: {}\n\
         \n\
         === NO₃ LIVE DISTRIBUTION (over {} livable cells) ===\n\
         min: {}, p10: {}, p25: {}, median: {}, p75: {}, p90: {}, max: {}, mean: {}\n\
         \n\
         === LOW O₂ CANDIDATE ANAEROBIC-REFUGE CELLS ===\n\
         cells with o2_live < 5:  {} ({:.1}%)\n\
         cells with o2_live < 10: {} ({:.1}%)\n\
         cells with o2_live < 20: {} ({:.1}%)\n\
         \n\
         === STATS WITHIN LOW-O₂ CELLS (o2_live < 10) ===\n\
         count: {}\n\
         mean no3_live: {}\n\
         max no3_live: {}\n\
         \n\
         === O₂-CAP PARTITION (median split at o2={}) ===\n\
         Low-O₂ bucket (n={}): mean_o2={}, mean_no3={}\n\
         High-O₂ bucket (n={}): mean_o2={}, mean_no3={}\n\
         \n\
         === PROBE DATA (for reference) ===\n\
         probe.n_livable: {}\n\
         probe.mean_o2_low_bucket: {}, probe.mean_o2_high_bucket: {}\n\
         probe.mean_no3_low_o2: {}, probe.mean_no3_high_o2: {}\n",
        MEASURE_TICKS,
        n_livable,
        n_livable,
        o2_min, o2_p10, o2_p25, o2_median, o2_p75, o2_p90, o2_max, o2_mean,
        no3_vals.len(),
        no3_min, no3_p10, no3_p25, no3_median, no3_p75, no3_p90, no3_max, no3_mean,
        count_o2_lt5, frac_lt5,
        count_o2_lt10, frac_lt10,
        count_o2_lt20, frac_lt20,
        low_o2_cells.len(),
        low_o2_no3_mean,
        low_o2_no3_max,
        o2_cap_median,
        low_o2_bucket.len(), mean_o2_low_bkt, mean_no3_low_bkt,
        high_o2_bucket.len(), mean_o2_high_bkt, mean_no3_high_bkt,
        probe.n_livable,
        probe.mean_o2_low_bucket, probe.mean_o2_high_bucket,
        probe.mean_no3_low_o2, probe.mean_no3_high_o2
    );

    panic!("{}", diag_msg);
}
