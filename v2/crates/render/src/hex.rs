//! Flat-top hex geometry (RnD `rendering/01` §1.1–1.3): axial `(q, r, s)` with `q + r + s = 0` for
//! neighbour arithmetic, stored as an "odd-q" offset that IS the sim's square `(x, z)` grid — the hex
//! is a render-space VISUAL over the square `WorldView`/`Vec2Fixed` sim grid; the sim is never
//! re-gridded (issue #223's fixed decision). Formulas are the canonical Red Blob Games flat-top /
//! odd-q layout (RnD 01 R1).

use macroquad::prelude::Vec3;
use std::f32::consts::PI;

/// Hex circumradius (center → corner) in world units. One `WorldView` cell ↦ one hex column.
pub const HEX_SIZE: f32 = 1.0;
/// World-Y units per unit of `WorldView::height` — keeps columns proportionate to [`HEX_SIZE`].
/// A tuned display constant, not load-bearing (render-space only). At 0.3 a full-HMAX (200) column
/// is 60 world-Y tall vs a hex circumradius of 1 — high relief reads clearly without the 80:1
/// "picket-fence" towers 0.4 produced on the diverse-relief terragen (tectonic/volcanic peaks).
pub const HEIGHT_SCALE: f32 = 0.3;

/// Canonical flat-top axial neighbour directions, `(dq, dr)` (Red Blob Games, RnD 01 R1). Order is
/// arbitrary but FIXED — [`edge_for_direction`] is derived against THIS order.
pub const AXIAL_DIRS: [(i64, i64); 6] = [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)];

/// Direction index `i` (into [`AXIAL_DIRS`]) → the [`hex_corner`] index pair `(k, k+1 mod 6)`
/// bounding that direction's shared edge.
///
/// Derivation: a flat-top hex's corner `k` sits at angle `60k°` (`hex_corner` below); the
/// `axial_to_pixel` direction for `AXIAL_DIRS[i]` (odd-q layout, §hex_center's formula, direction
/// component only) works out to angle `60i° + 30°` — i.e. the MIDPOINT of the edge between corners
/// `(6 - i) % 6` and `(6 - i) % 6 + 1`. Verified by hand for all 6 `i` (each direction's angle lands
/// exactly on the corresponding edge midpoint); this table just bakes that arithmetic.
pub const fn edge_for_direction(dir_index: usize) -> usize {
    (6 - dir_index) % 6
}

/// Offset `(col = x, row = z)` — the sim's square grid — → axial `(q, r)`, odd-q vertical layout.
pub fn offset_to_axial(col: i64, row: i64) -> (i64, i64) {
    let q = col;
    let r = row - (col - (col & 1)) / 2;
    (q, r)
}

/// Axial `(q, r)` → offset `(col, row)`, the inverse of [`offset_to_axial`].
pub fn axial_to_offset(q: i64, r: i64) -> (i64, i64) {
    let col = q;
    let row = r + (q - (q & 1)) / 2;
    (col, row)
}

/// World-space `(x, z)` center of hex `(col, row)` — odd-q offset → pixel, flat-top (Red Blob Games).
pub fn hex_center(col: i64, row: i64) -> (f32, f32) {
    let x = HEX_SIZE * 1.5 * col as f32;
    let z = HEX_SIZE * 3f32.sqrt() * (row as f32 + 0.5 * (col & 1) as f32);
    (x, z)
}

/// The `k`-th corner (`k` in `0..6`) of a flat-top hex centered at `(cx, cz)`, at world height `y`.
pub fn hex_corner(cx: f32, cz: f32, y: f32, k: usize) -> Vec3 {
    let angle = PI / 3.0 * k as f32; // 60°·k, flat-top corner convention
    Vec3::new(cx + HEX_SIZE * angle.cos(), y, cz + HEX_SIZE * angle.sin())
}

/// The 6 neighbours of `(col, row)` in the square `WorldView` grid, via an axial round-trip.
/// `neighbors(..)[[i]]` shares the edge given by [`edge_for_direction(i)`].
pub fn neighbors(col: i64, row: i64) -> [(i64, i64); 6] {
    let (q, r) = offset_to_axial(col, row);
    let mut out = [(0i64, 0i64); 6];
    for (i, &(dq, dr)) in AXIAL_DIRS.iter().enumerate() {
        out[i] = axial_to_offset(q + dq, r + dr);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The offset↔axial round-trip must be exact for every cell in a representative grid — a typo
    /// in either formula would silently misplace hexes/neighbours.
    #[test]
    fn offset_axial_round_trips() {
        for col in -3..8 {
            for row in -3..8 {
                let (q, r) = offset_to_axial(col, row);
                assert_eq!(axial_to_offset(q, r), (col, row), "round-trip failed at ({col},{row})");
            }
        }
    }

    /// `edge_for_direction` must be a bijection on `0..6` (each edge used exactly once) — if two
    /// directions mapped to the same edge, one side of the hex would get two cliff quads and another
    /// none.
    #[test]
    fn edge_for_direction_is_a_bijection() {
        let mut seen = [false; 6];
        for i in 0..6 {
            let e = edge_for_direction(i);
            assert!(!seen[e], "edge {e} claimed by more than one direction");
            seen[e] = true;
        }
    }

    /// Neighbours are reciprocal: if B is a neighbour of A, A must be a neighbour of B (a flat-top
    /// hex tiling has no one-way adjacency).
    #[test]
    fn neighbors_are_reciprocal() {
        for col in 0..6 {
            for row in 0..6 {
                for &(ncol, nrow) in neighbors(col, row).iter() {
                    let back = neighbors(ncol, nrow);
                    assert!(
                        back.contains(&(col, row)),
                        "({ncol},{nrow}) is a neighbour of ({col},{row}) but not vice versa"
                    );
                }
            }
        }
    }
}
