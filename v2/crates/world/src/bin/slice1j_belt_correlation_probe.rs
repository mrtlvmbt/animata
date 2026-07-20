//! **Slice-1j AC4(a): Per-belt correlation probe — real map evidence.**
//!
//! Generates real maps with plate tectonics and reports per-belt statistics:
//! - Convergent belt identity (via connected-component labeling on boundary cells)
//! - Per-belt average propagated convergence_magnitude
//! - Per-belt peak uplift height
//! - Per-belt width (max distance from boundary)
//!
//! Verifies that higher-convergence belts are materially taller+wider (NOT renormalization artifact).

use world::gen::plate::{compute_plate_fields, BoundaryType};
use std::collections::{HashMap, VecDeque};

fn main() {
    let dim = 256i64;
    let hmax = 256i64;
    let plate_strength = 100i64;
    let seeds = [1234567890u64, 9876543210u64];

    println!("=== Slice-1j AC4(a): Per-Belt Correlation on Real Maps ===\n");

    for (seed_idx, &seed) in seeds.iter().enumerate() {
        println!("SEED {} ({:x}):", seed_idx, seed);
        println!("-------");

        let fields = compute_plate_fields(seed, dim, 8u32);

        // Compute belt distance (BFS from convergent boundaries).
        let belt_distance = compute_belt_distance(dim, &fields.boundary_type);

        // Segment convergent belts via connected-component labeling.
        let belt_labels = label_convergent_belts(dim, &fields.boundary_type);

        // Compute uplift field.
        let uplift = world::gen::orogeny::generate_plate_uplift_field(
            &fields,
            dim,
            hmax,
            plate_strength,
        );

        // Propagate convergence.
        let conv_eff = propagate_convergence_to_belt(dim, &fields, (dim / 16).max(3));

        // Collect per-belt statistics.
        let belt_stats = compute_belt_statistics(
            dim,
            &belt_labels,
            &belt_distance,
            &uplift,
            &conv_eff,
        );

        // Report belts sorted by convergence.
        report_belt_statistics(&belt_stats);
    }

    println!("\n=== CORRELATION CHECK ===");
    println!("If higher-conv belts show higher peak heights and widths, AC4(a) is satisfied.");
    println!("If all belts are similar size regardless of convergence, renormalization is suspected.");
}

/// BFS-based belt distance (identical to orogeny's version).
fn compute_belt_distance(dim: i64, boundary_type: &[BoundaryType]) -> Vec<i64> {
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

/// Label convergent belts via connected-component labeling (8-neighbor connectivity).
fn label_convergent_belts(dim: i64, boundary_type: &[BoundaryType]) -> Vec<u32> {
    let dim_usize = dim as usize;
    let n = dim_usize * dim_usize;
    let mut labels = vec![u32::MAX; n];
    let mut next_label = 0u32;

    const NEIGHBOR_OFFSETS: &[(i64, i64)] = &[
        (-1, -1), (0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0),
    ];

    for z in 0..dim_usize {
        for x in 0..dim_usize {
            let idx = z * dim_usize + x;

            if boundary_type[idx] != BoundaryType::Convergent || labels[idx] != u32::MAX {
                continue;
            }

            // BFS to label this connected component.
            let label = next_label;
            next_label += 1;
            let mut queue = VecDeque::new();
            queue.push_back((x as i64, z as i64));
            labels[idx] = label;

            while let Some((cx, cz)) = queue.pop_front() {
                let cidx = (cz as usize) * dim_usize + (cx as usize);

                for &(dx, dz) in NEIGHBOR_OFFSETS {
                    let nx = cx + dx;
                    let nz = cz + dz;

                    if nx < 0 || nx >= dim || nz < 0 || nz >= dim {
                        continue;
                    }

                    let nidx = (nz as usize) * dim_usize + (nx as usize);
                    if boundary_type[nidx] == BoundaryType::Convergent && labels[nidx] == u32::MAX {
                        labels[nidx] = label;
                        queue.push_back((nx, nz));
                    }
                }
            }
        }
    }

    labels
}

/// Propagate convergence (identical to orogeny's version).
fn propagate_convergence_to_belt(
    dim: i64,
    fields: &world::gen::plate::PlateFields,
    belt_hw: i64,
) -> Vec<i64> {
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

/// Compute per-belt statistics.
#[derive(Clone, Debug)]
struct BeltStats {
    belt_id: u32,
    conv_eff_avg: i64,
    peak_height: i64,
    belt_width: i64,
    cell_count: usize,
}

fn compute_belt_statistics(
    dim: i64,
    belt_labels: &[u32],
    belt_distance: &[i64],
    uplift: &[i64],
    conv_eff: &[i64],
) -> Vec<BeltStats> {
    let dim_usize = dim as usize;
    let mut belt_map: HashMap<u32, BeltStats> = HashMap::new();

    // STEP 1: Collect boundary statistics and identify which belts exist.
    for z in 0..dim_usize {
        for x in 0..dim_usize {
            let idx = z * dim_usize + x;
            let belt_id = belt_labels[idx];

            if belt_id == u32::MAX {
                continue;
            }

            let entry = belt_map
                .entry(belt_id)
                .or_insert(BeltStats {
                    belt_id,
                    conv_eff_avg: 0,
                    peak_height: 0,
                    belt_width: 0,
                    cell_count: 0,
                });

            entry.conv_eff_avg += conv_eff[idx];
            entry.peak_height = entry.peak_height.max(uplift[idx]);
            entry.cell_count += 1;
        }
    }

    // STEP 2: For each belt, find the maximum distance from boundary (belt width).
    //         This captures how far the belt extends into the interior.
    for z in 0..dim_usize {
        for x in 0..dim_usize {
            let idx = z * dim_usize + x;
            if belt_distance[idx] < i64::MAX && belt_distance[idx] != 0 {
                // This is an interior cell near a boundary.
                // Assign it to the nearest boundary belt by looking at nearby boundary cells.
                let mut nearest_belt = u32::MAX;
                let mut nearest_dist = i64::MAX;

                for bz in (z.saturating_sub(20))..=(z + 20).min(dim_usize - 1) {
                    for bx in (x.saturating_sub(20))..=(x + 20).min(dim_usize - 1) {
                        let bidx = bz * dim_usize + bx;
                        if belt_labels[bidx] != u32::MAX {
                            let d = ((x as i64 - bx as i64).abs() + (z as i64 - bz as i64).abs());
                            if d < nearest_dist {
                                nearest_dist = d;
                                nearest_belt = belt_labels[bidx];
                            }
                        }
                    }
                }

                if nearest_belt != u32::MAX {
                    if let Some(stats) = belt_map.get_mut(&nearest_belt) {
                        stats.belt_width = stats.belt_width.max(belt_distance[idx]);
                    }
                }
            }
        }
    }

    // Finalize averages.
    for stats in belt_map.values_mut() {
        if stats.cell_count > 0 {
            stats.conv_eff_avg /= stats.cell_count as i64;
        }
    }

    let mut belts: Vec<_> = belt_map.into_values().collect();
    belts.sort_by_key(|b| b.conv_eff_avg);
    belts
}

fn report_belt_statistics(belts: &[BeltStats]) {
    if belts.is_empty() {
        println!("  (no convergent belts found)");
        return;
    }

    println!("  Belt | Conv_Eff_Avg | Peak_Height | Belt_Width | Cells");
    println!("  -----|--------------|-------------|-----------|-------");

    for belt in belts {
        println!(
            "  {:4} | {:12} | {:11} | {:9} | {:5}",
            belt.belt_id,
            belt.conv_eff_avg,
            belt.peak_height,
            belt.belt_width,
            belt.cell_count,
        );
    }

    // Correlation check.
    if belts.len() >= 2 {
        let min_belt = &belts[0];
        let max_belt = &belts[belts.len() - 1];
        let height_ratio = if min_belt.peak_height > 0 {
            max_belt.peak_height as f64 / min_belt.peak_height as f64
        } else {
            0.0
        };
        let width_ratio = if min_belt.belt_width > 0 {
            max_belt.belt_width as f64 / min_belt.belt_width as f64
        } else {
            0.0
        };

        println!("\n  Correlation Summary:");
        println!(
            "    Min-conv belt (id={}): peak={}, width={}",
            min_belt.belt_id, min_belt.peak_height, min_belt.belt_width
        );
        println!(
            "    Max-conv belt (id={}): peak={}, width={}",
            max_belt.belt_id, max_belt.peak_height, max_belt.belt_width
        );
        println!("    Height ratio (max/min): {:.2}×", height_ratio);
        println!("    Width ratio (max/min): {:.2}×", width_ratio);

        if height_ratio > 1.2 && width_ratio > 1.1 {
            println!("    ✓ CORRELATED: High-conv belts are taller and wider.");
        } else if height_ratio < 1.1 && width_ratio < 1.1 {
            println!("    ⚠ NOT CORRELATED: Heights/widths similar despite conv difference.");
        } else {
            println!("    ? PARTIAL: Mixed correlation signals.");
        }
    }

    println!();
}
