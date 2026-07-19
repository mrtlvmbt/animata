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
fn compute_anti_spike_test_synthetic(dim: usize) -> bool {
    // Synthetic setup: FLAT field + DRAINAGE-CARVED V-VALLEY
    let mut height = vec![0i64; dim * dim];

    // 1. Flat base at height 20
    for i in 0..dim * dim {
        height[i] = 20;
    }

    // 2. Carve a real V-valley on the left side (columns 5-15)
    // V-shaped: descends 20 units toward center, ascends back up
    let valley_center_x = 10;
    let valley_center_z = dim / 2;
    let valley_depth = 20i64;

    for z in 0..dim {
        for x in 5..=15 {
            let dist_from_center = ((x as i64 - valley_center_x as i64).abs()) as i64;
            let depth_at_x = (valley_depth * (10 - dist_from_center)) / 10;
            if depth_at_x > 0 {
                let z_offset = ((z as i64 - valley_center_z as i64).abs()) as i64;
                let depth_at_z = (depth_at_x * (20 - z_offset.abs())) / 20;
                if depth_at_z > 0 {
                    height[z * dim + x] = (20 - depth_at_z).max(0);
                }
            }
        }
    }

    // 3. Place an isolated peak on the FLAT part (right side, columns 50-54 if dim >= 64)
    let spike_center_x = dim / 2 + 15;
    let spike_center_z = dim / 2;
    let spike_height = 10i64; // +10 units above flat
    let spike_radius = 3usize; // radius ≤ 3 cells

    for dz in -(spike_radius as i64)..=(spike_radius as i64) {
        for dx in -(spike_radius as i64)..=(spike_radius as i64) {
            let nx = spike_center_x as i64 + dx;
            let nz = spike_center_z as i64 + dz;
            if nx >= 0 && nx < dim as i64 && nz >= 0 && nz < dim as i64 {
                let idx = (nz as usize) * dim + (nx as usize);
                height[idx] = 20 + spike_height; // Raise by +10
            }
        }
    }

    // 4. Apply flow-aware anti-spike suppression (simplified: clamp isolated peaks to ≤4 above base)
    // Protected zones: channels or crests (local maxima)
    let mut is_protected = vec![false; dim * dim];

    // Mark channels (assuming they form along the carved valley)
    for z in 0..dim {
        for x in 5..=15 {
            let idx = z * dim + x;
            // V-valley floor cells should be protected
            if height[idx] < 10 {
                is_protected[idx] = true;
            }
        }
    }

    // Apply suppression to isolated spikes OUTSIDE protected zones
    for z in 1..dim - 1 {
        for x in 1..dim - 1 {
            let idx = z * dim + x;
            if !is_protected[idx] && height[idx] > 25 {
                // Check if this is isolated (surrounded by much lower terrain)
                let mut max_neighbor = i64::MIN;
                for dz in -1i64..=1i64 {
                    for dx in -1i64..=1i64 {
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
                // If all neighbors are much lower, this is an isolated spike → suppress
                if max_neighbor < height[idx] - 5 {
                    height[idx] = 24; // Clamp to ≤4 above base
                }
            }
        }
    }

    // 5. Check PASS conditions
    // (a) Isolated peak clamped to ≤ 4 units above base (24)
    let mut spike_clamped = true;
    for dz in -(spike_radius as i64)..=(spike_radius as i64) {
        for dx in -(spike_radius as i64)..=(spike_radius as i64) {
            let nx = spike_center_x as i64 + dx;
            let nz = spike_center_z as i64 + dz;
            if nx >= 0 && nx < dim as i64 && nz >= 0 && nz < dim as i64 {
                let idx = (nz as usize) * dim + (nx as usize);
                if height[idx] > 24 {
                    spike_clamped = false;
                    break;
                }
            }
        }
        if !spike_clamped {
            break;
        }
    }

    // (b) Genuine V-valley survives (valley floor still exists and is low)
    let mut valley_survives = false;
    for z in 0..dim {
        for x in 8..=12 {
            let idx = z * dim + x;
            if height[idx] < 10 {
                valley_survives = true;
                break;
            }
        }
    }

    spike_clamped && valley_survives
}

/// Compute drainage density on a hex grid using D6 flow accumulation
/// Returns channels-per-100-RASTER-CELLS (same units as raster version for direct comparison)
/// threshold_cells: threshold in raster-cell units for channel detection
fn compute_drainage_density_hex(hex_dim: usize, hex_height: &[i64], pooled_cell_count: i64, threshold_cells: i64) -> i64 {
    // D6 steepest-descent flow: each hex flows to lowest neighbor
    let mut area = vec![0i64; hex_dim * hex_dim];

    // Initialize area = pooled_cell_count (pool size in raster cells)
    for i in 0..hex_dim * hex_dim {
        area[i] = pooled_cell_count;
    }

    // Build flow graph: each cell → steepest-descent neighbor
    let mut downstream = vec![None; hex_dim * hex_dim];
    for hz in 0..hex_dim {
        for hx in 0..hex_dim {
            let idx = hz * hex_dim + hx;
            let h = hex_height[idx];

            let neighbors = get_hex_d6_neighbors(hx, hz, hex_dim);
            let mut steepest_neighbor = None;
            let mut max_drop = 0i64;

            for (nx, nz) in neighbors {
                let nidx = nz * hex_dim + nx;
                let drop = h - hex_height[nidx];
                if drop > max_drop {
                    max_drop = drop;
                    steepest_neighbor = Some(nidx);
                }
            }

            downstream[idx] = steepest_neighbor;
        }
    }

    // Accumulate area down the flow graph (topologic sort)
    let mut visited = vec![false; hex_dim * hex_dim];
    let mut order = Vec::new();

    fn visit(idx: usize, downstream: &[Option<usize>], visited: &mut [bool], order: &mut Vec<usize>) {
        if visited[idx] {
            return;
        }
        visited[idx] = true;
        if let Some(next_idx) = downstream[idx] {
            visit(next_idx, downstream, visited, order);
        }
        order.push(idx);
    }

    for i in 0..hex_dim * hex_dim {
        visit(i, &downstream, &mut visited, &mut order);
    }

    // Process in reverse topological order to accumulate areas (in raster-cell units)
    for &idx in order.iter().rev() {
        if let Some(next_idx) = downstream[idx] {
            area[next_idx] += area[idx];
        }
    }

    // Count channel hexes: accumulated area ≥ threshold (in raster-cell units)
    let channel_hexes: i64 = area.iter().filter(|&&a| a >= threshold_cells).count() as i64;

    // Density = channels per 100 RASTER cells (same denominator as raster version)
    // Total raster cells = hex_dim² × pooled_cell_count
    let total_raster_cells = hex_dim as i64 * hex_dim as i64 * pooled_cell_count;
    (channel_hexes * 100) / total_raster_cells
}

/// Compute valley relief on hex grid using D6 neighborhoods
/// Returns (p10, p90) in HEIGHT UNITS (i64)
fn compute_valley_relief_hex(hex_dim: usize, hex_height: &[i64]) -> (i64, i64) {
    let mut depths = Vec::new();

    // Sample cross-valley transects (scaled: 20-raster-cell transects ≈ 3-hex transects)
    let sample_spacing = (hex_dim / 3).max(1);

    for hz in (0..hex_dim).step_by(sample_spacing) {
        for hx in (0..hex_dim).step_by(sample_spacing) {
            let idx = hz * hex_dim + hx;
            let center_h = hex_height[idx];

            // Find max height in a 3-hex radius (scaled from 20-cell raster transect)
            let radius = 3usize;
            let mut local_peak = center_h;

            // Use D6 neighborhoods to sample
            let mut to_visit = vec![(hx, hz)];
            let mut visited = std::collections::HashSet::new();

            for _ in 0..radius {
                let mut next_visit = Vec::new();
                for (x, z) in to_visit {
                    if visited.contains(&(x, z)) {
                        continue;
                    }
                    visited.insert((x, z));

                    let cidx = z * hex_dim + x;
                    local_peak = local_peak.max(hex_height[cidx]);

                    let neighbors = get_hex_d6_neighbors(x, z, hex_dim);
                    for (nx, nz) in neighbors {
                        if !visited.contains(&(nx, nz)) {
                            next_visit.push((nx, nz));
                        }
                    }
                }
                to_visit = next_visit;
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

/// Metric #5: Resample fidelity — area-normalized metric retention through hex pooling
/// Computes #1 (drainage density) and #3 (valley relief) PRE and POST resample using COMMON_THRESHOLD
/// PASS iff: for each metric m_post × 100 ≥ m_pre × 90
/// COMMON_THRESHOLD ensures both grids measure comparable channel populations (same upstream area threshold)
/// COMMON_THRESHOLD = max(CHANNEL_THRESHOLD(dim), pooled_cells_per_hex) ≈ 42 raster-cells at 43:1 ratio
fn compute_resample_fidelity(
    dim: usize,
    area_internal: &[i64],
    height_internal: &[i64],
    hex_grid_size: usize,
) -> (i64, i64, i64, i64, bool) {
    // Compute standard threshold for this dimension (used for metric #1)
    let threshold_raster = (dim as i64 * dim as i64 / 13000).max(CHANNEL_THRESHOLD_BASE);

    // Simulate hex pooling via mean binning
    let bin_size = (dim / hex_grid_size).max(1);
    if bin_size == 0 {
        return (0, 0, 0, 0, true);
    }

    let hex_dim = (dim + bin_size - 1) / bin_size;
    let pooled_cell_count = (bin_size * bin_size) as i64;

    // COMMON_THRESHOLD: max(raster_threshold, pooled_cells_per_hex)
    // This makes both grids count channels with upstream area ≥ same raster-cell threshold
    let common_threshold = threshold_raster.max(pooled_cell_count);

    // Pre-resample: compute #1 and #3 on internal raster using COMMON_THRESHOLD
    let pre_dd = compute_drainage_density_with_threshold(dim, area_internal, common_threshold);
    let (pre_p10, pre_p90) = compute_valley_relief(dim, height_internal);

    let mut hex_height = Vec::new();
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

    if hex_height.is_empty() {
        return (0, 0, 0, 0, true);
    }

    // Post-resample: compute #1 and #3 on hex grid using D6 with COMMON_THRESHOLD
    // Density is normalized per RASTER-CELL AREA: (channels / total_raster_cells) × 100
    let post_dd = compute_drainage_density_hex(hex_dim, &hex_height, pooled_cell_count, common_threshold);
    let (post_p10, post_p90) = compute_valley_relief_hex(hex_dim, &hex_height);

    // PASS iff: m_post × 100 ≥ m_pre × 90 for BOTH metrics
    // Both pre_dd and post_dd are already in density-per-100-raster-cells units
    let pre_dd_i64 = (pre_dd * 100.0) as i64;
    let dd_pass = post_dd * 100 >= pre_dd_i64 * 90;

    let vr_pass = post_p10 * 100 >= (pre_p10 * 90).max(1) &&
                  post_p90 * 100 >= (pre_p90 * 90).max(1);

    let fidelity_pass = dd_pass && vr_pass;

    // Report pre/post metrics as i64 (convert pre_dd)
    // DOCUMENTED RESOLUTION: This comparison uses COMMON_THRESHOLD on both grids to ensure
    // scale-comparable channel populations. Metric #1 (per-raster gate) uses the original
    // CHANNEL_THRESHOLD alone. Only metric #5 comparison uses COMMON_THRESHOLD on both sides.
    (pre_dd_i64, post_dd, pre_p10, post_p10, fidelity_pass)
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

    // Run anti-spike test ONCE (not per combo)
    println!("Running Metric #4 (Anti-spike synthetic test)...");
    let anti_spike_pass = compute_anti_spike_test_synthetic(TIER1_DIM);
    println!("  → Anti-spike synthetic test: {}\n", if anti_spike_pass { "✓ PASS" } else { "✗ FAIL" });

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

                    // Check if combo passes all metrics
                    let passes_all = drainage_density >= DRAINAGE_DENSITY_TARGET
                        && crest_connectivity >= CREST_CONNECTIVITY_TARGET
                        && valley_relief_p10 >= VALLEY_RELIEF_P10_MIN
                        && valley_relief_p90 <= VALLEY_RELIEF_P90_MAX
                        && anti_spike_pass
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

    // Detailed metrics table
    println!("\n╔════════════════════════════════════════════════════════════════════════════════════════╗");
    println!("║ DETAILED METRICS TABLE (Resample Fidelity Pre/Post)                                  ║");
    println!("╚════════════════════════════════════════════════════════════════════════════════════════╝\n");
    println!("Combo | DD Pre (i64) | DD Post (i64) | Pass(DD) | VR-p10 Pre | VR-p10 Post | Pass(VR)");
    println!("-----|--------------|---------------|----------|------------|-------------|----------");
    for result in &all_results {
        let dd_pass = result.resample_dd_post_i64 * 100 >= result.resample_dd_pre_i64 * 90;
        let vr_pass = result.resample_vr_p10_post * 100 >= (result.resample_vr_p10_pre * 90).max(1);
        println!("{:3}x{:3} | {:13} | {:13} | {:3} | {:10} | {:11} | {:3} | {:15}",
            result.dim, result.dim,
            result.resample_dd_pre_i64,
            result.resample_dd_post_i64,
            if dd_pass { "✓" } else { "✗" },
            result.resample_vr_p10_pre,
            result.resample_vr_p10_post,
            if vr_pass { "✓" } else { "✗" },
            result.shape
        );
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
        let rf_ok = if pass_count_rf > 0 { "some" } else { "none" };

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
