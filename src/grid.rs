//! Uniform spatial grid over the world — find nearby creatures (prey / threats) without an
//! O(N²) scan. Rebuilt once per tick from the creature positions; cells hold indices into that
//! slice. Ported from the archived sim (`sim-v1/grid.rs`), trimmed to what C2 predation needs.
//!
//! Queries use plain (non-wrapped) distance: creatures near the map edge don't sense across it.

use macroquad::math::Vec2;

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;
    use macroquad::math::vec2;

    fn brute_nearest(points: &[Vec2], from: Vec2, max: f32, ok: impl Fn(usize) -> bool) -> Option<usize> {
        points
            .iter()
            .enumerate()
            .filter(|&(i, &p)| ok(i) && (p - from).length() <= max)
            .min_by(|(_, a), (_, b)| {
                (**a - from).length_squared().partial_cmp(&(**b - from).length_squared()).unwrap()
            })
            .map(|(i, _)| i)
    }

    #[test]
    fn nearest_within_matches_brute_force() {
        let mut rng = Rng::new(1);
        let pts: Vec<Vec2> = (0..500).map(|_| vec2(rng.unit() * 200.0, rng.unit() * 200.0)).collect();
        let mut g = SpatialGrid::default();
        g.rebuild(&pts, 200.0, 200.0, 12.0);
        for _ in 0..200 {
            let from = vec2(rng.unit() * 200.0, rng.unit() * 200.0);
            let r = rng.unit() * 60.0;
            let (got, _) = g.nearest2_within(&pts, from, r, |_| true, |_| false);
            let want = brute_nearest(&pts, from, r, |_| true);
            // Same point, or (ties aside) the same distance.
            match (got, want) {
                (Some(a), Some(b)) => {
                    let (da, db) = ((pts[a] - from).length(), (pts[b] - from).length());
                    assert!((da - db).abs() < 1e-3, "grid {da} vs brute {db}");
                }
                (None, None) => {}
                _ => panic!("grid {got:?} vs brute {want:?}"),
            }
        }
    }

    #[test]
    fn nearest2_respects_both_predicates() {
        let pts = vec![vec2(0.0, 0.0), vec2(5.0, 0.0), vec2(10.0, 0.0), vec2(50.0, 0.0)];
        let mut g = SpatialGrid::default();
        g.rebuild(&pts, 100.0, 100.0, 8.0);
        // From the origin point: nearest "even index" and nearest "odd index" within 20.
        let (a, b) = g.nearest2_within(&pts, vec2(0.0, 0.0), 20.0, |i| i % 2 == 0 && i != 0, |i| i % 2 == 1);
        assert_eq!(a, Some(2)); // even, index 2 at x=10
        assert_eq!(b, Some(1)); // odd, index 1 at x=5
    }
}
