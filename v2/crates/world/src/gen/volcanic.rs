//! W-SIM-5 (#410): deterministic integer volcanic constructive relief — viscosity-selected additive
//! edifices, the third landform slice on the `worldgen-relief` ladder (RnD
//! `sim/world/15-volcanic-landforms.md`). Unlike erosion [10] and aeolian [13] (which sculpt/
//! redistribute EXISTING relief), this is CONSTRUCTIVE: it ADDS height, so it is inserted PRE-erosion
//! — `gen::erosion::erode_with_tectonics` folds this module's contribution into the initial height
//! field before the macro-loop runs, exactly where tectonics' fault scarp already injects (RnD 15
//! §1). **Pure integer / fixed-point throughout — no `f32`/`f64` anywhere in this file** (covered by
//! the recursive glob guard, `world/tests/no_float_guard_gen.rs`).
//!
//! ## Scope (this slice — #410's explicit out-of-scope list)
//!
//! Vent-centered additive radial edifices with a viscosity-selected slope class only. Lava-flow
//! routing (additive, self-modifying steepest-descent), calderas/maars (subtractive collapse AFTER
//! edifice construction), lava plateaus, columnar jointing are explicit follow-up slices (RnD 15
//! §4–6) — NOT built here.
//!
//! ## Master regulator: viscosity selects the slope class (RnD 15 §2)
//!
//! ONE per-vent viscosity draw selects between two edifice classes — a deliberate two-way
//! simplification of RnD 15 §2's five-way taxonomy (shield/stratocone/cinder cone/dome/maar), enough
//! to satisfy this slice's acceptance criterion ("both a gentle-wide class and a steep-compact class
//! realized"): [`SlopeClass::Shield`] (low viscosity — gentle, wide, [`SHIELD_MAX_RADIUS`]) and
//! [`SlopeClass::Cone`] (high viscosity — steep, compact, [`CONE_MAX_RADIUS`]). Both classes share
//! the SAME peak height ([`EDIFICE_PEAK_HEIGHT`]) — only the radial falloff (width/slope) differs, so
//! the mean-radial-slope comparison the acceptance criterion tests is driven purely by viscosity/
//! class, never by a height difference.
//!
//! ## Radial profile
//!
//! A linear cone: `height(r) = max(0, peak - r·peak/max_radius)` for `r ≤ max_radius`, else 0
//! (`r = isqrt(dx²+dz²)`, the SAME integer-distance primitive `erosion.rs`'s stream-power incision
//! uses) — a coherent, monotonically-non-increasing-outward radial profile BY CONSTRUCTION (integer
//! division of a non-decreasing numerator by a fixed positive denominator is non-decreasing, so
//! `peak` minus that is non-increasing). RnD 15 §3's convex(shield)/concave(stratocone) profile
//! nuance is a documented simplification for this slice — a linear ramp is the simplest faithful "a
//! positive additive cone the fBm+erode baseline cannot build."
//!
//! ## Overlap: SUM, never last-write (RnD 15 §7's #1 named pitfall)
//!
//! [`emplace_edifices`] accumulates EVERY vent's contribution into ONE delta buffer via plain integer
//! addition (commutative/associative — order-irrelevant by construction, no RNG in the summation
//! itself) and returns it UNCLAMPED; the caller (`erosion.rs`) clamps the fully-summed result into
//! `[0,hmax]` EXACTLY ONCE, never per-vent (a per-vent clamp would be non-associative — order would
//! matter — and would silently lose magma mass on overlap, RnD 15 §7).
//!
//! ## Determinism (RnD 15 §7)
//!
//! Vents are a pure function of `(seed, dim)` via [`sim_core::seed_fold`] — the SAME counter-based
//! keyed-hash technique `tectonics::build_faults` uses — byte-identical across repeated generation,
//! decorrelated from every other seeded field by [`SALT_VOLCANIC_VENT`]. This module has no iterative
//! stochastic roll chain (unlike aeolian's per-hop deposit rolls): a vent's radial contribution is a
//! pure geometric function of its position/class once placed, so only vent PLACEMENT (position +
//! slope-class selection) is randomized — a single deterministic draw per vent index, decorrelated
//! from every other vent by the index itself being part of the `seed_fold` parts tuple.

use sim_core::{isqrt, seed_fold};

use crate::gen::material::MaterialId;

/// Number of vents per world (implementer's call, mirrors `tectonics::N_FAULTS`'s "a handful is
/// enough to produce a visibly non-isotropic network" reasoning). Documented, locked by the
/// golden-vector test.
pub const N_VENTS: usize = 3;

/// Shared peak height for EVERY vent, regardless of slope class (implementer's call) — only the
/// radial falloff radius differs between classes, so the mean-slope comparison the acceptance
/// criterion tests is driven purely by viscosity/class, not by a height difference.
const EDIFICE_PEAK_HEIGHT: i64 = 16;
/// Shield (low-viscosity, gentle/wide) footprint radius, in cells.
const SHIELD_MAX_RADIUS: i64 = 8;
/// Cone (high-viscosity, steep/compact) footprint radius, in cells — much smaller than
/// [`SHIELD_MAX_RADIUS`], so the SAME peak height drops off over far fewer cells (steeper).
const CONE_MAX_RADIUS: i64 = 3;

/// Decorrelation salt for vent placement ("VOLCVENT", ASCII-folded — mirrors `tectonics.rs`'s
/// `FAULT_SEED_SALT` / `erosion.rs`'s `RESISTANCE_SALT` convention).
const SALT_VOLCANIC_VENT: u64 = 0x564F_4C43_5645_4E54;
/// Viscosity roll modulus and the Shield/Cone split point (implementer's call: an even split so a
/// handful of vents realizes both classes).
const VISCOSITY_ROLL_MOD: u64 = 100;
const VISCOSITY_SHIELD_MAX: u64 = 50;

/// A vent's viscosity-selected edifice class (RnD 15 §2's master regulator, simplified to a binary
/// choice for this slice — see the module doc).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlopeClass {
    /// Low viscosity: gentle, wide (shield-like).
    Shield,
    /// High viscosity: steep, compact (stratocone/dome-like).
    Cone,
}

impl SlopeClass {
    /// `pub(crate)`: `erosion.rs`'s post-erosion-survival test needs each vent's outer footprint
    /// radius to sample the comparison ring (mirrors `climate.rs`'s `WIND_DX` visibility precedent —
    /// a cross-module consumer reads the SAME constant rather than risk a duplicated copy drifting).
    pub(crate) fn max_radius(self) -> i64 {
        match self {
            SlopeClass::Shield => SHIELD_MAX_RADIUS,
            SlopeClass::Cone => CONE_MAX_RADIUS,
        }
    }

    /// The primary substrate material this class emplaces (RnD 15 §8: basalt for effusive
    /// low-viscosity flows, tuff for the higher-viscosity/fragmenting style).
    fn material(self) -> MaterialId {
        match self {
            SlopeClass::Shield => MaterialId::Basalt,
            SlopeClass::Cone => MaterialId::Tuff,
        }
    }
}

/// Pure viscosity-roll → slope-class mapping (RnD 15 §2's master regulator), extracted as its own
/// function so the class-selection contract ("driven by viscosity, not vent index/position") is
/// directly unit-testable against synthetic roll values.
fn class_from_viscosity_roll(roll: u64) -> SlopeClass {
    if roll < VISCOSITY_SHIELD_MAX { SlopeClass::Shield } else { SlopeClass::Cone }
}

/// A single volcanic vent: integer grid position + its viscosity-selected slope class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vent {
    pub x: i64,
    pub z: i64,
    pub class: SlopeClass,
}

/// Derive vent `index` (`0..N_VENTS`) as a pure function of `(seed, dim)` via `seed_fold` —
/// byte-identical across repeated calls, decorrelated by [`SALT_VOLCANIC_VENT`]. Mirrors
/// `tectonics::fault_at`'s derivation shape.
fn vent_at(index: usize, seed: u64, dim: usize) -> Vent {
    let h = seed_fold(seed, &[SALT_VOLCANIC_VENT, index as u64]);
    let dim_u = dim.max(1) as u64;
    let x = (h % dim_u) as i64;
    let z = ((h >> 16) % dim_u) as i64;
    let viscosity_roll = (h >> 32) % VISCOSITY_ROLL_MOD;
    Vent { x, z, class: class_from_viscosity_roll(viscosity_roll) }
}

/// Build the full [`N_VENTS`]-vent network for `(seed, dim)` — call ONCE per emplacement and reuse
/// the returned slice (avoids re-deriving the same vents per cell).
pub fn build_vents(seed: u64, dim: usize) -> Vec<Vent> {
    (0..N_VENTS).map(|i| vent_at(i, seed, dim)).collect()
}

/// This vent's height contribution at grid offset `(dx, dz)` from its own center — the linear radial
/// cone (module doc), 0 outside its footprint radius.
fn vent_height_at(vent: &Vent, dx: i64, dz: i64) -> i64 {
    let r = isqrt(dx * dx + dz * dz);
    let max_r = vent.class.max_radius();
    if r > max_r {
        return 0;
    }
    (EDIFICE_PEAK_HEIGHT - (r * EDIFICE_PEAK_HEIGHT) / max_r).max(0)
}

#[inline]
fn linear_index(x: usize, z: usize, dim: usize) -> usize {
    z * dim + x
}

/// Emplace every vent's edifice into a `dim × dim` height DELTA buffer (module doc: summed, never
/// last-write; UNCLAMPED — the caller clamps the fully-summed result exactly once). Order-independent
/// by construction (plain integer addition per cell, associative/commutative — iterating `vents` in
/// any order yields the identical buffer).
pub fn emplace_edifices(dim: usize, vents: &[Vent]) -> Vec<i64> {
    let n = dim * dim;
    let mut delta = vec![0i64; n];
    for vent in vents {
        for z in 0..dim {
            for x in 0..dim {
                let contribution = vent_height_at(vent, x as i64 - vent.x, z as i64 - vent.z);
                if contribution > 0 {
                    delta[linear_index(x, z, dim)] += contribution;
                }
            }
        }
    }
    delta
}

/// Per-cell PRIMARY volcanic material mask: `Some(material)` on any cell within at least one vent's
/// footprint, tie-broken by the vent contributing the MOST height there (ties by lowest vent index —
/// deterministic, `>` not `>=` on the running best), `None` elsewhere. Written as the primary
/// substrate by the caller (`caps.rs`, mirroring aeolian's sand reconciliation) — RnD 15 §8.
pub fn edifice_material_mask(dim: usize, vents: &[Vent]) -> Vec<Option<MaterialId>> {
    let n = dim * dim;
    let mut mask = vec![None; n];
    let mut best_contribution = vec![0i64; n];
    for vent in vents {
        for z in 0..dim {
            for x in 0..dim {
                let contribution = vent_height_at(vent, x as i64 - vent.x, z as i64 - vent.z);
                if contribution <= 0 {
                    continue;
                }
                let idx = linear_index(x, z, dim);
                if contribution > best_contribution[idx] {
                    best_contribution[idx] = contribution;
                    mask[idx] = Some(vent.class.material());
                }
            }
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: u64 = 0xA11A_2A11;
    const DIM: usize = 64;

    #[test]
    fn build_vents_is_deterministic_across_repeated_calls() {
        let a = build_vents(SEED, DIM);
        let b = build_vents(SEED, DIM);
        assert_eq!(a, b, "build_vents must be byte-identical across repeated calls");
    }

    #[test]
    fn different_seed_diverges() {
        let a = build_vents(SEED, DIM);
        let b = build_vents(SEED ^ 0xDEAD_BEEF, DIM);
        assert_ne!(a, b, "a different seed must produce a different vent network");
    }

    #[test]
    fn emplace_edifices_is_order_independent() {
        let vents = build_vents(SEED, DIM);
        let forward = emplace_edifices(DIM, &vents);
        let mut reversed = vents.clone();
        reversed.reverse();
        let backward = emplace_edifices(DIM, &reversed);
        assert_eq!(forward, backward, "vent processing order must not affect the summed delta (integer addition is commutative)");
    }

    /// Two deliberately overlapping vents (same class, footprints touching): the overlap cell must
    /// equal the SUM of both individual contributions, not either one alone (RnD 15 §7's named
    /// pitfall: last-write loses magma mass).
    #[test]
    fn emplace_edifices_overlap_sums_not_last_write() {
        let a = Vent { x: 10, z: 10, class: SlopeClass::Shield };
        let b = Vent { x: 13, z: 10, class: SlopeClass::Shield };
        // Midpoint-ish cell inside BOTH footprints (radius 8 each, centers 3 apart).
        let probe = (12usize, 10usize);
        let dim = 32usize;

        let solo_a = emplace_edifices(dim, std::slice::from_ref(&a));
        let solo_b = emplace_edifices(dim, std::slice::from_ref(&b));
        let both = emplace_edifices(dim, &[a, b]);

        let idx = linear_index(probe.0, probe.1, dim);
        assert!(solo_a[idx] > 0 && solo_b[idx] > 0, "the probe cell must be inside BOTH individual footprints for this test to be meaningful");
        assert_eq!(
            both[idx],
            solo_a[idx] + solo_b[idx],
            "an overlap cell must equal the SUM of both vents' individual contributions"
        );
    }

    #[test]
    fn class_from_viscosity_roll_selects_shield_below_split_and_cone_above() {
        assert_eq!(class_from_viscosity_roll(0), SlopeClass::Shield);
        assert_eq!(class_from_viscosity_roll(VISCOSITY_SHIELD_MAX - 1), SlopeClass::Shield);
        assert_eq!(class_from_viscosity_roll(VISCOSITY_SHIELD_MAX), SlopeClass::Cone);
        assert_eq!(class_from_viscosity_roll(VISCOSITY_ROLL_MOD - 1), SlopeClass::Cone);
    }

    /// Viscosity selects the SLOPE CLASS (#410 ТЗ): same peak height, but Shield's wider footprint
    /// means the SAME height drop happens over more cells — a strictly gentler mean radial slope
    /// than Cone's. Driven purely by class (== viscosity), never by vent index or position.
    #[test]
    fn shield_mean_radial_slope_is_gentler_than_cone() {
        let shield_mean_slope = EDIFICE_PEAK_HEIGHT / SHIELD_MAX_RADIUS;
        let cone_mean_slope = EDIFICE_PEAK_HEIGHT / CONE_MAX_RADIUS;
        assert!(
            shield_mean_slope < cone_mean_slope,
            "Shield's mean radial slope ({shield_mean_slope}) must be measurably below Cone's ({cone_mean_slope})"
        );
    }

    /// Across a realistic vent set (many indices, several seeds), BOTH classes actually occur — not
    /// just theoretically possible via `class_from_viscosity_roll` in isolation.
    #[test]
    fn both_slope_classes_are_realized_across_a_vent_set() {
        let mut saw_shield = false;
        let mut saw_cone = false;
        for seed in [SEED, SEED ^ 1, SEED ^ 2, SEED ^ 3, SEED ^ 4] {
            for i in 0..8 {
                match vent_at(i, seed, DIM).class {
                    SlopeClass::Shield => saw_shield = true,
                    SlopeClass::Cone => saw_cone = true,
                }
            }
        }
        assert!(saw_shield && saw_cone, "both Shield and Cone classes must be realized across a real vent set");
    }

    #[test]
    fn edifice_material_mask_writes_basalt_for_shield_and_tuff_for_cone() {
        let shield = Vent { x: 10, z: 10, class: SlopeClass::Shield };
        let cone = Vent { x: 40, z: 40, class: SlopeClass::Cone };
        let dim = 64usize;
        let mask = edifice_material_mask(dim, &[shield, cone]);

        assert_eq!(mask[linear_index(10, 10, dim)], Some(MaterialId::Basalt), "the Shield vent's summit must be Basalt");
        assert_eq!(mask[linear_index(40, 40, dim)], Some(MaterialId::Tuff), "the Cone vent's summit must be Tuff");
        assert_eq!(mask[linear_index(0, 0, dim)], None, "a cell outside every vent's footprint must be unmarked");
    }

    /// Count of cells that are a STRICT local D8 maximum (height strictly greater than all 8
    /// neighbors — undefined/false at the grid edge where a neighbor is missing, per the `continue`
    /// below, matching this module's other edge-handling convention).
    fn local_maximum_count(dim: usize, height: &[i64]) -> usize {
        const D8_OFFSETS: [(i64, i64); 8] =
            [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];
        let mut count = 0;
        for z in 0..dim {
            for x in 0..dim {
                let idx = linear_index(x, z, dim);
                let mut is_max = true;
                for &(dx, dz) in &D8_OFFSETS {
                    let nx = x as i64 + dx;
                    let nz = z as i64 + dz;
                    if nx < 0 || nz < 0 || nx as usize >= dim || nz as usize >= dim {
                        continue;
                    }
                    if height[linear_index(nx as usize, nz as usize, dim)] >= height[idx] {
                        is_max = false;
                        break;
                    }
                }
                if is_max {
                    count += 1;
                }
            }
        }
        count
    }

    /// Constructive-relief corridor (#410 ТЗ, anti-forcing-clean — the W-SIM-4a scarp-crank lesson:
    /// verify a sharp structural feature the baseline cannot produce, not bulk roughness). Measured
    /// right after the additive edifice stamp (this module's own output — exactly the "PRE-erosion
    /// volcanic field" the ТЗ specifies). The OFF baseline here is FLAT (not fBm noise) — an even
    /// STRONGER anti-forcing choice than fBm (which could incidentally produce a few local maxima of
    /// its own): a flat field has ZERO strict local maxima by construction (every neighbor is
    /// exactly equal, never `<`), so any local maximum observed ON is unambiguously the edifice
    /// stamp's doing, not baseline noise.
    #[test]
    fn constructive_relief_corridor_local_maxima() {
        let dim = 64usize;
        let flat = vec![50i64; dim * dim];
        let off_count = local_maximum_count(dim, &flat);
        assert_eq!(off_count, 0, "a flat field must have ZERO strict local maxima by construction");

        let vents = build_vents(SEED, dim);
        let delta = emplace_edifices(dim, &vents);
        let on_height: Vec<i64> = (0..dim * dim).map(|i| flat[i] + delta[i]).collect();
        let on_count = local_maximum_count(dim, &on_height);
        assert!(
            on_count > off_count,
            "additive edifices must produce local radial maxima the flat baseline cannot: OFF={off_count} ON={on_count}"
        );
    }

    /// Golden vector: pinned exact volcanic emplacement (delta + material) at fixed grid indices for
    /// the golden `(seed, dim)` fixture.
    ///
    /// Re-pinned for #410 pass 2b: CI-sourced — `left:` from both x86 debug (`v2 sim` job) and
    /// arm64 release (`v2 golden` job), run #29186984874, commit 5d82049; both arches agree
    /// (integer, arch-stable).
    #[test]
    fn golden_vector_matches_pinned_volcanic_fixture() {
        let dim = 16usize;
        let vents = build_vents(SEED, dim);
        let delta = emplace_edifices(dim, &vents);

        const INDICES: [usize; 4] = [0, 36, 100, 200];
        const EXPECTED: [i64; 4] = [6, 0, 0, 10];
        let actual: [i64; 4] = std::array::from_fn(|i| delta[INDICES[i]]);
        assert_eq!(actual, EXPECTED, "golden drift (or placeholder awaiting CI pin) at indices {INDICES:?}");
    }
}
