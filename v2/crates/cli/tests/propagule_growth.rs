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
fn firewall_config(seed: u64, founder_energy: i64, flat_cap: i64, econ_extra: EconParams) -> SimConfig {
    let mut founder = Genome::founder(2);
    founder.uptake_layer = 1;
    let layer1 = LayerSpec { regen_rate: 0, flux_alpha_num: 1, flux_alpha_den: 4, flat_cap, world_cap_mult: 0 };
    let layer0_inert = LayerSpec { flux_alpha_num: 0, flux_alpha_den: 1, ..LayerSpec::default() };
    SimConfig {
        n_founders: 1,
        founder_energy,
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
    let mut sim = build_sim(firewall_config(1, 300, FLAT_CAP, econ));

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
    // excrete: 0 — isolate the metabolic-lump value from the post-metabolism `stage_field_scatter`
    // excrete deposit (GOTCHAS.md's excrete-pollution rake, ~R30-1.1b): that stage runs every tick,
    // unconditionally, and deducts `econ.excrete` (default 8) AFTER metabolism but BEFORE this
    // test's post-step `energy_entity_probe()` read — an absolute-value assertion like this one
    // (unlike the refuge twin-comparison below, where excrete cancels in the delta) would otherwise
    // be off by exactly `excrete`. Excrete is a conserved field deposit (R15-neutral); it only
    // offsets the MEASURED energy here, it does not touch the firewall logic under test.
    let econ = EconParams { c_coord: 5, metab_reads_n_cells: true, income_mode: IncomeMode::Anchor, excrete: 0, ..EconParams::default() };
    // flat_cap=0 on the harvested layer ⇒ r_cell=0 ⇒ monod_demand=0 ⇒ income is exactly 0 (isolates
    // metabolism from income arithmetic entirely, for either arm).
    let mut sim3 = build_sim(firewall_config(10, 300, 0, econ.clone()));
    let mut sim9 = build_sim(firewall_config(10, 300, 0, econ));

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
    let mut sim3 = build_sim(firewall_config(11, 300, 0, econ.clone()));
    let mut sim9 = build_sim(firewall_config(11, 300, 0, econ));

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

// ── P-2a (#442): grow-gate 2-window reserve refactor + GrowGate classifier ─────────────────────────
//
// All fixtures below reuse the SAME world fixture as P-1's firewall tests above: a single founder
// (`firewall_config`), `lock_repro_probe()` (no births — a global counter is unambiguous), and
// `impose_graph_probe(hand_built_target_graph().rebuild_prefix(1))` so the entity reads as a 1-cell
// MATERIALISED body (`Grown == 1`, matching the trivial Ф0 spawn decode) against a K=9-cell TARGET
// (`growth_cells.len()`, carried unchanged by `rebuild_prefix`) — exactly one still-growing entity,
// so `ledger_snapshot()`'s buckets are a GLOBAL counter unambiguously attributable to it.

/// A trivial single-module flat body of `body_size` cells — for the PURE `grow_reserve` unit test
/// below, which needs a `Phenotype`/`CellGraph` fixture but no `Sim`.
fn flat_graph(body_size: i32) -> CellGraph {
    CellGraph {
        g_dev: 1,
        module_type: vec![CellType::A],
        module_cell_count: vec![body_size],
        module_is_germ: vec![false],
        module_reachable: vec![true],
        module_consortium: vec![0],
        cell_positions: (0..body_size).map(|i| (i as u8, 0)).collect(),
        growth_cells: (0..body_size).map(|i| (i as u8, 0, CellType::A, false)).collect(),
    }
}

/// (1) Cadence pin — `e_cell` UNAFFORDABLE (`1e9`) so the gate never reaches `Grow`: every metab
/// tick must land in `BlockedCell` (energy ≫ reserve but ≪ e_cell), never `BlockedLump`, never
/// `Grow`. `excrete=0` isolates the survival window to the flat (n-invariant, `c_coord=0`,
/// `metab_reads_n_cells=false`) Kleiber lump alone, so `founder_energy=100_000` trivially survives
/// the whole run (two orders of magnitude of slack either side, F97).
#[test]
fn grow_step_counts_pin_unaffordable_e_cell() {
    let econ = EconParams { e_cell: 1_000_000_000, excrete: 0, ..EconParams::default() };
    let mut sim = build_sim(firewall_config(20, 100_000, 0, econ));
    sim.lock_repro_probe();

    let bits = single_entity_bits(&mut sim);
    let mut graphs: BTreeMap<u64, CellGraph> = BTreeMap::new();
    graphs.insert(bits, hand_built_target_graph().rebuild_prefix(1));
    sim.impose_graph_probe(&graphs);

    let period = EconParams::default().metab_period;
    let t = 20u64;
    for _ in 0..t {
        sim.step();
    }

    assert!(
        sim.energy_entity_probe().contains_key(&bits),
        "founder must survive the isolated window — 100_000 ≫ any drain over 10 metab windows"
    );
    let snap = sim.ledger_snapshot();
    assert_eq!(
        snap.blocked_lump + snap.blocked_cell,
        t / period,
        "the gate must be evaluated on EVERY metab tick — no firing skipped or double-counted"
    );
    let grow_count = snap.grow_steps_total - snap.blocked_lump - snap.blocked_cell;
    assert_eq!(
        grow_count, 0,
        "e_cell=1e9 must never reach Grow — every step is BlockedCell (energy ≫ reserve, ≪ e_cell)"
    );
    assert_eq!(snap.maturations_total, 0);
}

/// (2) Progress+stop pin — affordable, normal `e_cell`, `metab_reads_n_cells=true` so the Kleiber
/// lump actually RISES with the materialising body (F96). `founder_energy=1_000_000` is generous
/// slack for all `K-1` growth payments (`8 × e_cell = 8_000`) plus metabolism at the rising `n_cells`
/// across the window (well under a few hundred `eu`) — every still-growing metab tick must land
/// `Grow`, never blocked. After `K-1` firings the body matures; further ticks must be silent (the
/// maturity `continue` never re-bumps a bucket for an already-mature body, F73).
#[test]
fn grow_step_counts_progress_then_stop() {
    let econ = EconParams { metab_reads_n_cells: true, excrete: 0, ..EconParams::default() };
    let mut sim = build_sim(firewall_config(21, 1_000_000, 0, econ));
    sim.lock_repro_probe();

    let bits = single_entity_bits(&mut sim);
    let hand_graph = hand_built_target_graph();
    let k = hand_graph.growth_cells.len() as i64; // 9
    let mut graphs: BTreeMap<u64, CellGraph> = BTreeMap::new();
    graphs.insert(bits, hand_graph.rebuild_prefix(1));
    sim.impose_graph_probe(&graphs);

    let period = EconParams::default().metab_period;
    for _ in 0..(k as u64 - 1) * period {
        sim.step();
    }

    assert_eq!(
        *sim.body_size_entity_probe().get(&bits).unwrap(),
        k,
        "body must have grown to the full K-cell target after K-1 firings"
    );
    let snap = sim.ledger_snapshot();
    let grow_count = snap.grow_steps_total - snap.blocked_lump - snap.blocked_cell;
    assert_eq!(grow_count, (k - 1) as u64, "the Grow slot specifically must fire K-1 times (not a tautology on grow_steps_total, F147)");
    assert_eq!(snap.blocked_lump, 0, "generous founder_energy must never hit BlockedLump");
    assert_eq!(snap.blocked_cell, 0, "generous founder_energy must never hit BlockedCell");
    assert_eq!(snap.maturations_total, 1, "exactly one maturation — reaching the target for the first time");

    for _ in 0..(2 * (k as u64 - 1) * period) {
        sim.step();
    }
    let snap2 = sim.ledger_snapshot();
    let grow_count2 = snap2.grow_steps_total - snap2.blocked_lump - snap2.blocked_cell;
    assert_eq!(grow_count2, (k - 1) as u64, "a matured body must never fire another grow step (the maturity continue, F73)");
    assert_eq!(snap2.maturations_total, 1, "no re-maturation");
}

/// (3) `grow_reserve` unit test — a settling+hazard config, isolating each term by DELTA (so the
/// un-reachable `pub(crate) base_metab_lump` never needs re-deriving, F71/F77): the settling
/// amortisation, the hazard drain, and the excrete term each cancel everything else when the same
/// `ph`/`g`/`n` is held fixed and only ONE `EconParams` field changes. This is the ONLY P-2a fixture
/// with settling/predation live (critic instruction) — a PURE test, no `Sim`.
#[test]
fn grow_reserve_settling_hazard_excrete_terms() {
    let g = Genome::founder(2);
    let ph = Phenotype { uptake_layer: 0, cell_type: None, graph: flat_graph(5), respiratory_pathway: None };
    let n = 6i64; // post-growth count (grown+1) — arbitrary here, cancels in every delta below.
    let body_size = ph.graph.body_size();

    let settling_spec = SettlingSpec { period: 100, strength: 500, settling_k: 4, shift: 8 };
    let size_refuge = SizeRefugeSpec { shift: 8, refuge_k: 4 };
    let pred_spec = PredationSpec {
        mode: PredationMode::Hazard,
        bite_shift: 2,
        combat_trait_scale: 0,
        efficiency_num: 200,
        size_refuge: Some(size_refuge),
        base_hazard: 100,
    };

    let base = EconParams { excrete: 8, ..EconParams::default() };
    let with_settling = EconParams { settling: Some(settling_spec), ..base.clone() };
    let with_hazard = EconParams { predation: Some(pred_spec), ..base.clone() };
    let with_both = EconParams { settling: Some(settling_spec), predation: Some(pred_spec), ..base.clone() };
    let with_both_no_excrete = EconParams { excrete: 0, ..with_both.clone() };

    // (a) settling_reserve isolated by delta — one pulse (2·metab_period=4 ≪ period=100).
    let pulses = (2 * base.metab_period as i64 + settling_spec.period as i64 - 1) / settling_spec.period as i64;
    assert_eq!(pulses, 1, "sanity: 2·metab_period ≪ settling period ⇒ exactly one amortised pulse");
    let expected_settling_reserve = settling_drain(&settling_spec, body_size) * pulses;
    assert_eq!(
        grow_reserve(&with_settling, &ph, &g, n) - grow_reserve(&base, &ph, &g, n),
        expected_settling_reserve,
        "settling_reserve must equal settling_drain_of · ceil(2·metab_period/period)"
    );

    // (b) the div-by-zero guard: period==0 and strength==0 must both be inert, not panic.
    let settling_period_zero = SettlingSpec { period: 0, ..settling_spec };
    let econ_period_zero = EconParams { settling: Some(settling_period_zero), ..base.clone() };
    assert_eq!(
        grow_reserve(&econ_period_zero, &ph, &g, n), grow_reserve(&base, &ph, &g, n),
        "period==0 (the shipped 'treat as None' compat case) must be inert, not integer-divide-by-zero"
    );
    let settling_strength_zero = SettlingSpec { strength: 0, ..settling_spec };
    let econ_strength_zero = EconParams { settling: Some(settling_strength_zero), ..base.clone() };
    assert_eq!(
        grow_reserve(&econ_strength_zero, &ph, &g, n), grow_reserve(&base, &ph, &g, n),
        "strength==0 must be inert"
    );

    // (c) hazard_drain isolated by delta — matches refuge_attenuate exactly, and is > 0.
    let expected_hazard = refuge_attenuate(pred_spec.base_hazard, body_size, size_refuge.shift, size_refuge.refuge_k);
    assert!(expected_hazard > 0, "sanity: base_hazard>0 + size_refuge=Some must yield a positive drain");
    assert_eq!(
        grow_reserve(&with_hazard, &ph, &g, n) - grow_reserve(&base, &ph, &g, n),
        2 * base.metab_period as i64 * expected_hazard,
        "hazard_drain must be routed into grow_reserve via the 2-window buffer, matching refuge_attenuate exactly"
    );

    // (d) the excrete term of window_drain, isolated by delta (settling+hazard both live and equal
    // on both sides, so only the excrete term's contribution survives the subtraction).
    assert_eq!(
        grow_reserve(&with_both, &ph, &g, n) - grow_reserve(&with_both_no_excrete, &ph, &g, n),
        2 * 8 * base.metab_period as i64,
        "excrete must contribute exactly 2·excrete·metab_period to grow_reserve"
    );
}

/// (4) Slot semantics — a PURE unit test on `grow_gate` (no `Sim`): every branch of the 3-variant
/// classifier, including `BlockedCell` with a nonzero `prov` (P-2b-forward — `stage_grow` itself
/// passes `prov=0` this slice, but the fn must already handle a real bank correctly).
#[test]
fn grow_gate_slot_semantics() {
    let econ = EconParams { e_cell: 1000, ..EconParams::default() };
    let reserve = 50;

    assert_eq!(grow_gate(&econ, 49, 0, reserve), GrowGate::BlockedLump, "energy < reserve ⇒ BlockedLump");
    assert_eq!(
        grow_gate(&econ, reserve, 200, reserve), GrowGate::BlockedCell,
        "energy ≥ reserve but min(prov,e_cell)+energy < e_cell+reserve ⇒ BlockedCell (exercised with nonzero prov)"
    );
    assert_eq!(
        grow_gate(&econ, econ.e_cell + reserve, 0, reserve), GrowGate::Grow,
        "energy == e_cell+reserve (boundary) ⇒ Grow"
    );
}

/// (5) The F142 reserve claim, made executable without a dead code path: the "P-1's narrow
/// `+next_lump` reserve would have grown here, P-2a's wide `+R` reserve refuses" claim, asserted
/// directly on `grow_gate` (P-1's gate no longer exists as a runnable path) — WHICH blocked variant
/// `R` returns is config-dependent and NOT asserted (F7/F9), only that it isn't `Grow`. Then ONE
/// growth-ON integration fixture lands the SAME entity's energy in that exact band via the real tick
/// pipeline, and confirms the gate was evaluated (not skipped) and did not grow, with R15 intact.
#[test]
fn grow_gate_wide_reserve_refuses_narrow_would_grow() {
    let econ = EconParams { excrete: 8, ..EconParams::default() }; // settling/predation None: R > 2·lump > next_lump from the 2-window buffer + excrete alone.
    let g = Genome::founder(2);
    let ph = Phenotype { uptake_layer: 0, cell_type: None, graph: flat_graph(1), respiratory_pathway: None };
    let n = 2i64; // grown=1, post-growth count = grown+1 = 2 (mirrors the real call site below).

    // P-1's OLD narrow reserve, re-derived via PUBLIC helpers only — `base_metab_lump` itself is
    // pub(crate) and unreachable from cli; this mirrors the ACCEPTED precedent already established
    // by `metabolism_and_coord_cost_read_3_not_9_cells` above (re-deriving the Kleiber lump via
    // `size_pow_three_quarters` + the public `EconParams` fields is not the stale-copy F71/F77 bars —
    // that bar is on re-deriving `grow_reserve`/hazard/settling, which this test does NOT do).
    let metab_units = size_pow_three_quarters(g.size) as i64;
    let next_lump = (econ.base_metab + econ.k_size_metab * metab_units + econ.k_move_cost * g.move_speed as i64
        + econ.k_sense_cost * g.sense_range as i64 + econ.c_coord * n)
        * econ.metab_period as i64;
    let r = grow_reserve(&econ, &ph, &g, n);
    assert!(r > 2 * next_lump, "sanity: the 2-window buffer + excrete alone must exceed P-1's single-lump reserve");

    let energy = econ.e_cell + next_lump + 8; // inside [e_cell+next_lump, e_cell+r)
    assert!(energy < econ.e_cell + r, "sanity: chosen energy must stay inside the claim's band");
    assert_ne!(
        grow_gate(&econ, energy, 0, r), GrowGate::Grow,
        "the wide reserve must refuse — which blocked variant is config-dependent, not asserted"
    );
    assert_eq!(
        grow_gate(&econ, energy, 0, next_lump), GrowGate::Grow,
        "the OLD narrow reserve would have grown at this same energy — that IS the F142 claim"
    );

    // The growth-ON integration fixture: land the SAME entity's energy in this exact band via the
    // real tick pipeline (one metab window), through the SAME fixture as (1)/(2) above.
    let founder_energy = energy + next_lump; // metabolism deducts `next_lump` (== the real lump at
                                              // this econ/genome/body) before stage_grow evaluates.
    let mut sim = build_sim(firewall_config(22, founder_energy, 0, econ.clone()));
    sim.lock_repro_probe();
    let bits = single_entity_bits(&mut sim);
    let mut graphs: BTreeMap<u64, CellGraph> = BTreeMap::new();
    graphs.insert(bits, hand_built_target_graph().rebuild_prefix(1));
    sim.impose_graph_probe(&graphs);

    let period = EconParams::default().metab_period;
    for _ in 0..period {
        sim.step();
    }

    assert!(
        sim.energy_entity_probe().contains_key(&bits),
        "entity must still be alive — else Grow==0 would hold vacuously (F8)"
    );
    let snap = sim.ledger_snapshot();
    assert_eq!(snap.blocked_lump + snap.blocked_cell, 1, "the gate must be evaluated exactly once over this one-window step");
    let grow_count = snap.grow_steps_total - snap.blocked_lump - snap.blocked_cell;
    assert_eq!(grow_count, 0, "the wide 2-window reserve must refuse the growth the narrow P-1 reserve would have allowed");
    assert_eq!(sim.conservation_residual(), 0);
}
