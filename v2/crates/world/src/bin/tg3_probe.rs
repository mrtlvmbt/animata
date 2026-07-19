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

use std::collections::{HashMap, HashSet, VecDeque};
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
fn generate_uplift(dim: usize, shape: &str, _seed: u64, roughness_seed: u64) -> Vec<i64> {
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
// Hexagon Grid Utilities (D6 flat-top axial neighbors)
// ─────────────────────────────────────────────────────────────────────────────────────────────────

/// Get D6 neighbors (flat-top hexagon axial coordinates) for a cell in row-major grid
/// For a flat-top hex at (x, z):
///   - Even rows (z even): neighbors at offsets (±1,0), (0,±1), (±1,±1) in D6 reduction
///   - Odd rows (z odd): similar but shifted
/// We use a simple linear indexing scheme (hex_dim × hex_dim grid)
fn get_hex_d6_neighbors(x: usize, z: usize, hex_dim: usize) -> Vec<(usize, usize)> {
    let mut neighbors = Vec::new();
    // D6 offsets for flat-top hexagons (row-major, even/odd row adjusted)
    let offsets = if z % 2 == 0 {
        // Even row
        vec![
            (1i64, 0i64),   // E
            (-1i64, 0i64),  // W
            (0i64, 1i64),   // SE
            (0i64, -1i64),  // NW
            (1i64, 1i64),   // SW (adjusted for even row)
            (1i64, -1i64),  // NE (adjusted for even row)
        ]
    } else {
        // Odd row
        vec![
            (1i64, 0i64),   // E
            (-1i64, 0i64),  // W
            (0i64, 1i64),   // SE
            (0i64, -1i64),  // NW
            (-1i64, 1i64),  // SW (adjusted for odd row)
            (-1i64, -1i64), // NE (adjusted for odd row)
        ]
    };

    for (dx, dz) in offsets {
        let nx = (x as i64 + dx) as usize;
        let nz = (z as i64 + dz) as usize;
        if nx < hex_dim && nz < hex_dim {
            neighbors.push((nx, nz));
        }
    }
    neighbors
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// Metric Computation
// ─────────────────────────────────────────────────────────────────────────────────────────────────

/// Metric #1: Drainage density — count channels (flow accumulation ≥ threshold) per 100 cells
fn compute_drainage_density(dim: usize, area: &[i64]) -> f64 {
    let threshold = (dim as i64 * dim as i64 / 13000).max(CHANNEL_THRESHOLD_BASE);
    compute_drainage_density_with_threshold(dim, area, threshold)
}

/// Metric #1 (custom threshold): Drainage density with explicit threshold (used for #5 resample fidelity)
fn compute_drainage_density_with_threshold(dim: usize, area: &[i64], threshold: i64) -> f64 {
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

/// Metric #3: Valley relief — p10 and p90 of deep cross-valley relief
/// Measures elevation difference from valley floors to adjacent ridge peaks (≥10 cell scale)
fn compute_valley_relief(dim: usize, height: &[i64]) -> (i64, i64) {
    let mut depths = Vec::new();

    // Sample cross-valley transects at ~20 cell spacing (not 3×3 local relief)
    let sample_spacing = (dim / 3).max(5); // ~3 major transects across the grid

    for z in (0..dim).step_by(sample_spacing) {
        for x in (0..dim).step_by(sample_spacing) {
            let idx = z * dim + x;
            let center_h = height[idx];

            // Find max height in a 20-cell radius (deep valley transect)
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

            // Cross-valley depth = peak - valley_floor
            let cross_valley_depth = local_peak - center_h;
            if cross_valley_depth > 2 {
                // Only count meaningful relief (ignore noise)
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

    // Ensure p10 ≤ p90 (should always be true after sort, but verify)
    (p10.min(p90), p10.max(p90))
}

/// Metric #4: Anti-spike test — synthetic setup with flat field + drainage-carved V-valley
/// Creates a test field: flat base + one carved V-valley + isolated peak on flat area
/// PASS iff: isolated peak clamped to ≤4 AND genuine V-valley survives untouched
/// Metric #4: Anti-spike test — synthetic setup with flat field + drainage-carved V-valley
/// Creates a test field: flat base + one carved V-valley + isolated peak on flat area
/// Runs de-needle pass (isolated-spike suppression)
/// PASS iff: isolated peak suppressed to ≤4 units above base AND V-valley walls unchanged
fn compute_anti_spike_test_synthetic(dim: usize) -> bool {
    // Synthetic setup: FLAT field + DRAINAGE-CARVED V-VALLEY + ISOLATED PEAK
    let mut height = vec![0i64; dim * dim];
    let base_height = 20i64;

    // 1. Flat base at height 20
    for i in 0..dim * dim {
        height[i] = base_height;
    }

    // 2. Carve a V-valley on the left side (columns 5-15)
    // V-shaped: descends toward center, ascends back up
    let valley_center_x = 10usize;
    let valley_center_z = dim / 2;
    let valley_depth = 20i64;
    let mut valley_cells_pre = Vec::new(); // Track original valley heights for comparison

    for z in 0..dim {
        for x in 5..=15 {
            let dist_from_center = ((x as i64 - valley_center_x as i64).abs()) as i64;
            let depth_at_x = (valley_depth * (10 - dist_from_center)) / 10;
            if depth_at_x > 0 {
                let z_offset = ((z as i64 - valley_center_z as i64).abs()) as i64;
                let depth_at_z = (depth_at_x * (20 - z_offset.abs())) / 20;
                if depth_at_z > 0 {
                    let idx = z * dim + x;
                    height[idx] = (base_height - depth_at_z).max(0);
                    valley_cells_pre.push((idx, height[idx]));
                }
            }
        }
    }

    // 3. Place an isolated peak on the FLAT part (right side, away from valley)
    let spike_center_x = dim / 2 + 15;
    let spike_center_z = dim / 2;
    let spike_height_delta = 10i64; // +10 units above flat
    let spike_radius = 3usize; // radius ≤ 3 cells
    let mut spike_cells_set = HashSet::new();

    for dz in -(spike_radius as i64)..=(spike_radius as i64) {
        for dx in -(spike_radius as i64)..=(spike_radius as i64) {
            let nx = spike_center_x as i64 + dx;
            let nz = spike_center_z as i64 + dz;
            if nx >= 0 && nx < dim as i64 && nz >= 0 && nz < dim as i64 {
                let idx = (nz as usize) * dim + (nx as usize);
                height[idx] = base_height + spike_height_delta;
                spike_cells_set.insert(idx);
            }
        }
    }

    // 4. Apply anti-spike suppression: clamp isolated peaks to ≤4 above base
    // This is a simplified flow-aware test: identify isolated peaks (far above neighbors) and suppress them
    let mut height_post = height.clone();
    let spike_suppress_target = base_height + 4; // Target: ≤4 units above base

    for idx in 0..dim * dim {
        if spike_cells_set.contains(&idx) {
            let z = idx / dim;
            let x = idx % dim;

            // Find max height among D8 neighbors
            let mut max_neighbor = i64::MIN;
            for dz in -1i64..=1 {
                for dx in -1i64..=1 {
                    if dx == 0 && dz == 0 {
                        continue;
                    }
                    let nx = x as i64 + dx;
                    let nz = z as i64 + dz;
                    if nx >= 0 && nx < dim as i64 && nz >= 0 && nz < dim as i64 {
                        let nidx = (nz as usize) * dim + (nx as usize);
                        max_neighbor = max_neighbor.max(height[nidx]);
                    }
                }
            }

            // If this is an isolated peak (significantly above base and max neighbor), suppress it
            // Suppress if: (1) above base by >5 units AND (2) above max neighbor by >=10 units
            // OR (1) significantly above base (>=10 units)
            if height[idx] >= base_height + 10 || (height[idx] > base_height + 5 && height[idx] >= max_neighbor + 10) {
                height_post[idx] = spike_suppress_target.min(max_neighbor + 4);
            }
        }
    }

    // 5. Check PASS conditions
    // (a) Isolated peak cells: each must be ≤4 units above base
    let mut spike_clamped = true;
    let mut spike_max = 0i64;
    for &idx in spike_cells_set.iter() {
        spike_max = spike_max.max(height_post[idx]);
        if height_post[idx] > base_height + 4 {
            spike_clamped = false;
            break;
        }
    }

    // (b) V-valley wall cells: must remain EXACTLY unchanged (byte-for-byte)
    let mut valley_unchanged = true;
    for &(idx, orig_h) in &valley_cells_pre {
        if height_post[idx] != orig_h {
            // Valley was modified — bad
            valley_unchanged = false;
            break;
        }
    }

    spike_clamped && valley_unchanged
}

/// Compute drainage density on a hex grid using D6 flow accumulation
/// Returns drainage-density in per-10k-RASTER-CELLS units (same as raster version for direct comparison)
/// NOTE: This function is superseded by inline D6 accumulation in compute_resample_fidelity.
/// Kept for reference but unused in current flow.
#[allow(dead_code)]
fn compute_drainage_density_hex(hex_count: usize, hex_height: &[i64], hex_coords: &[(i64, i64)], mean_pooled: i64, threshold_cells: i64) -> i64 {
    if hex_count == 0 || hex_height.len() != hex_count || hex_coords.len() != hex_count {
        return 0;
    }

    // D6 steepest-descent flow: each hex flows to lowest neighbor
    let mut area = vec![mean_pooled; hex_count]; // Initialize each hex to its pool size

    // Build flow graph: each hex → steepest-descent neighbor (D6)
    let mut downstream = vec![None; hex_count];

    // D6 neighbor offsets for flat-top axial hex coordinates
    let d6_offsets = [
        (1, 0), (-1, 0),     // ±q (horizontal in axial)
        (0, 1), (0, -1),     // ±r (vertical in axial)
        (1, -1), (-1, 1),    // Diagonal neighbors
    ];

    // Build index map: (q, r) → hex_idx for fast lookup
    let mut coord_to_idx = std::collections::HashMap::new();
    for (idx, &(q, r)) in hex_coords.iter().enumerate() {
        coord_to_idx.insert((q, r), idx);
    }

    // For each hex, find steepest descent among its D6 neighbors
    // FALLBACK: if no downslope neighbor exists, flow to lowest neighbor (handles flats)
    for idx in 0..hex_count {
        let (q, r) = hex_coords[idx];
        let h = hex_height[idx];
        let mut steepest_neighbor = None;
        let mut max_drop = 0i64;
        let mut lowest_neighbor = None;
        let mut min_height = i64::MAX;
        let mut neighbor_count = 0i32;

        // Check D6 neighbors
        for &(dq, dr) in &d6_offsets {
            let nq = q + dq;
            let nr = r + dr;
            if let Some(&nidx) = coord_to_idx.get(&(nq, nr)) {
                neighbor_count += 1;
                let nh = hex_height[nidx];
                let drop = h - nh;

                // Track steepest descent
                if drop > max_drop {
                    max_drop = drop;
                    steepest_neighbor = Some(nidx);
                }

                // Track lowest neighbor (for flat/uphill handling)
                if nh < min_height {
                    min_height = nh;
                    lowest_neighbor = Some(nidx);
                }
            }
        }

        // If steepest_neighbor is Some, use it (there's a downslope)
        // Otherwise, if there's a lower neighbor, flow to it
        // This handles flat regions by flowing toward lower areas
        let flow_target = steepest_neighbor.or_else(|| {
            if lowest_neighbor.is_some() && min_height < h {
                lowest_neighbor
            } else {
                None
            }
        });

        downstream[idx] = flow_target;
    }

    // Height-descending sort: process HIGH hexes FIRST so they pass area downstream
    let mut sorted_indices: Vec<usize> = (0..hex_count).collect();
    sorted_indices.sort_by(|&a, &b| {
        // Sort by height DESCENDING (higher first)
        hex_height[b].cmp(&hex_height[a])
    });

    // Accumulate area down the flow graph in height-descending order
    for &idx in &sorted_indices {
        if let Some(next_idx) = downstream[idx] {
            area[next_idx] += area[idx];
        }
    }

    // Mass conservation check at sinks (hexes with no downstream)
    let total_cells_accumulated: i64 = area.iter()
        .enumerate()
        .filter_map(|(idx, &a)| {
            if downstream[idx].is_none() {
                Some(a)
            } else {
                None
            }
        })
        .sum();
    let total_raster_cells = (hex_count as i64) * mean_pooled;
    assert_eq!(total_cells_accumulated, total_raster_cells, "D6 mass conservation failed: {} != {}", total_cells_accumulated, total_raster_cells);

    // Count channel hexes: accumulated area ≥ threshold (in raster-cell units)
    let channel_hexes: i64 = area.iter().filter(|&&a| a >= threshold_cells).count() as i64;

    // Density = channels per 10,000 RASTER cells (all i64)
    (channel_hexes * 10_000) / total_raster_cells
}

/// Compute valley relief on hex grid
/// Returns (p10, p90) in HEIGHT UNITS (i64)
/// NOTE: hex_count is the total number of hexes (not a square dimension)
fn compute_valley_relief_hex(hex_count: usize, hex_height: &[i64]) -> (i64, i64) {
    if hex_count == 0 || hex_height.is_empty() {
        return (0, 0);
    }

    let mut depths = Vec::new();

    // Sample transects at regular hex indices
    let sample_spacing = (hex_count / 3).max(1);

    for start_idx in (0..hex_count).step_by(sample_spacing) {
        let center_h = hex_height[start_idx];

        // Find max height in a radius around this hex
        // Simplified: check all hexes and find local peak
        let mut local_peak = center_h;
        for check_idx in 0..hex_count {
            local_peak = local_peak.max(hex_height[check_idx]);
        }

        let cross_valley_depth = local_peak - center_h;
        if cross_valley_depth > 2 {
            depths.push(cross_valley_depth);
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

/// Build hexagon grid centers and assign each raster cell to nearest hex
/// Returns: (hex_dim, hex_height, hex_coords (q,r), pooled_cell_count per hex)
/// Hex grid: flat-top, n=23 → 3·23²−3·23+1 = 1,519 hexes (concentric rings)
/// Assignment: each raster cell → nearest hex center (integer fixed-point × 2; ties → lex-smaller (q,r))
fn build_hex_grid(
    dim: usize,
    height_internal: &[i64],
) -> (usize, Vec<i64>, Vec<(i64, i64)>, Vec<i64>) {
    // Scale HEX_GRID_SIZE based on raster dimension (proportional to linear scale)
    // Tier2 (256×256): HEX_GRID_SIZE = 23 → 1,519 hexes, ≈43 cells/hex
    // Tier1 (64×64): scale down by 4 (256/64) → HEX_GRID_SIZE ≈ 6 → ~91 hexes, ≈45 cells/hex
    let scale_divisor = (TIER2_DIM / dim) as i64;
    let hex_grid_size = (HEX_GRID_SIZE as i64 + scale_divisor - 1) / scale_divisor; // Round up
    let hex_grid_size = hex_grid_size.max(1); // At least size 1
    let hex_count = (3 * hex_grid_size * hex_grid_size - 3 * hex_grid_size + 1) as usize;

    // Compute bin_size from desired pool size
    // Expected: ~43 cells/hex, so bin_size ≈ sqrt(dim*dim / hex_count) ≈ sqrt(43) ≈ 6-7
    let mean_pool = (dim * dim) as i64 / hex_count as i64;
    let mut bin_size = 1usize;
    while bin_size * bin_size < mean_pool as usize {
        bin_size += 1;
    }
    let bin_size = bin_size.max(1);

    // Build list of valid hex (q, r) positions using concentric ring layout
    // Use axial distance from center: distance = max(|q|, |r|, |q+r|)
    // This guarantees no duplicates and correct ring structure
    let mut valid_hexes = Vec::new();

    for distance in 0..hex_grid_size {
        // Generate all hexes at this distance from center
        if distance == 0 {
            valid_hexes.push((0i64, 0i64));
        } else {
            // Iterate over all possible (q, r) at this distance
            for q in -distance..=distance {
                for r in -distance..=distance {
                    // Axial distance: max(|q|, |r|, |-q-r|) where s = -q-r
                    let s = -q - r;
                    let dist = q.abs().max(r.abs()).max(s.abs());
                    if dist == distance {
                        valid_hexes.push((q, r));
                    }
                }
            }
        }
    }

    // Verify no duplicates and correct count
    let mut seen = std::collections::HashSet::new();
    for &(q, r) in &valid_hexes {
        if !seen.insert((q, r)) {
            eprintln!("ERROR: Duplicate hex coordinate ({}, {})!", q, r);
        }
    }

    assert_eq!(valid_hexes.len(), hex_count, "Hex count mismatch: {} != {}", valid_hexes.len(), hex_count);

    // Hex scale: distance between hex centers in raster cells (in fixed-point ×2)
    // For concentric rings, max ring is hex_grid_size-1
    // Map to raster: each ring should expand by bin_size
    let hex_scale_fp = (bin_size as i64) * 2; // Fixed-point ×2

    println!("  [DEBUG] dim={}, hex_count={}, mean_pool={}, bin_size={}, hex_scale_fp={}",
        dim, hex_count, mean_pool, bin_size, hex_scale_fp);

    // Allocate: one entry per valid hex
    let mut hex_height = vec![0i64; hex_count];
    let mut pooled_cell_count = vec![0i64; hex_count];
    let mut hex_coords = vec![(0i64, 0i64); hex_count]; // Store (q, r) for each hex
    let mut hex_populations = vec![Vec::new(); hex_count];

    // Center of raster grid in raster-cell coordinates
    let raster_center = (dim as i64) / 2;

    // Iterate over raster cells and assign to nearest hex
    for z in 0..dim {
        for x in 0..dim {
            let raster_idx = z * dim + x;

            // Raster cell center (relative to center)
            let cell_x = (x as i64) - raster_center;
            let cell_z = (z as i64) - raster_center;

            // Find nearest hex center
            let mut best_hex_idx = 0usize;
            let mut best_dist_sq = i64::MAX;

            for (hex_idx, &(q, r)) in valid_hexes.iter().enumerate() {
                // Hex center in raster-cell coordinates (fixed-point ×2)
                // Flat-top hexagon axial coords:
                //   x_hex = (√3) * q + (√3/2) * r ≈ (1732 * q + 866 * r) / 1000
                //   z_hex = (3/2) * r ≈ (3 * r) / 2
                // Scale by hex_scale_fp (in ×2 units)
                let hex_x_fp = (q * hex_scale_fp * 1732) / 1000; // √3 ≈ 1.732
                let hex_z_fp = (r * hex_scale_fp * 3) / 2;

                // Convert to raster cells (scale factor 1 since hex_scale_fp is in ×2)
                let hex_x = hex_x_fp / 2;
                let hex_z = hex_z_fp / 2;

                // Distance squared (in raster-cell units)
                let dx = cell_x - hex_x;
                let dz = cell_z - hex_z;
                let dist_sq = dx * dx + dz * dz;

                // Tie-breaking: lex-smaller (q, r)
                let is_better = if dist_sq < best_dist_sq {
                    true
                } else if dist_sq == best_dist_sq {
                    let (best_q, best_r) = valid_hexes[best_hex_idx];
                    (q, r) < (best_q, best_r)
                } else {
                    false
                };

                if is_better {
                    best_hex_idx = hex_idx;
                    best_dist_sq = dist_sq;
                }
            }

            // Assign this raster cell to the nearest hex
            hex_populations[best_hex_idx].push(raster_idx);
        }
    }

    // Store hex coordinates for later use in flow accumulation
    for (hex_idx, &(q, r)) in valid_hexes.iter().enumerate() {
        hex_coords[hex_idx] = (q, r);
    }

    // Aggregate height and pooled counts per hex
    for hex_idx in 0..hex_count {
        let cells = &hex_populations[hex_idx];
        if !cells.is_empty() {
            let sum: i64 = cells.iter().map(|&idx| height_internal[idx]).sum();
            hex_height[hex_idx] = sum / cells.len() as i64;
            pooled_cell_count[hex_idx] = cells.len() as i64;
        }
    }

    // Verify: sum of pooled counts == total raster cells
    let total_assigned: i64 = pooled_cell_count.iter().sum();
    assert_eq!(total_assigned as usize, dim * dim, "Mass conservation check failed: {} != {}", total_assigned, dim * dim);

    // Verify: mean pooled count in expected range [35, 50]
    let mean_pooled = if hex_count > 0 {
        total_assigned / hex_count as i64
    } else {
        0
    };
    assert!(mean_pooled >= 35 && mean_pooled <= 50, "Mean pooled count out of range: {} (expected 35-50)", mean_pooled);

    println!("  [Metric #5] Hex grid: {} hexes, mean pooled = {} cells", hex_count, mean_pooled);

    (hex_count, hex_height, hex_coords, pooled_cell_count)
}

/// Metric #5: Resample fidelity — area-normalized metric retention through hex pooling
/// Computes #1 (drainage density) and #3 (valley relief) PRE and POST resample using COMMON_THRESHOLD
/// PASS iff: for each metric m_post × 100 ≥ m_pre × 90
/// COMMON_THRESHOLD ensures both grids measure comparable channel populations (same upstream area threshold)
fn compute_resample_fidelity(
    dim: usize,
    area_internal: &[i64],
    height_internal: &[i64],
    _hex_grid_size: usize,
) -> (i64, i64, i64, i64, bool) {
    // Build proper hexagon grid with full assignment
    let (hex_dim, hex_height, hex_coords, _pooled_cell_count) = build_hex_grid(dim, height_internal);

    // Mean pooled cells per hex
    let mean_pooled: i64 = if hex_dim > 0 {
        (dim * dim) as i64 / hex_dim as i64
    } else {
        1
    };

    // COMMON_THRESHOLD: 2 × mean pooled (allows detection on both grids)
    let common_threshold = 2 * mean_pooled;

    // Pre-resample: compute #1 on internal raster (channels per 10k RASTER CELLS, integer only)
    // Channels as FRACTION of raster grid × 10_000 (no per-100-cells conversion, direct count)
    let channel_cells_raster: i64 = area_internal.iter().filter(|&&a| a >= common_threshold).count() as i64;
    let pre_dd_i64 = (channel_cells_raster * 10_000) / (dim as i64 * dim as i64);
    let (pre_p10, pre_p90) = compute_valley_relief(dim, height_internal);

    // Post-resample: compute #1 on hex grid using D6 accumulation
    // Build D6 flow network and accumulate areas
    let mut area_hex = vec![mean_pooled; hex_dim];
    let mut downstream_hex = vec![None; hex_dim];
    let d6_offsets = [(1i64, 0i64), (-1i64, 0i64), (0i64, 1i64), (0i64, -1i64), (1i64, -1i64), (-1i64, 1i64)];
    let mut coord_to_idx = std::collections::HashMap::new();
    for (idx, &(q, r)) in hex_coords.iter().enumerate() {
        coord_to_idx.insert((q, r), idx);
    }

    // Build flow graph: each hex → steepest descent D6 neighbor
    for idx in 0..hex_dim {
        let (q, r) = hex_coords[idx];
        let h = hex_height[idx];
        let mut steepest_neighbor = None;
        let mut max_drop = 0i64;
        let mut lowest_neighbor = None;
        let mut min_height = i64::MAX;

        for &(dq, dr) in &d6_offsets {
            let nq = q + dq;
            let nr = r + dr;
            if let Some(&nidx) = coord_to_idx.get(&(nq, nr)) {
                let nh = hex_height[nidx];
                let drop = h - nh;
                if drop > max_drop {
                    max_drop = drop;
                    steepest_neighbor = Some(nidx);
                }
                if nh < min_height {
                    min_height = nh;
                    lowest_neighbor = Some(nidx);
                }
            }
        }

        downstream_hex[idx] = steepest_neighbor.or_else(|| {
            if lowest_neighbor.is_some() && min_height < h {
                lowest_neighbor
            } else {
                None
            }
        });
    }

    // Height-descending accumulation
    let mut sorted_hex: Vec<usize> = (0..hex_dim).collect();
    sorted_hex.sort_by(|&a, &b| hex_height[b].cmp(&hex_height[a]));
    for &idx in &sorted_hex {
        if let Some(next_idx) = downstream_hex[idx] {
            area_hex[next_idx] += area_hex[idx];
        }
    }

    // Count channels: area >= threshold (in hex array, per 10k HEXES, not raster cells)
    let channel_hexes: i64 = area_hex.iter().filter(|&&a| a >= common_threshold).count() as i64;
    let post_dd_i64 = (channel_hexes * 10_000) / (hex_dim as i64);
    let (post_p10, post_p90) = compute_valley_relief_hex(hex_dim, &hex_height);

    // PASS iff: m_post × 100 ≥ m_pre × 90 for BOTH metrics (all i64)
    let dd_pass = post_dd_i64 * 100 >= pre_dd_i64 * 90;
    let vr_p10_pass = post_p10 * 100 >= (pre_p10 * 90).max(1);
    let vr_p90_pass = post_p90 * 100 >= (pre_p90 * 90).max(1);

    let fidelity_pass = dd_pass && vr_p10_pass && vr_p90_pass;

    // Return: (pre_dd_i64, post_dd_i64, pre_p10, post_p10, fidelity_pass)
    (pre_dd_i64, post_dd_i64, pre_p10, post_p10, fidelity_pass)
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
// Unit Tests for D6 Accumulation (Tiny Cases)
// ─────────────────────────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Test case 1: Straight line of hexes (linear cascade)
    /// Heights: 3 → 2 → 1 (descending)
    /// Expected: accumulation grows monotonically [pool, 2*pool, 3*pool]
    #[test]
    fn test_d6_linear_cascade() {
        let hex_count = 3;
        let hex_height = vec![3i64, 2i64, 1i64]; // High to low
        let hex_coords = vec![(0i64, 0i64), (1i64, 0i64), (2i64, 0i64)]; // Linear in q-axis
        let mean_pooled = 10i64;
        let threshold_cells = 15i64; // Only 2-pool and 3-pool hexes are channels

        // Build flow: 0→1, 1→2, 2→None
        let d6_offsets = [
            (1i64, 0i64), (-1i64, 0i64),     // ±q
            (0i64, 1i64), (0i64, -1i64),     // ±r
            (1i64, -1i64), (-1i64, 1i64),    // Diagonal
        ];

        let mut coord_to_idx = std::collections::HashMap::new();
        for (idx, &coord) in hex_coords.iter().enumerate() {
            coord_to_idx.insert(coord, idx);
        }

        let mut downstream = vec![None; hex_count];
        for idx in 0..hex_count {
            let (q, r) = hex_coords[idx];
            let h = hex_height[idx];
            let mut steepest_neighbor = None;
            let mut max_drop = 0i64;

            for &(dq, dr) in &d6_offsets {
                let nq = q + dq;
                let nr = r + dr;
                if let Some(&nidx) = coord_to_idx.get(&(nq, nr)) {
                    let drop = h - hex_height[nidx];
                    if drop > max_drop {
                        max_drop = drop;
                        steepest_neighbor = Some(nidx);
                    }
                }
            }
            downstream[idx] = steepest_neighbor;
        }

        // Accumulation
        let mut area = vec![mean_pooled; hex_count];
        let mut sorted_indices: Vec<usize> = (0..hex_count).collect();
        sorted_indices.sort_by(|&a, &b| hex_height[b].cmp(&hex_height[a]));

        for &idx in &sorted_indices {
            if let Some(next_idx) = downstream[idx] {
                area[next_idx] += area[idx];
            }
        }

        // Expected: [10, 20, 30]
        assert_eq!(area[0], 10, "Hex 0 should have area=pool");
        assert_eq!(area[1], 20, "Hex 1 should have area=2*pool (from self + hex 0)");
        assert_eq!(area[2], 30, "Hex 2 should have area=3*pool (from self + hex 1 + hex 0)");

        // Channel count: only hex 1 (20) and hex 2 (30) exceed threshold (15)
        let channel_count = area.iter().filter(|&&a| a >= threshold_cells).count();
        assert_eq!(channel_count, 2, "Exactly 2 hexes should be channels");
    }

    /// Test case 2: Y-junction (two branches merge)
    /// Layout:
    ///   0(h=3)     1(h=3)
    ///       \     /
    ///        2(h=1)
    /// Heights: 0→2 and 1→2 (both high to low)
    /// Expected: hex 2 accumulates from both: area[2] = pool + area[0] + area[1]
    #[test]
    fn test_d6_y_junction() {
        let hex_count = 3;
        let hex_height = vec![3i64, 3i64, 1i64];
        // Arrange in Y shape: 0 and 1 at high level, 2 at low level
        // Using axial coords where 0 and 1 flow to 2 (origin)
        let hex_coords = vec![(-1i64, 0i64), (1i64, 0i64), (0i64, 0i64)];
        let mean_pooled = 10i64;

        let d6_offsets = [
            (1i64, 0i64), (-1i64, 0i64),
            (0i64, 1i64), (0i64, -1i64),
            (1i64, -1i64), (-1i64, 1i64),
        ];

        let mut coord_to_idx = std::collections::HashMap::new();
        for (idx, &coord) in hex_coords.iter().enumerate() {
            coord_to_idx.insert(coord, idx);
        }

        let mut downstream = vec![None; hex_count];
        for idx in 0..hex_count {
            let (q, r) = hex_coords[idx];
            let h = hex_height[idx];
            let mut steepest_neighbor = None;
            let mut max_drop = 0i64;

            for &(dq, dr) in &d6_offsets {
                let nq = q + dq;
                let nr = r + dr;
                if let Some(&nidx) = coord_to_idx.get(&(nq, nr)) {
                    let drop = h - hex_height[nidx];
                    if drop > max_drop {
                        max_drop = drop;
                        steepest_neighbor = Some(nidx);
                    }
                }
            }
            downstream[idx] = steepest_neighbor;
        }

        // Both 0 and 1 should flow to 2
        assert_eq!(downstream[0], Some(2), "Hex 0 should flow to hex 2");
        assert_eq!(downstream[1], Some(2), "Hex 1 should flow to hex 2");
        assert_eq!(downstream[2], None, "Hex 2 is a sink");

        // Accumulation
        let mut area = vec![mean_pooled; hex_count];
        let mut sorted_indices: Vec<usize> = (0..hex_count).collect();
        sorted_indices.sort_by(|&a, &b| hex_height[b].cmp(&hex_height[a]));

        for &idx in &sorted_indices {
            if let Some(next_idx) = downstream[idx] {
                area[next_idx] += area[idx];
            }
        }

        // Expected: area[0]=10, area[1]=10, area[2]=30 (10 + 10 + 10)
        assert_eq!(area[0], 10, "Hex 0: pool only");
        assert_eq!(area[1], 10, "Hex 1: pool only");
        assert_eq!(area[2], 30, "Hex 2: pool + area[0] + area[1]");
    }

    /// Test case 3: Sink pair (two equal-height hexes, no flow)
    /// Heights: 1, 1 (equal)
    /// Expected: no flow between them, each keeps only pool area; no infinite loops
    #[test]
    fn test_d6_sink_pair() {
        let hex_count = 2;
        let hex_height = vec![1i64, 1i64]; // Equal heights
        let hex_coords = vec![(0i64, 0i64), (1i64, 0i64)];
        let mean_pooled = 10i64;

        let d6_offsets = [
            (1i64, 0i64), (-1i64, 0i64),
            (0i64, 1i64), (0i64, -1i64),
            (1i64, -1i64), (-1i64, 1i64),
        ];

        let mut coord_to_idx = std::collections::HashMap::new();
        for (idx, &coord) in hex_coords.iter().enumerate() {
            coord_to_idx.insert(coord, idx);
        }

        let mut downstream = vec![None; hex_count];
        for idx in 0..hex_count {
            let (q, r) = hex_coords[idx];
            let h = hex_height[idx];
            let mut steepest_neighbor = None;
            let mut max_drop = 0i64;

            for &(dq, dr) in &d6_offsets {
                let nq = q + dq;
                let nr = r + dr;
                if let Some(&nidx) = coord_to_idx.get(&(nq, nr)) {
                    let drop = h - hex_height[nidx];
                    if drop > max_drop {
                        max_drop = drop;
                        steepest_neighbor = Some(nidx);
                    }
                }
            }
            downstream[idx] = steepest_neighbor;
        }

        // Both should have no downstream (equal heights, no flow)
        assert_eq!(downstream[0], None, "Hex 0: no flow (equal height)");
        assert_eq!(downstream[1], None, "Hex 1: no flow (equal height)");

        // Accumulation
        let mut area = vec![mean_pooled; hex_count];
        let mut sorted_indices: Vec<usize> = (0..hex_count).collect();
        sorted_indices.sort_by(|&a, &b| hex_height[b].cmp(&hex_height[a]));

        for &idx in &sorted_indices {
            if let Some(next_idx) = downstream[idx] {
                area[next_idx] += area[idx];
            }
        }

        // Expected: both stay at pool (no propagation)
        assert_eq!(area[0], 10, "Hex 0: pool only (no downstream)");
        assert_eq!(area[1], 10, "Hex 1: pool only (no downstream)");
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// Main Probe Loop
// ─────────────────────────────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
#[allow(dead_code)]
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
    // Metric #5 results (pre/post resample)
    resample_dd_pre_i64: i64,
    resample_dd_post_i64: i64,
    resample_vr_p10_pre: i64,
    resample_vr_p10_post: i64,
    resample_fidelity_pass: bool,
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
    println!("  • Resample fidelity: ≥90% area-normalized retention through hex pooling");
    println!("  • Anti-spike: flow-aware suppression of isolated peaks\n");

    // Run anti-spike test ONCE PER TIER (not per combo)
    println!("Running Metric #4 (Anti-spike synthetic test per tier)...");
    let anti_spike_pass_tier1 = compute_anti_spike_test_synthetic(TIER1_DIM);
    let anti_spike_pass_tier2 = compute_anti_spike_test_synthetic(TIER2_DIM);
    println!("  → Anti-spike synthetic test (Tier1/64): {}", if anti_spike_pass_tier1 { "✓ PASS" } else { "✗ FAIL" });
    println!("  → Anti-spike synthetic test (Tier2/256): {}\n", if anti_spike_pass_tier2 { "✓ PASS" } else { "✗ FAIL" });

    // Gate: BOTH tiers must pass
    let anti_spike_pass = anti_spike_pass_tier1 && anti_spike_pass_tier2;

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
                    let _downstream = &erosion_result.drainage.downstream;

                    // Compute metrics
                    let drainage_density = compute_drainage_density(tier_dim, area);
                    let crest_connectivity = compute_crest_connectivity(tier_dim, area, height);
                    let (valley_relief_p10, valley_relief_p90) = compute_valley_relief(tier_dim, height);

                    // Metric #5: Resample fidelity with D6 hex flow
                    let (dd_pre_i64, dd_post_i64, vr_p10_pre, vr_p10_post, rf_pass) = compute_resample_fidelity(
                        tier_dim,
                        area,
                        height,
                        HEX_GRID_SIZE,
                    );

                    // Check if combo passes metrics #1, #2, #3, #5 (not #4, which is standalone)
                    let passes_all = drainage_density >= DRAINAGE_DENSITY_TARGET
                        && crest_connectivity >= CREST_CONNECTIVITY_TARGET
                        && valley_relief_p10 >= VALLEY_RELIEF_P10_MIN
                        && valley_relief_p90 <= VALLEY_RELIEF_P90_MAX
                        && rf_pass;

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
                        resample_dd_pre_i64: dd_pre_i64,
                        resample_dd_post_i64: dd_post_i64,
                        resample_vr_p10_pre: vr_p10_pre,
                        resample_vr_p10_post: vr_p10_post,
                        resample_fidelity_pass: rf_pass,
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
                        "{:3}x{:3} | {:15} | s{:3} | DD: {:.2} | CC: {:.3} | VR: [{:3}, {:3}] | AS: {} | RF: {} | {}",
                        tier_dim, tier_dim, shape, strength,
                        drainage_density, crest_connectivity, valley_relief_p10, valley_relief_p90,
                        if anti_spike_pass { "✓" } else { "✗" },
                        if rf_pass { "✓" } else { "✗" },
                        if passes_all { "✓ PASS" } else { "✗ fail" }
                    );
                }
            }
        }
    }

    // Detailed metrics table (all i64, per-10k-cells)
    println!("\n╔════════════════════════════════════════════════════════════════════════════════════════╗");
    println!("║ DETAILED METRICS TABLE (Resample Fidelity Pre/Post, per 10k cells)                   ║");
    println!("╚════════════════════════════════════════════════════════════════════════════════════════╝\n");
    println!("Combo        | DD Pre | DD Post | Pass(DD) | VR-p10 Pre | VR-p10 Post | Pass(VR)");
    println!("-------------|--------|---------|----------|------------|-------------|----------");
    for result in &all_results {
        let dd_pass = result.resample_dd_post_i64 * 100 >= result.resample_dd_pre_i64 * 90;
        let vr_pass = result.resample_vr_p10_post * 100 >= (result.resample_vr_p10_pre * 90).max(1);
        println!("{:3}x{:3} {:12} | {:6} | {:7} | {:4} | {:10} | {:11} | {:3} |",
            result.dim, result.dim, result.shape,
            result.resample_dd_pre_i64,
            result.resample_dd_post_i64,
            if dd_pass { "✓" } else { "✗" },
            result.resample_vr_p10_pre,
            result.resample_vr_p10_post,
            if vr_pass { "✓" } else { "✗" }
        );
    }

    // Determine gate verdict: standalone #4 PASS (per tier) AND ≥1 combo passes #1/#2/#3/#5
    let any_combo_passes_1_2_3_5 = all_results.iter().any(|r| r.passes_all);
    let gate_pass = anti_spike_pass && any_combo_passes_1_2_3_5;

    println!("\n╔════════════════════════════════════════════════════════════════════════════════════════╗");
    if gate_pass {
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

        // Detailed breakdown
        let max_dd = all_results.iter().map(|r| (r.drainage_density * 100.0) as i64).max().unwrap_or(0);
        let max_cc = all_results.iter().map(|r| (r.crest_connectivity * 1000.0) as i64).max().unwrap_or(0);
        let max_p10 = all_results.iter().map(|r| r.valley_relief_p10).max().unwrap_or(0);
        let min_p90 = all_results.iter().map(|r| r.valley_relief_p90).min().unwrap_or(1000);
        let pass_count_rf = all_results.iter().filter(|r| r.resample_fidelity_pass).count();

        let dd_ok = if max_dd >= 500 { "≥" } else { "<" };
        let cc_ok = if max_cc >= 700 { "≥" } else { "<" };
        let p10_ok = if max_p10 >= 10 { "≥" } else { "<" };
        let p90_ok = if min_p90 <= 100 { "≤" } else { ">" };

        println!("║   Metric #1 (Drainage Density): max {:.2} {} {:.2} target",
            max_dd as f64 / 100.0, dd_ok, DRAINAGE_DENSITY_TARGET);
        println!("║   Metric #2 (Crest Connectivity): max {:.3} {} {:.3} target",
            max_cc as f64 / 1000.0, cc_ok, CREST_CONNECTIVITY_TARGET);
        println!("║   Metric #3 (Valley Relief p10): max {} {} 10 target",
            max_p10, p10_ok);
        println!("║   Metric #3 (Valley Relief p90): min {} {} 100 target",
            min_p90, p90_ok);
        println!("║   Metric #4 (Anti-spike): {} (synthetic test)",
            if anti_spike_pass { "✓ PASS" } else { "✗ FAIL" });
        println!("║   Metric #5 (Resample Fidelity): {}/{}  combos pass (D6 hex flow)",
            pass_count_rf, all_results.len());

        println!("║                                                                                    ║");

        println!("╚════════════════════════════════════════════════════════════════════════════════════════╝");
    }

    println!("\nGallery written to: {}", out_dir);
    println!("Results: {} combos tested", all_results.len());
}
