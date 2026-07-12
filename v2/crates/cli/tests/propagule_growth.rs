//! P-1 propagule growth primitive (#429): the FIREWALL acceptance tests — a body imposed at 3/9
//! grown cells (via `CellGraph::rebuild_prefix`, the SAME mechanism `stage_grow`/the birth-seam
//! truncation use) must read as a 3-CELL body everywhere: Extent income (booked over 3
//! `cell_positions`, not 9), coordination + Kleiber metabolic cost, and predation refuge — AND its
//! germ/soma labels must match the TARGET body's (carried per-cell, critic F8), not a naive relabel
//! on the 3-cell prefix's own module sizes. Also covers R15 conservation with `enable_propagule=true`
//! across growth + the death exits (background/starvation/stillbirth/predation).
//!
//! Lives in the `cli` crate (not a sim-core `#[cfg(test)]`) so the income/refuge/cost checks run
//! through the REAL tick pipeline (`build_sim`/`Sim::step`), not a hand-called stage function.

use cli::{build_sim, config_with, driver_config, DEFAULT_THREADS};
use sim_core::*;
use std::collections::BTreeMap;

/// Hand-built 9-cell (g_dev=3) TARGET body: a 2-cell GERM module (`CellType::B`, `(0,0)`/`(1,0)`) +
/// a 7-cell SOMA module (`CellType::A`, everything else) — mirrors what a REAL decode with
/// `germ_threshold=2` would classify (count<=2 → germ). `growth_cells` is the BFS-forest order over
/// the full live 3x3 grid, root `(0,0)`, fixed neighbor order [up,down,left,right] — hand-traced to
/// match `CellGraph::from_gradient`'s own algorithm exactly (verified independently by
/// `growth_cells_covers_disconnected_body` in `sim-core`), so this fixture is representative of a
/// real decode; only hand-built to make the germ/soma split and the 3-cell prefix boundary
/// deterministic and inspectable without a live GRN/morphogen chain.
fn hand_built_target_graph() -> CellGraph {
    let growth_cells: Vec<(u8, u8, CellType, bool)> = vec![
        (0, 0, CellType::B, true),  // germ
        (0, 1, CellType::A, false), // soma
        (1, 0, CellType::B, true),  // germ
        (0, 2, CellType::A, false), // soma
        (1, 1, CellType::A, false), // soma
        (2, 0, CellType::A, false), // soma
        (1, 2, CellType::A, false), // soma
        (2, 1, CellType::A, false), // soma
        (2, 2, CellType::A, false), // soma
    ];
    CellGraph {
        g_dev: 3,
        module_type: vec![CellType::B, CellType::A],
        module_cell_count: vec![2, 7],
        module_is_germ: vec![true, false],
        module_reachable: vec![true, true],
        module_consortium: vec![0, 1],
        cell_positions: vec![
            (0, 0), (1, 0), (2, 0),
            (0, 1), (1, 1), (2, 1),
            (0, 2), (1, 2), (2, 2),
        ],
        growth_cells,
    }
}

/// F3/F8: a body truncated to its first 3 `growth_cells` entries must read as a 3-cell body with
/// germ/soma labels CARRIED from the TARGET's module (not recomputed on the prefix's own, much
/// smaller, module sizes). Pure `CellGraph::rebuild_prefix` check — no `Sim` needed.
#[test]
fn germ_soma_labels_carried_from_target_not_relabeled() {
    let target = hand_built_target_graph();
    let truncated = target.rebuild_prefix(3);

    assert_eq!(truncated.body_size(), 3, "3/9 prefix must materialise exactly 3 cells");
    assert_eq!(truncated.module_type, vec![CellType::B, CellType::A]);
    assert_eq!(truncated.module_cell_count, vec![2, 1], "prefix module sizes: 2 germ cells, 1 soma cell");
    assert_eq!(
        truncated.module_is_germ,
        vec![true, false],
        "the A (soma) module — count=1 in the PREFIX — must stay SOMA (carried from the 7-cell \
         TARGET module), not be relabeled GERM by a naive germ_threshold<=2 reapplied on the \
         prefix's own module_cell_count (F8)"
    );
    assert_eq!(
        truncated.cell_positions,
        vec![(0, 0), (1, 0), (0, 1)],
        "cell_positions must be row-major-sorted (z, then x) over the prefix (F7)"
    );
    assert_eq!(truncated.module_reachable, vec![true, true]);
    assert_eq!(truncated.module_consortium, vec![0, 1]);
    // growth_cells is carried forward UNCHANGED — a later, larger prefix rebuilds from the SAME
    // cold decode product (no information lost by truncating).
    assert_eq!(truncated.growth_cells, target.growth_cells);
}

fn placeholder_mspec() -> MorphogenSpec {
    // Only used to satisfy `build_sim`'s Hazard-predation size-variance guard
    // (`config.econ.morphogen.is_some()`); the founder below is never given this spec via
    // `with_specs`, so its OWN decode stays the trivial Ф0 path — `impose_graph_probe` is what
    // actually seeds the body under test.
    MorphogenSpec {
        g_dev: 3, n_dev: 8, boundary: Boundary::Reflecting, diffuse_shift: 3,
        decay_num: 1, decay_shift: 4, seed_scale: 4096, stop_threshold: 0,
        apoptosis_threshold: None, germ_threshold: None, supply_source: None,
        adhesion_threshold: None, body_plan: BodyPlan::Square,
    }
}

/// A single founder (Ф0 decode, `morphogen_spec=None`) whose `Phenotype.graph` is immediately
/// overwritten by the caller via `impose_graph_probe`. `uptake_layer=1` routes harvest to a flat,
/// KNOWN layer (mirrors `r30_1_1_income_extent.rs`'s `ring_extent_config`). `d0_scaled=0` disables
/// background death — these are single/short-tick determinism-sensitive checks, not a corridor.
fn firewall_config(seed: u64, flat_cap: i64, econ_extra: EconParams) -> SimConfig {
    let mut founder = Genome::founder(2);
    founder.uptake_layer = 1;
    let layer1 = LayerSpec { regen_rate: 0, flux_alpha_num: 1, flux_alpha_den: 4, flat_cap, world_cap_mult: 0 };
    let layer0_inert = LayerSpec { flux_alpha_num: 0, flux_alpha_den: 1, ..LayerSpec::default() };
    SimConfig {
        n_founders: 1,
        founder_energy: 300,
        founder_templates: Some(vec![(founder, 1)]),
        n_layers: 2,
        layer_specs: [layer0_inert, layer1, LayerSpec::default(), LayerSpec::default()],
        econ: EconParams { enable_propagule: true, d0_scaled: 0, dol_economy: false, division_of_labor: false, ..econ_extra },
        ..config_with(seed, DEFAULT_THREADS, MergeStrategy::Canonical)
    }
}

fn single_entity_bits(sim: &mut Sim) -> u64 {
    *sim.body_size_entity_probe().keys().next().expect("exactly one founder")
}

/// The named acceptance point: Extent income for a 3/9-grown body sums over its 3 MATERIALISED
/// `cell_positions`, not the 9-cell target (the F1/F12 subsidy the firewall exists to prevent).
#[test]
fn extent_income_reads_3_not_9_cells() {
    const FLAT_CAP: i64 = 1000;
    let econ = EconParams { income_mode: IncomeMode::Extent, ..EconParams::default() };
    let mut sim = build_sim(firewall_config(1, FLAT_CAP, econ));

    let bits = single_entity_bits(&mut sim);
    let truncated = hand_built_target_graph().rebuild_prefix(3);
    let mut graphs: BTreeMap<u64, CellGraph> = BTreeMap::new();
    graphs.insert(bits, truncated.clone());
    sim.impose_graph_probe(&graphs);
    assert_eq!(*sim.body_size_entity_probe().get(&bits).unwrap(), 3, "imposed body must read as 3 cells BEFORE stepping");

    sim.step();

    let tel = sim.telemetry();
    let (_photo, got) = *tel.income_probe.get(&bits).expect("the entity must have booked income this tick");
    let r_cell = FLAT_CAP / 2; // fields::CpuFieldStore::new_layered initial mass = cap/2
    let per_cell = monod_demand(EconParams::default().u_max, EconParams::default().km, r_cell);
    assert_eq!(
        got,
        per_cell * 3,
        "Extent income for a 3/9-grown body must equal Σ monod_demand over its 3 MATERIALISED \
         cell_positions, not 9 (the target) — the firewall's load-bearing invariant"
    );
}

/// Coordination cost (`c_coord · n_cells`) and the Kleiber term (`metab_reads_n_cells`) must both
/// read the MATERIALISED cell count. Twin comparison (3-cell prefix vs the untruncated 9-cell
/// target, same seed/config otherwise) with income routed to an EMPTY flat layer (isolates
/// metabolism) and grow blocked by a low `founder_energy` (isolates this tick's metabolism charge
/// from any grow-step deduction) — the post-tick energy must match the EXACT `base_metab_lump`
/// formula evaluated at n_cells=3 (not 9), using the same public pure helpers the stage itself calls.
#[test]
fn metabolism_and_coord_cost_read_3_not_9_cells() {
    let econ = EconParams { c_coord: 5, metab_reads_n_cells: true, income_mode: IncomeMode::Anchor, ..EconParams::default() };
    // flat_cap=0 on the harvested layer ⇒ r_cell=0 ⇒ monod_demand=0 ⇒ income is exactly 0 (isolates
    // metabolism from income arithmetic entirely, for either arm).
    let mut sim3 = build_sim(firewall_config(10, 0, econ.clone()));
    let mut sim9 = build_sim(firewall_config(10, 0, econ));

    let bits3 = single_entity_bits(&mut sim3);
    let bits9 = single_entity_bits(&mut sim9);
    let target = hand_built_target_graph();
    let truncated = target.rebuild_prefix(3);

    let mut g3: BTreeMap<u64, CellGraph> = BTreeMap::new();
    g3.insert(bits3, truncated);
    sim3.impose_graph_probe(&g3);

    let mut g9: BTreeMap<u64, CellGraph> = BTreeMap::new();
    g9.insert(bits9, target);
    sim9.impose_graph_probe(&g9);

    let energy_before = *sim3.energy_entity_probe().get(&bits3).unwrap();
    assert_eq!(energy_before, *sim9.energy_entity_probe().get(&bits9).unwrap(), "twin sims must start with identical founder_energy");

    sim3.step();
    sim9.step();

    let energy_after_3 = *sim3.energy_entity_probe().get(&bits3).expect("3-cell entity must survive one tick");
    let energy_after_9 = *sim9.energy_entity_probe().get(&bits9).expect("9-cell entity must survive one tick");

    // base_metab_lump (stages.rs) verbatim, evaluated with the SAME public pure helper the stage
    // itself calls (`size_pow_three_quarters`) — not a re-derived magic constant.
    let founder = Genome::founder(2);
    let lump = |n_cells: i64| -> i64 {
        let metab_units = size_pow_three_quarters(n_cells as i32) as i64;
        (EconParams::default().base_metab
            + EconParams::default().k_size_metab * metab_units
            + EconParams::default().k_move_cost * founder.move_speed as i64
            + EconParams::default().k_sense_cost * founder.sense_range as i64
            + 5 * n_cells)
            * EconParams::default().metab_period as i64
    };
    let expected_after_3 = energy_before - lump(3);
    let expected_after_9 = energy_before - lump(9);

    assert_eq!(energy_after_3, expected_after_3, "3/9-grown body's metabolism+coord-cost must be charged at n_cells=3, not 9");
    assert_eq!(energy_after_9, expected_after_9, "sanity: the untruncated 9-cell twin must be charged at n_cells=9");
    assert!(energy_after_3 > energy_after_9, "the smaller MATERIALISED body must pay strictly less coord+Kleiber cost");
}

/// Predation refuge (`refuge_attenuate` over `Σ module_cell_count`) must read the MATERIALISED cell
/// count. Twin comparison isolating predation ONLY: `c_coord=0` and `metab_reads_n_cells=false`
/// cancel the metabolism-cost difference between arms (both charge the SAME gene-based Kleiber term,
/// independent of the graph), income is 0 (empty flat layer) — so the ENTIRE post-tick energy
/// delta between the two arms is attributable to the refuge-attenuated hazard drain.
#[test]
fn predation_refuge_reads_3_not_9_cells() {
    let pred = PredationSpec {
        mode: PredationMode::Hazard,
        bite_shift: 2,
        combat_trait_scale: 0,
        efficiency_num: 200,
        size_refuge: Some(SizeRefugeSpec { shift: 8, refuge_k: 4 }),
        base_hazard: 100,
    };
    let econ = EconParams {
        c_coord: 0,
        metab_reads_n_cells: false,
        income_mode: IncomeMode::Anchor,
        predation: Some(pred),
        // build_sim's Hazard-predation guard requires morphogen.is_some() (size-variance check);
        // this founder is never given the spec via `with_specs`, so its own decode stays Ф0 —
        // `impose_graph_probe` is what actually seeds the body under test.
        morphogen: Some(placeholder_mspec()),
        ..EconParams::default()
    };
    let mut sim3 = build_sim(firewall_config(11, 0, econ.clone()));
    let mut sim9 = build_sim(firewall_config(11, 0, econ));

    let bits3 = single_entity_bits(&mut sim3);
    let bits9 = single_entity_bits(&mut sim9);
    let target = hand_built_target_graph();
    let truncated = target.rebuild_prefix(3);

    let mut g3: BTreeMap<u64, CellGraph> = BTreeMap::new();
    g3.insert(bits3, truncated);
    sim3.impose_graph_probe(&g3);
    let mut g9: BTreeMap<u64, CellGraph> = BTreeMap::new();
    g9.insert(bits9, target);
    sim9.impose_graph_probe(&g9);

    let energy_before = *sim3.energy_entity_probe().get(&bits3).unwrap();
    assert_eq!(energy_before, *sim9.energy_entity_probe().get(&bits9).unwrap());

    sim3.step();
    sim9.step();

    let energy_after_3 = *sim3.energy_entity_probe().get(&bits3).expect("3-cell entity must survive one tick");
    let energy_after_9 = *sim9.energy_entity_probe().get(&bits9).expect("9-cell entity must survive one tick");

    let drain_3 = refuge_attenuate(100, 3, 8, 4);
    let drain_9 = refuge_attenuate(100, 9, 8, 4);
    assert!(drain_3 > drain_9, "sanity: refuge_attenuate must be monotone-decreasing in body size (Boraas)");

    let observed_delta = energy_after_9 - energy_after_3; // both arms pay the SAME metabolism; only refuge differs
    let expected_delta = drain_3 - drain_9;
    assert_eq!(
        observed_delta, expected_delta,
        "predation refuge for a 3/9-grown body must attenuate against body=3, not body=9 \
         (Σ module_cell_count read from the MATERIALISED graph)"
    );
}

/// R15 conservation with `enable_propagule=true`, across growth events + all reachable death exits
/// (background d0, starvation, miscarried-division stillbirth, and predation — `driver_config` wires
/// Hazard predation on top of the real morphogen/GRN decode chain). Mirrors the existing
/// `extent_income_r15_conservation` pattern.
#[test]
fn r15_conservation_with_propagule_growth_enabled() {
    let mut cfg = driver_config(7);
    cfg.econ.enable_propagule = true;
    let mut sim = build_sim(cfg);
    for _ in 0..500 {
        sim.step();
    }
    let residual = sim.conservation_residual();
    assert_eq!(residual, 0, "enable_propagule=true must keep R15 residual exactly 0 across growth + all death exits");
}
