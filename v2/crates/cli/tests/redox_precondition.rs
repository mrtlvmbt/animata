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
const TEST_SEED: u64 = 0x1111_1111;

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

    // Measure the Eh-gradient at the end state.
    let probe = sim.redox_precondition_probe();

    // **Criterion (1): O₂ stratifies (not flat).**
    // The live O₂ field is spatially structured by respiration + biome source → chemocline forms.
    // Assert: mean_o2_high_bucket > mean_o2_low_bucket by a clear margin (>5% flat-band).
    {
        let pooled_o2_mean =
            (probe.mean_o2_low_bucket as f64 + probe.mean_o2_high_bucket as f64) / 2.0;
        let o2_diff = (probe.mean_o2_high_bucket - probe.mean_o2_low_bucket) as f64;
        let o2_sep_frac = if pooled_o2_mean > 0.0 {
            o2_diff / pooled_o2_mean
        } else {
            0.0
        };

        assert!(
            o2_sep_frac > FLAT_BAND_FRAC,
            "O₂ NOT stratified (PRECONDITION-NULL): separation={:.2}% < flat-band {:.0}%. \
             mean_o2_low_bucket={}, mean_o2_high_bucket={}, pooled_mean={}",
            o2_sep_frac * 100.0,
            FLAT_BAND_FRAC * 100.0,
            probe.mean_o2_low_bucket,
            probe.mean_o2_high_bucket,
            pooled_o2_mean as i64
        );
    }

    // **Criterion (2): NO₃ anti-correlates with O₂ (the Eh-gradient).**
    // NO₃ is higher exactly where live O₂ is lower. After diffusion at tick ~500, the inverse
    // gradient SURVIVES (not homogenized). Assert: mean_no3_low_o2 > mean_no3_high_o2 by a
    // clear margin (>5% flat-band).
    {
        let pooled_no3_mean =
            (probe.mean_no3_low_o2 as f64 + probe.mean_no3_high_o2 as f64) / 2.0;
        let no3_diff = (probe.mean_no3_low_o2 - probe.mean_no3_high_o2) as f64;
        let no3_sep_frac = if pooled_no3_mean > 0.0 {
            no3_diff / pooled_no3_mean
        } else {
            0.0
        };

        assert!(
            no3_sep_frac > FLAT_BAND_FRAC,
            "NO₃ NOT anti-correlated with O₂ (PRECONDITION-NULL): separation={:.2}% < flat-band {:.0}%. \
             mean_no3_low_o2={}, mean_no3_high_o2={}, pooled_mean={}",
            no3_sep_frac * 100.0,
            FLAT_BAND_FRAC * 100.0,
            probe.mean_no3_low_o2,
            probe.mean_no3_high_o2,
            pooled_no3_mean as i64
        );
    }

    // **Criterion (4): Determinism + neutrality.**
    // Same seed ⇒ identical probe report. Golden stays green (no edits to any *_golden / state_checksum).
    // This is implicit: the probe is pure integer read of a deterministic sim.
    // Real proof = `bash scripts/ci-report.sh` → exit 0 with no `*_golden*` diffs on HEAD.

    // **Criterion (5): PRECONDITION-NULL semantics documented.**
    // If this test ever fails: gradient did NOT form at default 64×64 (expected PASS).
    // That is the PRECONDITION-NULL signal → world-params insufficient, distinct from
    // ARCHITECTURE-NULL (gradient forms but mechanic is marginal, tested later in P5-4).
    // Do NOT knob-crank to force a pass — a genuine flat result is a real finding.
}
