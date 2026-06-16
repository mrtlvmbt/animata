//! Procedural biome map: a seeded value-noise "fertility" field over the world,
//! classified into biomes that bundle food density, movement cost, metabolism
//! and a display tint.

use crate::config::*;
use macroquad::math::Vec2;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Biome {
    Desert,
    Plains,
    Forest,
    Swamp,
    /// Impassable-ish river/lake: no food, very sluggish, harsh — a barrier that
    /// isolates regions and drives allopatric divergence.
    Water,
}

/// What a biome does to creatures and food in it.
pub struct BiomeProps {
    /// Relative food spawn density (vs the richest biome).
    pub food_mult: f32,
    /// Movement-distance multiplier (<1 == sluggish terrain).
    pub move_mult: f32,
    /// Metabolic upkeep multiplier (>1 == harsher climate).
    pub metab_mult: f32,
    /// The food "flavor" (0..1 niche axis) this biome grows. Distinct per biome
    /// so adapting to one biome's food means poorly digesting another's.
    pub flavor: f32,
    /// Background tint (r, g, b), drawn semi-transparently.
    pub tint: (f32, f32, f32),
}

impl Biome {
    pub fn props(self) -> BiomeProps {
        match self {
            Biome::Desert => BiomeProps {
                food_mult: 0.25,
                move_mult: 1.0,
                metab_mult: 1.4,
                flavor: 0.12,
                tint: (0.55, 0.48, 0.28),
            },
            Biome::Plains => BiomeProps {
                food_mult: 1.0,
                move_mult: 1.0,
                metab_mult: 1.0,
                flavor: 0.38,
                tint: (0.30, 0.40, 0.24),
            },
            Biome::Forest => BiomeProps {
                food_mult: 1.8,
                move_mult: 1.0,
                metab_mult: 1.0,
                flavor: 0.64,
                tint: (0.16, 0.40, 0.22),
            },
            Biome::Swamp => BiomeProps {
                food_mult: 1.4,
                move_mult: 0.62,
                metab_mult: 1.1,
                flavor: 0.88,
                tint: (0.18, 0.34, 0.40),
            },
            Biome::Water => BiomeProps {
                food_mult: 0.0,
                move_mult: WATER_MOVE_MULT,
                metab_mult: WATER_METAB_MULT,
                flavor: 0.5,
                tint: (0.10, 0.22, 0.45),
            },
        }
    }
}

/// Seeded fertility field + biome classification over the world.
/// Cache cell size: the biome is classified once per cell, then looked up O(1)
/// (the value-noise is far too costly to recompute per creature per step).
const BIOME_CELL: f32 = 16.0;

pub struct BiomeMap {
    pub seed: u64,
    cols: i32,
    rows: i32,
    /// Precomputed biome class per cell (row-major), filled in `new`.
    grid: Vec<Biome>,
}

impl BiomeMap {
    pub fn new(seed: u64) -> Self {
        let cols = (WORLD_W / BIOME_CELL).ceil() as i32;
        let rows = (WORLD_H / BIOME_CELL).ceil() as i32;
        let mut m = BiomeMap { seed, cols, rows, grid: Vec::new() };
        m.grid = (0..cols * rows)
            .map(|idx| {
                let gx = idx % cols;
                let gy = idx / cols;
                let p = Vec2::new(
                    gx as f32 * BIOME_CELL + BIOME_CELL * 0.5,
                    gy as f32 * BIOME_CELL + BIOME_CELL * 0.5,
                );
                m.classify(p)
            })
            .collect();
        m
    }

    /// Fertility in `0..=1` at a world point (smooth value noise, 2 octaves).
    pub fn fertility(&self, p: Vec2) -> f32 {
        let base = self.value_noise(p.x / BIOME_LATTICE, p.y / BIOME_LATTICE, 0);
        let detail = self.value_noise(p.x / (BIOME_LATTICE * 0.5), p.y / (BIOME_LATTICE * 0.5), 1);
        (base * 0.7 + detail * 0.3).clamp(0.0, 1.0)
    }

    /// River field: a separate low-frequency noise. The contour band around 0.5
    /// forms connected curves, so thresholding it carves rivers across the map.
    fn river(&self, p: Vec2) -> f32 {
        self.value_noise(p.x / BARRIER_LATTICE, p.y / BARRIER_LATTICE, 2)
    }

    /// Classify a point from live fertility (used only while building the cache).
    fn classify(&self, p: Vec2) -> Biome {
        // Rivers override everything: the contour band of the river noise.
        if (self.river(p) - 0.5).abs() < BARRIER_BAND {
            return Biome::Water;
        }
        let f = self.fertility(p);
        let [a, b, c] = BIOME_THRESHOLDS;
        if f < a {
            Biome::Desert
        } else if f < b {
            Biome::Plains
        } else if f < c {
            Biome::Forest
        } else {
            Biome::Swamp
        }
    }

    /// O(1) biome lookup via the precomputed cell grid.
    pub fn at(&self, p: Vec2) -> Biome {
        let gx = ((p.x / BIOME_CELL) as i32).clamp(0, self.cols - 1);
        let gy = ((p.y / BIOME_CELL) as i32).clamp(0, self.rows - 1);
        self.grid[(gy * self.cols + gx) as usize]
    }

    pub fn props_at(&self, p: Vec2) -> BiomeProps {
        self.at(p).props()
    }

    /// Bilinear value noise with smoothstep weights over an integer lattice.
    fn value_noise(&self, x: f32, y: f32, octave: u64) -> f32 {
        let x0 = x.floor() as i64;
        let y0 = y.floor() as i64;
        let tx = smoothstep(x - x0 as f32);
        let ty = smoothstep(y - y0 as f32);

        let c00 = self.lattice(x0, y0, octave);
        let c10 = self.lattice(x0 + 1, y0, octave);
        let c01 = self.lattice(x0, y0 + 1, octave);
        let c11 = self.lattice(x0 + 1, y0 + 1, octave);

        let top = lerp(c00, c10, tx);
        let bottom = lerp(c01, c11, tx);
        lerp(top, bottom, ty)
    }

    /// Deterministic pseudo-random value in `0..=1` at a lattice node.
    fn lattice(&self, x: i64, y: i64, octave: u64) -> f32 {
        let mut h = self.seed
            ^ (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
            ^ octave.wrapping_mul(0x1656_67B1_9E37_79F9);
        // xorshift-style mixing.
        h ^= h >> 33;
        h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
        h ^= h >> 33;
        (h >> 40) as f32 / (1u64 << 24) as f32
    }
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{WORLD_H, WORLD_W};

    #[test]
    fn fertility_in_range_and_deterministic() {
        let m = BiomeMap::new(42);
        let m2 = BiomeMap::new(42);
        for i in 0..400 {
            let p = Vec2::new(
                (i as f32 * 17.3) % WORLD_W,
                (i as f32 * 11.7) % WORLD_H,
            );
            let f = m.fertility(p);
            assert!((0.0..=1.0).contains(&f));
            assert_eq!(f, m2.fertility(p), "fertility must be deterministic by seed");
        }
    }

    #[test]
    fn rivers_carve_water_and_partition_land() {
        // Across seeds, rivers should cover a modest fraction of the map and
        // (usually) split the land into more than one region — the geographic
        // isolation that drives allopatric divergence.
        let mut split_seeds = 0;
        for seed in 0..12u64 {
            let m = BiomeMap::new(seed);
            let (cols, rows) = (m.cols, m.rows);
            let n = (cols * rows) as usize;
            let water: Vec<bool> = m.grid.iter().map(|&b| b == Biome::Water).collect();
            let wfrac = water.iter().filter(|&&w| w).count() as f32 / n as f32;
            assert!(
                (0.02..0.35).contains(&wfrac),
                "seed {seed}: water fraction {wfrac:.3} out of sane band"
            );
            // Flood-fill 4-connected land components; count those >2% of the map.
            let mut comp = vec![-1i32; n];
            let mut sizes = Vec::new();
            for start in 0..n {
                if water[start] || comp[start] >= 0 {
                    continue;
                }
                let id = sizes.len() as i32;
                let mut stack = vec![start];
                comp[start] = id;
                let mut size = 0usize;
                while let Some(c) = stack.pop() {
                    size += 1;
                    let (cx, cy) = ((c as i32 % cols), (c as i32 / cols));
                    for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                        let (nx, ny) = (cx + dx, cy + dy);
                        if nx < 0 || ny < 0 || nx >= cols || ny >= rows {
                            continue;
                        }
                        let ni = (ny * cols + nx) as usize;
                        if !water[ni] && comp[ni] < 0 {
                            comp[ni] = id;
                            stack.push(ni);
                        }
                    }
                }
                sizes.push(size);
            }
            let big = sizes.iter().filter(|&&s| s as f32 / n as f32 > 0.02).count();
            if big >= 2 {
                split_seeds += 1;
            }
        }
        assert!(
            split_seeds >= 6,
            "rivers should partition the land on most seeds (got {split_seeds}/12)"
        );
    }

    #[test]
    fn map_contains_more_than_one_biome() {
        let m = BiomeMap::new(7);
        let mut seen = std::collections::HashSet::new();
        let step = 24.0;
        let mut y = 0.0;
        while y < WORLD_H {
            let mut x = 0.0;
            while x < WORLD_W {
                seen.insert(m.at(Vec2::new(x, y)));
                x += step;
            }
            y += step;
        }
        assert!(seen.len() >= 2, "world should contain multiple biomes");
    }
}
