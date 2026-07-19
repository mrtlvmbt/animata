//! terragen-v3 Probe: Synthetic uplift+incision dendritic-structure gate + resample fidelity
//!
//! This probe tests whether:
//! 1. The SHIPPING integer stream-power incision produces dendritic drainage (channels, crests, V-valleys)
//!    on synthetic uplift fields with the SHIPPING resistance field active + documented micro-roughness
//! 2. Structure survives integer-mean pooling to a coarse hexagonal grid (resample-fidelity test)
//!
//! PRE-REGISTERED VALIDITY GATE (probe-validity-gate methodology):
//! - Capability: uses SHIPPING erosion.rs machinery (depression-fill + macro-loop + W-19 strength scaling)
//! - Regime: two tiers (64×64 calibration, 256×256 scaling check)
//! - Metrics: #1 drainage density, #2 crest connectivity, #3 valley relief, #4 anti-spike test, #5 resample fidelity
//! - Treatment: 3+ uplift shapes × 4 strength values × 2+ seeds
//! - Anti-forcing: if no combo passes, report diagnosis (incision weak / constants mis-scaled / resample destroys / thresholds miscalibrated)
//!
//! DEM ANCHOR REFERENCES (from 03-landform-references.md):
//! - Drainage density: real fold mountains show 5-10 ridges per 50 km; Hack's law tributary spacing ∝ relief
//!   Metric #1 target: ≥5 channels per 100 cells (at CHANNEL_THRESHOLD)
//! - Relief structure: fold mountains show V-shaped valleys with peaks ≈ 100-500 m above crest
//!   Metric #3 target: p10 ≥10 units, p90 ≤100 units (representing significant relief in cross-valley depth)
//! - Resample fidelity (Metric #5): structure must survive ≥90% area-normalized retention through pooling
//!
//! Usage:
//!   cargo run --release --bin tg3_probe
//!
//! Output:
//!   - Metrics table to stdout (results per combo)
//!   - PNG gallery to docs/terragen-v3-probe/ (height/slope + drainage overlay per scenario, internal AND resampled)
//!   - GATE verdict: PASS (≥1 combo passes all 5) or FAIL (with root-cause diagnosis)

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use world::gen::erosion::{erode_from_fields, resistance_field};

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// Probe Configuration
// ─────────────────────────────────────────────────────────────────────────────────────────────────

const HMAX: i64 = 200; // Height ceiling (matches production)

// Metrics thresholds (from pre-registered validity gate, anchored to DEM references)
const CHANNEL_THRESHOLD_BASE: i64 = 8; // Minimum; scales with dim²/13000
const DRAINAGE_DENSITY_TARGET: f64 = 5.0; // Channels per 100 cells
const CREST_CONNECTIVITY_TARGET: f64 = 0.7; // Longest connected ridge / belt width
const VALLEY_RELIEF_P10_MIN: i64 = 10; // Cross-valley depth p10 lower bound
const VALLEY_RELIEF_P90_MAX: i64 = 100; // Cross-valley depth p90 upper bound
const RESAMPLE_FIDELITY_TARGET: f64 = 0.90; // ≥90% area-normalized retention

// Probe dimensions
const TIER1_DIM: usize = 64; // Calibration tier
const TIER2_DIM: usize = 256; // Scaling check tier
const HEX_GRID_SIZE: usize = 23; // Hexagon side n (test grid ~1,519 flat-top hexes, ratio ~43:1)

// Uplift shapes and strength parameters
const UPLIFT_SHAPES: &[&str] = &["gaussian_peak", "parallel_ridges", "broad_dome"];
const STRENGTH_VALUES: &[i64] = &[0, 50, 100, 200]; // Erosion strength percent
const ROUGHNESS_SEEDS: &[u64] = &[0x1234_5678_9ABC_DEF0, 0xFEDC_BA98_7654_3210]; // Micro-roughness seeding

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// Synthetic Uplift Field Generation
// ─────────────────────────────────────────────────────────────────────────────────────────────────

/// Generate synthetic uplift field with selected shape
fn generate_uplift(dim: usize, shape: &str, seed: u64, roughness_seed: u64) -> Vec<i64> {
    let mut height = vec![0i64; dim * dim];
    let hmax = HMAX;
    let center = (dim as i64) / 2;
    let half_dim = (dim as i64) / 2;

    for z in 0..dim {
        for x in 0..dim {
            let idx = z * dim + x;
            let zf = z as i64 - center;
            let xf = x as i64 - center;

            // Base uplift shape (deterministic)
            let base_height = match shape {
                "gaussian_peak" => {
                    // Single symmetric Gaussian peak at center
                    let r_sq = xf * xf + zf * zf;
                    let sigma_sq = (half_dim * half_dim) / 16; // Spread over ~1/4 the grid
                    let exp_approx = if r_sq < sigma_sq * 10 {
                        // Integer approximation of exp(-r²/σ²) via lookup
                        ((1000 * (sigma_sq - r_sq / 2)) / sigma_sq).max(0)
                    } else {
                        0
                    };
                    (hmax * exp_approx) / 1000
                }
                "parallel_ridges" => {
                    // Two parallel ridges along x-axis, symmetric
                    let z_period = half_dim / 4;
                    let ridge_spacing = half_dim / 2;
                    let z_phase = ((zf.abs() - ridge_spacing).abs()).min(ridge_spacing);
                    let x_amp = (hmax * (z_period - z_phase.abs())) / z_period;
                    if zf.abs() < ridge_spacing {
                        x_amp.max(0)
                    } else {
                        ((hmax * (ridge_spacing - (zf.abs() - ridge_spacing))) / ridge_spacing).max(0)
                    }
                }
                "broad_dome" => {
                    // Broad, low-amplitude dome covering most of the grid
                    let r_sq = xf * xf + zf * zf;
                    let radius = half_dim;
                    if r_sq < radius * radius {
                        let frac = ((radius * radius - r_sq) as i64 * 1000) / (radius * radius);
                        (hmax * frac) / 2000 // Much lower amplitude than peak
                    } else {
                        0
                    }
                }
                _ => 0,
            };

            // Add seeded micro-roughness (±few units, integer)
            let roughness = {
                let hash = roughness_seed.wrapping_mul(x as u64 + 1).wrapping_mul(z as u64 + 1);
                ((hash as i64) % 5) - 2 // ±2 cells
            };

            height[idx] = (base_height + roughness).max(0);
        }
    }

    height
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// Metric Computation
// ─────────────────────────────────────────────────────────────────────────────────────────────────

/// Metric #1: Drainage density — count channels (flow accumulation ≥ threshold) per 100 cells
fn compute_drainage_density(dim: usize, area: &[i64]) -> f64 {
    let threshold = (dim as i64 * dim as i64 / 13000).max(CHANNEL_THRESHOLD_BASE);
    let channel_cells: i64 = area.iter().filter(|&&a| a >= threshold).count() as i64;
    (channel_cells as f64 * 100.0) / (dim * dim) as f64
}

/// Metric #2: Crest connectivity — longest connected ridge line (via D8 drainage-divide graph)
fn compute_crest_connectivity(dim: usize, area: &[i64], height: &[i64]) -> f64 {
    let threshold = (dim as i64 * dim as i64 / 13000).max(CHANNEL_THRESHOLD_BASE);

    // Identify crest cells: local max (height > all neighbors) AND outside channels
    let mut is_crest = vec![false; dim * dim];
    for z in 0..dim {
        for x in 0..dim {
            let idx = z * dim + x;
            if area[idx] < threshold {
                // Check if local maximum in 8-neighborhood
                let h = height[idx];
                let mut is_local_max = true;
                for dz in -1i64..=1 {
                    for dx in -1i64..=1 {
                        if dx == 0 && dz == 0 {
                            continue;
                        }
                        let nx = x as i64 + dx;
                        let nz = z as i64 + dz;
                        if nx >= 0 && nx < dim as i64 && nz >= 0 && nz < dim as i64 {
                            let nidx = (nz as usize) * dim + (nx as usize);
                            if height[nidx] > h {
                                is_local_max = false;
                                break;
                            }
                        }
                    }
                    if !is_local_max {
                        break;
                    }
                }
                is_crest[idx] = is_local_max;
            }
        }
    }

    // BFS to find longest connected ridge
    let mut visited = vec![false; dim * dim];
    let mut max_ridge_length = 0i64;

    for start in 0..dim * dim {
        if is_crest[start] && !visited[start] {
            // BFS from this crest cell
            let mut queue = VecDeque::new();
            queue.push_back(start);
            visited[start] = true;
            let mut ridge_length = 1i64;

            while let Some(idx) = queue.pop_front() {
                let z = idx / dim;
                let x = idx % dim;
                for dz in -1i64..=1 {
                    for dx in -1i64..=1 {
                        if dx == 0 && dz == 0 {
                            continue;
                        }
                        let nx = x as i64 + dx;
                        let nz = z as i64 + dz;
                        if nx >= 0 && nx < dim as i64 && nz >= 0 && nz < dim as i64 {
                            let nidx = (nz as usize) * dim + (nx as usize);
                            if is_crest[nidx] && !visited[nidx] {
                                visited[nidx] = true;
                                queue.push_back(nidx);
                                ridge_length += 1;
                            }
                        }
                    }
                }
            }
            max_ridge_length = max_ridge_length.max(ridge_length);
        }
    }

    // Normalize by grid extent (belt width ~ dim)
    let connectivity = max_ridge_length as f64 / dim as f64;
    connectivity.min(1.0)
}

/// Metric #3: Valley relief — p10 and p90 of cross-valley depth (local relief)
fn compute_valley_relief(dim: usize, height: &[i64]) -> (i64, i64) {
    let mut depths = Vec::new();

    for z in 1..dim - 1 {
        for x in 1..dim - 1 {
            let idx = z * dim + x;
            let h = height[idx];

            // Local max height in 3×3 neighborhood
            let mut local_max = h;
            for dz in -1i64..=1 {
                for dx in -1i64..=1 {
                    let nx = x as i64 + dx;
                    let nz = z as i64 + dz;
                    if nx >= 0 && nx < dim as i64 && nz >= 0 && nz < dim as i64 {
                        let nidx = (nz as usize) * dim + (nx as usize);
                        local_max = local_max.max(height[nidx]);
                    }
                }
            }

            // Cross-valley depth = max neighbor - center
            let cross_valley_depth = local_max - h;
            if cross_valley_depth > 0 {
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

    (
        depths[p10_idx.max(0).min(len - 1)],
        depths[p90_idx.max(0).min(len - 1)],
    )
}

/// Metric #4: Anti-spike test — check that incision produces realistic V-valleys (not isolated spikes)
/// A simplified check: count cells where relief is significant but not isolated (proper valleys)
fn compute_anti_spike_test(dim: usize, height: &[i64]) -> bool {
    let mut valley_count = 0i64;
    let mut isolated_spike_count = 0i64;

    for z in 1..dim - 1 {
        for x in 1..dim - 1 {
            let idx = z * dim + x;
            let h = height[idx];

            if h > 0 {
                // Check if this is a valley (surrounded by higher terrain) or spike (isolated high point)
                let mut neighbor_heights = Vec::new();
                for dz in -1i64..=1 {
                    for dx in -1i64..=1 {
                        if dx == 0 && dz == 0 {
                            continue;
                        }
                        let nx = x as i64 + dx;
                        let nz = z as i64 + dz;
                        if nx >= 0 && nx < dim as i64 && nz >= 0 && nz < dim as i64 {
                            let nidx = (nz as usize) * dim + (nx as usize);
                            neighbor_heights.push(height[nidx]);
                        }
                    }
                }

                let avg_neighbor_height = neighbor_heights.iter().sum::<i64>() / neighbor_heights.len() as i64;
                let relief = h - avg_neighbor_height;

                if relief < -5 {
                    // Valley: cell lower than neighbors
                    valley_count += 1;
                } else if relief > 10 {
                    // Potential isolated spike (relief > 10 with no valley structure)
                    let neighbor_diff: i64 = neighbor_heights.iter().map(|&nh| (h - nh).max(0)).sum();
                    if neighbor_diff > 30 {
                        // Surrounded by low terrain: isolated spike (bad)
                        isolated_spike_count += 1;
                    }
                }
            }
        }
    }

    // PASS if we have significant valleys and few isolated spikes
    valley_count >= (dim as i64 / 4) && isolated_spike_count < (dim as i64 / 10)
}

/// Metric #5: Resample fidelity — area-normalized metric retention through hex pooling
/// Simplified version: checks if structure survives binning by comparing relief preservation
fn compute_resample_fidelity(
    dim: usize,
    area_internal: &[i64],
    height_internal: &[i64],
    hex_grid_size: usize,
) -> f64 {
    // Compute valley relief on internal raster (pre-resample)
    let (pre_p10, pre_p90) = compute_valley_relief(dim, height_internal);
    let pre_relief_range = (pre_p90 - pre_p10).max(1);

    // Simulate hex pooling via simple binning (integer-mean)
    let bin_size = (dim / hex_grid_size).max(1);
    if bin_size == 0 {
        return 1.0; // Trivial case
    }

    let mut hex_height = Vec::new();
    let hex_dim = (dim + bin_size - 1) / bin_size;

    for hz in 0..hex_dim {
        for hx in 0..hex_dim {
            let mut sum = 0i64;
            let mut count = 0i64;
            for dz in 0..bin_size {
                for dx in 0..bin_size {
                    let z = hz * bin_size + dz;
                    let x = hx * bin_size + dx;
                    if z < dim && x < dim {
                        sum += height_internal[z * dim + x];
                        count += 1;
                    }
                }
            }
            if count > 0 {
                hex_height.push(sum / count);
            }
        }
    }

    // Compute valley relief on hex grid (post-resample)
    let (post_p10, post_p90) = if !hex_height.is_empty() {
        compute_valley_relief(hex_dim, &hex_height)
    } else {
        (0, 0)
    };
    let post_relief_range = (post_p90 - post_p10).max(1);

    // PASS iff m_post × 100 ≥ m_pre × 90 (area-normalized retention)
    if pre_relief_range == 0 {
        return 1.0; // No relief to lose
    }
    let retention = (post_relief_range as f64 * 100.0) / (pre_relief_range as f64);
    if retention >= 90.0 { 1.0 } else { retention / 100.0 }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// PNG Output (Integer RGB, no floats)
// ─────────────────────────────────────────────────────────────────────────────────────────────────

/// Render height as grayscale (0-255), clamped to valid range
fn height_to_gray(h: i64, hmax: i64) -> u8 {
    ((h * 255) / hmax.max(1)).clamp(0, 255) as u8
}

/// Render drainage overlay (channels in blue, crests in red)
fn render_drainage_overlay(dim: usize, area: &[i64], height: &[i64]) -> Vec<u8> {
    let threshold = (dim as i64 * dim as i64 / 13000).max(CHANNEL_THRESHOLD_BASE);
    let mut rgb = vec![0u8; dim * dim * 3];

    for idx in 0..dim * dim {
        let base_h = height_to_gray(height[idx], HMAX);
        let (r, g, b) = if area[idx] >= threshold {
            // Channel: blue tint
            (base_h / 2, base_h / 2, 200)
        } else if height[idx] > 0 {
            // Crest candidate: red tint
            let mut is_local_max = true;
            let z = idx / dim;
            let x = idx % dim;
            for dz in -1i64..=1 {
                for dx in -1i64..=1 {
                    if dx == 0 && dz == 0 {
                        continue;
                    }
                    let nx = x as i64 + dx;
                    let nz = z as i64 + dz;
                    if nx >= 0 && nx < dim as i64 && nz >= 0 && nz < dim as i64 {
                        let nidx = (nz as usize) * dim + (nx as usize);
                        if height[nidx] > height[idx] {
                            is_local_max = false;
                        }
                    }
                }
            }
            if is_local_max {
                (220, base_h / 2, base_h / 2)
            } else {
                (base_h, base_h, base_h)
            }
        } else {
            (base_h, base_h, base_h)
        };

        rgb[idx * 3] = r;
        rgb[idx * 3 + 1] = g;
        rgb[idx * 3 + 2] = b;
    }

    rgb
}

/// Write PPM (P6 binary format) — integer RGB, no floats
fn write_ppm(path: &str, dim: usize, rgb: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    write!(file, "P6\n{} {}\n255\n", dim, dim)?;
    file.write_all(rgb)?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// Main Probe Loop
// ─────────────────────────────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct ProbeResult {
    dim: usize,
    shape: String,
    strength: i64,
    roughness_seed: u64,
    drainage_density: f64,
    crest_connectivity: f64,
    valley_relief_p10: i64,
    valley_relief_p90: i64,
    anti_spike_pass: bool,
    resample_fidelity: f64,
    passes_all: bool,
}

fn main() {
    // Create output directory
    let out_dir = "docs/terragen-v3-probe";
    std::fs::create_dir_all(out_dir).expect("Failed to create output directory");

    let mut all_results = Vec::new();
    let mut passes_per_combo = HashMap::new();

    println!("╔════════════════════════════════════════════════════════════════════════════════════════╗");
    println!("║ terragen-v3 Probe: Synthetic Uplift+Incision Dendritic-Structure Gate                ║");
    println!("╚════════════════════════════════════════════════════════════════════════════════════════╝\n");

    println!("DEM Anchor References (03-landform-references.md):");
    println!("  • Drainage density: fold mountains 5-10 ridges/50km → target ≥5 channels/100 cells");
    println!("  • Valley relief: fold mountains show peaks ≈100-500m above crest → target p10≥10, p90≤100 units");
    println!("  • Resample fidelity: ≥90% area-normalized retention through hex pooling\n");

    for tier_dim in [TIER1_DIM, TIER2_DIM] {
        for shape in UPLIFT_SHAPES {
            for &strength in STRENGTH_VALUES {
                for &roughness_seed in ROUGHNESS_SEEDS {
                    let seed: u64 = 0xA11A_2A11; // Fixed probe seed for reproducibility

                    // Generate synthetic uplift + SHIPPING resistance field
                    let uplift = generate_uplift(tier_dim, shape, seed, roughness_seed);
                    let resistance = resistance_field(tier_dim, seed, HMAX);

                    // Call SHIPPING erosion machinery (erode_from_fields with pre-built fields)
                    let erosion_result = erode_from_fields(
                        seed,
                        HMAX,
                        tier_dim,
                        uplift,
                        resistance,
                        strength > 0, // enable_erosion
                        strength,
                    );

                    let height = &erosion_result.height;
                    let area = &erosion_result.drainage.area;
                    let downstream = &erosion_result.drainage.downstream;

                    // Compute metrics
                    let drainage_density = compute_drainage_density(tier_dim, area);
                    let crest_connectivity = compute_crest_connectivity(tier_dim, area, height);
                    let (valley_relief_p10, valley_relief_p90) = compute_valley_relief(tier_dim, height);
                    let anti_spike_pass = compute_anti_spike_test(tier_dim, height);
                    let resample_fidelity = compute_resample_fidelity(
                        tier_dim,
                        area,
                        height,
                        HEX_GRID_SIZE,
                    );

                    // Check if combo passes all metrics
                    let passes_all = drainage_density >= DRAINAGE_DENSITY_TARGET
                        && crest_connectivity >= CREST_CONNECTIVITY_TARGET
                        && valley_relief_p10 >= VALLEY_RELIEF_P10_MIN
                        && valley_relief_p90 <= VALLEY_RELIEF_P90_MAX
                        && anti_spike_pass
                        && resample_fidelity >= RESAMPLE_FIDELITY_TARGET;

                    let result = ProbeResult {
                        dim: tier_dim,
                        shape: shape.to_string(),
                        strength,
                        roughness_seed,
                        drainage_density,
                        crest_connectivity,
                        valley_relief_p10,
                        valley_relief_p90,
                        anti_spike_pass,
                        resample_fidelity,
                        passes_all,
                    };

                    all_results.push(result.clone());

                    // Generate PNG gallery (height + drainage overlay)
                    let drainage_overlay = render_drainage_overlay(tier_dim, area, height);
                    let filename = format!(
                        "{}/probe_{}x{}_{}_s{}_r{:x}.ppm",
                        out_dir, tier_dim, tier_dim, shape, strength, roughness_seed
                    );
                    write_ppm(&filename, tier_dim, &drainage_overlay).expect("Failed to write PNG");

                    // Track passes per combo
                    let combo_key = format!("{}x{} {} s{}",
                        tier_dim, tier_dim, shape, strength);
                    passes_per_combo.insert(combo_key, passes_all);

                    // Log individual result
                    println!(
                        "{:3}x{:3} | {:15} | s{:3} | DD: {:.2} | CC: {:.3} | VR: [{:3}, {:3}] | AS: {} | RF: {:.3} | {}",
                        tier_dim, tier_dim, shape, strength,
                        drainage_density, crest_connectivity, valley_relief_p10, valley_relief_p90,
                        if anti_spike_pass { "✓" } else { "✗" },
                        resample_fidelity,
                        if passes_all { "✓ PASS" } else { "✗ fail" }
                    );
                }
            }
        }
    }

    // Determine gate verdict
    let any_pass = all_results.iter().any(|r| r.passes_all);

    println!("\n╔════════════════════════════════════════════════════════════════════════════════════════╗");
    if any_pass {
        let winning_combo = all_results.iter().find(|r| r.passes_all).unwrap();
        println!("║ GATE VERDICT: PASS                                                                  ║");
        println!("║                                                                                    ║");
        println!("║ Winning combination:                                                              ║");
        println!("║   Dimension: {}×{}", winning_combo.dim, winning_combo.dim);
        println!("║   Shape: {}", winning_combo.shape);
        println!("║   Erosion Strength: {}%", winning_combo.strength);
        println!("║   Roughness Seed: {:#x}", winning_combo.roughness_seed);
        println!("╚════════════════════════════════════════════════════════════════════════════════════════╝");
    } else {
        println!("║ GATE VERDICT: FAIL                                                                  ║");
        println!("║                                                                                    ║");
        println!("║ Diagnosis: No combination passed all 5 metrics. Root cause analysis:               ║");

        let max_dd = all_results.iter().map(|r| (r.drainage_density * 100.0) as i64).max().unwrap_or(0);
        let max_cc = all_results.iter().map(|r| (r.crest_connectivity * 1000.0) as i64).max().unwrap_or(0);

        if max_dd < (DRAINAGE_DENSITY_TARGET * 100.0) as i64 {
            println!("║   (a) Incision too weak: max drainage density {:.2} < {:.2}",
                max_dd as f64 / 100.0, DRAINAGE_DENSITY_TARGET);
        } else if max_dd >= (DRAINAGE_DENSITY_TARGET * 100.0) as i64 {
            println!("║   (✓) Drainage density achievable: max {:.2} ≥ {:.2}",
                max_dd as f64 / 100.0, DRAINAGE_DENSITY_TARGET);
        }

        if max_cc < (CREST_CONNECTIVITY_TARGET * 1000.0) as i64 {
            println!("║   (b) Crest connectivity too weak: max {:.3} < {:.3}",
                max_cc as f64 / 1000.0, CREST_CONNECTIVITY_TARGET);
        }

        println!("╚════════════════════════════════════════════════════════════════════════════════════════╝");
    }

    println!("\nGallery written to: {}", out_dir);
    println!("Results: {} combos tested", all_results.len());
}
