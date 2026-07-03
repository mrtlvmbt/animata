//! W-2 cross-stage chain-golden (W-1 cold-review F2) — the phase's per-slice chain-golden
//! convention, established here for W-3+ to extend. Pins the FULL chain-so-far end-to-end:
//! `height → (T,P) → BiomeId → material`, over an EXPLICIT fixed `(seed, hmax, coordinate-set)`
//! chosen IN this test.
//!
//! **Independent of any config's `world_dim` (critic F2):** height/climate/biome/material are all
//! per-position PURE functions (like W-1's `height_at`), so this vector uses an explicit coord
//! list, NOT "the prod grid" — a later slice changing map size never re-pins W-2.
//!
//! The prod-scale + 1-vs-N chain-golden requirement in the phase plan applies to the GLOBAL flow
//! stages W-3/W-4, which read the whole field, not W-2's per-position fns.
//!
//! This catches a cross-stage bug AT this slice — a W-1-consumed-wrong, or a climate/biome/
//! material encoding bug — reddening HERE, not silently propagating to W-6. W-3+ extend this
//! chain in their own per-slice files (e.g. `w3_chain.rs` reading `w2_chain`'s coordinate set +
//! adding hydrology fields), per the convention this file establishes.

use world::gen::biome::{biome_at, BiomeId};
use world::gen::climate::climate_at;
use world::gen::height::height_at;
use world::gen::material::{material_at, MaterialId};

const CHAIN_SEED: u64 = 0xA11A_2A11;
const CHAIN_HMAX: i64 = 200;

/// Explicit fixed coordinate set: origin, small positive/negative, a large-magnitude pair, and an
/// arbitrary interior point — the SAME discipline `height.rs`'s own golden vector uses (critic F12:
/// negative/large coordinates are actually exercised, not just the positive quadrant).
const COORDS: &[(i64, i64)] = &[(0, 0), (1, 1), (-1, -1), (-1000, 2000), (1_000_000_000, -1_000_000_000), (37, 5)];

/// Full chain pinned per coordinate: `(x, z, expected_height, expected_t, expected_p,
/// expected_biome, expected_surface_material)`.
const CHAIN: &[(i64, i64, i64, i64, i64, BiomeId, MaterialId)] = &[
    (0, 0, 130, 243, 911, BiomeId::BorealForest, MaterialId::Soil),
    (1, 1, 129, 203, 892, BiomeId::BorealForest, MaterialId::Soil),
    (-1, -1, 130, 149, 848, BiomeId::BorealForest, MaterialId::Soil),
    (-1000, 2000, 146, -2803, 831, BiomeId::Tundra, MaterialId::Permafrost),
    (1_000_000_000, -1_000_000_000, 98, -1647, 892, BiomeId::Tundra, MaterialId::Permafrost),
    (37, 5, 93, 590, 895, BiomeId::TemperateGrassland, MaterialId::Soil),
];

/// Re-run identity: the full chain composition is byte-identical across repeated calls.
#[test]
fn chain_is_deterministic_across_repeated_calls() {
    for &(x, z) in COORDS {
        let h1 = height_at(x, z, CHAIN_SEED, CHAIN_HMAX);
        let h2 = height_at(x, z, CHAIN_SEED, CHAIN_HMAX);
        assert_eq!(h1, h2);

        let c1 = climate_at(x, z, CHAIN_SEED, CHAIN_HMAX);
        let c2 = climate_at(x, z, CHAIN_SEED, CHAIN_HMAX);
        assert_eq!(c1, c2);

        let b1 = biome_at(c1.0, c1.1);
        let b2 = biome_at(c2.0, c2.1);
        assert_eq!(b1, b2);

        let m1 = material_at(x, h1, z, CHAIN_SEED, CHAIN_HMAX);
        let m2 = material_at(x, h2, z, CHAIN_SEED, CHAIN_HMAX);
        assert_eq!(m1, m2);
    }
}

/// The cross-stage chain-golden itself: `height → (T,P) → BiomeId → material` at each fixed coord
/// must match the pinned exact values. A cross-stage bug (W-1-consumed-wrong, or a climate/biome/
/// material encoding mistake) reddens HERE.
#[test]
fn w2_chain_golden_height_climate_biome_material() {
    for &(x, z, exp_h, exp_t, exp_p, exp_biome, exp_material) in CHAIN {
        let h = height_at(x, z, CHAIN_SEED, CHAIN_HMAX);
        assert_eq!(h, exp_h, "chain golden drift at ({x},{z}): height");

        let (t, p) = climate_at(x, z, CHAIN_SEED, CHAIN_HMAX);
        assert_eq!((t, p), (exp_t, exp_p), "chain golden drift at ({x},{z}): climate (T,P)");

        let biome = biome_at(t, p);
        assert_eq!(biome, exp_biome, "chain golden drift at ({x},{z}): biome");

        let material = material_at(x, h, z, CHAIN_SEED, CHAIN_HMAX);
        assert_eq!(material, exp_material, "chain golden drift at ({x},{z}): surface material");
    }
}
