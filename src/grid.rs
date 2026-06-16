//! Uniform spatial grid over the world, used to find nearby food without
//! scanning every pellet. Rebuilt once per step from the food list; cells hold
//! indices into that list.
//!
//! Queries use plain (non-wrapped) distance, matching the rest of the sim:
//! creatures near an edge don't sense across the toroidal seam.

use macroquad::math::Vec2;

#[derive(Default)]
pub struct SpatialGrid {
    cols: i32,
    rows: i32,
    cell: f32,
    /// `cells[y * cols + x]` -> indices into the points slice.
    cells: Vec<Vec<usize>>,
}

impl SpatialGrid {
    pub fn build(points: &[Vec2], width: f32, height: f32, cell: f32) -> Self {
        let mut grid = SpatialGrid::default();
        grid.rebuild(points, width, height, cell);
        grid
    }

    /// Refill the grid for a new point set, reusing existing cell allocations
    /// (clear-and-push) so the per-step grid build does no fresh heap allocation.
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

    fn cell_of(&self, p: Vec2) -> (i32, i32) {
        let cx = (p.x / self.cell) as i32;
        let cy = (p.y / self.cell) as i32;
        (cx.clamp(0, self.cols - 1), cy.clamp(0, self.rows - 1))
    }

    /// Index of the point nearest to `from`, searched ring-by-ring outward and
    /// stopped as soon as no further ring could hold anything closer.
    #[allow(dead_code)] // convenience wrapper; the sim always bounds by radius
    pub fn nearest(&self, points: &[Vec2], from: Vec2) -> Option<usize> {
        self.nearest_where(points, from, |_| true)
    }

    /// Like [`SpatialGrid::nearest`], but only considers points for which
    /// `ok(index)` is true (e.g. "not myself", "still unmated").
    #[allow(dead_code)] // unbounded variant; the sim uses `nearest_within`
    pub fn nearest_where(
        &self,
        points: &[Vec2],
        from: Vec2,
        ok: impl Fn(usize) -> bool,
    ) -> Option<usize> {
        self.nearest_within(points, from, f32::INFINITY, ok)
    }

    /// Nearest matching point within `max_dist`, or `None`. Bounding the radius is
    /// the key optimization: a sparse predicate (e.g. "a predator", when none is
    /// near) otherwise expands rings across the whole grid. With a bound the
    /// search stops after `max_dist / cell` rings — local, not global.
    pub fn nearest_within(
        &self,
        points: &[Vec2],
        from: Vec2,
        max_dist: f32,
        ok: impl Fn(usize) -> bool,
    ) -> Option<usize> {
        let (cx, cy) = self.cell_of(from);
        let max_ring = self.cols.max(self.rows);
        let max_dist2 = max_dist * max_dist;
        let mut best: Option<(usize, f32)> = None;

        for ring in 0..=max_ring {
            // Closest a point in this (or any further) ring could be to `from`.
            let ring_min = (ring as f32 - 1.0) * self.cell;
            // Nothing reachable within the radius any more -> stop.
            if ring_min > max_dist {
                break;
            }
            // If our best is already closer than this ring's minimum -> stop.
            if let Some((_, bd2)) = best {
                if ring_min > 0.0 && ring_min * ring_min > bd2 {
                    break;
                }
            }
            for gy in (cy - ring)..=(cy + ring) {
                for gx in (cx - ring)..=(cx + ring) {
                    // Only the border of the ring (interior already scanned).
                    if (gx - cx).abs() != ring && (gy - cy).abs() != ring {
                        continue;
                    }
                    if gx < 0 || gy < 0 || gx >= self.cols || gy >= self.rows {
                        continue;
                    }
                    for &idx in &self.cells[(gy * self.cols + gx) as usize] {
                        if !ok(idx) {
                            continue;
                        }
                        let d2 = (points[idx] - from).length_squared();
                        if d2 <= max_dist2 && best.map_or(true, |(_, bd2)| d2 < bd2) {
                            best = Some((idx, d2));
                        }
                    }
                }
            }
        }
        best.map(|(idx, _)| idx)
    }

    /// Nearest matching point for *two* predicates in a single ring-bounded
    /// traversal (shares cell visits and distance math). Used for the threat and
    /// neighbour queries, which both scan the same creature grid every step.
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
            // Stop once neither best can still improve.
            let settled = |b: Option<(usize, f32)>| {
                b.map_or(false, |(_, d2)| ring_min > 0.0 && ring_min * ring_min > d2)
            };
            if settled(ba) && settled(bb) {
                break;
            }
            for gy in (cy - ring)..=(cy + ring) {
                for gx in (cx - ring)..=(cx + ring) {
                    if (gx - cx).abs() != ring && (gy - cy).abs() != ring {
                        continue;
                    }
                    if gx < 0 || gy < 0 || gx >= self.cols || gy >= self.rows {
                        continue;
                    }
                    for &idx in &self.cells[(gy * self.cols + gx) as usize] {
                        let d2 = (points[idx] - from).length_squared();
                        if d2 > max_dist2 {
                            continue;
                        }
                        if ba.map_or(true, |(_, bd)| d2 < bd) && ok_a(idx) {
                            ba = Some((idx, d2));
                        }
                        if bb.map_or(true, |(_, bd)| d2 < bd) && ok_b(idx) {
                            bb = Some((idx, d2));
                        }
                    }
                }
            }
        }
        (ba.map(|(i, _)| i), bb.map(|(i, _)| i))
    }

    /// Call `f` with the index of every point in the 3x3 block of cells around
    /// `from`. Valid for queries whose radius is smaller than the cell size.
    pub fn for_each_near(&self, from: Vec2, mut f: impl FnMut(usize)) {
        let (cx, cy) = self.cell_of(from);
        for gy in (cy - 1)..=(cy + 1) {
            for gx in (cx - 1)..=(cx + 1) {
                if gx < 0 || gy < 0 || gx >= self.cols || gy >= self.rows {
                    continue;
                }
                for &idx in &self.cells[(gy * self.cols + gx) as usize] {
                    f(idx);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use macroquad::rand::{gen_range, srand};

    fn brute_nearest(points: &[Vec2], from: Vec2) -> Option<usize> {
        points
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                (**a - from)
                    .length_squared()
                    .total_cmp(&(**b - from).length_squared())
            })
            .map(|(i, _)| i)
    }

    #[test]
    fn nearest_matches_brute_force() {
        srand(99);
        let w = 1000.0;
        let h = 700.0;
        let points: Vec<Vec2> = (0..500)
            .map(|_| Vec2::new(gen_range(0.0, w), gen_range(0.0, h)))
            .collect();
        let grid = SpatialGrid::build(&points, w, h, 64.0);
        for _ in 0..200 {
            let from = Vec2::new(gen_range(0.0, w), gen_range(0.0, h));
            let g = grid.nearest(&points, from);
            let b = brute_nearest(&points, from);
            // Compare by distance (ties may pick different equal-distance points).
            let gd = g.map(|i| (points[i] - from).length_squared());
            let bd = b.map(|i| (points[i] - from).length_squared());
            assert_eq!(gd, bd, "grid nearest disagrees with brute force");
        }
    }

    #[test]
    fn nearest_on_empty_is_none() {
        let grid = SpatialGrid::build(&[], 100.0, 100.0, 32.0);
        assert_eq!(grid.nearest(&[], Vec2::ZERO), None);
    }

    #[test]
    fn nearest_within_matches_brute_force_inside_radius_and_none_outside() {
        srand(123);
        let w = 1000.0;
        let h = 700.0;
        let points: Vec<Vec2> = (0..400)
            .map(|_| Vec2::new(gen_range(0.0, w), gen_range(0.0, h)))
            .collect();
        let grid = SpatialGrid::build(&points, w, h, 64.0);
        for _ in 0..200 {
            let from = Vec2::new(gen_range(0.0, w), gen_range(0.0, h));
            let radius = gen_range(20.0, 250.0);
            let got = grid.nearest_within(&points, from, radius, |_| true);
            // Brute-force nearest restricted to the radius.
            let brute = points
                .iter()
                .enumerate()
                .map(|(i, p)| (i, (*p - from).length_squared()))
                .filter(|(_, d2)| *d2 <= radius * radius)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            match (got, brute) {
                (Some(g), Some((_, bd2))) => {
                    assert!((points[g] - from).length_squared() <= radius * radius);
                    assert_eq!((points[g] - from).length_squared(), bd2);
                }
                (None, None) => {}
                _ => panic!("nearest_within disagrees with bounded brute force"),
            }
        }
    }
}
