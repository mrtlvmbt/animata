//! W-3: deterministic integer drainage network — the third world-gen pipeline stage (RnD
//! `sim/world/09`, determinism clause `[drainage]`). **Pure integer / fixed-point throughout — no
//! `f32`/`f64` anywhere in this file** (enforced by the recursive glob guard,
//! `world/tests/no_float_guard_gen.rs`).
//!
//! **W-6 status:** the low-level functions ([`priority_flood_fill`], [`d8_directions`],
//! [`kahn_accumulate`]) are called by production — W-4's `erode` reuses them on the eroding
//! surface, wired into `ProcgenWorld` via `gen::caps::classify_and_caps`. [`compute_drainage`]
//! itself (the `height_at`-baked convenience wrapper) remains available but is not the production
//! call path (production always goes through the eroding-terrain reuse, not a static heightmap).
//!
//! ## W-3 is the phase's first GLOBAL FLOW stage (unlike W-1/W-2's per-position pure fns)
//!
//! Drainage-area accumulation depends on the ENTIRE upstream basin, so — unlike `height_at`/
//! `climate_at`/`biome_at`/`material_at` (independently re-derivable at any single position) —
//! this stage operates over a bounded `dim × dim` grid (origin at `(0,0)`, row-major
//! `z * dim + x` indexing). The functions here are generic over ANY heightmap input (critic
//! F5/F6): `priority_flood_fill`/`d8_directions`/`kahn_accumulate` all take a `height: &[i64]`
//! slice, so W-4 (erosion) can re-run them on its OWN evolving terrain each macro-iteration — W-3's
//! reusable value is the algorithm, not a cached instance tied to the static W-1 heightmap.
//! [`compute_drainage`] is the convenience wrapper that samples `height_at` and calls them.
//!
//! ## Algorithm (locked by the golden-vector tests, re-derivable from this doc)
//!
//! 1. **Priority-Flood pit-fill** ([`priority_flood_fill`]) — eliminates every INTERIOR closed
//!    depression so the filled surface has a non-increasing path from any interior cell to some
//!    border cell (Barnes et al. 2014). Seeded from all border cells (in a fixed deterministic
//!    row-major scan order — never `HashMap` iteration), processed by a min-priority-queue keyed by
//!    `(elevation, insertion_counter)` — the counter is a TOTAL tie-break so equal-elevation cells
//!    pop in a reproducible order regardless of any heap implementation detail.
//! 2. **D8 flow direction** ([`d8_directions`]) — computed on the FILLED surface. Each cell's
//!    downstream is the 8-connected neighbor with the SMALLEST `(elevation, linear_index)` key that
//!    is STRICTLY LESS than the cell's own key (`linear_index = z*dim+x` is unique per cell, so this
//!    tuple is a genuine TOTAL order — no ties survive at the combined-key level even on a perfectly
//!    flat plateau). **Acyclic BY CONSTRUCTION (critic F2.1/F8/F10):** every edge strictly decreases
//!    this key, so no path can ever return to a cell it already visited — a structural invariant,
//!    not a runtime-caught coincidence. A cell with no neighbor of strictly smaller key has no
//!    downstream (`None`) — it is a sink draining OFF-MAP to a virtual outlet (critic F7). Border
//!    cells, having fewer in-grid neighbors, are the ones Priority-Flood naturally routes to this
//!    role (their filled elevation IS the original height — the flood's seed condition).
//! 3. **Kahn topological flow-accumulation** ([`kahn_accumulate`]) — `area[v] = 1 (self) + Σ
//!    area[u]` over all `u` with `downstream[u] == v`, computed by classic Kahn's algorithm (an
//!    O(n) topological traversal, no recursion): in-degree = count of upstream cells flowing into
//!    each cell; a `VecDeque` FIFO seeded in row-major order with in-degree-0 cells (ridges); pop,
//!    propagate `area` to `downstream`, decrement its in-degree, enqueue when it hits 0. The sum is
//!    INTEGER and ASSOCIATIVE (commutative addition) — order-independent by construction. **Serial
//!    by nature (critic F1):** Priority-Flood's priority queue and Kahn's topological order are
//!    inherently sequential, so drainage output is thread-count-INDEPENDENT trivially (proven by the
//!    1-vs-N gate in `w3_chain.rs`, not forced/faked parallelism).
//!
//! **Acyclicity is checked in ALL builds, not `debug_assert!`-gated (critic requirement):** if Kahn's
//! traversal ever fails to process every cell (would indicate a cycle — structurally impossible
//! given step 2's strictly-decreasing-key invariant, but checked defensively), `kahn_accumulate`
//! panics via a real `assert!`.
//!
//! Rivers: cells whose accumulated area exceeds [`RIVER_THRESHOLD`] ([`is_river`]).

use crate::gen::height::height_at;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};

/// 8-connected neighbor offsets (dx, dz) — order is irrelevant to correctness (the D8 rule takes
/// a full argmin over all in-grid neighbors), only used for enumeration.
const D8_OFFSETS: [(i64, i64); 8] =
    [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];

/// Drainage area threshold above which a cell is classified as a river (implementer's call,
/// documented, locked by the golden-vector tests — RnD `sim/world/09`).
pub const RIVER_THRESHOLD: i64 = 32;

/// `true` when `area` exceeds [`RIVER_THRESHOLD`] — the cell is a river.
pub const fn is_river(area: i64) -> bool {
    area > RIVER_THRESHOLD
}

/// The full W-3 drainage output over a `dim × dim` grid, row-major `z*dim+x` indexing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrainageState {
    pub dim: usize,
    /// Post-Priority-Flood elevation (same integer scale as `height_at`'s `[0,hmax]`), depression-free.
    pub filled: Vec<i64>,
    /// D8 downstream target: `Some(index)` into this grid, or `None` for an off-map sink (critic F7).
    pub downstream: Vec<Option<usize>>,
    /// Integer, associative drainage area (self + all upstream cells) — a routing weight, not
    /// transported mass.
    pub area: Vec<i64>,
}

#[inline]
const fn linear_index(x: usize, z: usize, dim: usize) -> usize {
    z * dim + x
}

/// Priority-Flood pit-fill (Barnes et al. 2014): eliminate every interior closed depression in
/// `height` (row-major `z*dim+x`, `dim × dim`), returning a depression-free `filled` elevation.
/// Generic over ANY heightmap input (critic F5/F6) — W-4 re-runs this on its own eroding terrain.
pub fn priority_flood_fill(dim: usize, height: &[i64]) -> Vec<i64> {
    assert_eq!(height.len(), dim * dim, "height slice must have dim*dim elements");
    let n = dim * dim;
    let mut filled = vec![i64::MIN; n];
    let mut visited = vec![false; n];
    let mut heap: BinaryHeap<Reverse<(i64, u64, usize)>> = BinaryHeap::new();
    let mut counter: u64 = 0;

    // Seed all border cells in a fixed deterministic row-major scan order (never HashMap
    // iteration — critic R8/R10). Border = x∈{0,dim-1} or z∈{0,dim-1} (dim==1 is entirely border).
    for z in 0..dim {
        for x in 0..dim {
            let is_border = x == 0 || x == dim - 1 || z == 0 || z == dim - 1;
            if !is_border {
                continue;
            }
            let idx = linear_index(x, z, dim);
            filled[idx] = height[idx];
            visited[idx] = true;
            heap.push(Reverse((filled[idx], counter, idx)));
            counter += 1;
        }
    }

    while let Some(Reverse((_prio, _cnt, idx))) = heap.pop() {
        let z = idx / dim;
        let x = idx % dim;
        for &(dx, dz) in &D8_OFFSETS {
            let nx = x as i64 + dx;
            let nz = z as i64 + dz;
            if nx < 0 || nz < 0 || nx as usize >= dim || nz as usize >= dim {
                continue;
            }
            let nidx = linear_index(nx as usize, nz as usize, dim);
            if visited[nidx] {
                continue;
            }
            let new_filled = height[nidx].max(filled[idx]);
            filled[nidx] = new_filled;
            visited[nidx] = true;
            heap.push(Reverse((new_filled, counter, nidx)));
            counter += 1;
        }
    }

    filled
}

/// D8 flow direction on the FILLED surface: each cell's downstream is the 8-connected neighbor
/// with the smallest `(elevation, linear_index)` key strictly less than the cell's own key.
/// Acyclic BY CONSTRUCTION (see module doc). `None` = off-map sink (critic F7).
pub fn d8_directions(dim: usize, filled: &[i64]) -> Vec<Option<usize>> {
    assert_eq!(filled.len(), dim * dim, "filled slice must have dim*dim elements");
    let n = dim * dim;
    let mut downstream = vec![None; n];

    for z in 0..dim {
        for x in 0..dim {
            let idx = linear_index(x, z, dim);
            let self_key = (filled[idx], idx);
            let mut best: Option<(i64, usize)> = None;
            for &(dx, dz) in &D8_OFFSETS {
                let nx = x as i64 + dx;
                let nz = z as i64 + dz;
                if nx < 0 || nz < 0 || nx as usize >= dim || nz as usize >= dim {
                    continue;
                }
                let nidx = linear_index(nx as usize, nz as usize, dim);
                let n_key = (filled[nidx], nidx);
                if n_key < self_key && best.is_none_or(|b| n_key < b) {
                    best = Some(n_key);
                }
            }
            downstream[idx] = best.map(|(_, i)| i);
        }
    }

    downstream
}

/// Kahn topological flow-accumulation: `area[v] = 1 + Σ area[u]` for all `u` with
/// `downstream[u] == v`. Integer, associative (order-independent commutative sum). Panics (a real
/// `assert!`, checked in ALL builds — not `debug_assert!`) if the traversal does not complete —
/// structurally impossible given `d8_directions`'s strictly-decreasing-key invariant, but this is
/// the in-algorithm acyclicity check the phase plan requires.
pub fn kahn_accumulate(dim: usize, downstream: &[Option<usize>]) -> Vec<i64> {
    assert_eq!(downstream.len(), dim * dim, "downstream slice must have dim*dim elements");
    let n = dim * dim;
    let mut in_degree = vec![0u32; n];
    for &d in downstream {
        if let Some(d) = d {
            in_degree[d] += 1;
        }
    }

    let mut area = vec![1i64; n];
    let mut queue: VecDeque<usize> = VecDeque::new();
    for (idx, &deg) in in_degree.iter().enumerate() {
        if deg == 0 {
            queue.push_back(idx);
        }
    }

    let mut processed = 0usize;
    while let Some(v) = queue.pop_front() {
        processed += 1;
        if let Some(d) = downstream[v] {
            area[d] += area[v];
            in_degree[d] -= 1;
            if in_degree[d] == 0 {
                queue.push_back(d);
            }
        }
    }

    assert_eq!(
        processed, n,
        "drainage DAG has a cycle (processed {processed}/{n}) — should be impossible by construction"
    );

    area
}

/// Sample `height_at` over a `dim × dim` grid (origin `(0,0)`, row-major `z*dim+x`) and compute the
/// full W-3 drainage: Priority-Flood fill → D8 directions → Kahn accumulation. Pure function of
/// `(seed, hmax, dim)` — no RNG-of-clock, no thread-dependence, no global mutable state.
pub fn compute_drainage(seed: u64, hmax: i64, dim: usize) -> DrainageState {
    let n = dim * dim;
    let mut height = vec![0i64; n];
    for z in 0..dim {
        for x in 0..dim {
            height[linear_index(x, z, dim)] = height_at(x as i64, z as i64, seed, hmax);
        }
    }

    let filled = priority_flood_fill(dim, &height);
    let downstream = d8_directions(dim, &filled);
    let area = kahn_accumulate(dim, &downstream);

    DrainageState { dim, filled, downstream, area }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: u64 = 0xA11A_2A11;
    const HMAX: i64 = 200;

    /// Re-run identity: the SAME `(seed, hmax, dim)` always produces the SAME drainage.
    #[test]
    fn compute_drainage_is_deterministic_across_repeated_calls() {
        let a = compute_drainage(SEED, HMAX, 16);
        let b = compute_drainage(SEED, HMAX, 16);
        assert_eq!(a, b, "compute_drainage must be byte-identical across repeated calls");
    }

    /// Priority-Flood eliminates interior closed depressions: a hand-built 3x3 grid with a pit in
    /// the CENTER (surrounded by higher cells) must have its center raised to the min surrounding
    /// elevation (its pour point) after fill, not left as a closed basin.
    #[test]
    fn priority_flood_fill_eliminates_interior_pit() {
        // 3x3, row-major. Border ring = 10, center (pit) = 0.
        #[rustfmt::skip]
        let height = vec![
            10, 10, 10,
            10,  0, 10,
            10, 10, 10,
        ];
        let filled = priority_flood_fill(3, &height);
        // Center index = 1*3+1 = 4. Its filled elevation must equal its pour point (10, the
        // min of the surrounding ring, all of which are 10 here) — NOT the original pit depth 0.
        assert_eq!(filled[4], 10, "interior pit must be filled to its pour point");
        // Border cells are unchanged (they ARE the flood seed).
        for i in [0, 1, 2, 3, 5, 6, 7, 8] {
            assert_eq!(filled[i], height[i], "border cell {i} must be unchanged by fill");
        }
    }

    /// A depression with an uneven rim fills to the LOWEST point on the rim (the true pour point),
    /// not the highest — proves the fill is a genuine watershed fill, not a flat raise-to-max.
    #[test]
    fn priority_flood_fill_fills_to_lowest_rim_point() {
        // 3x3: rim has one low point (5) and rest higher (10); center pit = 0.
        #[rustfmt::skip]
        let height = vec![
            10, 10, 10,
            10,  0,  5, // rightmost rim cell (index 5) is the low point on the rim
            10, 10, 10,
        ];
        let filled = priority_flood_fill(3, &height);
        assert_eq!(filled[4], 5, "pit must fill to the lowest rim/pour point (5), not the highest (10)");
    }

    /// D8 acyclicity: on a hand-built grid (including a perfectly FLAT plateau, the hardest case),
    /// `kahn_accumulate` must complete without panicking (proving no cycle, even on ties).
    #[test]
    fn d8_and_kahn_complete_on_a_flat_plateau() {
        let dim = 5;
        let height = vec![7i64; dim * dim]; // perfectly flat
        let filled = priority_flood_fill(dim, &height);
        let downstream = d8_directions(dim, &filled);
        let area = kahn_accumulate(dim, &downstream); // must not panic
        assert_eq!(area.len(), dim * dim);
        assert!(area.iter().all(|&a| a >= 1), "every cell's area must be >= 1 (self)");
    }

    /// D8 direction strictly decreases the `(elevation, linear_index)` key at every edge — the
    /// structural invariant that makes the DAG acyclic BY CONSTRUCTION (not just empirically).
    #[test]
    fn d8_direction_strictly_decreases_key() {
        let dim = 16;
        let mut height = vec![0i64; dim * dim];
        for z in 0..dim {
            for x in 0..dim {
                height[linear_index(x, z, dim)] = height_at(x as i64, z as i64, SEED, HMAX);
            }
        }
        let filled = priority_flood_fill(dim, &height);
        let downstream = d8_directions(dim, &filled);
        for (idx, &d) in downstream.iter().enumerate() {
            if let Some(d) = d {
                let self_key = (filled[idx], idx);
                let d_key = (filled[d], d);
                assert!(d_key < self_key, "downstream key must be strictly less at cell {idx}");
            }
        }
    }

    /// Kahn accumulation is a real sum: a cell's area must equal 1 + the sum of its direct
    /// upstream neighbors' areas (spot-checked on a small deterministic grid).
    #[test]
    fn kahn_accumulate_sums_upstream_areas() {
        let dim = 8;
        let mut height = vec![0i64; dim * dim];
        for z in 0..dim {
            for x in 0..dim {
                height[linear_index(x, z, dim)] = height_at(x as i64, z as i64, SEED, HMAX);
            }
        }
        let filled = priority_flood_fill(dim, &height);
        let downstream = d8_directions(dim, &filled);
        let area = kahn_accumulate(dim, &downstream);

        // Cross-check: area[v] == 1 + sum(area[u] for u where downstream[u]==v).
        for v in 0..dim * dim {
            let expected: i64 = 1 + downstream
                .iter()
                .enumerate()
                .filter(|&(_, &du)| du == Some(v))
                .map(|(u, _)| area[u])
                .sum::<i64>();
            assert_eq!(area[v], expected, "area[{v}] must equal 1 + sum of direct upstream areas");
        }
    }

    /// Golden vector: pinned exact filled elevation / D8 downstream / area at a handful of cells
    /// on a small `dim=8` fixture (localized coverage — critic requirement, distinct from the
    /// full-grid `w3_chain.rs` hash golden).
    #[test]
    fn golden_vector_matches_pinned_drainage_fixture() {
        const GOLDEN_SEED: u64 = 0xA11A_2A11;
        const GOLDEN_HMAX: i64 = 200;
        const DIM: usize = 8;
        let state = compute_drainage(GOLDEN_SEED, GOLDEN_HMAX, DIM);

        // (linear_index, expected_filled, expected_downstream, expected_area)
        const CASES: &[(usize, i64, Option<usize>, i64)] = &[
            (0, 130, Some(9), 1),
            (9, 129, Some(18), 3),
            (27, 127, Some(28), 6),
            (63, 122, Some(55), 1),
        ];
        for &(idx, exp_filled, exp_down, exp_area) in CASES {
            assert_eq!(state.filled[idx], exp_filled, "golden drift: filled[{idx}]");
            assert_eq!(state.downstream[idx], exp_down, "golden drift: downstream[{idx}]");
            assert_eq!(state.area[idx], exp_area, "golden drift: area[{idx}]");
        }
    }

    /// `is_river`/`RIVER_THRESHOLD` boundary sanity.
    #[test]
    fn is_river_boundary() {
        assert!(!is_river(RIVER_THRESHOLD));
        assert!(is_river(RIVER_THRESHOLD + 1));
        assert!(!is_river(0));
    }
}

