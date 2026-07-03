//! W-3 cross-stage chain-golden (phase plan, extending the `w2_chain.rs` convention) — the phase's
//! FIRST full-grid golden, since W-3 is the first GLOBAL FLOW stage (drainage accumulation depends
//! on the entire upstream basin, unlike W-1/W-2's per-position pure fns).
//!
//! Pins the full chain `height → drainage → moisture` over a documented `const W3_CHAIN_DIM = 64`
//! (64×64, matching the current prod default) on a fixed seed — a test constant, NOT read from any
//! config (that would break golden-neutrality; critic F3): "the golden grid; prod may differ, this
//! pins the algorithm."
//!
//! Pinning every one of the 4096 individual cell values by hand is impractical, so — mirroring
//! `sim-core`'s own `state_hash` convention (`fnv_mix`/`FNV_OFFSET` fold) — this file pins a SINGLE
//! canonical hash of the whole field (any drift anywhere in the 4096 cells changes the hash), plus
//! a small number of individual spot-check values for human-readable debuggability.
//!
//! Also hosts the **1-vs-N thread-determinism gate** (critic requirement for a global-flow stage):
//! `compute_drainage` must be byte-identical whether called from 1 thread or from N concurrent
//! threads — trivially true since Priority-Flood + Kahn are inherently serial pure functions with
//! no shared mutable state, but this is the load-bearing guard if any part is later parallelized.

use world::gen::drainage::{compute_drainage, DrainageState};
use world::gen::height::height_at;
use world::gen::moisture::moisture_field;

const CHAIN_SEED: u64 = 0xA11A_2A11;
const CHAIN_HMAX: i64 = 200;
/// The golden grid dimension — a TEST CONSTANT, not read from any config (critic F3). Matches the
/// current prod default (64×64); a later slice changing production map size does NOT re-pin this.
const W3_CHAIN_DIM: usize = 64;

/// Canonical fold of the full drainage + moisture field into one `u64`, reusing `sim-core`'s
/// `fnv_mix`/`FNV_OFFSET` primitive (the same one `Sim::state_hash` uses) so a drift ANYWHERE in
/// the 4096-cell field changes this hash. `downstream: None` is folded as `u64::MAX` (a real
/// downstream index is always `< dim*dim`, so this sentinel can never collide).
#[allow(clippy::needless_range_loop)] // four parallel slices indexed by the same `i` — clearer than a 4-way zip
fn chain_hash(state: &DrainageState, moisture: &[i64]) -> u64 {
    use sim_core::{fnv_mix, FNV_OFFSET};
    let mut h = FNV_OFFSET;
    for i in 0..state.dim * state.dim {
        h = fnv_mix(h, state.filled[i] as u64);
        h = fnv_mix(h, state.downstream[i].map(|d| d as u64).unwrap_or(u64::MAX));
        h = fnv_mix(h, state.area[i] as u64);
        h = fnv_mix(h, moisture[i] as u64);
    }
    h
}

/// Re-run identity: the full chain is byte-identical across repeated calls, at prod scale.
#[test]
fn chain_is_deterministic_across_repeated_calls() {
    let a = compute_drainage(CHAIN_SEED, CHAIN_HMAX, W3_CHAIN_DIM);
    let b = compute_drainage(CHAIN_SEED, CHAIN_HMAX, W3_CHAIN_DIM);
    assert_eq!(a, b, "compute_drainage must be byte-identical across repeated calls at prod scale");

    let ma = moisture_field(&a.area);
    let mb = moisture_field(&b.area);
    assert_eq!(ma, mb);
}

/// **1-vs-N thread-determinism gate.** `compute_drainage` (a pure function, no shared mutable
/// state) must produce a BYTE-IDENTICAL result whether invoked from the main thread alone or
/// concurrently from N `std::thread::spawn` workers — the guard the phase plan requires for any
/// GLOBAL flow stage (mirrors the sim's own R14 1-vs-N golden).
#[test]
fn drainage_is_thread_count_independent_1_vs_n() {
    let baseline = compute_drainage(CHAIN_SEED, CHAIN_HMAX, W3_CHAIN_DIM);
    let baseline_moisture = moisture_field(&baseline.area);

    const N: usize = 4;
    let handles: Vec<_> = (0..N)
        .map(|_| std::thread::spawn(|| compute_drainage(CHAIN_SEED, CHAIN_HMAX, W3_CHAIN_DIM)))
        .collect();
    for h in handles {
        let result = h.join().expect("worker thread must not panic");
        assert_eq!(result, baseline, "drainage must be byte-identical under N-thread concurrent invocation");
        let m = moisture_field(&result.area);
        assert_eq!(m, baseline_moisture);
    }
}

/// The cross-stage chain-golden itself: a single canonical hash of the full
/// `height → drainage → moisture` field at prod scale (`W3_CHAIN_DIM=64`), PLUS a handful of
/// individual spot-check cell values for human-readable debuggability. A cross-stage bug (a
/// W-1-consumed-wrong height sample, or a drainage/moisture encoding mistake) reddens HERE.
#[test]
fn w3_chain_golden_height_drainage_moisture() {
    let state = compute_drainage(CHAIN_SEED, CHAIN_HMAX, W3_CHAIN_DIM);
    let moisture = moisture_field(&state.area);

    // Re-derive the height field independently (via `height_at` directly) to prove `compute_drainage`
    // actually consumed W-1's heightmap correctly, not some other source.
    for z in 0..W3_CHAIN_DIM {
        for x in 0..W3_CHAIN_DIM {
            let h = height_at(x as i64, z as i64, CHAIN_SEED, CHAIN_HMAX);
            let idx = z * W3_CHAIN_DIM + x;
            // filled >= raw height always (Priority-Flood only ever RAISES a cell, never lowers).
            assert!(state.filled[idx] >= h, "filled elevation must never be below the raw height at idx={idx}");
        }
    }

    const GOLDEN_HASH: u64 = 0x648E_7676_9B6B_9170;
    let hash = chain_hash(&state, &moisture);
    assert_eq!(hash, GOLDEN_HASH, "W-3 chain golden drift: got {hash:#018x}, expected {GOLDEN_HASH:#018x}");

    // Spot-check a handful of individual cells for human-readable debuggability alongside the hash.
    const SPOT_CASES: &[(usize, i64, Option<usize>, i64, i64)] = &[
        // (linear_index, expected_filled, expected_downstream, expected_area, expected_moisture)
        (0, 130, Some(65), 1, 3),
        (1000, 81, Some(937), 882, 1000), // a river cell (area > RIVER_THRESHOLD=32), saturated moisture
        (2079, 101, Some(2014), 1, 3),
        (4095, 117, Some(4031), 1, 3),
    ];
    for &(idx, exp_filled, exp_down, exp_area, exp_moist) in SPOT_CASES {
        assert_eq!(state.filled[idx], exp_filled, "spot-check drift: filled[{idx}]");
        assert_eq!(state.downstream[idx], exp_down, "spot-check drift: downstream[{idx}]");
        assert_eq!(state.area[idx], exp_area, "spot-check drift: area[{idx}]");
        assert_eq!(moisture[idx], exp_moist, "spot-check drift: moisture[{idx}]");
    }
}

/// Acyclicity at prod scale: `compute_drainage` (which calls `kahn_accumulate` internally, which
/// `assert!`s completion) must not panic at the full `W3_CHAIN_DIM` grid — the real height-derived
/// surface, not just the small synthetic fixtures in `drainage.rs`'s unit tests.
#[test]
fn chain_completes_without_cycle_at_prod_scale() {
    let state = compute_drainage(CHAIN_SEED, CHAIN_HMAX, W3_CHAIN_DIM);
    assert_eq!(state.area.len(), W3_CHAIN_DIM * W3_CHAIN_DIM);
    assert!(state.area.iter().all(|&a| a >= 1));
}
