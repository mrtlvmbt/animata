//! W-9 Phase-0 measurement + sweep bin — measure talus_step_final effectiveness across config grid.
//! Usage: w9_sweep [phase0|sweep|all] [dim]
//! Outputs: crest counts (Phase-0), then sweep table (needles, max-step, p10 retention per landform, clip count)

use world::gen::caps::{
    classify_and_caps_staged, landform_amplitudes, measure_needles, measure_max_local_step,
    measure_de_needle_clip_count, AMPLITUDE_FLOOR,
};
use world::gen::erosion::{de_needle_pass, talus_step_final};

const HMAX: i64 = 200;

fn phase0_measurement(dim: usize) {
    println!("\n=== PHASE-0 MEASUREMENT (crest counts at AMPLITUDE_FLOOR={}@{}x2 seeds) ===", AMPLITUDE_FLOOR, dim);

    for seed in [1u64, 42] {
        let (_, staged, masks) = classify_and_caps_staged(
            seed, HMAX, dim, false, true, true, true, true, true, false  // All landforms ON, talus OFF
        );

        println!("\nSeed {} (all landforms ON, dim={})", seed, dim);
        println!("  Post-coastal phase (no talus smoothing yet):");

        let edifice_report = landform_amplitudes(dim, &staged.post_coastal, &staged.post_coastal, &masks.edifice);
        let till_report = landform_amplitudes(dim, &staged.post_coastal, &staged.post_coastal, &masks.till);
        let dune_report = landform_amplitudes(dim, &staged.post_coastal, &staged.post_coastal, &masks.dune);

        println!("    edifice crests: {} (median ring-1)", edifice_report.crest_count);
        println!("    till crests: {} (median ring-1)", till_report.crest_count);
        println!("    dune crests: {} (median ring-1)", dune_report.crest_count);

        let min_required = if dim == 512 { 16 } else { 4 };
        let precondition_ok = edifice_report.crest_count >= min_required &&
                             till_report.crest_count >= min_required &&
                             dune_report.crest_count >= min_required;
        println!("    PRECONDITION (>={} crests per mask): {}", min_required,
            if precondition_ok { "✓ PASS" } else { "✗ FAIL (using fallback crest detection)" });
    }
}

fn sweep_measurement(dim: usize) {
    println!("\n=== SWEEP GRID (SPIKE_MARGIN x iters @dim={}) ===", dim);
    println!("Margin Iter | Needles | MaxSpike | Edifice% | Till% | Dune% | DeNeedleClips");
    println!("------|------|---------|----------|----------|-------|-------|---------------");

    let spike_margins = [8i64, 12, 16];  // W-9 selective donor rule (NOT 24)
    let iters = [2usize, 4, 8];
    let seed = 1u64;

    // Generate base map once (with all landforms ON, talus OFF to get post-coastal)
    let (_, staged, masks) = classify_and_caps_staged(
        seed, HMAX, dim, false, true, true, true, true, true, false
    );

    for spike_margin in &spike_margins {
        for iter in &iters {
            // Apply talus_step_final with selective donor rule (spike_margin, iter) config
            let post_talus = talus_step_final(dim, &staged.post_coastal, *spike_margin, *iter);

            // Apply de_needle to post_talus (to measure clip count)
            let post_deneedle = de_needle_pass(dim, &post_talus);

            // Measure metrics on post-talus field
            let (needle_count, _) = measure_needles(dim, &post_talus);
            let max_spike = measure_max_local_step(dim, &post_talus);

            // Measure amplitude retention per landform (pre=post_coastal, post=post_talus)
            let edifice_report = landform_amplitudes(dim, &staged.post_coastal, &post_talus, &masks.edifice);
            let till_report = landform_amplitudes(dim, &staged.post_coastal, &post_talus, &masks.till);
            let dune_report = landform_amplitudes(dim, &staged.post_coastal, &post_talus, &masks.dune);

            // De-needle clip count on post-talus
            let clip_count = measure_de_needle_clip_count(dim, &post_talus, &post_deneedle);

            // Gate check: needles==0 AND max_second_spike<=12 (MAX_SPIKE_FINAL)
            let gate_pass = needle_count == 0 && max_spike <= 12;

            println!(
                "{:3}  {:3}  | {:7} | {:8} | {:8} | {:5} | {:5} | {:13} {}",
                spike_margin, iter, needle_count, max_spike,
                edifice_report.p10_retention_pct, till_report.p10_retention_pct, dune_report.p10_retention_pct,
                clip_count,
                if gate_pass { "✓ PASS" } else { "✗ FAIL" }
            );
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("all");
    let dim: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(512);

    eprintln!("W-9 Sweep Measurement (PM-authorized local run)");
    match mode {
        "phase0" => phase0_measurement(dim),
        "sweep" => sweep_measurement(dim),
        _ => {
            phase0_measurement(dim);
            sweep_measurement(dim);
        }
    }
}
