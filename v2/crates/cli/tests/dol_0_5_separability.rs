//! DL-0.5 deterministic germ:soma-separability test (resolves critic F1).
//!
//! **F1 Discriminant:** is germ:soma a heritable axis SEPARABLE from body_size?
//!
//! This test proves phenotypic separability: at a FIXED `g_dev` (⇒ fixed body_size = g_dev²),
//! varying the GRN input_weights must produce DIFFERENT germ:soma ratios. If it does, germ:soma
//! is not a pure function of size ⇒ separable axis EXISTS. If germ:soma is invariant to GRN
//! weights at fixed g_dev, F1 FAILS (the honest answer — report the numbers).
//!
//! **Scope:** Pure decode-level test, no ecology, no Sim, no ticks, no sim-run scenario.
//! Deterministic, runs in "v2 sim" CI job. Logic fully checkable by reading.
//!
//! **Design:**
//! 1. Build a base genome with FIXED g_dev=4, germ_threshold=Some(5), size=21 (viable).
//! 2. Build a GRN spec with m7a_live_drive pattern: weights/bias/initial fixed, input_weights[0] varies.
//! 3. Sweep input_weights[0] ∈ {0, 4, 8, 16, 32, 64}; hold all else constant.
//! 4. Decode each → Phenotype.graph (CellGraph).
//! 5. Compute per-genome: germ_cells = Σ count where is_germ, soma_cells = Σ count where !is_germ.
//! 6. Assert (a) ≥2 DISTINCT (germ,soma) outcomes, (b) ≥1 genuine mix (both > 0, n_modules ≥ 2).
//! 7. Print DOL-SEP: table per value for CI log visibility.

use cli::default_config;
use sim_core::{GrnSpec, MorphogenSpec, Boundary, Genome};
use std::sync::Arc;

const SEED: u64 = 0xA11A_2A11;
const VIABLE_SIZE: i32 = 21; // Must be > 3 to avoid is_viable_size gating out

/// Build a GRN spec with fixed weights/bias/initial and variable input_weights[0].
/// Pattern matches m7a_live_drive but with sweepable input_weights[0].
fn grn_spec_with_drive(input_weight_0: i32) -> GrnSpec {
    GrnSpec::new(
        2,                                               // n_genes
        vec![64, -64, -64, 64],                          // weights (fixed)
        vec![input_weight_0, 0],                          // input_weights (varies [0])
        vec![0, 0],                                      // bias (fixed)
        3,                                               // initial_idx
        12,                                              // output_idx
        0,                                               // refractory
        0,                                               // noise
        vec![0, sim_core::GRN_EXPR_MAX],                 // output_range
    )
}

/// Build a morphogen spec with fixed g_dev=4, germ_threshold=Some(5), all others neutral.
fn morphogen_spec() -> MorphogenSpec {
    MorphogenSpec {
        g_dev: 4,                           // Fixed: size = g_dev² = 16 cells
        n_dev: 8,                           // Standard value
        boundary: Boundary::Reflecting,     // Standard
        diffuse_shift: 3,                   // Standard (CFL-safe)
        decay_num: 1,                       // Standard
        decay_shift: 4,                     // Standard
        seed_scale: 64,                     // Standard
        stop_threshold: 0,                  // No early stop
        apoptosis_threshold: None,          // Gate off (germ/soma separable from apoptosis)
        germ_threshold: Some(5),            // GATE ON: module <= 5 cells is germ
        supply_source: None,                // Gate off (germ/soma separable from supply)
        adhesion_threshold: None,           // Gate off (germ/soma separable from adhesion)
    }
}

#[test]
fn dol_germsoma_separable_from_size() {
    let econ = default_config(SEED).econ;
    let mspec = morphogen_spec();

    // Sweep input_weights[0] across {0, 4, 8, 16, 32, 64}.
    let sweep_values = vec![0i32, 4, 8, 16, 32, 64];

    let mut outcomes: Vec<(i32, i32, usize)> = Vec::new(); // (germ, soma, n_modules)
    let mut distinct_outcomes = std::collections::HashSet::new();
    let mut has_genuine_mix = false;

    eprintln!("DOL-SEP: input_weight_0, n_modules, germ_cells, soma_cells");

    for input_weight_0 in &sweep_values {
        // Build genome with this input_weights[0].
        let gspec = grn_spec_with_drive(*input_weight_0);
        let mut genome = Genome::founder(2).with_specs(Some(Arc::new(gspec)), Some(mspec));
        genome.size = VIABLE_SIZE;

        // Decode to phenotype.
        let phenotype = match genome.decode(&econ) {
            Some(ph) => ph,
            None => {
                panic!(
                    "Genome with input_weight_0={} failed to decode; size={}, germ_threshold=Some(5)",
                    input_weight_0, VIABLE_SIZE
                );
            }
        };

        // Extract germ/soma counts from CellGraph.
        let graph = &phenotype.graph;
        let germ_cells: i32 = graph
            .module_cell_count
            .iter()
            .zip(graph.module_is_germ.iter())
            .filter(|(_, &is_germ)| is_germ)
            .map(|(count, _)| count)
            .sum();
        let soma_cells: i32 = graph
            .module_cell_count
            .iter()
            .zip(graph.module_is_germ.iter())
            .filter(|(_, &is_germ)| !is_germ)
            .map(|(count, _)| count)
            .sum();
        let n_modules = graph.num_modules();

        outcomes.push((germ_cells, soma_cells, n_modules));
        distinct_outcomes.insert((germ_cells, soma_cells));

        // Check for genuine mix: both germ > 0 AND soma > 0, n_modules ≥ 2.
        if germ_cells > 0 && soma_cells > 0 && n_modules >= 2 {
            has_genuine_mix = true;
        }

        eprintln!("DOL-SEP: {}, {}, {}, {}", input_weight_0, n_modules, germ_cells, soma_cells);
    }

    // Assertion (a): ≥2 distinct (germ,soma) outcomes.
    assert!(
        distinct_outcomes.len() >= 2,
        "FAIL F1: germ:soma not separable from size — only {} distinct outcome(s) across input_weights sweep: {:?}",
        distinct_outcomes.len(),
        outcomes
    );

    // Assertion (b): ≥1 genuine mix (germ > 0, soma > 0, n_modules ≥ 2).
    assert!(
        has_genuine_mix,
        "FAIL F1: no genuine germ/soma mix found — substrate can't self-organize; outcomes: {:?}",
        outcomes
    );

    eprintln!("PASS: germ:soma is separable from body_size (F1 satisfied)");
}
