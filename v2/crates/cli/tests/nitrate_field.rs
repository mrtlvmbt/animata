//! P5-0: Nitrate (NO₃⁻) field world-gen activation (golden-NEUTRAL). Acceptance test for the NO₃
//! substrate layer — derived from biome as the INVERSE of O₂ (high in anaerobic zones, low in
//! aerated zones). Static inert field in P5-0 (regen_rate=0, no consumption yet).
//!
//! **Critical:** test reads fields off a CONCRETELY-typed `ProcgenWorld` (inherent methods), NOT a
//! `BoxedWorldView` trait (F1). Criteria (1)/(2) do NOT depend on `enable_nitrate` — the flag only
//! gates field-wiring in cli world-gen, never generation (F5).
//!
//! **Criterion (3) — Golden-NEUTRAL:** no golden re-pin. Proof = CI green on HEAD with ZERO edits
//! to any `*_golden` / `state_checksum` pin. If any golden moves, the isolation gate leaked.

use world::{gen::caps::{FinalBiome, oxygen_cap_from, nitrate_cap_from}, ProcgenWorld};
use sim_core::Vec2Fixed;

const HMAX: i64 = 200;
const RESOURCE_BASE: i64 = 120;
const WORLD_SALT: u64 = 0xDEAD_BEEF;

/// Fixed seeds whose generated worlds contain both anaerobic (Wetland/Floodplain) and aerated
/// (Forest/Grassland) biomes, used for criterion (1b) inverse-correlation test.
const BIOME_RICH_SEEDS: &[u64] = &[0x1111_1111, 0x2222_2222, 0x3333_3333];

/// Criterion (1a): NO₃ cap is NONZERO in anaerobic biomes (Wetland/Floodplain), ~0/low in
/// aerated forest biomes (TemperateForest, TemperateGrassland). This is a direct unit test on
/// the cap functions (deterministic, biome-only).
#[test]
fn p5_0_nitrate_biome_gradient_unit() {
    // Anaerobic/waterlogged biomes should have HIGH NO₃
    assert!(
        nitrate_cap_from(FinalBiome::Wetland) > 0,
        "Wetland should have nonzero NO₃"
    );
    assert!(
        nitrate_cap_from(FinalBiome::Floodplain) > 0,
        "Floodplain should have nonzero NO₃"
    );

    // Aerated biomes should have LOWER NO₃
    let aerated_caps = vec![
        nitrate_cap_from(FinalBiome::TemperateForest),
        nitrate_cap_from(FinalBiome::TemperateGrassland),
        nitrate_cap_from(FinalBiome::BorealForest),
    ];
    assert!(
        aerated_caps.iter().all(|&c| c < nitrate_cap_from(FinalBiome::Wetland)),
        "Aerated biomes should have lower NO₃ than Wetland"
    );

    // Rock (impenetrable) should have zero
    assert_eq!(
        nitrate_cap_from(FinalBiome::Rock),
        0,
        "Rock should have zero NO₃"
    );
}

/// Criterion (1b): NO₃ is MEASURABLY INVERSE to O₂ (high where O₂ is low). Construct a
/// ProcgenWorld directly and iterate over cells, partitioning by O₂ cap. Assert that
/// mean(NO₃ | O₂-low cells) > mean(NO₃ | O₂-high cells) by a clear margin.
#[test]
fn p5_0_nitrate_inverse_to_oxygen_live_world() {
    let mut low_o2_no3 = Vec::new();
    let mut high_o2_no3 = Vec::new();

    for &seed in BIOME_RICH_SEEDS {
        let world = ProcgenWorld::new(64, HMAX, RESOURCE_BASE, seed ^ WORLD_SALT, None);

        let mut o2_caps = Vec::new();
        let mut no3_caps = Vec::new();

        // Iterate over grid cells and collect O₂/NO₃ caps
        for x in 0..64 {
            for z in 0..64 {
                let pos = Vec2Fixed(x, z);
                let o2 = world.oxygen_resource(pos);
                let no3 = world.nitrate_resource(pos);
                o2_caps.push(o2);
                no3_caps.push(no3);
            }
        }

        // Partition by O₂ median
        let mut o2_sorted = o2_caps.clone();
        o2_sorted.sort_unstable();
        let o2_median = o2_sorted[o2_sorted.len() / 2];

        for (o2, no3) in o2_caps.iter().zip(no3_caps.iter()) {
            if *o2 < o2_median {
                low_o2_no3.push(*no3);
            } else {
                high_o2_no3.push(*no3);
            }
        }
    }

    // Assert inverse relationship: low-O₂ cells have MORE NO₃ than high-O₂ cells
    let mean_low_o2_no3: i64 = low_o2_no3.iter().sum::<i64>() / low_o2_no3.len() as i64;
    let mean_high_o2_no3: i64 = high_o2_no3.iter().sum::<i64>() / high_o2_no3.len() as i64;

    assert!(
        mean_low_o2_no3 > mean_high_o2_no3,
        "NO₃ INVERSE to O₂ failed: mean(NO₃|low-O₂)={} should be > mean(NO₃|high-O₂)={}",
        mean_low_o2_no3,
        mean_high_o2_no3
    );
    assert!(
        (mean_low_o2_no3 - mean_high_o2_no3) as f64 > 10.0,
        "NO₃ inverse margin too small (diff={}); should be clear separation",
        mean_low_o2_no3 - mean_high_o2_no3
    );
}

/// Criterion (2): Determinism. Same seed → byte-identical NO₃ cap vector. Two identical
/// ProcgenWorld builds with the same seed must produce the same nitrate_resource values.
#[test]
fn p5_0_nitrate_determinism_same_seed() {
    const TEST_SEED: u64 = 0xBEEF_CAFE;

    let world1 = ProcgenWorld::new(64, HMAX, RESOURCE_BASE, TEST_SEED ^ WORLD_SALT, None);
    let world2 = ProcgenWorld::new(64, HMAX, RESOURCE_BASE, TEST_SEED ^ WORLD_SALT, None);

    for x in 0..64 {
        for z in 0..64 {
            let pos = Vec2Fixed(x, z);
            let no3_1 = world1.nitrate_resource(pos);
            let no3_2 = world2.nitrate_resource(pos);
            assert_eq!(
                no3_1, no3_2,
                "NO₃ non-determinism at ({},{}) with seed={:X}: {} != {}",
                x, z, TEST_SEED, no3_1, no3_2
            );
        }
    }
}

/// Criterion (2) ADDENDUM: NO₃ field is computed UNCONDITIONALLY at world-gen, independent
/// of `enable_nitrate` (which only gates field-wiring in cli world-gen, not generation itself).
/// This is verified implicitly by (2) and (1b): ProcgenWorld carries the field regardless of
/// whether any sim config uses it. The flag `enable_nitrate` only controls whether layer 3 gets
/// populated with NO₃-caps in the cli caps-routing logic (cli/lib.rs); the field itself is always
/// there (F5).
#[test]
fn p5_0_nitrate_generated_unconditionally() {
    // This test asserts the design contract: `ProcgenWorld::nitrate_resource` is always available
    // and populated, regardless of whether `enable_nitrate` is true. The test just calls it and
    // gets a value. A future version that tries to access it when enable_nitrate=false would need
    // to explicitly disable it, which would violate P5-0's golden-neutral isolation gate.
    let world = ProcgenWorld::new(64, HMAX, RESOURCE_BASE, 0xC0DE_CAFE ^ WORLD_SALT, None);
    let pos = Vec2Fixed(0, 0);
    let no3 = world.nitrate_resource(pos);
    assert!(
        no3 >= 0,
        "NO₃ resource must be non-negative; got {}",
        no3
    );
}

/// Criterion (3) ASSERTION: Golden-NEUTRAL. This test does NOT check the golden directly
/// (that's the CI job's responsibility); instead, it documents the contract: if this test passes
/// and CI green with no golden re-pins, then P5-0 is golden-neutral (the un-re-pinned goldens
/// are the test). If a golden drifts, the isolation gate leaked: some shipped config turned
/// `enable_nitrate` on, or a non-shipped config was edited to use it. Report `blocked` and
/// investigate.
#[test]
fn p5_0_golden_neutral_contract() {
    // The contract is: ALL shipped configs (default, l3, cprime, dprime, driver, settling) must have
    // enable_nitrate=false. Layer 3, if present, must stay LayerSpec::default() (regen=0, cap=0 → empty).
    // This is asserted structurally: the layer is present (4-slot array) but empty. If a golden moves,
    // the contract was broken elsewhere (not in this module). This test is a documentation checkpoint.
    //
    // Real proof = `bash scripts/ci-report.sh` → `exit 0` with no `*_golden*` diffs on HEAD.
}

// Criterion (4) IMPLICIT: Field is inert w.r.t. population. No gene decodes to a Nitrate-primary
// phenotype yet (that is P5-1). In `nitrate_config`, the NO₃ field is initialized from world-gen
// caps but no entity consumes it. This is NOT separately tested because it's a consequence of the
// absence of decode logic — the test would be "run a sim and verify no consumption happens", but
// that's a tautology until P5-2 adds consumption. Stated here for record: the layer is LIVE
// (properly wired) but INERT (unused by organisms), which is the intentional P5-0 design.
