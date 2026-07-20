//! Slice-1O: Float droplet-erosion probe on plate uplift.
//!
//! **Purpose:** Validate that v1-style droplet erosion (bilinear deposition, brush eroding)
//! produces clean dendritic valleys + deposited floors (no needles) on our narrow-belt/flat-datum
//! plate uplift BEFORE investing in the expensive integer-deterministic production port.
//!
//! **Scope:** Throwaway float probe bin. Not production; no golden; no cross-arch determinism claim.
//! Tests GEOMETRY only: does droplet erosion dissect our plate field? Real risk (fixed-point
//! `sqrt`, bilinear-deposit rounding) is tackled in the production port (Slice-1P).
//!
//! **AC0 API confirmed:** `compute_plate_fields(seed, dim, requested_plate_count)` and
//! `generate_plate_uplift_field(fields, dim, hmax, plate_strength)`.

use std::fs;
use world::gen::orogeny::generate_plate_uplift_field;
use world::gen::plate::compute_plate_fields;

// Droplet parameters (ported from v1 `crates/animata-sim/src/erosion.rs`).
const MAX_LIFETIME: u32 = 40;
const INERTIA: f32 = 0.05;
const SEDIMENT_CAPACITY: f32 = 12.0;
const MIN_CAPACITY: f32 = 0.005;
const ERODE_SPEED: f32 = 0.35;
const DEPOSIT_SPEED: f32 = 0.30;
const EVAPORATE: f32 = 0.02;
const GRAVITY: f32 = 6.0;
const START_WATER: f32 = 1.0;
const START_SPEED: f32 = 1.0;
/// Brush radius (cells) for erosion footprint.
const EROSION_RADIUS: i32 = 3;
/// Droplets as fraction of grid cells.
const DROPLET_FRACTION: f32 = 0.6;

/// Bilinear height + gradient at float position.
fn height_grad(elev: &[f32], dim: usize, px: f32, py: f32) -> (f32, f32, f32) {
    let (cx, cy) = (px.floor() as usize, py.floor() as usize);
    let (fx, fy) = (px - cx as f32, py - cy as f32);

    let clamp_x = (cx + 1).min(dim - 1);
    let clamp_y = (cy + 1).min(dim - 1);

    let i = cy * dim + cx;
    let i_xp = cy * dim + clamp_x;
    let i_yp = clamp_y * dim + cx;
    let i_xyp = clamp_y * dim + clamp_x;

    let (h00, h10, h01, h11) = (elev[i], elev[i_xp], elev[i_yp], elev[i_xyp]);

    let gx = (h10 - h00) * (1.0 - fy) + (h11 - h01) * fy;
    let gy = (h01 - h00) * (1.0 - fx) + (h11 - h10) * fx;
    let height = h00 * (1.0 - fx) * (1.0 - fy)
        + h10 * fx * (1.0 - fy)
        + h01 * (1.0 - fx) * fy
        + h11 * fx * fy;
    (height, gx, gy)
}

/// Compute erosion brush (circular, normalized weights).
fn build_brush(dim: usize) -> Vec<(i32, i32, f32)> {
    let mut brush = Vec::new();
    let mut wsum = 0.0;
    for by in -EROSION_RADIUS..=EROSION_RADIUS {
        for bx in -EROSION_RADIUS..=EROSION_RADIUS {
            let d = ((bx * bx + by * by) as f32).sqrt();
            if d <= EROSION_RADIUS as f32 {
                let w = 1.0 - d / EROSION_RADIUS as f32;
                brush.push((bx, by, w));
                wsum += w;
            }
        }
    }
    for (_, _, w) in &mut brush {
        *w /= wsum;
    }
    brush
}

/// Simple RNG for droplet seeding (deterministic per seed + index).
fn seed_droplet(seed: u64, index: u64) -> f32 {
    let mut h = seed ^ index;
    h = h.wrapping_mul(0x9e3779b97f4a7c15);
    h = (h ^ (h >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    h = (h ^ (h >> 27)) >> 33;
    ((h & 0xffffff) as f32) / (0xffffff as f32)
}

/// Check if a cell is part of the relief (non-zero gradient).
fn is_relief_cell(elev: &[f32], dim: usize, cx: usize, cy: usize) -> bool {
    let h = elev[cy * dim + cx];
    for dy in -1i32..=1 {
        for dx in -1i32..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let nx = (cx as i32 + dx).max(0).min((dim - 1) as i32) as usize;
            let ny = (cy as i32 + dy).max(0).min((dim - 1) as i32) as usize;
            if (elev[ny * dim + nx] - h).abs() > 0.01 {
                return true;
            }
        }
    }
    false
}

/// Build weighted list of relief cells for preferential droplet seeding.
/// AC1, F3: seed droplets weighted toward relief (non-flat cells), not uniformly.
fn build_relief_weights(elev: &[f32], dim: usize) -> Vec<usize> {
    let mut relief_cells = Vec::new();

    for y in 0..dim {
        for x in 0..dim {
            let idx = y * dim + x;
            if is_relief_cell(elev, dim, x, y) {
                relief_cells.push(idx);
            }
        }
    }

    relief_cells
}

/// Apply droplet erosion to the elevation field in place.
/// Returns (survivors, total_droplets) for reporting survivor fraction.
fn erode_with_droplets(
    seed: u64,
    elev: &mut [f32],
    dim: usize,
) -> (usize, usize) {
    let brush = build_brush(dim);
    let num_droplets = ((dim * dim) as f32 * DROPLET_FRACTION) as u64;

    // AC1, F3: Build relief-weighted seeding pool.
    let relief_cells = build_relief_weights(elev, dim);
    let flat_cells = dim * dim - relief_cells.len();
    let relief_fraction = relief_cells.len() as f32 / (dim * dim) as f32;

    println!(
        "Relief seeding: {} relief cells ({:.1}%), {} flat ({:.1}%)",
        relief_cells.len(),
        relief_fraction * 100.0,
        flat_cells,
        (1.0 - relief_fraction) * 100.0
    );

    let mut survivors = 0usize;

    for i in 0..num_droplets {
        // Weight toward relief: with 80% chance use relief cell, 20% use any cell.
        let rng_val = seed_droplet(seed, i * 2 + 1000);
        let (start_x, start_y) = if !relief_cells.is_empty() && rng_val < 0.8 {
            // Pick random relief cell.
            let relief_idx = seed_droplet(seed, i * 3 + 2000);
            let cell_idx = ((relief_idx * relief_cells.len() as f32).floor() as usize)
                .min(relief_cells.len() - 1);
            let idx = relief_cells[cell_idx];
            (idx % dim, idx / dim)
        } else {
            // Pick random cell anywhere.
            let rx = seed_droplet(seed, i * 2) * ((dim - 1) as f32);
            let ry = seed_droplet(seed, i * 2 + 1) * ((dim - 1) as f32);
            (rx as usize, ry as usize)
        };

        let mut edits = Vec::new();

        // Seed droplet at selected position (still use float sub-cell positioning).
        let px = start_x as f32 + seed_droplet(seed, i * 10);
        let py = start_y as f32 + seed_droplet(seed, i * 11);

        // Simulate with relief-weighted start.
        if simulate_droplet_at(seed, i, px, py, elev, dim, &brush, &mut edits) {
            survivors += 1;
        }

        // Apply edits (accumulate in place).
        for (idx, dz) in edits {
            elev[idx] += dz;
        }
    }

    // Clamp to [0, 1].
    for h in elev.iter_mut() {
        *h = h.clamp(0.0, 1.0);
    }

    (survivors as usize, num_droplets as usize)
}

/// Simulate droplet starting at explicit position (for weighted seeding).
fn simulate_droplet_at(
    seed: u64,
    index: u64,
    start_px: f32,
    start_py: f32,
    elev: &[f32],
    dim: usize,
    brush: &[(i32, i32, f32)],
    edits: &mut Vec<(usize, f32)>,
) -> bool {
    let mut px = start_px;
    let mut py = start_py;
    let (mut dx, mut dy) = (0.0f32, 0.0f32);
    let mut speed = START_SPEED;
    let mut water = START_WATER;
    let mut sediment = 0.0f32;

    let mut step_count = 0u32;

    for _ in 0..MAX_LIFETIME {
        let (cx, cy) = (px.floor() as i32, py.floor() as i32);

        if cx < 0 || cy < 0 || cx >= (dim - 1) as i32 || cy >= (dim - 1) as i32 {
            break;
        }

        let (h, gx, gy) = height_grad(elev, dim, px, py);

        // Update direction: blend gradient with inertia.
        dx = dx * INERTIA - gx * (1.0 - INERTIA);
        dy = dy * INERTIA - gy * (1.0 - INERTIA);
        let len = (dx * dx + dy * dy).sqrt();

        if len < 1e-6 {
            break; // flat / pit
        }

        dx /= len;
        dy /= len;

        let npx = px + dx;
        let npy = py + dy;

        if npx < 0.0 || npy < 0.0 || npx >= (dim - 1) as f32 || npy >= (dim - 1) as f32 {
            break; // off map
        }

        let (nh, _, _) = height_grad(elev, dim, npx, npy);
        let dh = nh - h; // >0 going uphill

        // Capacity: amount of sediment the droplet can carry.
        let capacity = (-dh).max(MIN_CAPACITY) * speed * water * SEDIMENT_CAPACITY;

        if sediment > capacity || dh > 0.0 {
            // Deposit: bilinear across 4 sub-cell nodes (anti-needle mechanism).
            let drop = if dh > 0.0 {
                sediment.min(dh)
            } else {
                (sediment - capacity) * DEPOSIT_SPEED
            };

            sediment -= drop;

            let (fx, fy) = (px - cx as f32, py - cy as f32);
            let node = (cy as usize) * dim + (cx as usize);

            edits.push((node, drop * (1.0 - fx) * (1.0 - fy)));
            if cx + 1 < dim as i32 {
                edits.push((node + 1, drop * fx * (1.0 - fy)));
            }
            if cy + 1 < dim as i32 {
                edits.push((node + dim, drop * (1.0 - fx) * fy));
            }
            if cx + 1 < dim as i32 && cy + 1 < dim as i32 {
                edits.push((node + dim + 1, drop * fx * fy));
            }
        } else {
            // Erode: take from brush footprint.
            let amount = ((capacity - sediment) * ERODE_SPEED).min(-dh);

            for &(bx, by, w) in brush {
                let (ex, ey) = (cx + bx, cy + by);
                if ex < 0 || ey < 0 || ex >= dim as i32 || ey >= dim as i32 {
                    continue;
                }
                let e = (ey as usize) * dim + (ex as usize);
                let taken = (amount * w).min(elev[e]);
                edits.push((e, -taken));
                sediment += taken;
            }
        }

        speed = (speed * speed + dh * GRAVITY).max(0.0).sqrt();
        water *= 1.0 - EVAPORATE;

        if water < 1e-3 {
            break;
        }

        px = npx;
        py = npy;
        step_count += 1;
    }

    step_count > 0  // Survivor: lived past step 0
}

/// Count needle spikes: cells exceeding their 8-neighbor max by > 30 units.
/// Production constant NEEDLE_MARGIN=30 calibrated to hmax=200.
/// For normalized [0,1] field from [0,200]: 30/200 = 0.15.
fn count_needles(elev: &[f32], dim: usize) -> usize {
    const NEEDLE_MARGIN: f32 = 0.15; // 30/200 = 0.15 (matches hmax=200 normalization)
    let mut count = 0;

    for y in 0..dim {
        for x in 0..dim {
            let idx = y * dim + x;
            let h = elev[idx];

            let mut max_neighbor = h;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let nx = (x as i32 + dx).max(0).min((dim - 1) as i32) as usize;
                    let ny = (y as i32 + dy).max(0).min((dim - 1) as i32) as usize;
                    max_neighbor = max_neighbor.max(elev[ny * dim + nx]);
                }
            }

            if h - max_neighbor > NEEDLE_MARGIN {
                count += 1;
            }
        }
    }

    count
}

/// Compute relief contrast: bimodal spread (floor vs ridge).
fn compute_relief_contrast(elev: &[f32]) -> (f32, f32) {
    let mut min_h = f32::MAX;
    let mut max_h = f32::MIN;

    for &h in elev {
        if h.is_finite() {
            min_h = min_h.min(h);
            max_h = max_h.max(h);
        }
    }

    (min_h, max_h)
}

/// Save elevation field to PGM (grayscale image, 16-bit).
fn save_pgm(filename: &str, elev: &[f32], dim: usize) -> std::io::Result<()> {
    let mut data = Vec::new();

    // PGM header.
    data.extend_from_slice(b"P5\n");
    data.extend_from_slice(format!("{} {}\n", dim, dim).as_bytes());
    data.extend_from_slice(b"65535\n");

    // Normalize [0,1] to [0, 65535].
    for &h in elev {
        let val = (h * 65535.0).clamp(0.0, 65535.0) as u16;
        data.extend_from_slice(&val.to_be_bytes());
    }

    fs::write(filename, data)?;
    println!("Saved {}", filename);
    Ok(())
}

fn main() {
    let dim = 256i64;
    let hmax = 200i64;
    let plate_strength = 100i64;
    let seeds = [42u64, 12345u64];

    // Create output directory.
    fs::create_dir_all(".claude/w1o-gallery").ok();

    for (seed_idx, seed) in seeds.iter().enumerate() {
        println!("\n=== Seed {} (seed={}) ===", seed_idx + 1, seed);

        // Step 1: Build plate fields.
        println!("Building plate fields (dim={}, plate_count=10)...", dim);
        let fields = compute_plate_fields(*seed, dim, 10);

        // Step 2: Generate plate uplift field (on flat datum, base=false).
        println!("Generating plate uplift field...");
        let uplift = generate_plate_uplift_field(&fields, dim, hmax, plate_strength);

        // Step 3: Convert to float [0, 1] for droplet erosion.
        let mut elev: Vec<f32> = uplift
            .iter()
            .map(|&u| ((u as f32) / (hmax as f32)).clamp(0.0, 1.0))
            .collect();

        // Report initial relief.
        let (min_h, max_h) = compute_relief_contrast(&elev);
        println!("Initial relief: min={:.4}, max={:.4}, contrast={:.4}",
                 min_h, max_h, max_h - min_h);

        // Step 4: Run droplet erosion.
        println!("Running droplet erosion ({} droplets)...",
                 ((dim * dim) as f32 * DROPLET_FRACTION) as u64);
        let (survivors, total) = erode_with_droplets(*seed, &mut elev, dim as usize);
        let survivor_frac = survivors as f32 / total as f32;
        println!("Survivor fraction: {} / {} = {:.4}", survivors, total, survivor_frac);

        // Step 5: Compute post-erosion metrics.
        let (min_post, max_post) = compute_relief_contrast(&elev);
        println!("Post-erosion relief: min={:.4}, max={:.4}, contrast={:.4}",
                 min_post, max_post, max_post - min_post);

        // Step 6: Count needles.
        let needle_count = count_needles(&elev, dim as usize);
        println!("Needle count (>30 units above 8-neighbor max): {}", needle_count);

        // Step 7: Save to PGM.
        let filename = format!(".claude/w1o-gallery/droplet_probe_seed{:02}.pgm", seed_idx + 1);
        save_pgm(&filename, &elev, dim as usize).expect("Failed to save PGM");

        // Report summary.
        println!("✓ Seed {}: relief_contrast={:.4}, needles={}, survivors={:.1}%",
                 seed_idx + 1, max_post - min_post, needle_count, survivor_frac * 100.0);
    }

    println!("\n=== Probe complete ===");
    println!("PNGs saved to .claude/w1o-gallery/");
}
