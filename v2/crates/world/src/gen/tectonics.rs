//! W-SIM-4a: deterministic integer tectonic relief — fault network, fault-scarp height step, and
//! fault-aligned resistance-lineament structure (issue #396, first landform slice of the
//! `worldgen-relief` roadmap). **Pure integer / fixed-point throughout — no `f32`/`f64` anywhere in
//! this file** (enforced by the recursive glob guard, `world/tests/no_float_guard_gen.rs`).
//!
//! ## Two structural ingredients (both consumed by `gen::erosion`)
//!
//! 1. **[`fault_scarp_delta`]** — a vertical height STEP applied across each fault line, BEFORE
//!    erosion (so erosion then dissects the raw scarp).
//! 2. **[`is_in_fault_band`]** — marks cells within a fixed perpendicular band of any fault line, so
//!    `gen::erosion::erode_with_tectonics` can force the rock-resistance field softer there (fault
//!    gouge — geologically the fault zone is the WEAKEST rock, not the hardest), giving the existing
//!    differential-erosion machinery a LINEAR structure to carve into valleys, instead of the
//!    isotropic-noise blobs `resistance_field` alone produces.
//!
//! ## Fault representation — infinite lines via integer cross-product, no trig/float/sqrt
//!
//! A fault is a point `(px, pz)` + an integer direction vector `(dx, dz)` drawn from a small fixed
//! set ([`FAULT_DIRECTIONS`]) — never an arbitrary angle (no `sin`/`cos`, no float). For any grid
//! cell `(x, z)`, the signed 2D cross product
//! `cross = dx·(z − pz) − dz·(x − px)`
//! is proportional (by the direction vector's length) to the perpendicular distance from the cell to
//! the INFINITE line through `(px, pz)` in direction `(dx, dz)`; its SIGN tells which side of the
//! line the cell is on. Both consumers below use `cross` directly (sign for the scarp side, squared
//! magnitude vs. a squared threshold for the resistance band) — **never a division or a square
//! root**, so everything stays exact integer arithmetic.
//!
//! Each fault is a pure function of `(index, seed, dim)` via [`sim_core::seed_fold`] (the same
//! technique `height.rs`'s lattice-corner hash and `erosion.rs`'s `RESISTANCE_SALT` decorrelation
//! use) — byte-identical across repeated generation of the same `(seed, dim)`, and genuinely linear
//! (a line extends without bound along its own direction — see
//! `fault_band_extends_along_its_line_not_a_bounded_blob` below), not an isotropic blob.

use sim_core::seed_fold;

/// Number of fault lines superposed per world (implementer's call, documented, locked by the
/// golden-vector test). A handful is enough to produce a visibly non-isotropic network on the
/// golden grid without the scarp/band effects burying each other.
pub const N_FAULTS: usize = 3;

/// Decorrelation salt for the fault network — XORed via `seed_fold`'s salt-part convention (mirrors
/// `erosion.rs`'s `RESISTANCE_SALT` / `caps.rs`'s `PATCH_SEED_SALT`), so fault placement is
/// statistically independent of both height and resistance noise.
const FAULT_SEED_SALT: u64 = 0x4641_554C_5453_5F30; // "FAULTS_0" (ASCII, folded)

/// Small fixed set of integer direction vectors a fault line may take — deliberately NOT an
/// arbitrary angle (would need trig/float): axis-aligned and the two diagonals. A line and its
/// negation `(-dx, -dz)` describe the SAME infinite line, so 4 entries already give 4 distinct
/// orientations.
const FAULT_DIRECTIONS: [(i64, i64); 4] = [(1, 0), (0, 1), (1, 1), (1, -1)];

/// Fault-scarp half-step, as an `hmax` fraction (numerator/denominator, mirrors
/// `erosion.rs`'s `RESIST_THRESH_NUM` convention so it scales with any `hmax`). Each fault
/// contributes `± step_half` to a cell's height depending on which side of the line it falls on;
/// implementer's call, documented, locked by the golden-vector test.
const FAULT_STEP_NUM: i64 = 1;
const FAULT_STEP_DEN: i64 = 12;

/// Resistance-lineament half-band width, in grid cells, measured perpendicular to the fault line.
/// Implementer's call, documented, locked — wide enough to give the erosion loop a real linear
/// target (not a single-cell-wide, sub-resolution sliver), narrow enough to stay a "lineament" (not
/// swallow the whole map).
const FAULT_BAND_HALFWIDTH: i64 = 2;

/// One fault line: an integer point + direction (defines an INFINITE line, never a bounded segment
/// — see the module doc's linearity proof), a cached squared direction length (avoids recomputing
/// `dx*dx+dz*dz` per cell), and a `polarity` (`+1`/`-1`) deciding which side of the line is uplifted
/// vs. down-dropped by the scarp step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault {
    px: i64,
    pz: i64,
    dx: i64,
    dz: i64,
    dlen_sq: i64,
    polarity: i64,
}

/// Derive fault `index` (`0..N_FAULTS`) as a pure function of `(seed, dim)` via `seed_fold` —
/// byte-identical across repeated calls, decorrelated from height/resistance noise by
/// [`FAULT_SEED_SALT`]. `dim` bounds the base point to (roughly) the grid, though the line itself is
/// infinite and freely extends outside `[0, dim)`.
fn fault_at(index: usize, seed: u64, dim: usize) -> Fault {
    let h = seed_fold(seed, &[FAULT_SEED_SALT, index as u64]);
    let dim_u = dim.max(1) as u64;
    let px = (h % dim_u) as i64;
    let pz = ((h >> 16) % dim_u) as i64;
    let dir_idx = ((h >> 32) % FAULT_DIRECTIONS.len() as u64) as usize;
    let (dx, dz) = FAULT_DIRECTIONS[dir_idx];
    let polarity = if (h >> 48) & 1 == 0 { 1 } else { -1 };
    Fault { px, pz, dx, dz, dlen_sq: dx * dx + dz * dz, polarity }
}

/// Build the full [`N_FAULTS`]-line network for `(seed, dim)` — call ONCE per `erode` invocation
/// and reuse the returned slice for every cell (avoids re-deriving the same faults per cell).
pub fn build_faults(seed: u64, dim: usize) -> Vec<Fault> {
    (0..N_FAULTS).map(|i| fault_at(i, seed, dim)).collect()
}

/// Signed 2D cross product of `(x,z) − (px,pz)` against the fault's direction — see the module doc
/// for the geometric meaning (sign = side, magnitude ∝ perpendicular distance × direction length).
#[inline]
fn cross(f: &Fault, x: i64, z: i64) -> i64 {
    f.dx * (z - f.pz) - f.dz * (x - f.px)
}

/// Combined fault-scarp height delta at `(x, z)`: the signed sum, over every fault in `faults`, of
/// `polarity × step_half × sign(cross)` — a superposed network of vertical steps, one per fault
/// line. Applied to the height field BEFORE erosion (`gen::erosion::erode_with_tectonics`), then
/// clamped into `[0, hmax]` by the caller (this function itself is unclamped — a pure per-fault
/// sum, since the caller already knows the pre-scarp height to clamp against).
pub fn fault_scarp_delta(x: i64, z: i64, faults: &[Fault], hmax: i64) -> i64 {
    let step_half = (hmax * FAULT_STEP_NUM) / FAULT_STEP_DEN;
    let mut total = 0i64;
    for f in faults {
        total += f.polarity * step_half * cross(f, x, z).signum();
    }
    total
}

/// Whether `(x, z)` lies within the fault-aligned resistance-lineament band of ANY fault in
/// `faults` — `cross² ≤ (FAULT_BAND_HALFWIDTH² · dlen_sq)`, the squared-magnitude form of "is the
/// perpendicular distance to this line ≤ `FAULT_BAND_HALFWIDTH` cells", entirely avoiding
/// division/`isqrt` (unlike `erosion.rs`'s incision, which genuinely needs a distance magnitude,
/// this only needs a threshold COMPARISON, so squaring both sides keeps it exact-integer without a
/// root at all).
pub fn is_in_fault_band(x: i64, z: i64, faults: &[Fault]) -> bool {
    let half_sq = FAULT_BAND_HALFWIDTH * FAULT_BAND_HALFWIDTH;
    faults.iter().any(|f| {
        let c = cross(f, x, z);
        c * c <= half_sq * f.dlen_sq
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: u64 = 0xA11A_2A11;
    const HMAX: i64 = 200;
    const DIM: usize = 64;

    #[test]
    fn build_faults_is_deterministic_across_repeated_calls() {
        let a = build_faults(SEED, DIM);
        let b = build_faults(SEED, DIM);
        assert_eq!(a, b, "build_faults must be byte-identical across repeated calls");
    }

    #[test]
    fn fault_scarp_delta_and_band_are_deterministic() {
        let faults = build_faults(SEED, DIM);
        for &(x, z) in &[(0i64, 0i64), (-5, 12), (37, 5), (1_000_000, -1_000_000)] {
            let d1 = fault_scarp_delta(x, z, &faults, HMAX);
            let d2 = fault_scarp_delta(x, z, &faults, HMAX);
            assert_eq!(d1, d2, "fault_scarp_delta({x},{z}) must be byte-identical across repeated calls");
            let b1 = is_in_fault_band(x, z, &faults);
            let b2 = is_in_fault_band(x, z, &faults);
            assert_eq!(b1, b2, "is_in_fault_band({x},{z}) must be byte-identical across repeated calls");
        }
    }

    #[test]
    fn different_seed_diverges() {
        let a = build_faults(SEED, DIM);
        let b = build_faults(SEED ^ 0xDEAD_BEEF, DIM);
        assert_ne!(a, b, "a different seed must produce a different fault network");
    }

    /// Linearity proof (acceptance criterion — "linear lineaments, NOT isotropic blobs"): a point
    /// arbitrarily far along a fault's OWN direction from its own base point stays exactly ON that
    /// fault's line (band membership holds for unbounded `t`), while a point offset far
    /// PERPENDICULAR to the line falls out of band. A bounded "blob" region could never satisfy the
    /// first half — its membership would be bounded in every direction, including along the line.
    #[test]
    fn fault_band_extends_along_its_line_not_a_bounded_blob() {
        let f = fault_at(0, SEED, DIM);
        let single = [f];
        for &t in &[0i64, 10, 50, 200, -50, -200, 1_000_000] {
            let x = f.px + f.dx * t;
            let z = f.pz + f.dz * t;
            assert!(
                is_in_fault_band(x, z, &single),
                "point at t={t} along fault 0's own line must stay in-band for unbounded t (linear, not a bounded blob)"
            );
        }
        // Perpendicular direction to (dx,dz) is (-dz,dx); offset far along it must exit the band.
        let perp_x = f.px - f.dz * 50;
        let perp_z = f.pz + f.dx * 50;
        assert!(
            !is_in_fault_band(perp_x, perp_z, &single),
            "a point far perpendicular to fault 0's line must be out of band"
        );
    }

    /// Golden vector: pinned exact fault-scarp delta / band membership at explicit coordinates for
    /// the golden `(seed, dim, hmax)` — re-derivable from this file's algorithm doc (critic F10
    /// idiom, mirrors `height.rs`/`erosion.rs`'s golden-vector tests).
    ///
    /// Restored for #397: the scarp-step widening (`FAULT_STEP_DEN` 12→8) was a magnitude crank,
    /// reverted (PM decision) in favor of the hard-fault resistance flip alone. `FAULT_STEP_DEN` is
    /// back at its pre-#397 value 12, so this is byte-identical to the original #396 pin — restored
    /// from that value directly (no fresh CI reveal needed, it's a pure function of unchanged
    /// inputs), originally pinned from `v2-golden-arm64` CI run #29170719244, commit cde3c68.
    #[test]
    fn golden_vector_matches_pinned_tectonics_fixture() {
        let faults = build_faults(SEED, DIM);
        const COORDS: &[(i64, i64)] = &[(0, 0), (7, 3), (32, 32), (63, 63)];
        const EXPECTED: &[(i64, bool)] = &[(-16, false), (-16, false), (16, false), (-16, false)];
        let actual: Vec<(i64, bool)> = COORDS
            .iter()
            .map(|&(x, z)| (fault_scarp_delta(x, z, &faults, HMAX), is_in_fault_band(x, z, &faults)))
            .collect();
        assert_eq!(actual, EXPECTED, "golden drift in fault_scarp_delta/is_in_fault_band (or: pass-1 placeholder awaiting CI pin)");
    }
}
