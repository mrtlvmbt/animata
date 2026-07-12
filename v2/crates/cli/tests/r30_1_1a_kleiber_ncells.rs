//! R30-1.1a (#412): weld the Kleiber metabolic term to the body's LIVE cell count
//! (`Σ ph.graph.module_cell_count`), gated behind `EconParams.metab_reads_n_cells` (default
//! `false`, byte-identical). Lives in the `cli` crate (not a sim-core `#[cfg(test)]`) so it runs
//! under the same build the golden CI job compiles.
//!
//! Two acceptance checks:
//! - `kleiber_charge_scales_with_live_cells_not_gene`: a full Square body (`n_cells = g_dev²`) and
//!   a sparse Ring body (`n_cells = 4(g_dev-1)`) sharing the SAME `Genome::size` gene pay DIFFERENT
//!   Kleiber charges when the flag is on — the Ring's dead center cell does NOT inflate its charge.
//!   Built via a REAL decode (`build_sim`), never `cellgraph_with_cells` (F1: that helper leaves
//!   `cell_positions` empty while populating `module_cell_count`, which would make this invariant
//!   pass vacuously — irrelevant here since the impl reads `module_cell_count` directly, but the
//!   fixture must still prove it against a REAL body, not a synthetic graph).
//! - `kleiber_ncells_r15_conservation`: with the flag ON, the metabolic charge still routes through
//!   the same energy-debit ledger — residual stays exactly 0 (R15).

use cli::{build_sim, config_with, DEFAULT_THREADS};
use sim_core::{
    grn_resolve, size_pow_three_quarters, BodyPlan, Boundary, CellType, EconParams, Genome,
    Gradient, GrnSpec, LayerSpec, MergeStrategy, MorphogenSpec, SimConfig,
};
use std::sync::Arc;

/// Bistable-matrix GRN spec (verbatim from `r30_1_1_income_extent.rs`'s `ring_gspec` — already
/// F7-validated): `input_weights=[0,0]` keeps the per-cell sampled gradient dead, so EVERY cell
/// resolves the SAME attractor (`CellType::B`) regardless of position or body plan — the two
/// fixtures below differ ONLY in `body_plan`, never in cell-type resolution.
fn kleiber_gspec() -> GrnSpec {
    GrnSpec::new(2, vec![32, -32, -32, 32], vec![0, 0], vec![0, 0], 3, 12, 0, 0, vec![112, 144])
}

#[test]
fn kleiber_gspec_resolves_to_cell_type_b() {
    let gspec = kleiber_gspec();
    let gradient = Gradient { g_dev: 1, cells: vec![0] };
    let (_state, ct, _steps) = grn_resolve(&gradient, &gspec);
    assert_eq!(ct, CellType::B, "kleiber_gspec's initial=[112,144] must resolve to CellType::B");
}

fn kleiber_mspec(g_dev: usize, body_plan: BodyPlan) -> MorphogenSpec {
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

/// One founder shaped by `body_plan` at `g_dev=3`, with the Kleiber weld gated by
/// `metab_reads_n_cells`. `d0_scaled: 0` disables background-death RNG (economy/01 §3, C-1) so a
/// same-seed/same-tick roll can't kill one fixture and not the other — a determinism-safe isolation
/// override, not a golden-touching one (background death is off in every shipped config's test
/// probes too). Genome fields other than `body_plan` are IDENTICAL across both fixtures (same
/// `Genome::founder(2)`: `size=4`, `move_speed=1`, `sense_range=1`), and `c_coord`/`enable_oxygen`/
/// `enable_mutation_load`/`ambient_tolerance` all stay at their inert defaults — so the ONLY
/// per-tick cost term that can differ between the two fixtures is the Kleiber term (F4 isolation).
fn kleiber_config(seed: u64, body_plan: BodyPlan, metab_reads_n_cells: bool) -> SimConfig {
    let founder =
        Genome::founder(2).with_specs(Some(Arc::new(kleiber_gspec())), Some(kleiber_mspec(3, body_plan)));
    let layer1 = LayerSpec { regen_rate: 0, flux_alpha_num: 1, flux_alpha_den: 4, flat_cap: 1000, world_cap_mult: 0 };
    let layer0_inert = LayerSpec { flux_alpha_num: 0, flux_alpha_den: 1, ..LayerSpec::default() };
    SimConfig {
        n_founders: 1,
        founder_templates: Some(vec![(founder, 1)]),
        n_layers: 2,
        layer_specs: [layer0_inert, layer1, LayerSpec::default(), LayerSpec::default()],
        econ: EconParams { metab_reads_n_cells, d0_scaled: 0, ..EconParams::default() },
        ..config_with(seed, DEFAULT_THREADS, MergeStrategy::Canonical)
    }
}

#[test]
fn kleiber_charge_scales_with_live_cells_not_gene() {
    const G_DEV: usize = 3;
    let n_square: i64 = (G_DEV * G_DEV) as i64; // 9 — full grid
    let n_ring: i64 = 4 * (G_DEV as i64 - 1); // 8 — perimeter only, center dead

    let mut square = build_sim(kleiber_config(1, BodyPlan::Square, true));
    let mut ring = build_sim(kleiber_config(1, BodyPlan::Ring, true));

    // Structural sanity — the two bodies really do have the claimed LIVE cell counts.
    assert_eq!(square.body_size_probe(), vec![n_square]);
    assert_eq!(ring.body_size_probe(), vec![n_ring]);

    square.step();
    ring.step();

    // Both fixtures share: same seed (identical world/RNG), same founder genome (same `size` gene,
    // same move_speed/sense_range), same GRN (same resolved CellType::B → same uptake_layer), and
    // `IncomeMode::Anchor` (default: income is flat in body size, EXT-0a's whole point) — so their
    // per-tick income is bit-identical. Any difference in post-tick energy is therefore attributable
    // ENTIRELY to the metabolic (Kleiber) term.
    let econ = EconParams::default();
    let n = econ.metab_period as i64;
    // Hand-computed Kleiber term per body, reading LIVE n_cells (not the shared `size` gene):
    let expected_delta =
        econ.k_size_metab * (size_pow_three_quarters(n_square as i32) - size_pow_three_quarters(n_ring as i32)) * n;
    assert!(expected_delta > 0, "the full Square must have a strictly larger Kleiber term than the sparse Ring");

    let ring_energy = ring.avg_energy();
    let square_energy = square.avg_energy();
    assert_eq!(
        ring_energy - square_energy,
        expected_delta,
        "Ring (8 live cells, dead center) must pay exactly {expected_delta} eu less than Square (9 live cells) \
         this tick — the dead center cell must not inflate the Ring's charge"
    );
}

#[test]
fn kleiber_ncells_r15_conservation() {
    let mut sim = build_sim(kleiber_config(2, BodyPlan::Ring, true));
    for _ in 0..200 {
        sim.step();
    }
    let residual = sim.conservation_residual();
    assert_eq!(residual, 0, "metab_reads_n_cells=true must still route the Kleiber charge through the same energy ledger (R15)");
}
