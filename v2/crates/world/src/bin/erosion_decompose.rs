//! Slice-1L: DECOMPOSE probe — why does plate-belt erosion carve no dendritic valleys?
//!
//! **Measurement only, no fix.**
//!
//! Runs the EXISTING plate-path erosion at dim=256 over ≥2 seeds and emits per-iteration
//! distributions (over convergent-belt cells only, belt_distance ≤ belt_hw) to diagnose
//! which hypothesis (H1–H4) explains the lack of dendritic valley dissection:
//!
//! - H1: smooth-ramp / no channel nucleation
//! - H2: talus erases incision
//! - H3: integer truncation no-op
//! - H4: too few iterations
//!
//! Metrics per iteration: slope, drainage area, channelization, confluence count,
//! incision delta, talus delta, net height change.

use world::gen::plate::BoundaryType;
use std::collections::VecDeque;

const DIM: usize = 256;
const HMAX: i64 = 200;
const RIVER_THRESHOLD: i64 = 100;

fn main() {
    let seeds = [1234567890u64, 9876543210u64];

    for (seed_idx, &seed) in seeds.iter().enumerate() {
        println!("\n=== SEED {} ({}) ===", seed_idx, seed);
        run_probe(seed);
    }
}

fn run_probe(seed: u64) {
    // Build plate fields with convergent boundaries
    let plate_count = 15u32;
    let plate_count_clamped = world::gen::plate::clamp_plate_count(plate_count, DIM as i64);
    let plate_fields = world::gen::plate::compute_plate_fields(seed, DIM as i64, plate_count_clamped);

    // Compute belt distance from convergent boundaries
    let belt_distance = compute_belt_distance(DIM as i64, &plate_fields.boundary_type);
    let belt_hw = (DIM as i64 / 16).max(3);

    println!("Config: dim={}, hmax={}, enable_plate_sim=true, belt_hw={}", DIM, HMAX, belt_hw);

    // Create a stats collector
    let mut collector = StatsCollector::new(DIM, belt_hw as usize);

    // Run the instrumented erosion
    let erosion_result = world::gen::erosion::erode_with_tectonics(
        seed, HMAX, DIM,
        true,  // enable_base
        true,  // enable_fault_scarp
        true,  // enable_fault_resistance
        false, // enable_volcanic
        true,  // enable_ridges
        true,  // enable_erosion
        100,   // erosion_strength
        true,  // enable_plate_sim
        100,   // plate_strength
        0,     // _plate_repose_threshold
        Some(&belt_distance),
        Some(belt_hw),
        Some(&mut collector),
    );

    // Emit per-iteration tables
    collector.emit_tables(&erosion_result, &belt_distance, belt_hw as usize);
}

/// Compute belt distance via multi-source BFS from convergent boundaries.
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

/// Collects per-iteration statistics on the belt cell set.
struct StatsCollector {
    dim: usize,
    belt_hw: usize,
    iterations: Vec<IterationStats>,
}

struct IterationStats {
    slope_min: i64,
    slope_med: i64,
    slope_p90: i64,
    slope_max: i64,
    area_min: i64,
    area_med: i64,
    area_p90: i64,
    area_max: i64,
    channelized_fraction: f64,
    confluence_count: i64,
    incision_min: i64,
    incision_med: i64,
    incision_p90: i64,
    incision_max: i64,
    incision_zero_fraction: f64,
    talus_min: i64,
    talus_med: i64,
    talus_p90: i64,
    talus_max: i64,
    talus_zero_fraction: f64,
    net_height_med: i64,
    net_height_max: i64,
}

impl StatsCollector {
    fn new(dim: usize, belt_hw: usize) -> Self {
        Self {
            dim,
            belt_hw,
            iterations: Vec::new(),
        }
    }

    fn emit_tables(&self, erosion_result: &world::gen::erosion::ErosionState, belt_distance: &[i64], belt_hw: usize) {
        println!("\n--- Per-Iteration Statistics (belt cells only) ---\n");
        println!("Iteration | Slope(min/med/p90/max) | Area(min/med/p90/max) | Chann% | Conf# | Incis(min/med/p90/max) | Incis0% | Talus(min/med/p90/max) | Talus0% | Net(med/max)");
        println!("{}", "-".repeat(180));

        for (iter_idx, stats) in self.iterations.iter().enumerate() {
            println!(
                "{:3} | {:3}/{:5}/{:5}/{:5} | {:4}/{:5}/{:5}/{:5} | {:5.1} | {:4} | {:3}/{:5}/{:5}/{:5} | {:5.1} | {:3}/{:5}/{:5}/{:5} | {:5.1} | {:6}/{:6}",
                iter_idx,
                stats.slope_min, stats.slope_med, stats.slope_p90, stats.slope_max,
                stats.area_min, stats.area_med, stats.area_p90, stats.area_max,
                stats.channelized_fraction * 100.0,
                stats.confluence_count,
                stats.incision_min, stats.incision_med, stats.incision_p90, stats.incision_max,
                stats.incision_zero_fraction * 100.0,
                stats.talus_min, stats.talus_med, stats.talus_p90, stats.talus_max,
                stats.talus_zero_fraction * 100.0,
                stats.net_height_med, stats.net_height_max,
            );
        }

        println!("\n--- H1–H4 Hypothesis Ranking ---\n");
        self.rank_hypotheses(&self.iterations);
    }

    fn rank_hypotheses(&self, iterations: &[IterationStats]) {
        if iterations.is_empty() {
            println!("No iterations recorded.");
            return;
        }

        let last = &iterations[iterations.len() - 1];

        // Rank hypotheses based on signatures
        let mut scores: Vec<(&str, f64)> = vec![
            ("H1 (smooth-ramp)", score_h1(iterations)),
            ("H2 (talus erases incision)", score_h2(iterations)),
            ("H3 (integer truncation)", score_h3(iterations)),
            ("H4 (too few iterations)", score_h4(iterations)),
        ];
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        println!("Ranking (strongest to weakest):");
        for (i, (name, score)) in scores.iter().enumerate() {
            println!("  {}. {}: {:.2}", i + 1, name, score);
        }

        println!("\nKey observations:");
        println!("  - Channelized fraction at iteration {}: {:.1}%", iterations.len() - 1, last.channelized_fraction * 100.0);
        println!("  - Confluence count at iteration {}: {}", iterations.len() - 1, last.confluence_count);
        println!("  - Incision zero fraction at iteration {}: {:.1}%", iterations.len() - 1, last.incision_zero_fraction * 100.0);
        println!("  - Talus zero fraction at iteration {}: {:.1}%", iterations.len() - 1, last.talus_zero_fraction * 100.0);
        println!("  - Net height change (final iteration): median={}, max={}", last.net_height_med, last.net_height_max);
    }
}

// H1 score: high if channelized_fraction stays low and confluences are sparse
fn score_h1(iterations: &[IterationStats]) -> f64 {
    if iterations.is_empty() {
        return 0.0;
    }
    let last = &iterations[iterations.len() - 1];
    let chann_score = 1.0 - (last.channelized_fraction.min(0.5) / 0.5);
    let conf_score = if last.confluence_count < 5 { 1.0 } else { 0.0 };
    (chann_score + conf_score) / 2.0
}

// H2 score: high if |talus| >> |incision|
fn score_h2(iterations: &[IterationStats]) -> f64 {
    if iterations.is_empty() {
        return 0.0;
    }
    let last = &iterations[iterations.len() - 1];
    let talus_ratio = (last.talus_max as f64 + 1.0) / ((last.incision_max as f64).max(1.0) + 1.0);
    (talus_ratio.min(2.0) / 2.0).min(1.0)
}

// H3 score: high if incision_zero_fraction is near 1.0
fn score_h3(iterations: &[IterationStats]) -> f64 {
    if iterations.is_empty() {
        return 0.0;
    }
    let last = &iterations[iterations.len() - 1];
    last.incision_zero_fraction
}

// H4 score: high if net_height_max is still large (not converged)
fn score_h4(iterations: &[IterationStats]) -> f64 {
    if iterations.is_empty() {
        return 0.0;
    }
    let last = &iterations[iterations.len() - 1];
    (last.net_height_max as f64).min(50.0) / 50.0
}

impl world::gen::erosion::StatsSink for StatsCollector {
    fn record_iteration(
        &mut self,
        _iteration: usize,
        belt_distance: &[i64],
        belt_hw: i64,
        height: &[i64],
        downstream: &[Option<usize>],
        area: &[i64],
        incision_delta: &[i64],
        talus_delta_net: &[i64],
        dim: usize,
    ) {
        let mut belt_cells = Vec::new();
        let mut slopes = Vec::new();
        let mut areas = Vec::new();
        let mut channelized_count = 0i64;
        let mut confluence_count = 0i64;
        let mut incisions = Vec::new();
        let mut taluses = Vec::new();
        let mut net_deltas = Vec::new();

        for (idx, &bd) in belt_distance.iter().enumerate() {
            if bd <= belt_hw {
                belt_cells.push(idx);

                // Slope to D8 receiver
                if let Some(recv_idx) = downstream[idx] {
                    let slope = (height[idx] - height[recv_idx]).max(0);
                    slopes.push(slope);
                } else {
                    slopes.push(0);
                }

                // Area and channelization
                areas.push(area[idx]);
                if area[idx] > RIVER_THRESHOLD {
                    channelized_count += 1;
                }

                // Incision and talus
                incisions.push(incision_delta[idx].abs());
                taluses.push(talus_delta_net[idx].abs());
                net_deltas.push((incision_delta[idx] + talus_delta_net[idx]).abs());
            }
        }

        // Count confluences: cells whose D8 receiver is targeted by ≥2 upstream cells in belt
        let mut confluence_tally: std::collections::HashMap<usize, i64> = std::collections::HashMap::new();
        for &cell_idx in &belt_cells {
            if let Some(recv_idx) = downstream[cell_idx] {
                if belt_distance[recv_idx] <= belt_hw {
                    *confluence_tally.entry(recv_idx).or_insert(0) += 1;
                }
            }
        }
        confluence_count = confluence_tally.values().filter(|&&c| c >= 2).count() as i64;

        // Compute percentiles
        let stats = IterationStats {
            slope_min: slopes.iter().copied().min().unwrap_or(0),
            slope_med: percentile(&slopes, 0.5),
            slope_p90: percentile(&slopes, 0.9),
            slope_max: slopes.iter().copied().max().unwrap_or(0),
            area_min: areas.iter().map(|a| a.integer_sqrt()).min().unwrap_or(0),
            area_med: percentile_by(&areas, |a| a.integer_sqrt(), 0.5),
            area_p90: percentile_by(&areas, |a| a.integer_sqrt(), 0.9),
            area_max: areas.iter().map(|a| a.integer_sqrt()).max().unwrap_or(0),
            channelized_fraction: if !areas.is_empty() { channelized_count as f64 / areas.len() as f64 } else { 0.0 },
            confluence_count,
            incision_min: incisions.iter().copied().min().unwrap_or(0),
            incision_med: percentile(&incisions, 0.5),
            incision_p90: percentile(&incisions, 0.9),
            incision_max: incisions.iter().copied().max().unwrap_or(0),
            incision_zero_fraction: if !incisions.is_empty() { incisions.iter().filter(|&&x| x == 0).count() as f64 / incisions.len() as f64 } else { 0.0 },
            talus_min: taluses.iter().copied().min().unwrap_or(0),
            talus_med: percentile(&taluses, 0.5),
            talus_p90: percentile(&taluses, 0.9),
            talus_max: taluses.iter().copied().max().unwrap_or(0),
            talus_zero_fraction: if !taluses.is_empty() { taluses.iter().filter(|&&x| x == 0).count() as f64 / taluses.len() as f64 } else { 0.0 },
            net_height_med: percentile(&net_deltas, 0.5),
            net_height_max: net_deltas.iter().copied().max().unwrap_or(0),
        };

        self.iterations.push(stats);
    }
}

fn percentile(values: &[i64], p: f64) -> i64 {
    if values.is_empty() {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort();
    let idx = ((sorted.len() as f64 - 1.0) * p).ceil() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn percentile_by<T: Copy, F: Fn(T) -> i64>(values: &[T], f: F, p: f64) -> i64 {
    let mapped: Vec<i64> = values.iter().map(|&v| f(v)).collect();
    percentile(&mapped, p)
}

trait IntegerSqrt {
    fn integer_sqrt(self) -> i64;
}

impl IntegerSqrt for i64 {
    fn integer_sqrt(self) -> i64 {
        if self <= 0 {
            return 0;
        }
        let mut x = self;
        let mut y = (x + 1) / 2;
        while y < x {
            x = y;
            y = (x + self / x) / 2;
        }
        x
    }
}
