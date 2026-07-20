//! **Diagnostic: Propagated convergence_eff distribution.**
//!
//! Measures the distribution of conv_eff values AFTER propagation (not raw boundaries),
//! to determine correct breakpoints for amplitude/width mapping.

use world::gen::plate::compute_plate_fields;
use std::collections::VecDeque;

fn main() {
    let dims = [256i64, 512i64];
    let seeds = [1234567890u64, 9876543210, 0x0102030405060708, 0xfedcba9876543210];

    println!("=== Propagated Conv_Eff Distribution ===\n");

    for &dim in &dims {
        println!("DIM = {}", dim);
        println!("-------");

        let mut all_propagated = Vec::new();

        for (seed_idx, &seed) in seeds.iter().enumerate() {
            let fields = compute_plate_fields(seed, dim, 8u32);
            let belt_hw = (dim / 16).max(3);

            // Propagate convergence.
            let conv_eff = propagate_convergence_to_belt(dim, &fields, belt_hw);

            // Collect all non-zero propagated values (interior cells).
            let mut seed_propagated: Vec<i64> = conv_eff
                .iter()
                .copied()
                .filter(|&v| v > 0)
                .collect();

            if !seed_propagated.is_empty() {
                seed_propagated.sort();
                let min = *seed_propagated.first().unwrap();
                let max = *seed_propagated.last().unwrap();
                let median = seed_propagated[seed_propagated.len() / 2];
                let p90_idx = (seed_propagated.len() * 90) / 100;
                let p90 = seed_propagated[p90_idx];

                println!(
                    "  Seed {} ({}): min={:4}, p50={:4}, p90={:4}, max={:4}, count={}",
                    seed_idx,
                    format!("{:x}", seed).chars().take(8).collect::<String>(),
                    min, median, p90, max,
                    seed_propagated.len()
                );

                all_propagated.extend(seed_propagated);
            }
        }

        // Aggregate across all seeds.
        all_propagated.sort();
        if !all_propagated.is_empty() {
            let min = *all_propagated.first().unwrap();
            let max = *all_propagated.last().unwrap();
            let median = all_propagated[all_propagated.len() / 2];
            let p90_idx = (all_propagated.len() * 90) / 100;
            let p90 = all_propagated[p90_idx];
            let p75_idx = (all_propagated.len() * 75) / 100;
            let p75 = all_propagated[p75_idx];

            println!(
                "\n  AGGREGATE: min={:4}, p50={:4}, p75={:4}, p90={:4}, max={:4}",
                min, median, p75, p90, max
            );
        }

        println!();
    }

    println!("\n=== RECOMMENDED BREAKPOINTS ===");
    println!("Pin CONV_AMP_LOW to ~p50 (majority weak)");
    println!("Pin CONV_AMP_HIGH to ~p90 (rare strong)");
    println!("This spreads belts from AMP_MIN (majority) to AMP_MAX (rare).");
}

/// Propagate convergence (identical to orogeny's version).
fn propagate_convergence_to_belt(
    dim: i64,
    fields: &world::gen::plate::PlateFields,
    belt_hw: i64,
) -> Vec<i64> {
    use world::gen::plate::BoundaryType;

    let dim_usize = dim as usize;
    let n = dim_usize * dim_usize;
    let blur_reach = (belt_hw * 2).max(dim / 8);

    let mut conv_eff = vec![0i64; n];
    let belt_distance = compute_belt_distance(dim, &fields.boundary_type);

    for z in 0..dim_usize {
        for x in 0..dim_usize {
            let idx = z * dim_usize + x;

            if belt_distance[idx] > belt_hw {
                conv_eff[idx] = 0;
                continue;
            }

            let mut sum = 0i64;
            let mut count = 0i64;

            for source_z in ((z as i64 - blur_reach).max(0) as usize)
                ..=((z as i64 + blur_reach).min(dim - 1) as usize)
            {
                for source_x in ((x as i64 - blur_reach).max(0) as usize)
                    ..=((x as i64 + blur_reach).min(dim - 1) as usize)
                {
                    let source_idx = source_z * dim_usize + source_x;

                    if fields.boundary_type[source_idx] == BoundaryType::Convergent {
                        let source_conv = fields.convergence_magnitude[source_idx];
                        if source_conv > 0 {
                            sum = sum.saturating_add(source_conv);
                            count += 1;
                        }
                    }
                }
            }

            if count == 0 {
                conv_eff[idx] = 0;
            } else {
                conv_eff[idx] = sum / count;
            }
        }
    }

    conv_eff
}

fn compute_belt_distance(dim: i64, boundary_type: &[world::gen::plate::BoundaryType]) -> Vec<i64> {
    use world::gen::plate::BoundaryType;

    let dim_usize = dim as usize;
    let n = dim_usize * dim_usize;
    let mut distance = vec![i64::MAX; n];
    let mut queue = VecDeque::new();

    const NEIGHBOR_OFFSETS: &[(i64, i64)] = &[
        (-1, -1), (0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0),
    ];

    for z in 0..dim_usize {
        for x in 0..dim_usize {
            let idx = z * dim_usize + x;
            if boundary_type[idx] == BoundaryType::Convergent {
                distance[idx] = 0;
                queue.push_back((x as i64, z as i64));
            }
        }
    }

    while let Some((x, z)) = queue.pop_front() {
        let idx = (z as usize) * dim_usize + (x as usize);
        let cur_dist = distance[idx];

        for &(dx, dz) in NEIGHBOR_OFFSETS {
            let nx = x + dx;
            let nz = z + dz;

            if nx < 0 || nx >= dim || nz < 0 || nz >= dim {
                continue;
            }

            let nidx = (nz as usize) * dim_usize + (nx as usize);
            let next_dist = cur_dist + 1;

            if next_dist < distance[nidx] {
                distance[nidx] = next_dist;
                queue.push_back((nx, nz));
            }
        }
    }

    distance
}
