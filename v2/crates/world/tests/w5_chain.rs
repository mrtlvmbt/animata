//! W-5 cross-stage chain-golden (phase plan, extending the `w2_chain.rs`/`w3_chain.rs`/`w4_chain.rs`
//! convention) — the LAST substrate slice's chain-golden. Pins the FULL final classification
//! `height → erosion → final-biome → caps` over a documented `const W5_CHAIN_DIM = 64` /
//! `const W5_CHAIN_HMAX = 200` (matching the current prod default) on a fixed seed — test
//! constants, NOT read from any config (that would break golden-neutrality; critic F3). **The SAME
//! `W5_CHAIN_HMAX` threads through every stage of the chain (critic F5)** — a mismatched `hmax`
//! between the erosion stage and the climate-reclassification stage would silently diverge the
//! pinned vector, since `classify_and_caps` calls `erode(seed, hmax, dim)` internally with this
//! one value.
//!
//! No 1-vs-N gate here (per the phase plan): W-5 is a per-position pure classification/caps
//! function over PRECOMPUTED W-4 fields, not a new global iterative flow — the global determinism
//! was already gated at W-3 (drainage) and W-4 (erosion, load-bearing there).

use world::gen::caps::{classify_and_caps, FinalBiome};
use world::gen::height::height_at;

const CHAIN_SEED: u64 = 0xA11A_2A11;
/// The single `hmax` threaded through EVERY stage of this chain (critic F5) — erosion, climate
/// reclassification, everything. A TEST constant, not read from any config (critic F3).
const W5_CHAIN_HMAX: i64 = 200;
/// The golden grid dimension — a TEST constant (critic F3), matching the current prod default.
const W5_CHAIN_DIM: usize = 64;

/// Canonical fold of the full final-classification field into one `u64`, reusing `sim-core`'s
/// `fnv_mix`/`FNV_OFFSET` primitive (the same one `Sim::state_hash`/`w3_chain.rs`/`w4_chain.rs`
/// use) so a drift ANYWHERE in the 4096-cell field changes this hash.
fn chain_hash(fields: &world::gen::caps::WorldFields) -> u64 {
    use sim_core::{fnv_mix, FNV_OFFSET};
    let mut h = FNV_OFFSET;
    for i in 0..fields.dim * fields.dim {
        h = fnv_mix(h, fields.final_biome[i] as u8 as u64);
        h = fnv_mix(h, fields.caps[i] as u64);
    }
    h
}

/// Re-run identity: the full chain is byte-identical across repeated calls, at prod scale.
#[test]
fn chain_is_deterministic_across_repeated_calls() {
    let a = classify_and_caps(CHAIN_SEED, W5_CHAIN_HMAX, W5_CHAIN_DIM, false, false, false, false, false, false);
    let b = classify_and_caps(CHAIN_SEED, W5_CHAIN_HMAX, W5_CHAIN_DIM, false, false, false, false, false, false);
    assert_eq!(a, b, "classify_and_caps must be byte-identical across repeated calls at prod scale");
}

/// Bounded-caps property, at prod scale — includes any interior-sink cells (critic F7: they
/// classify normally on their low local moisture, no special case, no crash).
#[test]
fn caps_are_nonneg_and_bounded_at_prod_scale() {
    let fields = classify_and_caps(CHAIN_SEED, W5_CHAIN_HMAX, W5_CHAIN_DIM, false, false, false, false, false, false);
    for (i, &c) in fields.caps.iter().enumerate() {
        assert!(
            (0..=world::gen::caps::CAP_MAX).contains(&c),
            "cap at idx={i} is {c}, out of [0,{}]", world::gen::caps::CAP_MAX
        );
    }
}

/// The cross-stage chain-golden itself: a single canonical hash of the full final `BiomeId`/caps
/// field at prod scale, PLUS a handful of individual spot-check cells for human-readable
/// debuggability. A cross-stage bug (a wrong `hmax` thread, a border-rule slip, a
/// climate/override/caps encoding mistake) reddens HERE.
#[test]
fn w5_chain_golden_final_biome_and_caps() {
    let fields = classify_and_caps(CHAIN_SEED, W5_CHAIN_HMAX, W5_CHAIN_DIM, false, false, false, false, false, false);

    // Sanity: the entry point actually consumed W-1's heightmap (indirectly, via erosion) — the
    // grid is fully populated at the documented dim.
    assert_eq!(fields.final_biome.len(), W5_CHAIN_DIM * W5_CHAIN_DIM);
    assert_eq!(fields.caps.len(), W5_CHAIN_DIM * W5_CHAIN_DIM);
    let _ = height_at(0, 0, CHAIN_SEED, W5_CHAIN_HMAX); // same seed/hmax family as the chain

    // W-7 gate (patchiness default-off): hash reverts to pre-W-7 byte-identical value.
    // Height/biome fields unchanged; spot values are canonical pre-patchiness values.
    const GOLDEN_HASH: u64 = 0x2705_C8AE_0DE7_1117;
    let hash = chain_hash(&fields);
    assert_eq!(hash, GOLDEN_HASH, "W-5 chain golden drift: got {hash:#018x}, expected {GOLDEN_HASH:#018x}");

    const SPOT_CASES: &[(usize, FinalBiome, i64)] = &[
        (0, FinalBiome::BorealForest, 220),
        (1000, FinalBiome::Floodplain, 288),
        (2079, FinalBiome::TemperateGrassland, 180),
        (4095, FinalBiome::BorealForest, 220),
    ];
    for &(idx, exp_biome, exp_cap) in SPOT_CASES {
        assert_eq!(fields.final_biome[idx], exp_biome, "spot-check drift: final_biome[{idx}]");
        assert_eq!(fields.caps[idx], exp_cap, "spot-check drift: caps[{idx}]");
    }
}
