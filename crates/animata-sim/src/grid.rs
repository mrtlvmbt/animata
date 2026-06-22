//! Uniform spatial grid over the world — find nearby creatures (prey / threats) without an
//! O(N²) scan. Rebuilt once per tick from the creature positions.
//!
//! Layout: a **counting-sort cell list** (flat arrays), not a `Vec<Vec>`. `rebuild` buckets the
//! points by cell with a prefix-sum, so each cell owns a contiguous run of `sorted_idx` AND a
//! contiguous run of `sorted_pos` (positions reordered to match). The distance loop in
//! `nearest2_within` then reads positions sequentially instead of chasing `points[idx]` all over
//! the heap — the cache win over the old nested `Vec<Vec<usize>>`. Points are scattered in
//! ascending original index within each cell (the same order the old `push` produced), so every
//! query result is **bit-identical** to the previous structure (the determinism golden holds).
//!
//! Queries use plain (non-wrapped) distance: creatures near the map edge don't sense across it.

use glam::Vec2;

#[derive(Default)]
pub struct SpatialGrid {
    cols: i32,
    rows: i32,
    cell: f32,
    /// Prefix sums: cell `c` owns `sorted_idx[cell_start[c]..cell_start[c + 1]]`. Len = ncells + 1.
    cell_start: Vec<u32>,
    /// Point indices, grouped by cell (ascending original index within a cell — matches the old
    /// push order, so queries stay bit-identical). Len = n.
    sorted_idx: Vec<u32>,
    /// Positions in the same cell-grouped order as `sorted_idx`, so the distance loop reads
    /// contiguous memory instead of chasing `points[idx]`. Len = n.
    sorted_pos: Vec<Vec2>,
    /// Per-cell write cursor, reused across rebuilds (counting-sort scratch). Len = ncells.
    cursor: Vec<u32>,
}

impl SpatialGrid {
    /// Refill the grid for a new point set via counting sort, reusing the buffers (resize, no fresh
    /// heap allocation once warmed up).
    pub fn rebuild(&mut self, points: &[Vec2], width: f32, height: f32, cell: f32) {
        self.cols = ((width / cell).ceil() as i32).max(1);
        self.rows = ((height / cell).ceil() as i32).max(1);
        self.cell = cell;
        let ncells = (self.cols * self.rows) as usize;
        let n = points.len();
        // 1. Count points per cell (reuse `cursor` as the count buffer).
        self.cursor.clear();
        self.cursor.resize(ncells, 0);
        for &p in points {
            let c = self.cell_index(p);
            self.cursor[c] += 1;
        }
        // 2. Prefix-sum into `cell_start`; leave `cursor[c]` = cell c's first slot (the write cursor).
        self.cell_start.clear();
        self.cell_start.resize(ncells + 1, 0);
        let mut acc = 0u32;
        for c in 0..ncells {
            let cnt = self.cursor[c];
            self.cell_start[c] = acc;
            self.cursor[c] = acc;
            acc += cnt;
        }
        self.cell_start[ncells] = acc;
        // 3. Scatter in ascending point index ⇒ ascending-within-cell order (bit-identical to the
        //    old `cells[c].push(i)`), recording both the index and the position for cache-friendly reads.
        self.sorted_idx.clear();
        self.sorted_idx.resize(n, 0);
        self.sorted_pos.clear();
        self.sorted_pos.resize(n, Vec2::ZERO);
        for (i, &p) in points.iter().enumerate() {
            let c = self.cell_index(p);
            let slot = self.cursor[c] as usize;
            self.cursor[c] += 1;
            self.sorted_idx[slot] = i as u32;
            self.sorted_pos[slot] = p;
        }
    }

    /// Number of cells in the current grid (`cols * rows`), valid after `rebuild`. The index space
    /// for `sum_in_radius`'s `counts` and `cell_index`.
    pub fn num_cells(&self) -> usize {
        self.cell_start.len().saturating_sub(1)
    }

    /// Linear cell index `cy*cols + cx` for a position — same indexing as the internal layout and
    /// as `sum_in_radius`. For building a per-cell occupancy/count parallel to the grid.
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
                    let c = (gy * self.cols + gx) as usize;
                    let (s, e) = (self.cell_start[c] as usize, self.cell_start[c + 1] as usize);
                    for k in s..e {
                        let d2 = (self.sorted_pos[k] - from).length_squared();
                        if d2 > max_dist2 {
                            continue;
                        }
                        let idx = self.sorted_idx[k] as usize;
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
