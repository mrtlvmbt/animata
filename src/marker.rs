//! Stigmergic marker field: a small stack of decaying, diffusing scent grids the
//! creatures emit into (brain-gated) and sense from (via evolved receptor organs).
//!
//! There are `N_MARKER_CHANNELS` independent channels per vertical layer. The
//! engine assigns **no meaning** to any channel — a channel only comes to mean
//! "food here" / "predator!" / a display / a lure if a lineage evolves to emit it
//! in that context and others evolve a receptor tuned to it. The field is the
//! substrate on which such signalling can self-organise (or collapse to noise).

use crate::config::*;
use macroquad::math::Vec2;

/// `data` is `N_LAYERS * N_MARKER_CHANNELS` planes of `cols*rows` cells, laid out
/// plane-major: index = ((layer*N_MARKER_CHANNELS + ch) * rows + y) * cols + x.
#[derive(Default)]
pub struct MarkerField {
    cols: i32,
    rows: i32,
    cell: f32,
    data: Vec<f32>,
    scratch: Vec<f32>,
}

impl MarkerField {
    pub fn new(width: f32, height: f32, cell: f32) -> Self {
        let cols = ((width / cell).ceil() as i32).max(1);
        let rows = ((height / cell).ceil() as i32).max(1);
        let n = (N_LAYERS * N_MARKER_CHANNELS) * (cols * rows) as usize;
        MarkerField { cols, rows, cell, data: vec![0.0; n], scratch: vec![0.0; n] }
    }

    pub fn cols(&self) -> i32 {
        self.cols
    }
    pub fn rows(&self) -> i32 {
        self.rows
    }
    pub fn cell(&self) -> f32 {
        self.cell
    }

    fn cell_of(&self, p: Vec2) -> (i32, i32) {
        let x = (p.x / self.cell) as i32;
        let y = (p.y / self.cell) as i32;
        (x.clamp(0, self.cols - 1), y.clamp(0, self.rows - 1))
    }

    fn plane_base(&self, layer: u8, ch: usize) -> usize {
        (layer as usize * N_MARKER_CHANNELS + ch) * (self.cols * self.rows) as usize
    }

    /// Value of channel `ch` on `layer` at the grid cell `(x, y)` (for the overlay
    /// and instrumentation).
    pub fn at(&self, layer: u8, ch: usize, x: i32, y: i32) -> f32 {
        self.data[self.plane_base(layer, ch) + (y * self.cols + x) as usize]
    }

    /// Channel `ch` of `layer` sampled at a world position (what a receptor reads).
    pub fn sample(&self, layer: u8, p: Vec2, ch: usize) -> f32 {
        let (x, y) = self.cell_of(p);
        self.data[self.plane_base(layer, ch) + (y * self.cols + x) as usize]
    }

    /// Add a creature's per-channel emission into its cell on `layer`.
    pub fn deposit(&mut self, layer: u8, p: Vec2, emit: &[f32; N_MARKER_CHANNELS]) {
        let (x, y) = self.cell_of(p);
        let off = (y * self.cols + x) as usize;
        for (ch, &e) in emit.iter().enumerate() {
            if e > 0.0 {
                let i = self.plane_base(layer, ch) + off;
                self.data[i] += e * MARKER_DEPOSIT;
            }
        }
    }

    /// Fade every cell (markers are transient).
    pub fn decay(&mut self) {
        for v in &mut self.data {
            *v *= MARKER_DECAY;
        }
    }

    /// Light box-blur per plane, so deposits spread into climbable gradients.
    pub fn diffuse(&mut self) {
        if MARKER_DIFFUSE <= 0.0 {
            return;
        }
        self.scratch.copy_from_slice(&self.data);
        let (cols, rows) = (self.cols, self.rows);
        let plane = (cols * rows) as usize;
        for pbase in (0..self.data.len()).step_by(plane) {
            for y in 0..rows {
                for x in 0..cols {
                    let i = pbase + (y * cols + x) as usize;
                    let c = self.scratch[i];
                    let mut sum = 0.0;
                    let mut cnt = 0.0;
                    for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                        let (nx, ny) = (x + dx, y + dy);
                        if nx >= 0 && nx < cols && ny >= 0 && ny < rows {
                            sum += self.scratch[pbase + (ny * cols + nx) as usize];
                            cnt += 1.0;
                        }
                    }
                    let avg = if cnt > 0.0 { sum / cnt } else { c };
                    self.data[i] = c * (1.0 - MARKER_DIFFUSE) + avg * MARKER_DIFFUSE;
                }
            }
        }
    }
}
