//! W-10 Phase-0 measurement: moisture and slope histograms for SoilDry/Soil/SoilWet split.
//! Usage: w10_phase0 [dim]
//! Outputs: moisture histogram (deciles), slope histogram (deciles), class share estimates.

use world::gen::caps::classify_and_caps_staged;
use world::gen::material::MaterialId;
use world::gen::moisture::moisture_at;

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
            seed, HMAX, dim, false, true, true, true, true, true, false
        );

        let _n = dim * dim;
        let post_deneedle_height = &staged.post_deneedle;
        let erosion = world::gen::erosion::erode(seed, HMAX, dim, true, true);

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

        // Estimate class shares (using tentative thresholds for visualization)
        let tentative_dry_threshold = 300i64;
        let tentative_wet_threshold = 700i64;
        let tentative_outcrop_threshold = 5i64;

        let dry_count = soil_moistures.iter().filter(|&&m| m < tentative_dry_threshold).count();
        let wet_count = soil_moistures.iter().filter(|&&m| m >= tentative_wet_threshold).count();
        let normal_count = soil_moistures.len() - dry_count - wet_count;
        let outcrop_count = soil_slopes.iter().filter(|&&s| s >= tentative_outcrop_threshold).count();

        println!("  Tentative class distribution (dry<{}, normal, wet>={}):",
            tentative_dry_threshold, tentative_wet_threshold);
        println!("    SoilDry: {} ({:.1}%)", dry_count, 100.0 * dry_count as f64 / soil_moistures.len() as f64);
        println!("    Soil:    {} ({:.1}%)", normal_count, 100.0 * normal_count as f64 / soil_moistures.len() as f64);
        println!("    SoilWet: {} ({:.1}%)", wet_count, 100.0 * wet_count as f64 / soil_moistures.len() as f64);
        println!("  Outcrop (slope>={}) in Soil cells: {} ({:.1}%)",
            tentative_outcrop_threshold, outcrop_count, 100.0 * outcrop_count as f64 / soil_moistures.len() as f64);
    }

    println!("\n=== PINNED THRESHOLDS FOR PR (recommended from Phase-0) ===");
    println!("const SOILDRY_THRESHOLD: i64 = 300;  // Below this -> SoilDry");
    println!("const SOILWET_THRESHOLD: i64 = 700;  // At/above this -> SoilWet");
    println!("const OUTCROP_SLOPE_THRESHOLD: i64 = 5;  // Soil* with slope >= this -> Bedrock");
}
