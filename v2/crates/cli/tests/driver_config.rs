//! D-2 (#270): `driver_config` — the multicellular-predation cost↔benefit economy. Combines
//! `phase2_config`'s ontogenesis chain (bodies can be multicellular) with predation + a per-prey
//! size-refuge (D-1, `#268`, the benefit) and `c_coord > 0` (M7-e-a, `#251`, the cost). Parameters
//! are chosen for VIABILITY, not tuned for emergence (D-3's job — out of scope here).
//!
//! Arch-independent integer invariants — run on BOTH CI jobs (x86 + arm64). The additive golden
//! (`v2_golden_conserved_driver`, `golden_conserved.rs`) is arm64-only (PM-pinned separately).

use cli::{apply_overrides, build_sim, driver_config, run};
use sim_core::EconParams;

const SEED: u64 = 0xBE_EF_5EED;
const TICKS: u64 = 512;

/// `d2_driver_config_viable`: non-collapse floor over the standard local acceptance length —
/// mirrors `predation_no_collapse`/`differentiation_no_collapse`. If this fails at the chosen
/// defaults, the refuge/c_coord/predation calibration needs adjustment (an early calibration
/// signal to report, not silently patch around).
#[test]
fn d2_driver_config_viable() {
    if cfg!(debug_assertions) {
        return;
    }
    let mut sim = build_sim(driver_config(SEED));
    let mut pop_min = u64::MAX;
    let mut pop_max = 0u64;
    for _ in 0..TICKS {
        sim.step();
        let pop = sim.population();
        pop_min = pop_min.min(pop);
        pop_max = pop_max.max(pop);
    }
    const POP_FLOOR: u64 = 10;
    assert!(
        pop_min >= POP_FLOOR,
        "population collapsed below {POP_FLOOR} on driver_config at tick {TICKS} \
         (pop_min={pop_min}) — the predation/refuge/c_coord defaults are not viable"
    );
    const POP_CEIL: u64 = 100_000;
    assert!(
        pop_max <= POP_CEIL,
        "population exploded to {pop_max} on driver_config — conservation or encounter logic is broken"
    );
}

/// `d2_bodies_can_be_multicellular`: driver_config decodes bodies with `Σ module_cell_count`
/// reaching >1 for some genomes — the multicellular substrate is live, not inert.
#[test]
fn d2_bodies_can_be_multicellular() {
    if cfg!(debug_assertions) {
        return;
    }
    let mut sim = build_sim(driver_config(SEED));
    for _ in 0..TICKS {
        sim.step();
    }
    let (max_body_size, count_multicellular) = sim.body_size_stats();
    assert!(
        max_body_size > 1,
        "driver_config must produce at least one body with Σ module_cell_count > 1 \
         (max observed = {max_body_size}) — the ontogenesis chain looks inert"
    );
    assert!(
        count_multicellular > 0,
        "driver_config must have at least one live multicellular body at tick {TICKS}"
    );
}

/// `d2_predation_size_refuge_active`: with driver_config's own predation spec, a large-bodied prey
/// suffers strictly less predation loss than an equal-energy unicell — the D-1 refuge is ON and
/// biting at the chosen calibration (`DRIVER_REFUGE_K`), not a degenerate no-op.
#[test]
fn d2_predation_size_refuge_active() {
    let spec = driver_config(SEED)
        .econ
        .predation
        .expect("driver_config must configure predation");
    assert!(spec.size_refuge.is_some(), "driver_config must configure size_refuge");

    let predator = sim_core::Genome::founder(1);
    let prey_energy = 10_000i64;

    let loss_unicell = sim_core::resolve_encounter(&predator, prey_energy, 1, &spec).prey_loss;
    let loss_large_body =
        sim_core::resolve_encounter(&predator, prey_energy, 20, &spec).prey_loss;

    assert!(
        loss_large_body < loss_unicell,
        "a large-bodied prey (body_size=20) must lose LESS than an equal-energy unicell under \
         driver_config's size-refuge: loss_large_body={loss_large_body}, loss_unicell={loss_unicell}"
    );
}

/// `d2_c_coord_charged`: `c_coord > 0` in driver_config must genuinely alter the trajectory versus
/// an otherwise-identical `c_coord=0` twin — proving the coordination-cost sink (M7-e-a) is wired
/// AND active in this config (not dead weight because bodies never reach >1 cell — see
/// `d2_bodies_can_be_multicellular` for that half of the proof).
#[test]
fn d2_c_coord_charged() {
    if cfg!(debug_assertions) {
        return;
    }
    assert!(driver_config(SEED).econ.c_coord > 0, "driver_config must ship c_coord > 0");

    let with_cost = run(driver_config(SEED), TICKS);
    let mut cfg_no_cost = driver_config(SEED);
    cfg_no_cost.econ.c_coord = 0;
    let without_cost = run(cfg_no_cost, TICKS);

    assert_ne!(
        with_cost, without_cost,
        "c_coord>0 must alter driver_config's trajectory vs a c_coord=0 twin — the coordination \
         cost must be genuinely charged, not dead weight"
    );
}

/// `d2_conservation_R15`: driver_config closes the energy ledger (residual 0) every tick with
/// refuge + c_coord + predation all composed.
#[test]
fn d2_conservation_r15() {
    if cfg!(debug_assertions) {
        return;
    }
    let mut sim = build_sim(driver_config(SEED));
    for _ in 0..TICKS {
        sim.step();
        assert_eq!(
            sim.conservation_residual(),
            0,
            "energy not conserved at tick {} on driver_config (predation/c_coord/refuge composed)",
            sim.tick()
        );
    }
}

/// `d2_determinism`: driver_config replay bit-identical (1-vs-N is exercised via `r14.rs`'s
/// generic sweep; this is the same-seed repeated-run half already used by every sibling config's
/// `_r14_determinism` test).
#[test]
fn d2_determinism() {
    if cfg!(debug_assertions) {
        return;
    }
    let a = run(driver_config(SEED), TICKS);
    let b = run(driver_config(SEED), TICKS);
    for t in 0..TICKS as usize {
        assert_eq!(
            a[t], b[t],
            "driver_config non-deterministic at tick {t} — state_hash depends on RNG or thread-order"
        );
    }
}

/// `d2_set_overrides`: `--set c_coord=<v>` and `--set refuge_k=<v>` apply + range-guard (reject
/// negative); no-flag path stays byte-identical to `driver_config` itself.
#[test]
fn d2_set_overrides() {
    // Apply: c_coord updates econ.c_coord.
    let mut econ = driver_config(SEED).econ;
    apply_overrides(&mut econ, &[("c_coord".to_string(), "7".to_string())])
        .expect("c_coord=7 must be accepted");
    assert_eq!(econ.c_coord, 7);

    // Apply: refuge_k updates the nested SizeRefugeSpec.
    apply_overrides(&mut econ, &[("refuge_k".to_string(), "9".to_string())])
        .expect("refuge_k=9 must be accepted on a config with predation.size_refuge configured");
    assert_eq!(econ.predation.unwrap().size_refuge.unwrap().refuge_k, 9);

    // Range-guard: negative values rejected for both keys.
    let mut econ_neg = driver_config(SEED).econ;
    let r_c = apply_overrides(&mut econ_neg, &[("c_coord".to_string(), "-1".to_string())]);
    assert!(r_c.is_err(), "c_coord=-1 must return Err");
    assert!(r_c.unwrap_err().starts_with("error:"));

    let r_k = apply_overrides(&mut econ_neg, &[("refuge_k".to_string(), "-1".to_string())]);
    assert!(r_k.is_err(), "refuge_k=-1 must return Err");
    assert!(r_k.unwrap_err().starts_with("error:"));

    // refuge_k is rejected when no predation.size_refuge is configured (structural — plain default).
    let mut econ_plain = EconParams::default();
    let r_no_pred = apply_overrides(&mut econ_plain, &[("refuge_k".to_string(), "3".to_string())]);
    assert!(r_no_pred.is_err(), "refuge_k must be rejected when predation is None");
    assert!(r_no_pred.unwrap_err().starts_with("error:"));

    // No-flag byte-identical: empty override set must leave driver_config's trajectory untouched.
    if !cfg!(debug_assertions) {
        let baseline = run(driver_config(SEED), TICKS);
        let mut econ_empty = driver_config(SEED).econ;
        apply_overrides(&mut econ_empty, &[]).expect("empty override set is always Ok");
        let mut cfg_empty = driver_config(SEED);
        cfg_empty.econ = econ_empty;
        let overridden = run(cfg_empty, TICKS);
        assert_eq!(
            baseline, overridden,
            "empty --set must be byte-identical to driver_config's own trajectory"
        );
    }
}
