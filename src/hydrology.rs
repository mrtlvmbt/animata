//! Hydrology — rivers and lakes derived from the eroded heightmap, once per seed.
//!
//! - **Rivers**: D8 flow accumulation. Every column drains one unit of rain to its
//!   steepest-descent neighbour; processing high→low sums the drainage area passing
//!   through each column. Columns above a drainage threshold are rivers (the trunks of
//!   the dendritic network the erosion pass already carved).
//! - **Lakes**: priority-flood depression filling (Barnes). Flooding inward from the map
//!   border raises each column to the lowest pour point reachable — any column whose
//!   filled level sits above its terrain is underwater, i.e. a lake, and the filled level
//!   is the lake surface.
//!
//! Both feed the per-column water level the renderer floats a translucent plane at.

use crate::config::*;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Drainage area (columns) above which a land column reads as a river. Scales with the
/// map area so river density is similar at any `MAP_SCALE`.
const RIVER_THRESHOLD: f32 = 28.0 * (MAP_SCALE * MAP_SCALE) as f32;
/// Minimum fill depth (elevation units) to count a column as lake (ignore numeric dust).
const LAKE_EPS: f32 = 0.0040;

pub struct Hydrology {
    pub river: Vec<bool>,
    pub lake: Vec<bool>,
    /// Depression-filled surface; where `filled > elev` this is the lake water level.
    pub filled: Vec<f32>,
}

pub fn compute(elev: &[f32]) -> Hydrology {
    let n = COLS * ROWS;
    // Priority-flood the terrain: fills depressions AND records, for every column, the
    // neighbour it was flooded from (its drainage receiver) plus the pop order. Following
    // receivers always leads to the map border (acyclic), even across flat lake surfaces —
    // which a plain steepest-descent can't do, so without this rivers die in micro-pits.
    let (filled, receiver, order) = priority_flood(elev);
    // Flow accumulation: every column drains one unit of rain; summing upstream→downstream
    // (reverse pop order) gives the drainage area through each column.
    let mut accum = vec![1.0f32; n];
    for &iu in order.iter().rev() {
        let i = iu as usize;
        let r = receiver[i] as usize;
        if r != i {
            accum[r] += accum[i];
        }
    }
    let mut river = vec![false; n];
    let mut lake = vec![false; n];
    for i in 0..n {
        if filled[i] - elev[i] > LAKE_EPS {
            lake[i] = true;
        } else if accum[i] > RIVER_THRESHOLD {
            river[i] = true;
        }
    }
    Hydrology { river, lake, filled }
}

fn quant(e: f32) -> i64 {
    (e * 1_000_000.0) as i64
}

/// Priority-flood (Barnes): flood inward from the border, always expanding the lowest
/// frontier cell. Returns the depression-filled surface, each cell's drainage receiver
/// (the cell it was reached from; border cells point to themselves) and the pop order.
fn priority_flood(elev: &[f32]) -> (Vec<f32>, Vec<u32>, Vec<u32>) {
    let n = COLS * ROWS;
    let (w, h) = (COLS as i32, ROWS as i32);
    let mut water = vec![0f32; n];
    let mut receiver = vec![u32::MAX; n];
    let mut closed = vec![false; n];
    let mut order: Vec<u32> = Vec::with_capacity(n);
    let mut heap: BinaryHeap<(Reverse<i64>, u32)> = BinaryHeap::new();
    let seed = |i: usize, water: &mut [f32], receiver: &mut [u32], closed: &mut [bool], heap: &mut BinaryHeap<(Reverse<i64>, u32)>| {
        if !closed[i] {
            closed[i] = true;
            water[i] = elev[i];
            receiver[i] = i as u32; // border outlet
            heap.push((Reverse(quant(elev[i])), i as u32));
        }
    };
    for x in 0..w {
        seed(x as usize, &mut water, &mut receiver, &mut closed, &mut heap);
        seed(((h - 1) * w + x) as usize, &mut water, &mut receiver, &mut closed, &mut heap);
    }
    for y in 0..h {
        seed((y * w) as usize, &mut water, &mut receiver, &mut closed, &mut heap);
        seed((y * w + w - 1) as usize, &mut water, &mut receiver, &mut closed, &mut heap);
    }
    while let Some((_, iu)) = heap.pop() {
        let i = iu as usize;
        order.push(iu);
        let lvl = water[i];
        let (x, y) = ((i % COLS) as i32, (i / COLS) as i32);
        for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
            if nx < 0 || ny < 0 || nx >= w || ny >= h {
                continue;
            }
            let j = (ny * w + nx) as usize;
            if !closed[j] {
                closed[j] = true;
                water[j] = elev[j].max(lvl);
                receiver[j] = iu;
                heap.push((Reverse(quant(water[j])), j as u32));
            }
        }
    }
    (water, receiver, order)
}
