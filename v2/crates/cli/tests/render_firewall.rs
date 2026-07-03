//! R-1 DETERMINISM FIREWALL (RnD 02 §det-orthogonal, R26/R17/R19) — the load-bearing tooth for the
//! render seam. `Sim::observe_render` is a NEW read-only ECS query on a CI'd crate (`sim-core`); this
//! test proves it can never perturb the tick trajectory.
//!
//! Method: run the SAME seed for N ticks TWICE — once calling `observe_render()` after every step,
//! once never calling it — and assert the FULL per-tick `state_hash` (folds BOTH the conserved field
//! AND the f32 signal field, R19) plus population are byte-identical between the two runs. Both runs
//! execute on the same machine/arch/profile, so the f32 signal is bit-identical between them
//! regardless of arch — asserting the FULL hash (not just `conserved_field_hash`) is safe here and
//! strictly stronger: a future regression that sneaks ANY mutation into `observe_render` (even one
//! that only touches the signal field) turns this red. Runs on BOTH CI arches (not `v2_golden_*`
//! namespaced) since it is a same-run relative comparison, not a fixed golden constant.

use cli::{build_sim, default_config};

const TICKS: u64 = 384;

#[test]
fn v2_observe_render_is_golden_neutral() {
    let seed = 0xA11A_2A11;
    let mut with_observe = build_sim(default_config(seed));
    let mut without_observe = build_sim(default_config(seed));

    let mut trace_with = Vec::with_capacity(TICKS as usize);
    let mut trace_without = Vec::with_capacity(TICKS as usize);

    for _ in 0..TICKS {
        with_observe.step();
        // The read-only observation under test — discarded immediately, exactly as a render thread
        // would consume it, but the point is that calling it here must not move the trajectory below.
        let snap = with_observe.observe_render();
        std::hint::black_box(&snap);
        trace_with.push((with_observe.state_hash(), with_observe.population()));

        without_observe.step();
        trace_without.push((without_observe.state_hash(), without_observe.population()));
    }

    assert_eq!(
        trace_with, trace_without,
        "observe_render perturbed the sim trajectory — the R-1 determinism firewall (R26/R17/R19) is broken"
    );
}

/// Sanity: `observe_render` actually returns live data (not a vacuous always-empty snapshot), so the
/// firewall test above isn't trivially passing because nothing was ever observed.
#[test]
fn v2_observe_render_returns_live_creatures() {
    let mut sim = build_sim(default_config(0xA11A_2A11));
    for _ in 0..8 {
        sim.step();
    }
    let snap = sim.observe_render();
    assert_eq!(snap.tick, sim.tick());
    assert!(!snap.creatures.is_empty(), "observe_render returned no creatures on a live population");
    assert_eq!(
        snap.creatures.len() as i64,
        snap.population,
        "creatures.len() must match the telemetry population it reports"
    );
    for w in snap.creatures.windows(2) {
        assert!(w[0].id < w[1].id, "creatures must be sorted by entity id (determinism)");
    }
}
