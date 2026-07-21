//! F6 carve-metric test for Slice-1b/1d: plate uplift through erosion + dendritic carving,
//! with tunable repose threshold sweep (Slice-1d).
//! Tests whether production erosion K-constants carve sharp plate peaks into dendritic valleys,
//! and sweeps repose threshold to find balanced relief (peaks + valleys).

fn main() {
    const HMAX: i64 = 200;
    const DIM: usize = 64;

    println!("=== F6 Carve-Metric Test + Repose Sweep (Slice-1d) ===");
    println!("Config: base=false, enable_plate_sim=true, erosion=true");
    println!("Strengths: {{50, 100, 200}}");
    println!("Repose thresholds: {{0, 4, 6, 8, 12}}");
    println!();

    let strengths = [50i64, 100, 200];
    let repose_thresholds = [0i64, 4, 6, 8, 12];
    let seeds = [0x1234567890ABCDEFu64];

    for &seed in &seeds {
        println!("Seed: 0x{:016X}", seed);
        println!();

        // Table header
        println!("{:6} | {:6} | {:6} | {:6} | {:6} | {:6}",
                 "Repose", "Strength", "Drainage%", "Valley_p10", "Valley_p90", "Peak_Ret");
        println!("{}", "-".repeat(60));

        for &repose in &repose_thresholds {
            for &strength in &strengths {
                // Create minimal height field from flat base + plate uplift
                let flat_base = HMAX / 2;
                let mut height = vec![flat_base; DIM * DIM];

                // Compute plate fields and uplift
                let plate_count = 15u32;
                let plate_count_clamped = world::gen::plate::clamp_plate_count(plate_count, DIM as i64);
                let plate_fields = world::gen::plate::compute_plate_fields(seed, DIM as i64, plate_count_clamped);
                let plate_uplift = world::gen::orogeny::generate_plate_uplift_field(&plate_fields, DIM as i64, HMAX, strength);

                // Add uplift to flat base
                for i in 0..DIM*DIM {
                    height[i] = (height[i] + plate_uplift[i]).clamp(0, HMAX);
                }
                let height_post_uplift = height.clone();

                // Run erosion on the plate field with the given repose threshold
                let resistance = compute_resistance(seed, DIM, HMAX);
                let erosion = world::gen::erosion::erode_from_fields(seed, HMAX, DIM, height.clone(), resistance, true, strength, repose, None, None, None);

                // Measure metrics on eroded field
                let drainage_density = compute_drainage_density(DIM, &erosion.drainage.area);
                let (valley_p10, valley_p90) = compute_valley_relief(DIM, &erosion.height);
                let peak_retained = compute_peak_retention(&erosion.height, &height_post_uplift, DIM);

                println!("{:6} | {:6} | {:6.1}% | {:9} | {:9} | {:8}",
                         repose, strength, drainage_density, valley_p10, valley_p90, peak_retained);
            }
            println!();
        }
    }

    println!("REPORT: Sweep complete. Choose repose where peak_retained ≥ 35-45 AND valley_p90 grows monotonically.");
}

fn compute_resistance(seed: u64, dim: usize, hmax: i64) -> Vec<i64> {
    const SALT: u64 = 0x5245_5349_5354_414E;
    const N_CLASSES: i64 = 4;

    let mut resistance = Vec::with_capacity(dim * dim);
    for idx in 0..dim*dim {
        let x = (idx % dim) as i64;
        let z = (idx / dim) as i64;
        let h = world::gen::height::height_at(x, z, seed ^ SALT, hmax);
        let class = ((h * N_CLASSES) / hmax).min(N_CLASSES - 1);
        resistance.push(class);
    }
    resistance
}

fn compute_drainage_density(dim: usize, area: &[i64]) -> f64 {
    let threshold = (dim as i64 * dim as i64 / 13000).max(8);
    let channel_cells: i64 = area.iter().filter(|&&a| a >= threshold).count() as i64;
    (channel_cells as f64 * 100.0) / (dim * dim) as f64
}

fn compute_valley_relief(dim: usize, height: &[i64]) -> (i64, i64) {
    let mut depths = Vec::new();
    let sample_spacing = (dim / 3).max(5);

    for z in (0..dim).step_by(sample_spacing) {
        for x in (0..dim).step_by(sample_spacing) {
            let idx = z * dim + x;
            let center_h = height[idx];

            let radius = 20i64.min((dim / 4) as i64);
            let mut local_peak = center_h;
            for dz in -(radius)..=(radius) {
                for dx in -(radius)..=(radius) {
                    let nx = x as i64 + dx;
                    let nz = z as i64 + dz;
                    if nx >= 0 && nx < dim as i64 && nz >= 0 && nz < dim as i64 {
                        let nidx = (nz as usize) * dim + (nx as usize);
                        local_peak = local_peak.max(height[nidx]);
                    }
                }
            }

            let cross_valley_depth = local_peak - center_h;
            if cross_valley_depth > 2 {
                depths.push(cross_valley_depth);
            }
        }
    }

    if depths.is_empty() {
        return (0, 0);
    }

    depths.sort_unstable();
    let len = depths.len();
    let p10_idx = len / 10;
    let p90_idx = (len * 9) / 10;

    let p10 = depths[p10_idx.max(0).min(len - 1)];
    let p90 = depths[p90_idx.max(0).min(len - 1)];

    (p10.min(p90), p10.max(p90))
}

fn compute_peak_retention(eroded: &[i64], pre_erosion: &[i64], dim: usize) -> i64 {
    // Compute p90 of the pre-erosion field (the peaks we want to retain)
    let mut peaks = Vec::new();
    for h in pre_erosion {
        peaks.push(*h);
    }
    peaks.sort_unstable();
    let len = peaks.len();
    let p90_idx = (len * 9) / 10;
    let p90_pre = peaks[p90_idx.max(0).min(len - 1)];

    // Count how many cells still exceed this p90 threshold after erosion
    let retained = eroded.iter().filter(|&&h| h >= p90_pre).count() as i64;
    let total = dim as i64 * dim as i64;
    (retained * 100) / total
}
