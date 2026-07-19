//! W-10 Phase-0 measurement: moisture and slope histograms for SoilDry/Soil/SoilWet split.
//! Usage: w10_phase0 [dim]
//! Outputs: moisture histogram (deciles), slope histogram (deciles), class share estimates.

use world::gen::caps::classify_and_caps_staged;
use world::gen::material::MaterialId;
use world::gen::moisture::moisture_at;
use world::gen::LandformFlags;

const HMAX: i64 = 200;

fn compute_deciles(values: &[i64]) -> Vec<i64> {
    if values.is_empty() {
        return vec![0; 11];
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    (0..=10)
        .map(|i| {
            let idx = (i * n) / 10;
            sorted[idx.min(n - 1)]
        })
        .collect()
}

fn main() {
    let dim: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(512);

    println!("\n=== W-10 PHASE-0 MEASUREMENT (moisture/slope histograms @dim={}, seeds 1..2) ===", dim);

    for seed in [1u64, 2] {
        println!("\n--- Seed {} ---", seed);

        // Run the post-W-9 classification (all landforms enabled, talus disabled to match Phase-0 intent).
        // Actually, we want the POST-talus field since that's what classify uses.
        let (_, staged, _) = classify_and_caps_staged(
            seed, HMAX, dim, false, LandformFlags::from_five(true, true, true, true, true), false, true  // enable_w10=true
        );

        let _n = dim * dim;
        let post_deneedle_height = &staged.post_deneedle;
        let erosion = world::gen::erosion::erode(seed, HMAX, dim, true, true, true, false, true, 100);

        // Collect moisture and slope for Soil cells only
        let mut soil_moistures = Vec::new();
        let mut soil_slopes = Vec::new();

        for z in 0..dim {
            for x in 0..dim {
                let idx = z * dim + x;

                // Check if substrate is Soil (MaterialId::Soil = 3)
                let material_byte = erosion.surface_material[idx] as u8;
                if material_byte != MaterialId::Soil as u8 {
                    continue; // Skip non-Soil cells
                }

                // Compute moisture
                let area = erosion.drainage.area[idx];
                let moisture = moisture_at(area);
                soil_moistures.push(moisture);

                // Compute slope
                let slope = match erosion.drainage.downstream[idx] {
                    Some(d) => (post_deneedle_height[idx] - post_deneedle_height[d]).max(0),
                    None => 0,
                };
                soil_slopes.push(slope);
            }
        }

        // Compute deciles
        let moisture_deciles = compute_deciles(&soil_moistures);
        let slope_deciles = compute_deciles(&soil_slopes);

        println!("  Soil cells: {}", soil_moistures.len());
        println!("  Moisture (0=dry, 1000=wet): deciles = {:?}", moisture_deciles);
        println!("  Slope: deciles = {:?}", slope_deciles);

        // Test threshold candidates from deciles (target: 40–60% / 25–35% / 15–25%)
        // Deciles show steep distribution: p50≈10–11, p90≈80–170
        let candidates: &[(i64, i64, &str)] = &[
            (15, 80, "CANDIDATE-A"),
            (20, 100, "CANDIDATE-B"),
            (25, 120, "CANDIDATE-C"),
        ];

        for (dry_thresh, wet_thresh, label) in candidates {
            let dry_count = soil_moistures.iter().filter(|&&m| m < *dry_thresh).count();
            let wet_count = soil_moistures.iter().filter(|&&m| m >= *wet_thresh).count();
            let normal_count = soil_moistures.len() - dry_count - wet_count;
            let outcrop_count = soil_slopes.iter().filter(|&&s| s >= 5).count();

            println!("  {} (dry<{}, wet>={}): SoilDry {:.1}% | Soil {:.1}% | SoilWet {:.1}% | Outcrop {:.1}%",
                label, dry_thresh, wet_thresh,
                100.0 * dry_count as f64 / soil_moistures.len() as f64,
                100.0 * normal_count as f64 / soil_moistures.len() as f64,
                100.0 * wet_count as f64 / soil_moistures.len() as f64,
                100.0 * outcrop_count as f64 / soil_moistures.len() as f64);
        }
    }

    println!("\n=== CHOOSE CANDIDATE ABOVE (target: 40–60% / 25–35% / 15–25%) ===");
    println!("Pick the candidate closest to those shares, then pin:");
    println!("// Example: if CANDIDATE-B (20/100) gives best distribution:");
    println!("const SOILDRY_THRESHOLD: i64 = 20;   // Below this -> SoilDry");
    println!("const SOILWET_THRESHOLD: i64 = 100;  // At/above this -> SoilWet");
    println!("const OUTCROP_SLOPE_THRESHOLD: i64 = 5;  // Soil* with slope >= this -> Bedrock");
}
