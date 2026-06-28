//! B-2 metabolic-profile integration tests (issue #155). All arch-independent (no golden constants,
//! no float equality) — they run on BOTH CI jobs.

use cli::{build_sim, default_config};
use sim_core::Genome;

const TICKS: u64 = 384;

/// genome_hash_includes_layers (F9 contract): changing `uptake_layer` or `excrete_layer` must
/// produce a DIFFERENT `hash_contribution` result. Verifies the B-2 fields entered the
/// determinism lock (a field NOT in the hash would silently decouple mutation from state).
#[test]
fn b2_genome_hash_includes_layers() {
    let base = Genome::founder(2);
    let hash_base = base.hash_contribution(0);

    let mut g_ul = base;
    g_ul.uptake_layer = 1; // flip to layer 1
    assert_ne!(
        hash_base,
        g_ul.hash_contribution(0),
        "uptake_layer change must alter hash"
    );

    let mut g_el = base;
    g_el.excrete_layer = 0; // founder default is 1; flip to 0
    assert_ne!(
        hash_base,
        g_el.hash_contribution(0),
        "excrete_layer change must alter hash"
    );
}

/// genome_layer_targeting: across a real L=2 run, every organism's layer traits stay within
/// `[0, n_layers-1]`. No out-of-bounds indexing is possible from mutation.
#[test]
fn b2_genome_layer_targeting() {
    let mut sim = build_sim(default_config(0xB2_B2B2B2));
    for _ in 0..TICKS {
        sim.step();
    }
    let tel = sim.telemetry();
    assert!(tel.population > 0, "population went extinct before layer-trait check");
    // Trait slot 6 = uptake_layer, slot 7 = excrete_layer (see TraitSample doc).
    for s in &tel.samples {
        let ul = s.traits[6];
        let el = s.traits[7];
        assert!(
            ul >= 0 && ul <= 1,
            "uptake_layer={ul} out of [0,1] at n_layers=2"
        );
        assert!(
            el >= 0 && el <= 1,
            "excrete_layer={el} out of [0,1] at n_layers=2"
        );
    }
}

/// crossfeed_conserves: cross-layer excretion (excrete_layer=1 while uptake_layer=0) must keep
/// the conserved-field ledger residual exactly 0 every tick. The transfer is agent→field on one
/// layer; total mass stays invariant.
#[test]
fn b2_crossfeed_conserves() {
    let mut sim = build_sim(default_config(0xB2_CFCF));
    for _ in 0..TICKS {
        sim.step();
        assert_eq!(
            sim.conservation_residual(),
            0,
            "energy leaked at tick {} with cross-layer excretion",
            sim.tick()
        );
    }
}

/// phase1_prod_non_collapse (F9 hard merge gate): with the L=2 production config, the population
/// must survive at least FLOOR ticks — the cross-feeding structure (founders eat layer 0,
/// excrete to layer 1) must not starve the system into extinction. FLOOR=384 is the calibrated
/// safe-run cutoff (observed pop ≥ 100 on the x86 spatial equilibrium trajectory at this tick).
#[test]
fn b2_phase1_prod_non_collapse() {
    const FLOOR: u64 = 384;
    let mut sim = build_sim(default_config(0xB2_CAFE));
    let mut min_pop = u64::MAX;
    for _ in 0..FLOOR {
        sim.step();
        let p = sim.population();
        min_pop = min_pop.min(p);
    }
    assert!(
        min_pop > 0,
        "population collapsed to 0 before tick {FLOOR} under L=2 production config"
    );
    // Sanity: field has material (layer 0 substrate non-empty, total conserved sum > 0).
    assert!(
        sim.telemetry().field_total > 0,
        "conserved field completely drained at tick {FLOOR} — substrate gone"
    );
}
