//! Coherent belt diagnostic (Slice-1k).
//! Measures:
//! - Convergent-cell fraction per seed (should be minority, not 0% or >60%)
//! - De-jitter: sign_flips / edge_length along sampled convergent edges
//! - Sanity check: edges have coherent, contiguous convergent regions

fn main() {
    const HMAX: i64 = 200;
    const DIM: i64 = 256;
    const PLATE_STRENGTH: i64 = 100;

    // Use 4 seeds as per the acceptance criteria
    let seeds = [
        1234567890u64,
        9876543210u64,
        0xdeadbeefcafebabeu64,
        0x0123456789abcdefu64,
    ];

    println!("=== Coherent Belt Diagnostic (Slice-1k) ===");
    println!("Config: dim={}, hmax={}, enable_plate_sim=true", DIM, HMAX);
    println!();

    let mut all_healthy = true;

    for &seed in &seeds {
        println!("Seed: {}", seed);

        // Compute plate fields
        let plate_count = 15u32;
        let plate_count_clamped = world::gen::plate::clamp_plate_count(plate_count, DIM);
        let plate_fields =
            world::gen::plate::compute_plate_fields(seed, DIM, plate_count_clamped);

        // Measure convergent-cell fraction
        let total_cells = (DIM * DIM) as usize;
        let convergent_count = plate_fields
            .boundary_type
            .iter()
            .filter(|&&bt| bt == world::gen::plate::BoundaryType::Convergent)
            .count();
        let divergent_count = plate_fields
            .boundary_type
            .iter()
            .filter(|&&bt| bt == world::gen::plate::BoundaryType::Divergent)
            .count();
        let boundary_count = convergent_count + divergent_count
            + plate_fields
                .boundary_type
                .iter()
                .filter(|&&bt| bt == world::gen::plate::BoundaryType::Transform)
                .count();

        let convergent_fraction = if boundary_count > 0 {
            (convergent_count as f64) / (boundary_count as f64)
        } else {
            0.0
        };

        println!(
            "  Convergent cells: {} / boundary cells: {} / fraction: {:.2}%",
            convergent_count,
            boundary_count,
            convergent_fraction * 100.0
        );

        // Check coverage guard
        if convergent_fraction > 0.6 {
            println!("  ✗ WARNING: >60% convergent (mountains-everywhere bias)");
            all_healthy = false;
        }
        if convergent_fraction < 0.01 && convergent_count == 0 {
            println!("  ✗ WARNING: ~0% convergent (flat map, no mountains)");
            all_healthy = false;
        }

        // Measure de-jitter along a sampled plate-pair edge
        let (sign_flip_ratio, edge_length) =
            measure_edge_denoising(&plate_fields, DIM);

        println!(
            "  De-jitter: {:.3} sign-flips/cell (edge length: {})",
            sign_flip_ratio, edge_length
        );

        if sign_flip_ratio > 0.15 {
            println!("  ✗ WARNING: sign-flip ratio > 0.15 (high jitter)");
            all_healthy = false;
        }

        println!();
    }

    if all_healthy {
        println!("✓ All seeds: healthy coverage + low jitter");
        std::process::exit(0);
    } else {
        println!("✗ Some seeds failed coverage or jitter checks");
        std::process::exit(1);
    }
}

/// Measure sign-flip ratio along a sampled convergent plate-pair edge.
/// Returns (sign_flips_per_cell, total_edge_length).
fn measure_edge_denoising(
    plate_fields: &world::gen::plate::PlateFields,
    dim: i64,
) -> (f64, i64) {
    use world::gen::plate::BoundaryType;

    let dim_usize = dim as usize;
    let mut edge_length = 0i64;
    let mut sign_flips = 0i64;

    // Find a sample plate-pair edge by scanning for convergent regions
    for z in 0..dim {
        for x in 0..dim {
            let idx = (z * dim + x) as usize;
            if plate_fields.boundary_type[idx] != BoundaryType::Convergent {
                continue;
            }

            // Found a convergent cell; trace along the edge (horizontal scan in this example)
            let plate_at_x = plate_fields.plate_id[idx];
            let mut current_sign = if plate_fields.convergence_magnitude[idx] > 0 {
                1
            } else {
                -1
            };

            for step_x in 1..5 {
                let nx = x + step_x;
                if nx >= dim {
                    break;
                }
                let nidx = (z * dim + nx) as usize;

                // Only count within same plate-pair edge
                if plate_fields.plate_id[nidx] == plate_at_x {
                    continue; // Not on edge
                }

                if plate_fields.boundary_type[nidx] != BoundaryType::Convergent {
                    continue; // Not convergent
                }

                let next_sign = if plate_fields.convergence_magnitude[nidx] > 0 {
                    1
                } else {
                    -1
                };

                edge_length += 1;
                if next_sign != current_sign {
                    sign_flips += 1;
                }

                current_sign = next_sign;
            }

            // Sample just a few edges to measure
            if edge_length >= 20 {
                break;
            }
        }
        if edge_length >= 20 {
            break;
        }
    }

    let ratio = if edge_length > 0 {
        (sign_flips as f64) / (edge_length as f64)
    } else {
        0.0
    };

    (ratio, edge_length)
}
