//! R30-1.1b (#414): newborn endowment welded to live body size (`e_cell · body_size(child)`),
//! TRANSFERRED from the parent's ledger under a STRUCTURAL affordability gate
//! (`energy ≥ endowment + c_div`, alongside `repro_bar`), behind `EconParams.newborn_energy_per_cell`
//! (default `false`, byte-identical). Lives in the `cli` crate (not a sim-core `#[cfg(test)]`) so it
//! runs under the same build the golden CI job compiles, using REAL decode via `build_sim` — never
//! `cellgraph_with_cells`.
//!
//! Four acceptance checks (critic F3 — the risky conservation branches):
//! - `success_multicellular_newborn_gets_n_scaled_endowment`: a multicellular (N>1) newborn is
//!   spawned with `Energy = e_cell · N`, strictly more than the flat `e_cell` a 1-cell body gets.
//! - `stillbirth_under_flag_conserves_energy_and_spawns_no_child`: a REAL size-viability stillbirth
//!   (`genome.rs`'s `(Some, Some)` decode arm — `force_decode_none` is `#[cfg(test)]`-gated inside
//!   `sim-core` and unreachable from this crate) under the flag spawns NO child and leaves
//!   `conservation_residual() == 0`.
//! - `cannot_afford_yet_then_divides_with_full_endowment`: a repro-eligible parent too poor for
//!   `endowment + c_div` does not divide while accumulating, then divides with the FULL endowment
//!   (no clamp, no death) once it can afford it.
//! - `r15_conservation_across_reproducing_multicellular_run`: `conservation_residual() == 0` over a
//!   multi-hundred-tick flag-ON run with a reproducing multicellular population.

use cli::{build_sim, config_with, DEFAULT_THREADS};
use sim_core::{BodyPlan, Boundary, EconParams, Genome, GrnSpec, MergeStrategy, MorphogenSpec, SimConfig};
use std::collections::BTreeSet;
use std::sync::Arc;

/// Bistable-matrix GRN spec (verbatim from `r30_1_1a_kleiber_ncells.rs`'s `kleiber_gspec`):
/// `input_weights=[0,0]` keeps the per-cell sampled gradient dead, so EVERY cell resolves the SAME
/// attractor (`CellType::B`) regardless of position or mutation — body shape is driven purely by
/// `body_plan`/`g_dev`, never by GRN drift, so `body_size()` stays deterministic across mutated
/// lineages sharing this spec.
fn newborn_gspec() -> GrnSpec {
    GrnSpec::new(2, vec![32, -32, -32, 32], vec![0, 0], vec![0, 0], 3, 12, 0, 0, vec![112, 144])
}

fn newborn_mspec(g_dev: usize, body_plan: BodyPlan) -> MorphogenSpec {
    MorphogenSpec {
        g_dev,
        n_dev: 8,
        boundary: Boundary::Reflecting,
        diffuse_shift: 3,
        decay_num: 1,
        decay_shift: 4,
        seed_scale: 4096,
        stop_threshold: 0,
        apoptosis_threshold: None,
        germ_threshold: None,
        supply_source: None,
        adhesion_threshold: None,
        body_plan,
    }
}

/// One founder shaped by a Square `body_plan` at `g_dev=3` (9 live cells — a full grid, matching
/// `r30_1_1a_kleiber_ncells.rs`'s `kleiber_config`). `mutation_rate` is the caller's choice: `0`
/// pins the child genome IDENTICAL to the parent (deterministic body shape, no stillbirth risk —
/// used by the success/afford tests to isolate the endowment mechanism from mutation noise); the
/// default `32` lets `size` drift so a REAL size-viability stillbirth can occur (used by the
/// stillbirth/R15 tests, which need that drift).
fn newborn_founder(mutation_rate: i32) -> Genome {
    let mut founder = Genome::founder(2)
        .with_specs(Some(Arc::new(newborn_gspec())), Some(newborn_mspec(3, BodyPlan::Square)));
    founder.mutation_rate = mutation_rate;
    founder
}

fn newborn_config(seed: u64, founder_energy: i64, mutation_rate: i32, econ_overrides: EconParams) -> SimConfig {
    SimConfig {
        n_founders: 1,
        founder_energy,
        founder_templates: Some(vec![(newborn_founder(mutation_rate), 1)]),
        econ: econ_overrides,
        ..config_with(seed, DEFAULT_THREADS, MergeStrategy::Canonical)
    }
}

fn live_ids(sim: &mut sim_core::Sim) -> BTreeSet<u64> {
    sim.energy_entity_probe().keys().copied().collect()
}

#[test]
fn success_multicellular_newborn_gets_n_scaled_endowment() {
    // founder_energy=20000 comfortably affords a 9-cell endowment (9*1000 + c_div(100) = 9100) even
    // after this tick's ordinary income/metabolism; mutation_rate=0 pins the child's body IDENTICAL
    // to the parent's (9 live cells, matching kleiber_config's validated Square@g_dev=3 fixture) —
    // no stillbirth risk, isolating the endowment formula itself (not decode luck).
    let econ = EconParams { newborn_energy_per_cell: true, d0_scaled: 0, ..EconParams::default() };
    let e_cell = econ.e_cell;
    let mut sim = build_sim(newborn_config(1, 20_000, 0, econ));

    let before = live_ids(&mut sim);
    sim.step();
    let after_energy = sim.energy_entity_probe();
    let after_size = sim.body_size_entity_probe();

    assert_eq!(after_energy.len(), 2, "division must have occurred this tick (mutation_rate=0 \
        guarantees a real, viable decode; founder_energy=20000 affords endowment+c_div)");

    let child_id = *after_energy.keys().find(|k| !before.contains(k))
        .expect("exactly one new entity id must appear this tick");
    let child_n = after_size[&child_id];
    let child_energy = after_energy[&child_id];

    assert!(child_n > 1, "child must be multicellular (N>1), got N={child_n}");
    assert_eq!(
        child_energy, e_cell * child_n,
        "newborn endowment must be EXACTLY e_cell*N_child (N={child_n}), got {child_energy}"
    );
    assert!(
        child_energy > e_cell,
        "N-scaled endowment ({child_energy}) must strictly exceed the flat 1-cell baseline ({e_cell})"
    );
}

#[test]
fn stillbirth_under_flag_conserves_energy_and_spawns_no_child() {
    // Income AND metabolism zeroed: the founder's own capital (huge, 10_000_000) is the ONLY energy
    // source for the whole run. Every successful division grants a child EXACTLY endowment=9000
    // (9 cells * e_cell=1000) — below its OWN 9100 afford-threshold, and with zero income a child can
    // never grow past that, so children are permanently frozen at "cannot afford" (test 3 proves this
    // outcome is otherwise harmless). Only the FOUNDER ever attempts a fresh division on any given
    // tick, so on the tick its `size` gene mutation happens to draw -1 (child size=3=floor, a REAL
    // size-viability stillbirth, genome.rs `(Some,Some)` arm — P≈1/24 per attempt, mutation_rate=32
    // default), no OTHER entity is simultaneously dividing — the live entity id-set is provably
    // unchanged by that specific tick, for ANY seed (no brute-force luck required).
    let econ = EconParams {
        newborn_energy_per_cell: true,
        d0_scaled: 0,
        u_max: 0,
        base_metab: 0,
        k_size_metab: 0,
        k_move_cost: 0,
        k_sense_cost: 0,
        excrete: 0,
        ..EconParams::default()
    };
    let mut sim = build_sim(newborn_config(7, 10_000_000, 32, econ));

    let mut found = false;
    for _ in 0..600u64 {
        let before_ids = live_ids(&mut sim);
        let still_before = sim.stillbirth_count();
        sim.step();
        let still_after = sim.stillbirth_count();
        if still_after > still_before {
            let after_ids = live_ids(&mut sim);
            assert_eq!(
                after_ids, before_ids,
                "a stillbirth must spawn NO child — the live entity id-set must be unchanged"
            );
            assert_eq!(
                sim.conservation_residual(), 0,
                "stillbirth under the flag must close R15 exactly (c_div spent -> dissipated, \
                 nothing lost, no endowment granted)"
            );
            found = true;
            break;
        }
    }
    assert!(
        found,
        "expected a real size-viability stillbirth within 600 ticks (P≈1/24 per founder-tick \
         attempt makes non-occurrence astronomically unlikely — calibration drifted if this fires)"
    );
}

#[test]
fn cannot_afford_yet_then_divides_with_full_endowment() {
    // founder_energy=2000 clears repro_bar (genome.repro_threshold=1500) from tick 0 — the parent is
    // repro-ELIGIBLE immediately — but is far short of the 9-cell afford-threshold (endowment 9000 +
    // c_div 100 = 9100). Income stays at its normal default (unlike the stillbirth fixture above) so
    // the SAME parent genuinely accumulates energy tick over tick; mutation_rate=0 isolates the
    // affordability gate from decode/stillbirth risk (test 2 already covers that branch).
    let econ = EconParams { newborn_energy_per_cell: true, d0_scaled: 0, ..EconParams::default() };
    let e_cell = econ.e_cell;
    let mut sim = build_sim(newborn_config(3, 2_000, 0, econ));

    let mut found = false;
    for tick in 0..2000u64 {
        let before_ids = live_ids(&mut sim);
        sim.step();
        let after_energy = sim.energy_entity_probe();
        let after_ids: BTreeSet<u64> = after_energy.keys().copied().collect();

        if after_ids.len() > before_ids.len() {
            assert!(
                tick > 0,
                "must genuinely accumulate across ticks before dividing, not divide immediately \
                 (founder_energy=2000 < the 9100 afford-threshold at tick 0)"
            );
            assert_eq!(
                after_ids.len(), before_ids.len() + 1,
                "exactly one child must be spawned once affordable"
            );
            assert_eq!(sim.population(), 2, "parent must survive division (no clamp-induced death)");

            let after_size = sim.body_size_entity_probe();
            let child_id = *after_ids.difference(&before_ids).next().unwrap();
            let child_n = after_size[&child_id];
            let child_energy = after_energy[&child_id];
            assert_eq!(
                child_energy, e_cell * child_n,
                "once affordable, the child must get the FULL N-scaled endowment — NO clamp"
            );

            assert_eq!(
                sim.conservation_residual(), 0,
                "the afford-transition tick must close R15 exactly"
            );
            found = true;
            break;
        }
        assert_eq!(
            after_ids.len(), before_ids.len(),
            "the parent must NOT divide while it cannot yet afford endowment+c_div (tick {tick})"
        );
    }
    assert!(found, "the accumulating parent must eventually divide within the horizon (no deadlock)");
}

#[test]
fn r15_conservation_across_reproducing_multicellular_run() {
    // Ordinary income+metabolism (no isolation tricks): a real reproducing multicellular population
    // under the flag, over a multi-hundred-tick run. `dol_economy`/mineral stay at their EconParams
    // defaults (false/None) — the endowment effect is isolated (issue's isolation instruction).
    let econ = EconParams { newborn_energy_per_cell: true, d0_scaled: 0, ..EconParams::default() };
    let mut sim = build_sim(newborn_config(11, 2_000, 32, econ));

    for _ in 0..500u64 {
        sim.step();
    }

    let sizes = sim.body_size_entity_probe();
    assert!(
        sim.population() > 1,
        "population must have grown via reproduction over 500 ticks under the flag"
    );
    assert!(
        sizes.values().any(|&n| n > 1),
        "at least one live entity must be multicellular (N>1) — the reproducing population must \
         actually be exercising the N-scaled endowment path, not staying unicellular"
    );

    let residual = sim.conservation_residual();
    assert_eq!(
        residual, 0,
        "newborn_energy_per_cell=true must still close R15 exactly across a long reproducing run"
    );
}
