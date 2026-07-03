//! W-2: baseline material field (RnD `sim/world/02 §4`) — the material W-4 erosion later REFINES
//! and W-5 reads. Stating it here means there is never a null-material state downstream. **Pure
//! integer / fixed-point throughout — no `f32`/`f64` anywhere in this file** (enforced by the
//! recursive glob guard, `world/tests/no_float_guard_gen.rs`).
//!
//! ## Algorithm
//!
//! [`material_at`] classifies a voxel `(x, y, z)` (`y` = world-vertical coordinate, same axis as
//! `height_at`'s output range) by **depth-from-surface layering**:
//!
//! - `depth = surface_height(x,z) − y`. `depth < 0` ⇒ above the surface ⇒ [`MaterialId::Air`].
//! - `depth == 0` ⇒ the surface voxel itself ⇒ a **biome-derived surface material** (e.g.
//!   desert → sand, tundra → permafrost; everything else → the baseline soil).
//! - `0 < depth <= SOIL_DEPTH` ⇒ [`MaterialId::Soil`] (generic subsoil).
//! - `depth > SOIL_DEPTH` ⇒ [`MaterialId::Bedrock`].
//!
//! This is the BASELINE only — W-4 (erosion) later refines material by transport/deposition and
//! `rock_resistance`; W-5 reads it for resource-caps. No null-material state ever exists downstream
//! of this slice. Integer, deterministic, arch-identical (composes `height_at` + `climate_at` +
//! `biome_at`, all pure integer functions).

use crate::gen::biome::{biome_at, BiomeId};
use crate::gen::climate::climate_at;
use crate::gen::height::height_at;

/// Baseline material at a voxel. Small `#[repr(u8)]` id, append-only (matches `BiomeId`'s idiom).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MaterialId {
    Air = 0,
    Sand = 1,
    Permafrost = 2,
    Soil = 3,
    Bedrock = 4,
}

/// Depth (in `height_at` integer units) below the surface at which subsoil transitions to bedrock.
const SOIL_DEPTH: i64 = 4;

/// Biome-derived surface material (depth==0 layer only). Everything not explicitly listed gets the
/// baseline [`MaterialId::Soil`] — matches `Guild::classify`'s default-fallthrough idiom.
fn surface_material_for_biome(b: BiomeId) -> MaterialId {
    match b {
        BiomeId::Desert => MaterialId::Sand,
        BiomeId::Tundra => MaterialId::Permafrost,
        _ => MaterialId::Soil,
    }
}

/// Deterministic baseline material at world voxel `(x, y, z)` for `seed`, reading the W-1
/// heightmap (`height_at`) for the surface elevation and the W-2 climate/biome
/// (`climate_at`/`biome_at`) for the surface material. Pure function — no RNG-of-clock, no
/// thread-dependence, no global mutable state; chunk-ready.
pub fn material_at(x: i64, y: i64, z: i64, seed: u64, hmax: i64) -> MaterialId {
    let surface_h = height_at(x, z, seed, hmax);
    let depth = surface_h - y;
    if depth < 0 {
        return MaterialId::Air;
    }
    if depth == 0 {
        let (t, p) = climate_at(x, z, seed, hmax);
        let biome = biome_at(t, p);
        return surface_material_for_biome(biome);
    }
    if depth <= SOIL_DEPTH {
        return MaterialId::Soil;
    }
    MaterialId::Bedrock
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: u64 = 0xA11A_2A11;
    const HMAX: i64 = 200;

    /// Re-run identity: the SAME `(x, y, z, seed, hmax)` always produces the SAME material.
    #[test]
    fn material_at_is_deterministic_across_repeated_calls() {
        for &(x, y, z) in &[(0i64, 0i64, 0i64), (-1, 50, -1), (37, 100, 5)] {
            let a = material_at(x, y, z, SEED, HMAX);
            let b = material_at(x, y, z, SEED, HMAX);
            assert_eq!(a, b, "material_at({x},{y},{z}) must be byte-identical across repeated calls");
        }
    }

    /// Depth-from-surface layering: strictly above the surface is Air; strictly deep is Bedrock.
    #[test]
    fn material_at_layers_air_above_and_bedrock_deep() {
        let (x, z) = (10i64, 10i64);
        let surface_h = height_at(x, z, SEED, HMAX);
        assert_eq!(
            material_at(x, surface_h + 1, z, SEED, HMAX),
            MaterialId::Air,
            "one voxel above the surface must be Air"
        );
        assert_eq!(
            material_at(x, surface_h - (SOIL_DEPTH + 10), z, SEED, HMAX),
            MaterialId::Bedrock,
            "well below SOIL_DEPTH must be Bedrock"
        );
        assert_eq!(
            material_at(x, surface_h - 1, z, SEED, HMAX),
            MaterialId::Soil,
            "just below the surface (within SOIL_DEPTH) must be Soil"
        );
    }

    /// The surface voxel's material must match the biome-derived mapping — consistency check
    /// against the SAME `climate_at`+`biome_at` composition `material_at` uses internally.
    #[test]
    fn material_at_surface_matches_biome_derived_mapping() {
        for x in (0..200i64).step_by(23) {
            let z = 0i64;
            let surface_h = height_at(x, z, SEED, HMAX);
            let (t, p) = climate_at(x, z, SEED, HMAX);
            let biome = biome_at(t, p);
            let expected = surface_material_for_biome(biome);
            assert_eq!(material_at(x, surface_h, z, SEED, HMAX), expected);
        }
    }

    /// Golden vector (material over a fixture): pinned exact material at explicit `(x, y, z)`
    /// voxels, including an above-surface (Air), surface (biome-derived), mid-depth (Soil), and
    /// deep (Bedrock) case at each fixture coordinate.
    #[test]
    fn golden_vector_matches_pinned_material() {
        const GOLDEN_SEED: u64 = 0xA11A_2A11;
        const GOLDEN_HMAX: i64 = 200;
        // (x, z, y_offset_from_surface, expected) — y_offset is surface_h + offset (offset=0 is
        // the surface voxel itself; positive = above; negative = below).
        const CASES: &[(i64, i64, i64, MaterialId)] = &[
            (0, 0, 1, MaterialId::Air),
            (0, 0, 0, MaterialId::Soil),
            (0, 0, -2, MaterialId::Soil),
            (0, 0, -20, MaterialId::Bedrock),
            (-1000, 2000, 0, MaterialId::Permafrost),
            (1_000_000_000, -1_000_000_000, 0, MaterialId::Permafrost),
        ];
        for &(x, z, y_offset, expected) in CASES {
            let surface_h = height_at(x, z, GOLDEN_SEED, GOLDEN_HMAX);
            let y = surface_h + y_offset;
            let m = material_at(x, y, z, GOLDEN_SEED, GOLDEN_HMAX);
            assert_eq!(m, expected, "golden drift at (x={x},z={z},y_offset={y_offset}): got {m:?}, expected {expected:?}");
        }
    }
}
