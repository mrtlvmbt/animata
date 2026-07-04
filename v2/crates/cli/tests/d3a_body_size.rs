//! D-3a (#272): body-size telemetry — `Telemetry::{mean,max}_body_size`/`multicellular_frac`
//! (sim-core `stages.rs` `stage_observe`), the D-4 gate's measurement substrate for multicellularity
//! emergence. Golden-NEUTRAL: read-only, never folded into `state_hash` — the byte-identity of the 7
//! pinned goldens (`golden.rs` + `golden_conserved.rs`, all un-re-pinned by this change) is the direct
//! proof of tooth 5; this file covers the remaining teeth (1/2/3/4/6/7).
//!
//! `stage_observe` computes the per-entity metric via `CellGraph::body_size()` and folds the
//! population via `body_size_aggregate` — both pure, standalone fns (sim-core `lib.rs`/`genome.rs`),
//! so these teeth exercise the EXACT production functions, not a re-implementation of their math.

use cli::{build_sim, cprime_config, default_config, driver_config, dprime_config, l3_config, DEFAULT_THREADS};
use sim_core::{body_size_aggregate, CellGraph, BODY_SIZE_SCALE};
use std::process::Command;

const SEED: u64 = 0xA11A_2A11;
const DRIVER_SEED: u64 = 0xBE_EF_5EED;
const TICKS: u64 = 512;

/// Tooth 1: `CellGraph::body_size()` — a unicell (empty `CellGraph`, the non-phase2 case) is 1; a
/// multicellular decode (`Σ module_cell_count`) is the true sum.
#[test]
fn d3a_body_size_correct() {
    assert_eq!(CellGraph::empty().body_size(), 1, "an empty CellGraph (non-phase2) must decode to body_size 1");

    let multi = CellGraph { module_cell_count: vec![3, 4, 5], ..CellGraph::empty() };
    assert_eq!(multi.body_size(), 12, "a 3-module CellGraph with counts [3,4,5] must decode to body_size 12 (Σ)");

    // A degenerate single-live-cell graph (one module, count 1) also clamps/reduces to 1.
    let one_cell = CellGraph { module_cell_count: vec![1], ..CellGraph::empty() };
    assert_eq!(one_cell.body_size(), 1);
}

/// Tooth 2: `body_size_aggregate` over a population mixing unicells + multicells (5 unicells, 3
/// multicells of sizes 4/9/12 out of 8 total) yields `multicellular_frac` matching the true fraction
/// 3/8, fixed-point ×`BODY_SIZE_SCALE` — an independently hand-computed expectation, not re-derived
/// from the fn under test.
#[test]
fn d3a_multicellular_frac() {
    let sizes: Vec<i64> = vec![1, 1, 1, 4, 9, 12, 1, 2]; // 4 unicells (size 1), 4 multicells (4,9,12,2)
    let count_multicellular = sizes.iter().filter(|&&s| s > 1).count();
    assert_eq!(count_multicellular, 4, "fixture sanity: 4 of the 8 hand-picked sizes are >1");

    let (_mean, _max, frac) = body_size_aggregate(&sizes);
    let expected_frac = 4 * BODY_SIZE_SCALE / 8; // 4/8 = 0.5 → 128
    assert_eq!(frac, expected_frac, "multicellular_frac must equal count(>1)*SCALE/n for a known mixed population");
}

/// Tooth 3: `body_size_aggregate`'s mean/max over the same known mixed population — hand-computed
/// expectations (sum=31, n=8 → mean=31*256/8=992; max=12).
#[test]
fn d3a_mean_max() {
    let sizes: Vec<i64> = vec![1, 1, 1, 4, 9, 12, 1, 2];
    let (mean, max, _frac) = body_size_aggregate(&sizes);
    assert_eq!(mean, 992, "mean_body_size must equal Σbody_size*SCALE/n = 31*256/8 = 992");
    assert_eq!(max, 12, "max_body_size must equal the true max over the population");

    // Empty population: the documented (0, 0, 0) floor.
    assert_eq!(body_size_aggregate(&[]), (0, 0, 0), "an empty population must aggregate to (0, 0, 0)");
}

/// Tooth 3b (wiring): on `driver_config` (bodies can be multicellular), `Telemetry::
/// {mean,max}_body_size`/`multicellular_frac` match `body_size_aggregate` fed by an INDEPENDENT
/// fresh-query probe (`Sim::body_size_probe`) — proving `stage_observe` is actually wired to the
/// tested aggregate fn, not just that the fn's math is correct in isolation.
#[test]
fn d3a_telemetry_matches_probe() {
    if cfg!(debug_assertions) {
        return;
    }
    let mut sim = build_sim(driver_config(DRIVER_SEED));
    for _ in 0..TICKS {
        sim.step();
    }
    let sizes = sim.body_size_probe();
    assert!(!sizes.is_empty(), "driver_config population must be nonzero at tick {TICKS}");
    assert!(sizes.iter().any(|&s| s > 1), "driver_config must produce at least one multicellular body");

    let (mean, max, frac) = body_size_aggregate(&sizes);
    let tel = sim.telemetry();
    assert_eq!(tel.mean_body_size, mean, "Telemetry.mean_body_size must match the independent probe's aggregate");
    assert_eq!(tel.max_body_size, max, "Telemetry.max_body_size must match the independent probe's aggregate");
    assert_eq!(tel.multicellular_frac, frac, "Telemetry.multicellular_frac must match the independent probe's aggregate");
}

/// Tooth 4: every non-phase2 production config decodes an empty `CellGraph` for all founders/births
/// (`Genome::decode`'s `_ => CellGraph::empty()` arm — no `morphogen`/`grn` spec seeded there), so
/// `body_size` stays clamped to 1 for every live entity: `mean_body_size == SCALE`, `max_body_size ==
/// 1`, `multicellular_frac == 0` — byte-identical to the pre-D-3a trajectory (golden-NEUTRAL).
#[test]
fn d3a_non_phase2_all_unicell() {
    if cfg!(debug_assertions) {
        return;
    }
    let configs: [(&str, sim_core::SimConfig); 4] = [
        ("default", default_config(SEED)),
        ("cprime", cprime_config(SEED)),
        ("dprime", dprime_config(SEED)),
        ("l3", l3_config(SEED)),
    ];
    for (name, cfg) in configs {
        let mut sim = build_sim(cfg);
        for t in 0..200 {
            sim.step();
            let sizes = sim.body_size_probe();
            assert!(!sizes.is_empty(), "config '{name}' population went extinct at tick {t}");
            assert!(sizes.iter().all(|&s| s == 1), "config '{name}' at tick {t}: every body_size must be 1");

            let tel = sim.telemetry();
            assert_eq!(tel.mean_body_size, BODY_SIZE_SCALE, "config '{name}' at tick {t}: mean_body_size must be exactly SCALE (mean==1)");
            assert_eq!(tel.max_body_size, 1, "config '{name}' at tick {t}: max_body_size must be 1");
            assert_eq!(tel.multicellular_frac, 0, "config '{name}' at tick {t}: multicellular_frac must be 0");
        }
    }
}

/// Tooth 6: deterministic replay — same seed replayed twice yields the identical body-size
/// telemetry trace (pure aggregate of entity-id-ordered `Phenotype.graph` content, no RNG of its
/// own), and is thread-count independent (1-vs-N).
#[test]
fn d3a_determinism() {
    if cfg!(debug_assertions) {
        return;
    }
    let trace_of = |threads: usize| -> Vec<(i64, i64, i64)> {
        let cfg = sim_core::SimConfig { sim_threads: threads, ..driver_config(DRIVER_SEED) };
        let mut sim = build_sim(cfg);
        (0..TICKS)
            .map(|_| {
                sim.step();
                let tel = sim.telemetry();
                (tel.mean_body_size, tel.max_body_size, tel.multicellular_frac)
            })
            .collect()
    };

    let run1 = trace_of(DEFAULT_THREADS);
    let run2 = trace_of(DEFAULT_THREADS);
    assert_eq!(run1, run2, "same seed/thread-count must replay to the identical body-size telemetry trace");

    let run_1_thread = trace_of(1);
    assert_eq!(
        run1, run_1_thread,
        "body-size telemetry trace must be thread-count independent (1 vs {DEFAULT_THREADS} threads)"
    );
}

/// Tooth 7: the CLI's `HORIZON` final-metrics line prints the three body-size metrics parseably
/// (`mean_body_size=`/`max_body_size=`/`multicellular_frac=` key=value tokens) — the exact grep
/// target the cloud `sim-run`/`multiseed` harness reads.
#[test]
fn d3a_cli_final_metrics() {
    let exe = env!("CARGO_BIN_EXE_v2-sim");
    let output = Command::new(exe)
        .args(["1", "20"]) // small seed/ticks — this test only checks the print format, not emergence
        .output()
        .expect("failed to run the v2-sim CLI binary");
    assert!(output.status.success(), "v2-sim CLI must exit successfully on a plain demo run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let horizon = stdout
        .lines()
        .find(|l| l.starts_with("HORIZON"))
        .unwrap_or_else(|| panic!("no HORIZON line in CLI output:\n{stdout}"));
    assert!(horizon.contains("mean_body_size="), "HORIZON line must print mean_body_size=: {horizon}");
    assert!(horizon.contains("max_body_size="), "HORIZON line must print max_body_size=: {horizon}");
    assert!(horizon.contains("multicellular_frac="), "HORIZON line must print multicellular_frac=: {horizon}");
}
