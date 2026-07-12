//! R30-1.1 (#408): `IncomeMode::Extent` — the footprint harvest rewired to read `CellGraph.cell_positions`
//! (the LIVE shape, R30-1.0/#405) instead of a filled `side²` square. Lives in the `cli` crate (not a
//! sim-core `#[cfg(test)]`) so it runs under the same build the golden CI job compiles.
//!
//! Three acceptance checks:
//! - `extent_income_sums_live_cells_only`: a sparse Ring body's booked income equals Σ over its LIVE
//!   cells ONLY, hand-computed via the pure `monod_demand` formula against a KNOWN flat field value —
//!   never by running `Footprint` mode on a sparse body (that lane's `body_size == side²` debug_assert
//!   would PANIC on a Ring body, F3).
//! - `extent_income_r15_conservation`: the Extent lane books through the same `conserved_take`/ledger
//!   as Footprint — energy residual stays exactly 0 (R15).
//! - `extent_empty_cell_positions_yields_zero_income_no_panic`: a fully-apoptosed body (every cell
//!   dead, `cell_positions` empty) earns ZERO income and does not panic — no anchor-fallback (F2/F5).

use cli::{build_sim, config_with, DEFAULT_THREADS};
use sim_core::{
    monod_demand, BodyPlan, Boundary, EconParams, Genome, GrnSpec, IncomeMode, LayerSpec,
    MergeStrategy, MorphogenSpec, SimConfig, GRN_EXPR_MAX,
};
use std::sync::Arc;

/// Bistable-matrix GRN spec (reused verbatim from `cli::phase2_config` — already F7-validated).
/// The per-cell TYPE it resolves is irrelevant to `cell_positions` (live/dead comes from
/// `body_plan`/`apoptosis_threshold` only), so any valid spec works here.
fn ring_gspec() -> GrnSpec {
    GrnSpec::new(2, vec![32, -32, -32, 32], vec![0, 0], vec![0, 0], 3, 12, 0, 0, vec![144, 112])
}

fn base_mspec(g_dev: usize, apoptosis_threshold: Option<i32>, body_plan: BodyPlan) -> MorphogenSpec {
    MorphogenSpec {
        g_dev,
        n_dev: 8,
        boundary: Boundary::Reflecting,
        diffuse_shift: 3,
        decay_num: 1,
        decay_shift: 4,
        seed_scale: 4096,
        stop_threshold: 0,
        apoptosis_threshold,
        germ_threshold: None,
        supply_source: None,
        adhesion_threshold: None,
        body_plan,
    }
}

/// A single founder shaped `BodyPlan::Ring` at `g_dev=3` (perimeter-only: `4*(g_dev-1)=8` live cells
/// out of the 9-cell grid, center dead — `topology_mask`, already unit-pinned in `morphogen.rs`).
/// `uptake_layer` is redirected to layer 1, a flat/uniform cap with `regen_rate=0` and no other
/// consumer, so its per-cell resource level is a KNOWN CONSTANT (`flat_cap/2`, `fields::new_layered`)
/// — the test can hand-compute expected income without reading any live field state.
fn ring_extent_config(seed: u64, flat_cap: i64) -> SimConfig {
    let mut founder = Genome::founder(2).with_specs(Some(Arc::new(ring_gspec())), Some(base_mspec(3, None, BodyPlan::Ring)));
    founder.uptake_layer = 1;

    let layer1 = LayerSpec { regen_rate: 0, flux_alpha_num: 1, flux_alpha_den: 4, flat_cap, world_cap_mult: 0 };
    // Layer 0 is unused (founder.uptake_layer=1 redirects harvest to layer1 above), but
    // `build_sim` computes `flux_k_from_alpha` for every layer index < n_layers regardless —
    // `LayerSpec::default()` has `flux_alpha_den=0`, which divides by zero there. `flux_alpha_num:
    // 0` (α=0 ⇒ k=0 ⇒ no diffusion) with a non-zero `flux_alpha_den` is the inert, safe spec
    // (mirrors `cli::L2_DETRITUS_SPEC`'s no-diffusion pattern).
    let layer0_inert = LayerSpec { flux_alpha_num: 0, flux_alpha_den: 1, ..LayerSpec::default() };
    SimConfig {
        n_founders: 1,
        founder_templates: Some(vec![(founder, 1)]),
        n_layers: 2,
        layer_specs: [layer0_inert, layer1, LayerSpec::default(), LayerSpec::default()],
        econ: EconParams {
            income_mode: IncomeMode::Extent,
            // F4: dol_economy multiplies demand by soma count; an Extent copy under it would
            // double-count size (N_live contestants × soma). Keep it off for this test.
            dol_economy: false,
            ..EconParams::default()
        },
        ..config_with(seed, DEFAULT_THREADS, MergeStrategy::Canonical)
    }
}

#[test]
fn extent_income_sums_live_cells_only() {
    const FLAT_CAP: i64 = 1000;
    let mut sim = build_sim(ring_extent_config(1, FLAT_CAP));
    sim.step();

    let tel = sim.telemetry();
    assert_eq!(tel.income_probe.len(), 1, "exactly one founder must have booked income this tick");
    let (_photo, got) = *tel.income_probe.values().next().unwrap();

    // Hand-computed expectation — NOT derived by running Footprint (which would panic on this
    // sparse body's body_size != side² debug_assert, F3):
    // - N_live = 4*(g_dev-1) = 8 (Ring perimeter at g_dev=3, morphogen.rs's own pinned formula).
    // - r_cell = flat_cap/2 (the layer's initial mass, `fields::CpuFieldStore::new_layered`),
    //   identical at every grid cell since layer 1 is flat/uniform and regen_rate=0.
    // - Each of the 8 live cells maps to a DISTINCT field cell (offsets 0..2 inside a 3x3 block,
    //   world_dim=64 >> 3, no wrap), so there's no self-contention: grant == full monod demand.
    let n_live: i64 = 4 * (3 - 1);
    let r_cell = FLAT_CAP / 2;
    let per_cell = monod_demand(EconParams::default().u_max, EconParams::default().km, r_cell);
    let expected = per_cell * n_live;

    assert_eq!(
        got, expected,
        "Extent income must equal Σ monod_demand over the 8 LIVE Ring cells only, not 9 (g_dev²)"
    );
}

#[test]
fn extent_income_r15_conservation() {
    let mut sim = build_sim(ring_extent_config(2, 1000));
    for _ in 0..200 {
        sim.step();
    }
    let residual = sim.conservation_residual();
    assert_eq!(residual, 0, "IncomeMode::Extent must book through conserved_take/ledger with zero residual (R15)");
}

/// F2/F5 PINNED fixture (mirrors `genome.rs`'s `m7b_empty_body_valid`): a threshold strictly above
/// `GRN_EXPR_MAX` guarantees `state[0] < t` for EVERY cell, regardless of GRN spec — the whole grid
/// apoptoses, `cell_positions` is empty, `decode()` still returns `Some` (never panics).
fn fully_apoptosed_extent_config(seed: u64) -> SimConfig {
    SimConfig {
        n_founders: 3,
        econ: EconParams {
            morphogen: Some(base_mspec(3, Some(GRN_EXPR_MAX + 1), BodyPlan::Square)),
            grn: Some(ring_gspec()),
            income_mode: IncomeMode::Extent,
            dol_economy: false,
            ..EconParams::default()
        },
        ..config_with(seed, DEFAULT_THREADS, MergeStrategy::Canonical)
    }
}

#[test]
fn extent_empty_cell_positions_yields_zero_income_no_panic() {
    let mut sim = build_sim(fully_apoptosed_extent_config(3));

    // Structural sanity: every founder really did decode to an empty graph (body_size floors to 1).
    for (&bits, &size) in sim.body_size_entity_probe().iter() {
        assert_eq!(size, 1, "entity {bits:x} must have body_size==1 (empty CellGraph floor)");
    }

    for _ in 0..5 {
        sim.step(); // must not panic — the Extent flat_map over an empty cell_positions yields nothing
    }

    // R30 north-star (F2/F5): a fully-apoptosed body earns ZERO income under Extent — no
    // anchor-fallback (which would harvest from a dead anchor cell, the exact cheat R30 kills).
    // Zero contestants means these entities never enter `entity_income_map` at all this tick, so
    // the map itself must be empty, not merely absent-of-nonzero-entries.
    let tel = sim.telemetry();
    assert!(
        tel.income_probe.is_empty(),
        "a fully-apoptosed body must generate ZERO Extent contestants (got {:?})",
        tel.income_probe
    );
}
