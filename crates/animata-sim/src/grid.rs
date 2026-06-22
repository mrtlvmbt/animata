//! Uniform spatial grid over the world — find nearby creatures (prey / threats) without an
//! O(N²) scan. Rebuilt once per tick from the creature positions; cells hold indices into that
//! slice. Ported from the archived sim (`sim-v1/grid.rs`), trimmed to what C2 predation needs.
//!
//! Queries use plain (non-wrapped) distance: creatures near the map edge don't sense across it.

use glam::Vec2;

#[derive(Default)]
pub struct SpatialGrid {
    cols: i32,
    rows: i32,
    cell: f32,
    /// `cells[y*cols + x]` → indices into the points slice.
    cells: Vec<Vec<usize>>,
}

impl SpatialGrid {
    /// Refill the grid for a new point set, reusing cell allocations (clear-and-push) so the
    /// per-tick rebuild does no fresh heap allocation once warmed up.
    pub fn rebuild(&mut self, points: &[Vec2], width: f32, height: f32, cell: f32) {
        self.cols = ((width / cell).ceil() as i32).max(1);
        self.rows = ((height / cell).ceil() as i32).max(1);
        self.cell = cell;
        let n = (self.cols * self.rows) as usize;
        self.cells.resize_with(n, Vec::new);
        for c in &mut self.cells {
            c.clear();
        }
        for (i, &p) in points.iter().enumerate() {
            let (cx, cy) = self.cell_of(p);
            self.cells[(cy * self.cols + cx) as usize].push(i);
        }
    }

    /// Number of cells in the current grid (`cols * rows`), valid after `rebuild`. The index space
    /// for `sum_in_radius`'s `counts` and `cell_index`.
    pub fn num_cells(&self) -> usize {
        self.cells.len()
    }

    /// Linear cell index `cy*cols + cx` for a position — same indexing as the internal `cells`
    /// and as `sum_in_radius`. For building a per-cell occupancy/count parallel to the grid.
    pub fn cell_index(&self, p: Vec2) -> usize {
        let (cx, cy) = self.cell_of(p);
        (cy * self.cols + cx) as usize
    }

    fn cell_of(&self, p: Vec2) -> (i32, i32) {
        let cx = (p.x / self.cell) as i32;
        let cy = (p.y / self.cell) as i32;
        (cx.clamp(0, self.cols - 1), cy.clamp(0, self.rows - 1))
    }

    /// Nearest matching point for *two* predicates in one ring-bounded traversal (shares the
    /// cell visits + distance math) — e.g. nearest prey AND nearest threat in a single pass.
    /// Searched ring-by-ring outward and stopped as soon as no further ring could hold anything
    /// closer; bounding the radius keeps a sparse predicate (no prey near) local, not global.
    pub fn nearest2_within(
        &self,
        points: &[Vec2],
        from: Vec2,
        max_dist: f32,
        ok_a: impl Fn(usize) -> bool,
        ok_b: impl Fn(usize) -> bool,
    ) -> (Option<usize>, Option<usize>) {
        let (cx, cy) = self.cell_of(from);
        let max_ring = self.cols.max(self.rows);
        let max_dist2 = max_dist * max_dist;
        let mut ba: Option<(usize, f32)> = None;
        let mut bb: Option<(usize, f32)> = None;

        for ring in 0..=max_ring {
            let ring_min = (ring as f32 - 1.0) * self.cell;
            if ring_min > max_dist {
                break;
            }
            let settled = |b: Option<(usize, f32)>| {
                b.is_some_and(|(_, d2)| ring_min > 0.0 && ring_min * ring_min > d2)
            };
            if settled(ba) && settled(bb) {
                break;
            }
            for gy in (cy - ring)..=(cy + ring) {
                for gx in (cx - ring)..=(cx + ring) {
                    if (gx - cx).abs() != ring && (gy - cy).abs() != ring {
                        continue; // only the border of this ring (interior already scanned)
                    }
                    if gx < 0 || gy < 0 || gx >= self.cols || gy >= self.rows {
                        continue;
                    }
                    for &idx in &self.cells[(gy * self.cols + gx) as usize] {
                        let d2 = (points[idx] - from).length_squared();
                        if d2 > max_dist2 {
                            continue;
                        }
                        if ba.is_none_or(|(_, bd)| d2 < bd) && ok_a(idx) {
                            ba = Some((idx, d2));
                        }
                        if bb.is_none_or(|(_, bd)| d2 < bd) && ok_b(idx) {
                            bb = Some((idx, d2));
                        }
                    }
                }
            }
        }
        (ba.map(|(i, _)| i), bb.map(|(i, _)| i))
    }

    /// Sum `counts[cell]` over EVERY grid cell that `nearest2_within` could visit for the same
    /// `from`/`max_dist` — i.e. the exact ring double-loop of `nearest2_within`, but WITHOUT the
    /// `settled` early-exit and WITHOUT the per-candidate inner scan. Visiting every ring up to
    /// `ring_min > max_dist` makes this cell set a guaranteed SUPERSET of whatever `nearest2_within`
    /// actually touches (it may exit early via `settled`), so:
    ///   `sum == 0` ⟹ no counted entity lies in ANY cell the real scan visits ⟹ the real scan
    /// finds nothing. Used as a cheap conservative gate: a non-predator can skip the full threat
    /// scan iff the predator-count sum over its reach is 0 (byte-identical to running the scan,
    /// which would return `None`). Over-approximation (ignores per-candidate distance/biomass/strata
    /// filters) only ever costs a missed skip, never a wrong skip.
    pub fn sum_in_radius(&self, counts: &[u32], from: Vec2, max_dist: f32) -> u32 {
        let (cx, cy) = self.cell_of(from);
        let max_ring = self.cols.max(self.rows);
        let mut sum = 0u32;
        for ring in 0..=max_ring {
            let ring_min = (ring as f32 - 1.0) * self.cell;
            if ring_min > max_dist {
                break;
            }
            for gy in (cy - ring)..=(cy + ring) {
                for gx in (cx - ring)..=(cx + ring) {
                    if (gx - cx).abs() != ring && (gy - cy).abs() != ring {
                        continue; // only the border of this ring (interior already summed)
                    }
                    if gx < 0 || gy < 0 || gx >= self.cols || gy >= self.rows {
                        continue;
                    }
                    sum += counts[(gy * self.cols + gx) as usize];
                }
            }
        }
        sum
    }
}

#[cfg(test)]
#[path = "grid_tests.rs"]
mod tests;
