//! W-9 Phase-0 measurement + sweep bin — measure talus_step_final effectiveness across config grid.
//! Usage: w9_sweep [phase0|sweep|all] [dim]
//! Outputs: crest counts (Phase-0), then sweep table (needles, max-step, p10 retention per landform, clip count)

use world::gen::caps::{
    classify_and_caps, classify_and_caps_staged, landform_amplitudes, measure_needles, measure_max_local_step,
    count_spikes_exceeding, AMPLITUDE_FLOOR,
};
use world::gen::erosion::talus_step_final;
use world::gen::LandformFlags;
use std::io::Write;

const HMAX: i64 = 200;

/// Export heights as ATDMP1 binary (magic ATDMP1, dim u32, then dim*dim records of h:i16, material:u8).
fn export_atdmp1(path: &str, dim: usize, heights: &[i64], materials: &[u8]) -> std::io::Result<()> {
    assert_eq!(heights.len(), dim * dim);
    assert_eq!(materials.len(), dim * dim);

    let mut buf = Vec::with_capacity(8 + 4 + dim * dim * 3);
    buf.extend_from_slice(b"ATDMP1\0\0");
    buf.extend_from_slice(&(dim as u32).to_le_bytes());
    for i in 0..dim * dim {
        let h = heights[i].clamp(i16::MIN as i64, i16::MAX as i64) as i16;
        buf.extend_from_slice(&h.to_le_bytes());
        buf.push(materials[i]);
    }

    let mut fp = std::fs::File::create(path)?;
    fp.write_all(&buf)?;
    eprintln!("wrote {}", path);
    Ok(())
}

fn phase0_measurement(dim: usize) {
    println!("\n=== PHASE-0 MEASUREMENT (crest counts at AMPLITUDE_FLOOR={}@{}x2 seeds) ===", AMPLITUDE_FLOOR, dim);

    for seed in [1u64, 42] {
        let (_, staged, masks) = classify_and_caps_staged(
            seed, HMAX, dim, false, LandformFlags::from_five(true, true, true, true, true), false, true  // All landforms ON, talus OFF, enable_w10=true
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
    let spike_margins = [8i64, 12, 16];  // W-9 selective donor rule (amendment-specified)
    let iters = [2usize, 4, 8];  // Amendment-specified iterations
    let seed = 1u64;

    // Generate base map once (with all landforms ON, talus OFF to get post-coastal)
    let (_, staged, masks) = classify_and_caps_staged(
        seed, HMAX, dim, false, LandformFlags::from_five(true, true, true, true, true), false, true  // enable_w10=true
    );

    // BASELINE: Measure counts for talus-OFF (post-coastal only)
    println!("\n=== BASELINE (talus OFF @dim={}) ===", dim);
    let baseline_c12 = count_spikes_exceeding(dim, &staged.post_coastal, 12);
    let baseline_c20 = count_spikes_exceeding(dim, &staged.post_coastal, 20);
    let baseline_c30 = count_spikes_exceeding(dim, &staged.post_coastal, 30);
    println!("Seed {} (post-coastal, no talus): count(h-2max>12)={}, count(>20)={}, count(>30)={}",
        seed, baseline_c12, baseline_c20, baseline_c30);

    println!("\n=== SWEEP GRID (SPIKE_MARGIN x iters @dim={}) ===", dim);
    println!("Margin Iter | c>12 | c>20 | c>30 | MaxSpike | Till% | Gate");
    println!("------|------|----- |----- |---------|-------|------");

    for spike_margin in &spike_margins {
        for iter in &iters {
            // Apply talus_step_final with selective donor rule (spike_margin, iter) config
            let post_talus = talus_step_final(dim, &staged.post_coastal, *spike_margin, *iter);

            // Measure metrics on post-talus field
            let max_spike = measure_max_local_step(dim, &post_talus);
            let count_12 = count_spikes_exceeding(dim, &post_talus, 12);
            let count_20 = count_spikes_exceeding(dim, &post_talus, 20);
            let count_30 = count_spikes_exceeding(dim, &post_talus, 30);

            // Measure amplitude retention per landform (pre=post_coastal, post=post_talus)
            let till_report = landform_amplitudes(dim, &staged.post_coastal, &post_talus, &masks.till);

            // Gate check: needles==0 AND max_second_spike<=12 (MAX_SPIKE_FINAL)
            let (needle_count, _) = measure_needles(dim, &post_talus);
            let gate_pass = needle_count == 0 && max_spike <= 12;

            println!(
                "{:3}  {:3}  | {:4} | {:4} | {:4} | {:8} | {:5} | {}",
                spike_margin, iter, count_12, count_20, count_30, max_spike,
                till_report.p10_retention_pct,
                if gate_pass { "✓ PASS" } else { "✗ FAIL" }
            );
        }
    }
}

fn export_candidates(dim: usize) {
    let seed = 1u64;
    let out_dir = "/Users/spopov/projects/animata/A/w9-candidates";
    std::fs::create_dir_all(out_dir).expect("create candidates dir");

    // Generate base map once for staged heights
    let (_, staged, _) = classify_and_caps_staged(
        seed, HMAX, dim, false, LandformFlags::from_five(true, true, true, true, true), false, true  // enable_w10=true
    );

    // Get production materials from baseline (talus OFF) — materials don't change with talus smoothing
    let baseline = classify_and_caps(seed, HMAX, dim, false, LandformFlags::from_five(true, true, true, true, true));
    let materials = &baseline.surface_material;

    // (a) Baseline: talus OFF (post-coastal only)
    let path_a = format!("{}/candidate-a.atdmp", out_dir);
    export_atdmp1(&path_a, dim, &staged.post_coastal, materials).ok();

    // (b) SPIKE_MARGIN=12, iters=4
    let talus_b = talus_step_final(dim, &staged.post_coastal, 12, 4);
    let path_b = format!("{}/candidate-b.atdmp", out_dir);
    export_atdmp1(&path_b, dim, &talus_b, materials).ok();

    // (c) SPIKE_MARGIN=8, iters=8
    let talus_c = talus_step_final(dim, &staged.post_coastal, 8, 8);
    let path_c = format!("{}/candidate-c.atdmp", out_dir);
    export_atdmp1(&path_c, dim, &talus_c, materials).ok();

    // (d) SPIKE_MARGIN=8, iters=32 (gate-passing extreme)
    let talus_d = talus_step_final(dim, &staged.post_coastal, 8, 32);
    let path_d = format!("{}/candidate-d.atdmp", out_dir);
    export_atdmp1(&path_d, dim, &talus_d, materials).ok();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("all");
    let dim: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(512);

    eprintln!("W-9 Sweep Measurement (PM-authorized local run)");
    match mode {
        "phase0" => phase0_measurement(dim),
        "sweep" => sweep_measurement(dim),
        "export" => export_candidates(dim),
        _ => {
            phase0_measurement(dim);
            sweep_measurement(dim);
            export_candidates(dim);
        }
    }
}
