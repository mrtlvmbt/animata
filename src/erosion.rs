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

// ---- Thermal parameters ----
const THERMAL_PASSES: u32 = 8;
/// Max height difference (elevation units) allowed to a neighbour before material slumps.
const TALUS: f32 = 0.012;
const THERMAL_RATE: f32 = 0.5;

/// Deterministic per-droplet PRNG (splitmix64).
struct Rng(u64);
impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}

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
pub fn erode(seed: u64, elev: &mut [f32]) {
    hydraulic(seed, elev);
    thermal(elev);
    for h in elev.iter_mut() {
        *h = h.clamp(0.0, 1.0);
    }
}

fn hydraulic(seed: u64, elev: &mut [f32]) {
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

    let mut rng = Rng(seed ^ 0xE051_0051_0051_0051);
    let num = ((COLS * ROWS) as f32 * DROPLET_FRACTION) as u64;
    for _ in 0..num {
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
                elev[node] += drop * (1.0 - fx) * (1.0 - fy);
                elev[node + 1] += drop * fx * (1.0 - fy);
                elev[node + COLS] += drop * (1.0 - fx) * fy;
                elev[node + COLS + 1] += drop * fx * fy;
            } else {
                // Erode: take from the brush footprint, never more than the step depth.
                let amount = ((capacity - sediment) * ERODE_SPEED).min(-dh);
                for &(bx, by, w) in &brush {
                    let (ex, ey) = (cx + bx, cy + by);
                    if ex < 0 || ey < 0 || ex >= COLS as i32 || ey >= ROWS as i32 {
                        continue;
                    }
                    let e = ey as usize * COLS + ex as usize;
                    let taken = (amount * w).min(elev[e]);
                    elev[e] -= taken;
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
}

/// Thermal/talus relaxation: where the drop to the steepest-descent neighbour exceeds the
/// talus threshold, move material down. Smooths cliffs into scree and caps the slope.
fn thermal(elev: &mut [f32]) {
    let n = COLS * ROWS;
    let mut delta = vec![0f32; n];
    for _ in 0..THERMAL_PASSES {
        for v in delta.iter_mut() {
            *v = 0.0;
        }
        for y in 0..ROWS as i32 {
            for x in 0..COLS as i32 {
                let i = y as usize * COLS + x as usize;
                let h = elev[i];
                let mut low_i = i;
                let mut low_h = h;
                for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                    if nx < 0 || ny < 0 || nx >= COLS as i32 || ny >= ROWS as i32 {
                        continue;
                    }
                    let j = ny as usize * COLS + nx as usize;
                    if elev[j] < low_h {
                        low_h = elev[j];
                        low_i = j;
                    }
                }
                let diff = h - low_h;
                if low_i != i && diff > TALUS {
                    let move_amt = (diff - TALUS) * 0.5 * THERMAL_RATE;
                    delta[i] -= move_amt;
                    delta[low_i] += move_amt;
                }
            }
        }
        for i in 0..n {
            elev[i] += delta[i];
        }
    }
}
