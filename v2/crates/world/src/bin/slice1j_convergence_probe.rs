//! **Slice-1j AC0:** Measure convergence_magnitude distribution on convergent belts at dim=256 and dim=512.
//!
//! Probes the distribution (min/median/p90/max) of per-cell convergence_magnitude on convergent
//! boundary cells, at two dimensions, ≥4 seeds each. Determines whether units scale with dim.

use world::gen::plate::{compute_plate_fields, BoundaryType};
use std::collections::BTreeMap;

fn main() {
    let dims = [256i64, 512i64];
    let seeds = [1234567890u64, 9876543210, 0x0102030405060708, 0xfedcba9876543210];

    println!("=== Slice-1j AC0: Convergence Distribution Probe ===\n");

    for &dim in &dims {
        println!("DIM = {}", dim);
        println!("-----");

        let mut all_convergences = Vec::new();

        for (seed_idx, &seed) in seeds.iter().enumerate() {
            let fields = compute_plate_fields(seed, dim, 8u32);

            // Collect convergence_magnitude values from convergent boundary cells only.
            let mut seed_convergences = Vec::new();
            for z in 0..dim as usize {
                for x in 0..dim as usize {
                    let idx = z * dim as usize + x;
                    if fields.boundary_type[idx] == BoundaryType::Convergent {
                        let conv = fields.convergence_magnitude[idx];
                        seed_convergences.push(conv);
                        all_convergences.push(conv);
                    }
                }
            }

            // Sort for percentile computation.
            seed_convergences.sort();

            if !seed_convergences.is_empty() {
                let min = *seed_convergences.first().unwrap();
                let max = *seed_convergences.last().unwrap();
                let median = seed_convergences[seed_convergences.len() / 2];
                let p90_idx = (seed_convergences.len() * 90) / 100;
                let p90 = seed_convergences[p90_idx];

                println!(
                    "  Seed {} ({:16x}): min={:6}, median={:6}, p90={:6}, max={:6}, count={:5}",
                    seed_idx,
                    seed,
                    min,
                    median,
                    p90,
                    max,
                    seed_convergences.len()
                );
            } else {
                println!("  Seed {}: NO convergent boundaries found", seed_idx);
            }
        }

        // Aggregate across all seeds for this dim.
        all_convergences.sort();
        if !all_convergences.is_empty() {
            let min = *all_convergences.first().unwrap();
            let max = *all_convergences.last().unwrap();
            let median = all_convergences[all_convergences.len() / 2];
            let p90_idx = (all_convergences.len() * 90) / 100;
            let p90 = all_convergences[p90_idx];

            println!(
                "\n  AGGREGATE (all {} seeds): min={:6}, median={:6}, p90={:6}, max={:6}, total_cells={:7}",
                seeds.len(),
                min,
                median,
                p90,
                max,
                all_convergences.len()
            );

            // Quick estimate of scaling: ratio of dim=512 to dim=256 metrics.
            println!("  (Will compute dim scaling ratio below)\n");
        }
    }

    // Cross-dim comparison: does convergence scale with dim?
    println!("\n=== DIM SCALING ANALYSIS ===");
    println!("Comparing dim=256 vs dim=512:");
    println!("If max(256) / max(512) or median(256) / median(512) is stable ~0.5 → units scale with dim");
    println!("If ratio is ~0.25 or ~1.0 → units are dimension-independent or scale differently");
    println!("(Actual ratio will be computed after first run with probe output)\n");
}
