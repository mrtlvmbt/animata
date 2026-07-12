//! R30-1.0 (#405): `CellGraph::cell_positions` correctness — proves the cold field folded into
//! `from_gradient`'s existing dead/live decode loop (`genome.rs:333-358`) is EXACTLY the kept set
//! `{(x,z) : !dead}`, row-major, not `live_mask` alone (critic F1: an apoptosed cell must be ABSENT).
//! Lives in the `cli` crate (not a sim-core `#[cfg(test)]`) so it runs under the same build the
//! golden CI job compiles, per #405's F3.

use sim_core::{grn_resolve, BodyPlan, CellGraph, Gradient, GrnSpec};

/// Linear (non-bistable) readout GRN: `state[0]` is monotone in the raw gradient value, so the
/// per-cell classification/apoptosis directly reflects the hand-picked gradient below.
fn linear_gspec() -> GrnSpec {
    GrnSpec::new(2, vec![0, 0, 0, 0], vec![1, 0], vec![0, 0], 0, 1, 0, 0, vec![0, 0])
}

/// 4x4 gradient with a dead "wall" row at `z=1` (value 0) surrounded by live value-10 rows.
fn wall_gradient() -> Gradient {
    Gradient {
        g_dev: 4,
        cells: vec![
            10, 10, 10, 10,
            0, 0, 0, 0,
            10, 10, 10, 10,
            10, 10, 10, 10,
        ],
    }
}

/// Threshold that kills exactly the wall row (derived from the real resolved gene-0 states, not a
/// guessed constant).
fn wall_apoptosis_threshold() -> i32 {
    let gspec = linear_gspec();
    let mut cg = gspec.clone();
    cg.sample_x = 0;
    cg.sample_z = 0;
    let dead_gradient = Gradient { g_dev: 1, cells: vec![0] };
    let live_gradient = Gradient { g_dev: 1, cells: vec![10] };
    let (dead_state, _, _) = grn_resolve(&dead_gradient, &cg);
    let (live_state, _, _) = grn_resolve(&live_gradient, &cg);
    assert!(
        dead_state[0] < live_state[0],
        "wall fixture must have a genuine state gap to threshold on; got dead={dead_state:?} live={live_state:?}"
    );
    dead_state[0] + 1
}

/// Shipped-default shape: `apoptosis_threshold=None`, `body_plan=Square` ⇒ every cell is live ⇒
/// `cell_positions` is the FULL `g_dev²` grid, row-major.
#[test]
fn square_no_apoptosis_yields_full_grid() {
    let gradient = Gradient { g_dev: 3, cells: vec![1; 9] };
    let gspec = linear_gspec();
    let graph = CellGraph::from_gradient(&gradient, &gspec, None, None, None, None, BodyPlan::Square);

    let expected: Vec<(u8, u8)> = (0..3u8).flat_map(|z| (0..3u8).map(move |x| (x, z))).collect();
    assert_eq!(graph.cell_positions, expected, "Square + no apoptosis must record every grid cell, row-major");
    assert_eq!(graph.cell_positions.len(), 9);
}

/// The load-bearing check (critic F1): an APOPTOSED cell must be ABSENT from `cell_positions` —
/// deriving from `live_mask` alone (ignoring apoptosis) would wrongly keep it.
#[test]
fn apoptosed_row_absent_from_cell_positions() {
    let gradient = wall_gradient();
    let gspec = linear_gspec();
    let t = wall_apoptosis_threshold();
    let graph = CellGraph::from_gradient(&gradient, &gspec, Some(t), None, None, None, BodyPlan::Square);

    let expected: Vec<(u8, u8)> = [0u8, 2, 3]
        .into_iter()
        .flat_map(|z| (0..4u8).map(move |x| (x, z)))
        .collect();
    assert_eq!(
        graph.cell_positions, expected,
        "apoptosed wall row (z=1) must be absent from cell_positions; only !dead cells kept, row-major"
    );
    assert!(
        graph.cell_positions.iter().all(|&(_, z)| z != 1),
        "no cell_positions entry may reference the apoptosed row z=1"
    );

    // Same kept set the union-find/module machinery uses: total live cells == Σ module_cell_count.
    let live_cell_total: i32 = graph.module_cell_count.iter().sum();
    assert_eq!(
        graph.cell_positions.len() as i32, live_cell_total,
        "cell_positions length must equal the graph's live-cell count (the same !dead kept set)"
    );
}
