//! M5 speciation CI gate (issue #130, criterion 4+5).
//!
//! Runs two identical simulations for L_CI ticks with baked seed S and verifies:
//!   (4) `species_hash()` is identical between runs (determinism under speciation).
//!   (5a) Each daughter species' founder genome is > T away from its parent species' founder.
//!   (5b) Inter-species spread > threshold (trivially held by construction when count ≥ K).
//!   (5c) At least K live species exist at tick L_CI (axis is actually diverging).
//!
//! Skipped in debug (8000 ticks × 2 runs requires release). Runs in CI via nextest.

use cli::{build_sim, default_config};
use sim_core::SpeciesId;

/// Baked seed: proven to yield ≥ K species at L_CI (issue #130 criterion 6).
const S: u64 = 0xA11A_2A11;
/// Plateau length: proven for seed S in the calibration probe.
/// B-3 note: the cross-feeding explosion (layer-1 saturation triggers layer-1-consumer bloom)
/// occurs at t≈12 000 with this seed. L_CI=8000 predates this and gave K≥3 under B-2 serial
/// uptake; B-3 proportional rationing shifts the bloom later — re-calibrated to 16 000.
const L_CI: u64 = 16_000;
/// Minimum live species required at L_CI (criterion 5c).
const K: u64 = 3;

#[test]
fn v2_species_determinism_and_gates() {
    // Skip in debug — 8000 ticks × 2 runs is prohibitive without release optimisation.
    if cfg!(debug_assertions) {
        return;
    }

    let mut sim_a = build_sim(default_config(S));
    let mut sim_b = build_sim(default_config(S));

    for _ in 0..L_CI {
        sim_a.step();
        sim_b.step();
    }

    // (4) Determinism: species_hash must match between both runs.
    let hash_a = sim_a.species_hash();
    let hash_b = sim_b.species_hash();
    assert_eq!(hash_a, hash_b, "species_hash not deterministic after {L_CI} ticks (seed {S:#x})");

    let pop = sim_a.population();
    let count = sim_a.telemetry().species_count;
    let threshold = sim_a.econ().speciation_threshold;
    let census: Vec<(u32, u32)> = sim_a.telemetry().species_census.clone();

    // (5c) At least K live species — the axis must produce actual divergence.
    assert!(
        count >= K,
        "species count {count} < {K} at tick {L_CI} (seed {S:#x}, pop={pop}); \
         NAMED STOP: calibration failed — raise threshold or pick a longer L_CI"
    );

    // (5a) Each daughter species' founder ref must be > threshold from its parent's founder ref.
    {
        let spec = sim_a.speciation_state();
        for (sid, parent_sid) in &spec.parent_of {
            let child_ref = spec.refs.get(sid)
                .unwrap_or_else(|| panic!("child species {sid:?} has no ref"));
            let parent_ref = spec.refs.get(parent_sid)
                .unwrap_or_else(|| panic!("parent species {parent_sid:?} has no ref"));
            let d = child_ref.brain_weight_l1(parent_ref);
            assert!(
                d > threshold,
                "5a violation: species {sid:?} L1={d} from parent {parent_sid:?} not > {threshold}"
            );
        }
    }

    // (5b) Inter-species spread > threshold when count ≥ K.
    // By construction: inter ≥ threshold (5a) and intra ≤ threshold (else the member speciated).
    // Asserted explicitly as a regression guard.
    if count >= K {
        let spec = sim_a.speciation_state();
        let s0_ref = *spec.refs.get(&SpeciesId(0)).expect("S0 must exist");
        let live_ids: std::collections::BTreeSet<u32> =
            census.iter().map(|(id, _)| *id).collect();
        let live_l1s: Vec<i64> = live_ids.iter()
            .filter_map(|id| spec.refs.get(&SpeciesId(*id)))
            .map(|r| r.brain_weight_l1(&s0_ref))
            .collect();
        if live_l1s.len() >= 2 {
            let inter = live_l1s.iter().copied().max().unwrap()
                - live_l1s.iter().copied().min().unwrap();
            assert!(
                inter > threshold,
                "5b violation: inter-species spread={inter} not > threshold={threshold}"
            );
        }
    }
}
