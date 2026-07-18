//! W-4 cross-stage chain-golden (phase plan, extending the `w2_chain.rs`/`w3_chain.rs` convention)
//! — the phase's SECOND full-grid golden and second GLOBAL flow stage, now ITERATIVE (erosion
//! depends on the entire upstream basin AND all `MACRO_ITERATIONS` macro-iterations).
//!
//! Pins the full post-erosion field `height → erosion → post-erosion height + material + drainage`
//! over a documented `const W4_CHAIN_DIM = 64` (matching the current prod default) on a fixed seed
//! — a test constant, NOT read from any config (that would break golden-neutrality; critic F3).
//!
//! Also hosts the **1-vs-N thread-determinism gate — LOAD-BEARING here** (unlike W-3's inherently
//! serial Priority-Flood): `incision_step`/`talus_step`/`accumulate_and_export` are per-cell Jacobi
//! passes that would be embarrassingly parallel if `erode` were ever threaded — this gate is the
//! tooth that would catch a scatter-race or a non-associative sediment-gather regression, not a
//! trivial pass like W-3's.

use world::gen::drainage::DrainageState;
use world::gen::erosion::{erode, ErosionState};
use world::gen::height::height_at;
use world::gen::material::MaterialId;

const CHAIN_SEED: u64 = 0xA11A_2A11;
const CHAIN_HMAX: i64 = 200;
/// The golden grid dimension — a TEST CONSTANT, not read from any config (critic F3). Matches the
/// current prod default (64×64); a later slice changing production map size does NOT re-pin this.
const W4_CHAIN_DIM: usize = 64;

/// Canonical fold of the full post-erosion state into one `u64`, reusing `sim-core`'s
/// `fnv_mix`/`FNV_OFFSET` primitive (the same one `Sim::state_hash`/`w3_chain.rs` use) so a drift
/// ANYWHERE in the 4096-cell field changes this hash. `downstream: None` folds as `u64::MAX` (a
/// real index is always `< dim*dim`, so this sentinel can never collide).
#[allow(clippy::needless_range_loop)] // several parallel slices indexed by the same `i` — clearer than an N-way zip
fn chain_hash(state: &ErosionState) -> u64 {
    use sim_core::{fnv_mix, FNV_OFFSET};
    let mut h = FNV_OFFSET;
    let d: &DrainageState = &state.drainage;
    for i in 0..state.dim * state.dim {
        h = fnv_mix(h, state.height[i] as u64);
        h = fnv_mix(h, state.surface_material[i] as u8 as u64);
        h = fnv_mix(h, d.filled[i] as u64);
        h = fnv_mix(h, d.downstream[i].map(|x| x as u64).unwrap_or(u64::MAX));
        h = fnv_mix(h, d.area[i] as u64);
    }
    fnv_mix(h, state.export_total as u64)
}

/// Re-run identity: the full erosion chain is byte-identical across repeated calls, at prod scale.
#[test]
fn chain_is_deterministic_across_repeated_calls() {
    let a = erode(CHAIN_SEED, CHAIN_HMAX, W4_CHAIN_DIM, true, false, false, false, true);
    let b = erode(CHAIN_SEED, CHAIN_HMAX, W4_CHAIN_DIM, true, false, false, false, true);
    assert_eq!(a, b, "erode must be byte-identical across repeated calls at prod scale");
}

/// **1-vs-N thread-determinism gate — LOAD-BEARING for W-4** (see module doc): `erode` must produce
/// a BYTE-IDENTICAL result whether invoked from the main thread alone or concurrently from N
/// `std::thread::spawn` workers. Unlike W-3's inherently-serial Priority-Flood, W-4's per-cell
/// incision/talus/sediment-gather passes are the kind of code that WOULD race under a careless
/// parallel port — this test is the tooth that catches that regression.
#[test]
fn erosion_is_thread_count_independent_1_vs_n() {
    let baseline = erode(CHAIN_SEED, CHAIN_HMAX, W4_CHAIN_DIM, true, false, false, false, true);

    const N: usize = 4;
    let handles: Vec<_> = (0..N)
        .map(|_| std::thread::spawn(|| erode(CHAIN_SEED, CHAIN_HMAX, W4_CHAIN_DIM, true, false, false, false, true)))
        .collect();
    for h in handles {
        let result = h.join().expect("worker thread must not panic");
        assert_eq!(result, baseline, "erode must be byte-identical under N-thread concurrent invocation");
    }
}

/// Sediment conservation at prod scale: `Σheight + export_total == initial Σheight` exactly, over
/// the FULL `W4_CHAIN_DIM` grid and all `MACRO_ITERATIONS` — not just the small fixtures in
/// `erosion.rs`'s own unit tests.
#[test]
fn erosion_conserves_sediment_at_prod_scale() {
    let state = erode(CHAIN_SEED, CHAIN_HMAX, W4_CHAIN_DIM, true, false, false, false, true);
    let mut initial_height = vec![0i64; W4_CHAIN_DIM * W4_CHAIN_DIM];
    for z in 0..W4_CHAIN_DIM {
        for x in 0..W4_CHAIN_DIM {
            initial_height[z * W4_CHAIN_DIM + x] = height_at(x as i64, z as i64, CHAIN_SEED, CHAIN_HMAX);
        }
    }
    let initial_sum: i64 = initial_height.iter().sum();
    let final_sum: i64 = state.height.iter().sum();
    assert_eq!(
        final_sum + state.export_total,
        initial_sum,
        "Σheight + export must equal the initial Σheight exactly at prod scale"
    );
}

/// The cross-stage chain-golden itself: a single canonical hash of the full post-erosion
/// `height`/`surface_material`/`drainage` state at prod scale (`W4_CHAIN_DIM=64`), PLUS a handful
/// of individual spot-check cells for human-readable debuggability. A cross-stage bug (a
/// W-1/W-3-consumed-wrong sample, or an incision/talus/material encoding mistake) reddens HERE.
#[test]
fn w4_chain_golden_erosion_post_state() {
    let state = erode(CHAIN_SEED, CHAIN_HMAX, W4_CHAIN_DIM, true, false, false, false, true);

    // Sanity: post-erosion height must never exceed the RAW pre-erosion height at that position
    // (erosion + talus can only remove/redistribute mass downhill, never inject height above the
    // original terrain — a receiving talus neighbor CAN rise, so this per-cell monotonicity does
    // NOT hold pointwise; instead assert the GLOBAL sum invariant, already covered by the
    // conservation test above). Here we just confirm the field is fully populated and finite-range.
    assert_eq!(state.height.len(), W4_CHAIN_DIM * W4_CHAIN_DIM);
    assert!(state.height.iter().all(|&h| h >= 0), "post-erosion height must never go negative");

    const GOLDEN_HASH: u64 = 0xB0DC_CE2C_7731_4358;
    let hash = chain_hash(&state);
    assert_eq!(hash, GOLDEN_HASH, "W-4 chain golden drift: got {hash:#018x}, expected {GOLDEN_HASH:#018x}");

    const SPOT_CASES: &[(usize, i64, MaterialId)] = &[
        (0, 129, MaterialId::Soil),
        (1000, 69, MaterialId::Soil),
        (2079, 97, MaterialId::Soil),
        (4095, 117, MaterialId::Soil),
    ];
    for &(idx, exp_height, exp_material) in SPOT_CASES {
        assert_eq!(state.height[idx], exp_height, "spot-check drift: height[{idx}]");
        assert_eq!(state.surface_material[idx], exp_material, "spot-check drift: surface_material[{idx}]");
    }
}
