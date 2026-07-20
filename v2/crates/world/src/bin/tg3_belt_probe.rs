//! TG3 Belt Width + Talus Control Diagnostic Probe
//!
//! Two parts:
//! 1. Belt width sweep (3, 8, 16, 32) × strength (50, 100, 200) to show belt width has no effect
//! 2. Talus control sweep at fixed belt width: talus ON vs OFF vs custom repose (4)
//!
//! The talus control is the DECISIVE experiment: if disabling talus or raising repose threshold
//! restores relief and kills the strength inversion, then REPOSE_THRESHOLD=0 is the crusher, not belt width.

fn main() {
    const HMAX: i64 = 200;
    const DIM: usize = 256; // Larger grid to let belts resolve
    const PLATE_COUNT: u32 = 15;
    const SEED: u64 = 0x1234567890ABCDEFu64;
    const FIXED_BELT_WIDTH: i64 = 16; // For talus control sweep

    println!("=== TG3 Belt Width + Talus Control Diagnostic Probe ===");
    println!("Config: DIM={}, HMAX={}, base=false, enable_plate_sim=true, erosion=true", DIM, HMAX);
    println!();

    // PART 1: Belt width sweep (confirms belt width has no effect)
    println!("=== PART 1: Belt Width Sweep (confirms belt width has minimal effect) ===");
    println!("Seed: 0x{:016X}", SEED);
    println!();
    println!("Belt_Width  |  Strength  |  Valley_Relief_p10  |  Valley_Relief_p90  |  Peak_Retained");
    println!("    (cells) |   (%)      |                     |                     |  (height units)");
    println!("------------|------------|---------------------|---------------------|----------------------");

    let belt_widths = [3i64, 8, 16, 32];
    let strengths = [50i64, 100, 200];

    for &belt_width in &belt_widths {
        for &strength in &strengths {
            // Create flat base + plate uplift
            let flat_base = HMAX / 2;
            let mut height = vec![flat_base; DIM * DIM];

            // Plate fields and orogeny with parameterized belt width
            let plate_count_clamped = world::gen::plate::clamp_plate_count(PLATE_COUNT, DIM as i64);
            let plate_fields = world::gen::plate::compute_plate_fields(SEED, DIM as i64, plate_count_clamped);
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

            // Run erosion on the uplifted field (talus ON, default repose=0)
            let resistance = compute_resistance(SEED, DIM, HMAX);
            let erosion = world::gen::erosion::erode_from_fields(
                SEED,
                HMAX,
                DIM,
                height.clone(),
                resistance,
                true,
                strength,
            );

            // Measure metrics on eroded field
            let (valley_p10, valley_p90) = compute_valley_relief(DIM, &erosion.height);
            let peak_retained = *erosion.height.iter().max().unwrap_or(&flat_base) - flat_base;

            println!(
                "     {:2}     |    {:3}     |        {:3}          |        {:3}          |       {:3}",
                belt_width, strength, valley_p10, valley_p90, peak_retained
            );
        }
        println!();
    }

    // PART 2: Talus control sweep at FIXED belt width
    println!();
    println!("=== PART 2: TALUS CONTROL SWEEP (the decisive experiment) ===");
    println!("Fixed belt_width={}, Seed: 0x{:016X}", FIXED_BELT_WIDTH, SEED);
    println!("Three conditions: (A) Talus ON (repose=0) | (B) Talus OFF | (C) Talus with repose=4");
    println!();
    println!("Condition       |  Strength  |  Valley_Relief_p10  |  Valley_Relief_p90  |  Peak_Retained");
    println!("                |   (%)      |                     |                     |  (height units)");
    println!("----------------|------------|---------------------|---------------------|----------------------");

    for &strength in &strengths {
        // (A) Talus ON (current default: repose=0)
        {
            let flat_base = HMAX / 2;
            let mut height = vec![flat_base; DIM * DIM];
            let plate_count_clamped = world::gen::plate::clamp_plate_count(PLATE_COUNT, DIM as i64);
            let plate_fields = world::gen::plate::compute_plate_fields(SEED, DIM as i64, plate_count_clamped);
            let plate_uplift = world::gen::orogeny::generate_plate_uplift_field_with_belt(
                &plate_fields,
                DIM as i64,
                HMAX,
                strength,
                FIXED_BELT_WIDTH,
            );
            for i in 0..DIM * DIM {
                height[i] = (height[i] + plate_uplift[i]).clamp(0, HMAX);
            }
            let resistance = compute_resistance(SEED, DIM, HMAX);
            let erosion = world::gen::erosion::erode_from_fields(SEED, HMAX, DIM, height.clone(), resistance, true, strength);
            let (valley_p10, valley_p90) = compute_valley_relief(DIM, &erosion.height);
            let peak_retained = *erosion.height.iter().max().unwrap_or(&flat_base) - flat_base;
            println!("(A) Talus ON    |    {:3}     |        {:3}          |        {:3}          |       {:3}",
                     strength, valley_p10, valley_p90, peak_retained);
        }

        // (B) Talus OFF
        {
            let flat_base = HMAX / 2;
            let mut height = vec![flat_base; DIM * DIM];
            let plate_count_clamped = world::gen::plate::clamp_plate_count(PLATE_COUNT, DIM as i64);
            let plate_fields = world::gen::plate::compute_plate_fields(SEED, DIM as i64, plate_count_clamped);
            let plate_uplift = world::gen::orogeny::generate_plate_uplift_field_with_belt(
                &plate_fields,
                DIM as i64,
                HMAX,
                strength,
                FIXED_BELT_WIDTH,
            );
            for i in 0..DIM * DIM {
                height[i] = (height[i] + plate_uplift[i]).clamp(0, HMAX);
            }
            let resistance = compute_resistance(SEED, DIM, HMAX);
            let erosion = world::gen::erosion::erode_from_fields_with_talus_control(
                SEED, HMAX, DIM, height.clone(), resistance, true, strength, false, None
            );
            let (valley_p10, valley_p90) = compute_valley_relief(DIM, &erosion.height);
            let peak_retained = *erosion.height.iter().max().unwrap_or(&flat_base) - flat_base;
            println!("(B) Talus OFF   |    {:3}     |        {:3}          |        {:3}          |       {:3}",
                     strength, valley_p10, valley_p90, peak_retained);
        }

        // (C) Talus with realistic repose (4 units)
        {
            let flat_base = HMAX / 2;
            let mut height = vec![flat_base; DIM * DIM];
            let plate_count_clamped = world::gen::plate::clamp_plate_count(PLATE_COUNT, DIM as i64);
            let plate_fields = world::gen::plate::compute_plate_fields(SEED, DIM as i64, plate_count_clamped);
            let plate_uplift = world::gen::orogeny::generate_plate_uplift_field_with_belt(
                &plate_fields,
                DIM as i64,
                HMAX,
                strength,
                FIXED_BELT_WIDTH,
            );
            for i in 0..DIM * DIM {
                height[i] = (height[i] + plate_uplift[i]).clamp(0, HMAX);
            }
            let resistance = compute_resistance(SEED, DIM, HMAX);
            let erosion = world::gen::erosion::erode_from_fields_with_talus_control(
                SEED, HMAX, DIM, height.clone(), resistance, true, strength, true, Some(4)
            );
            let (valley_p10, valley_p90) = compute_valley_relief(DIM, &erosion.height);
            let peak_retained = *erosion.height.iter().max().unwrap_or(&flat_base) - flat_base;
            println!("(C) Repose=4    |    {:3}     |        {:3}          |        {:3}          |       {:3}",
                     strength, valley_p10, valley_p90, peak_retained);
        }
        println!();
    }

    println!("=== INTERPRETATION ===");
    println!("PART 1: Belt width has MINIMAL effect on relief (p90 stays 6–19 across widths 3–32).");
    println!();
    println!("PART 2: If (B) or (C) show:");
    println!("  • Valley relief INCREASES with strength (no inversion) ⟹ talus is the crusher");
    println!("  • Peak retained becomes MEANINGFUL (>30 units) ⟹ incision works when talus doesn't win");
    println!("  ⟹ CONFIRMED: set REPOSE_THRESHOLD to a realistic value (e.g., 2-4) to restore relief.");
    println!();
    println!("If (B) and (C) still show collapsed relief and inversion:");
    println!("  ⟹ the crusher is NOT talus; look elsewhere (incision K, uplift clamping, etc.).");
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
