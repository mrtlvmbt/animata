//! Erosion — the realism pass over the elevation field, run once per seed after
//! tectonics + noise. Two physical processes carve the macro shape into something that
//! looks weathered:
//!
//! - **Hydraulic (droplet)**: thousands of rain droplets flow down the gradient, picking
//!   up sediment on steep fast stretches and dropping it where they slow — cutting
//!   dendritic valleys and river networks, depositing deltas. (Sebastian-Lague-style.)
//! - **Thermal (talus)**: material on slopes steeper than the talus angle slumps downhill,
//!   smoothing cliffs into scree and capping the maximum slope (kills knife edges).
//!
//! Operates on the continuous `[0, 1]` elevation field BEFORE the sea/land split, so a
//! valley cut below the shoreline becomes a fjord-like inlet. Pure function of the seed.

use crate::config::*;

// ---- Hydraulic droplet parameters (tuned for the [0,1] elevation range) ----
/// Droplets simulated, as a fraction of the column count — denser = more carved.
const DROPLET_FRACTION: f32 = 0.6;
const MAX_LIFETIME: u32 = 40;
/// Blend between the previous direction and the downhill gradient (0 = pure gradient).
const INERTIA: f32 = 0.05;
const SEDIMENT_CAPACITY: f32 = 12.0;
const MIN_CAPACITY: f32 = 0.005;
const ERODE_SPEED: f32 = 0.35;
const DEPOSIT_SPEED: f32 = 0.30;
const EVAPORATE: f32 = 0.02;
const GRAVITY: f32 = 6.0;
const START_WATER: f32 = 1.0;
const START_SPEED: f32 = 1.0;
/// Radius (columns) of the deposit/erode brush — spreads a droplet's effect so it carves
/// smooth channels instead of single-cell pits.
const EROSION_RADIUS: i32 = 3;
/// Per-droplet RNG salt (folded with the global seed + droplet index). Each droplet is
/// seeded INDEPENDENTLY by its global index, not drawn from one shared stream — so the
/// result is identical regardless of how many threads/groups run, and droplets can be
/// simulated in parallel. (The bit pattern is the old single-stream XOR constant, reused.)
const SALT_EROSION: u64 = 0xE051_0051_0051_0051;
/// Fixed number of parallel droplet groups. CONSTANT (not `current_num_threads()`) so the
/// merge order — and thus the bit-exact result — does not depend on the host's core count.
const DROPLET_GROUPS: usize = 16;
/// Droplets per snapshot batch. Within a batch all droplets read the SAME (stale) elevation
/// and accumulate edits applied at batch end; between batches the surface updates so incision
/// still compounds. Small enough that channel cells don't over-erode (many droplets in ONE
/// snapshot each taking `min(amount, elev[e])` would otherwise stack into a deep pit → knife
/// cliff); large enough to keep the groups busy. Edit-buffer memory is trivial at this size.
const DROPLET_BATCH: u64 = 1 << 13;

// ---- Thermal parameters ----
const THERMAL_PASSES: u32 = 8;
/// Max height difference (elevation units) allowed to a neighbour before material slumps.
const TALUS: f32 = 0.012;
const THERMAL_RATE: f32 = 0.5;

use crate::rng::{seed_fold, Rng};
use rayon::prelude::*;

/// Bilinear height + gradient at a float position inside `[0, W-1) × [0, H-1)`.
fn height_grad(elev: &[f32], px: f32, py: f32) -> (f32, f32, f32) {
    let (cx, cy) = (px.floor() as usize, py.floor() as usize);
    let (fx, fy) = (px - cx as f32, py - cy as f32);
    let i = cy * COLS + cx;
    let (h00, h10, h01, h11) = (elev[i], elev[i + 1], elev[i + COLS], elev[i + COLS + 1]);
    let gx = (h10 - h00) * (1.0 - fy) + (h11 - h01) * fy;
    let gy = (h01 - h00) * (1.0 - fx) + (h11 - h10) * fx;
    let height = h00 * (1.0 - fx) * (1.0 - fy)
        + h10 * fx * (1.0 - fy)
        + h01 * (1.0 - fx) * fy
        + h11 * fx * fy;
    (height, gx, gy)
}

/// Run hydraulic then thermal erosion over `elev` in place. Returns nothing; rivers are
/// derived separately (flow accumulation) by the caller.
pub fn erode(seed: u64, elev: &mut [f32], progress: &dyn Fn(f32)) {
    // Hydraulic dominates the runtime; give it most of the [0,1] band, thermal the tail.
    hydraulic(seed, elev, &|f| progress(f * 0.9));
    thermal(elev, &|f| progress(0.9 + f * 0.1));
    for h in elev.iter_mut() {
        *h = h.clamp(0.0, 1.0);
    }
}

fn hydraulic(seed: u64, elev: &mut [f32], progress: &dyn Fn(f32)) {
    // Precompute the circular brush (offsets + normalised weights) once.
    let mut brush: Vec<(i32, i32, f32)> = Vec::new();
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
    for (_, _, w) in brush.iter_mut() {
        *w /= wsum;
    }

    let num = ((COLS * ROWS) as f32 * DROPLET_FRACTION) as u64;
    // Snapshot batches: simulate DROPLET_BATCH droplets against a frozen `elev`, in
    // DROPLET_GROUPS parallel groups, each emitting (index, Δheight) edits; then apply the
    // groups IN ORDER (group 0, then 1, …; within a group in emission order) so the float-sum
    // order is fixed and the result is deterministic + thread-count-independent. The next
    // batch sees the updated surface, so channels keep deepening across batches.
    let mut done = 0u64;
    while done < num {
        let batch_end = (done + DROPLET_BATCH).min(num);
        let snapshot: &[f32] = elev;
        let group_edits: Vec<Vec<(u32, f32)>> = (0..DROPLET_GROUPS)
            .into_par_iter()
            .map(|g| {
                let mut edits: Vec<(u32, f32)> = Vec::new();
                let mut d = done + g as u64;
                while d < batch_end {
                    simulate_droplet(seed, d, snapshot, &brush, &mut edits);
                    d += DROPLET_GROUPS as u64;
                }
                edits
            })
            .collect();
        for edits in &group_edits {
            for &(idx, dz) in edits {
                elev[idx as usize] += dz;
            }
        }
        done = batch_end;
        progress(done as f32 / num as f32);
    }
}

/// Trace one droplet (seeded independently by its global index `d`) over the frozen
/// `elev` snapshot, pushing every height change as an `(index, Δ)` edit rather than mutating
/// the grid — so droplets in a batch are independent and the caller applies edits in a fixed
/// order. Reads see the snapshot, not this droplet's own in-flight edits (standard for
/// batched/GPU droplet erosion); cross-droplet over-erosion in a batch is bounded and the
/// final clamp in `erode` keeps `elev` in range.
fn simulate_droplet(seed: u64, d: u64, elev: &[f32], brush: &[(i32, i32, f32)], edits: &mut Vec<(u32, f32)>) {
    let mut rng = Rng::new(seed_fold(seed, &[SALT_EROSION, d]));
    let mut px = rng.unit() * (COLS - 1) as f32;
    let mut py = rng.unit() * (ROWS - 1) as f32;
    let (mut dx, mut dy) = (0.0f32, 0.0f32);
    let mut speed = START_SPEED;
    let mut water = START_WATER;
    let mut sediment = 0.0f32;

    for _ in 0..MAX_LIFETIME {
        let (cx, cy) = (px.floor() as i32, py.floor() as i32);
        let node = cy as usize * COLS + cx as usize;
        let (h, gx, gy) = height_grad(elev, px, py);

        // New direction: blend gradient descent with inertia, then step one cell.
        dx = dx * INERTIA - gx * (1.0 - INERTIA);
        dy = dy * INERTIA - gy * (1.0 - INERTIA);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-6 {
            break; // flat / pit
        }
        dx /= len;
        dy /= len;
        let (npx, npy) = (px + dx, py + dy);
        if npx < 0.0 || npy < 0.0 || npx >= (COLS - 1) as f32 || npy >= (ROWS - 1) as f32 {
            break; // ran off the map
        }

        let (nh, _, _) = height_grad(elev, npx, npy);
        let dh = nh - h; // >0 going uphill

        // Carrying capacity grows with downhill slope, speed and water.
        let capacity = (-dh).max(MIN_CAPACITY) * speed * water * SEDIMENT_CAPACITY;

        if sediment > capacity || dh > 0.0 {
            // Deposit: fill toward the (uphill) step or shed the excess. Drop at the
            // current node bilinearly so deposits don't spike.
            let drop = if dh > 0.0 {
                (sediment).min(dh)
            } else {
                (sediment - capacity) * DEPOSIT_SPEED
            };
            sediment -= drop;
            let (fx, fy) = (px - cx as f32, py - cy as f32);
            edits.push((node as u32, drop * (1.0 - fx) * (1.0 - fy)));
            edits.push((node as u32 + 1, drop * fx * (1.0 - fy)));
            edits.push((node as u32 + COLS as u32, drop * (1.0 - fx) * fy));
            edits.push((node as u32 + COLS as u32 + 1, drop * fx * fy));
        } else {
            // Erode: take from the brush footprint, never more than the step depth.
            let amount = ((capacity - sediment) * ERODE_SPEED).min(-dh);
            for &(bx, by, w) in brush {
                let (ex, ey) = (cx + bx, cy + by);
                if ex < 0 || ey < 0 || ex >= COLS as i32 || ey >= ROWS as i32 {
                    continue;
                }
                let e = ey as usize * COLS + ex as usize;
                let taken = (amount * w).min(elev[e]);
                edits.push((e as u32, -taken));
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
    }
}

/// Thermal/talus relaxation: where the slope to a DOWNHILL neighbour exceeds the talus angle,
/// shed the excess material downhill. Distributes to ALL lower 8-neighbours weighted by their
/// slope (not just the single steepest) so scree spreads evenly with no directional bias, and
/// uses distance-normalised slopes so diagonals are weighted correctly. Smooths cliffs, caps
/// the slope. Serial (per-cell scatter into a shared delta) so the float-sum order is fixed.
fn thermal(elev: &mut [f32], progress: &dyn Fn(f32)) {
    const INV_SQRT2: f32 = std::f32::consts::FRAC_1_SQRT_2;
    // 8-neighbour offsets with the reciprocal of their distance (orthogonal 1, diagonal 1/√2).
    const NB: [(i32, i32, f32); 8] = [
        (1, 0, 1.0),
        (-1, 0, 1.0),
        (0, 1, 1.0),
        (0, -1, 1.0),
        (1, 1, INV_SQRT2),
        (1, -1, INV_SQRT2),
        (-1, 1, INV_SQRT2),
        (-1, -1, INV_SQRT2),
    ];
    let n = COLS * ROWS;
    let mut delta = vec![0f32; n];
    for pass in 0..THERMAL_PASSES {
        progress(pass as f32 / THERMAL_PASSES as f32);
        for v in delta.iter_mut() {
            *v = 0.0;
        }
        for y in 0..ROWS as i32 {
            for x in 0..COLS as i32 {
                let i = y as usize * COLS + x as usize;
                let h = elev[i];
                // Gather downhill neighbours whose (distance-normalised) slope exceeds talus.
                let mut lowers: [(usize, f32); 8] = [(0, 0.0); 8];
                let mut k = 0usize;
                let mut total = 0.0f32;
                let mut smax = 0.0f32;
                for (dx, dy, inv) in NB {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= COLS as i32 || ny >= ROWS as i32 {
                        continue;
                    }
                    let j = ny as usize * COLS + nx as usize;
                    let s = (h - elev[j]) * inv; // drop per unit distance = slope
                    if s > TALUS {
                        lowers[k] = (j, s);
                        k += 1;
                        total += s;
                        if s > smax {
                            smax = s;
                        }
                    }
                }
                if k > 0 {
                    // Move the steepest excess, shared out by each neighbour's slope fraction.
                    let move_amt = (smax - TALUS) * 0.5 * THERMAL_RATE;
                    delta[i] -= move_amt;
                    for &(j, s) in &lowers[..k] {
                        delta[j] += move_amt * (s / total);
                    }
                }
            }
        }
        for i in 0..n {
            elev[i] += delta[i];
        }
    }
}
