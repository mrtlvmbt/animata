//! TG3 Belt Width Diagnostic Probe
//!
//! Tests hypothesis: wider fold belts (belt_half_width > 3) give better-retained
//! relief and resolve the plate_strength inversion (relief growing with strength,
//! not shrinking). Sweeps belt_half_width × plate_strength and reports relief/drainage/peak metrics.

fn main() {
    const HMAX: i64 = 200;
    const DIM: usize = 256; // Larger grid to let belts resolve
    const PLATE_COUNT: u32 = 15;

    println!("=== TG3 Belt Width Diagnostic Probe ===");
    println!("Config: DIM={}, HMAX={}, base=false, enable_plate_sim=true, erosion=true", DIM, HMAX);
    println!();

    let belt_widths = [3i64, 8, 16, 32];
    let strengths = [50i64, 100, 200];
    let seed = 0x1234567890ABCDEFu64;

    println!("Seed: 0x{:016X}", seed);
    println!();
    println!("Belt_Width  |  Strength  |  Valley_Relief_p10  |  Valley_Relief_p90  |  Drainage_Density_%  |  Peak_Retained");
    println!("    (cells) |   (%)      |                     |                     |                      |  (height units)");
    println!("------------|------------|---------------------|---------------------|----------------------|----------------------");

    for &belt_width in &belt_widths {
        for &strength in &strengths {
            // Create flat base + plate uplift
            let flat_base = HMAX / 2;
            let mut height = vec![flat_base; DIM * DIM];

            // Plate fields and orogeny with parameterized belt width
            let plate_count_clamped = world::gen::plate::clamp_plate_count(PLATE_COUNT, DIM as i64);
            let plate_fields = world::gen::plate::compute_plate_fields(seed, DIM as i64, plate_count_clamped);
            let plate_uplift = world::gen::orogeny::generate_plate_uplift_field_with_belt(
                &plate_fields,
                DIM as i64,
                HMAX,
                strength,
                belt_width,
            );

            // Add uplift to flat base
            for i in 0..DIM * DIM {
                height[i] = (height[i] + plate_uplift[i]).clamp(0, HMAX);
            }

            // Run erosion on the uplifted field
            let resistance = compute_resistance(seed, DIM, HMAX);
            let erosion = world::gen::erosion::erode_from_fields(
                seed,
                HMAX,
                DIM,
                height.clone(),
                resistance,
                true,
                strength,
            );

            // Measure metrics on eroded field
            let drainage_density = compute_drainage_density(DIM, &erosion.drainage.area);
            let (valley_p10, valley_p90) = compute_valley_relief(DIM, &erosion.height);
            let peak_retained = *erosion.height.iter().max().unwrap_or(&flat_base) - flat_base;

            println!(
                "     {:2}     |    {:3}     |        {:3}          |        {:3}          |        {:6.2}        |       {:3}",
                belt_width, strength, valley_p10, valley_p90, drainage_density, peak_retained
            );
        }
        println!();
    }

    println!("=== INTERPRETATION ===");
    println!("• Valley_Relief_p10/p90: cross-valley depth (local peak - center) at 10th and 90th percentiles");
    println!("• Drainage_Density: percentage of cells in channels (area >= (dim^2 / 13000).max(8))");
    println!("• Peak_Retained: max height after erosion minus flat base (in hmax units)");
    println!();
    println!("HYPOTHESIS: Wider belts (higher belt_half_width) should:");
    println!("  1. Increase valley relief (more room for incision to dissect the uplift)");
    println!("  2. Resolve strength inversion (relief should grow or plateau with strength, not shrink)");
    println!("  3. Retain higher peaks (the uplifted belt stands above erosion)");
}

fn compute_resistance(seed: u64, dim: usize, hmax: i64) -> Vec<i64> {
    const SALT: u64 = 0x5245_5349_5354_414E;
    const N_CLASSES: i64 = 4;

    let mut resistance = Vec::with_capacity(dim * dim);
    for idx in 0..dim * dim {
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
    let sample_spacing = (dim / 4).max(5); // Coarser sampling for larger grid

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
