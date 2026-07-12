//! STEP A0 — concavity pre-gate: income(soma) shape under dol_economy (fixed-structure population).
//!
//! Closes #398. Supersedes the artefactual Rung-0 probe in PR #391 (income computed from a
//! FIXED-R static Monod ⇒ income = demand·soma = linear by construction ⇒ a guaranteed-PEAK
//! parabola that could never fire NULL — F1-F4 from the kit-critic round, see the issue for the
//! full trace).
//!
//! **The question:** under `dol_economy=true` (`stages.rs:557`/`583`, gated `if econ.dol_economy`;
//! the socket Rung-0 will run under — `dr0_config`, `cli/src/lib.rs:465`), is per-soma income
//! BOUNDED (clamps early — concave, Michod's specialist-loses precondition) or UNBOUNDED/LINEAR
//! (no clamp) at the densities that matter?
//!
//! **Design (RnD-corrected, F3/F5/F6):**
//! - `build_imposed_split_body` (copied from `feat/topo-diff-rung0`'s
//!   `dol_germ_repro_interior_optimum_probe.rs` — that branch's helper is not reachable from
//!   `main`) builds a hand-constructed `CellGraph` at a FIXED `body_size` with an arbitrary
//!   germ:soma split.
//! - A LIVE population of `n_founders` such bodies (FIXED body_size = g_dev² = 16, g_dev=4, the
//!   `gdev_cap`/`phase2_config` default) is imposed onto a freshly built `Sim` via
//!   `Sim::impose_graph_probe` (bypasses `Genome::decode` — `decode()` only runs once at spawn, so
//!   this holds for the entity's whole life) — F5: body-size never varies within a curve.
//! - `Sim::lock_repro_probe` sets every founder's `repro_threshold=i32::MAX`/`mutation_rate=0` so
//!   reproduction never fires (the only other way `Phenotype.graph` could change) — F6: the swept
//!   soma range persists for the whole run instead of drifting to the natural small attractor.
//! - `econ.d0_scaled=0` (background death off): with reproduction disabled there is no birth to
//!   replenish the population, and the shipped d0 hazard has a ~1000-tick mean lifetime — over an
//!   8000-tick horizon that would extinguish an unreplenished population long before the horizon,
//!   starving the late-run bins of samples. STARVATION death (`energy<=0`, driven by the very
//!   income this probe measures) stays fully active — it is part of the real economy, not a
//!   confound to remove.
//! - Soma ∈ {1..body_size−1} is swept across founders (cycled assignment, deterministic
//!   entity-id order). Germ is always ≥1 across the sweep (germ=0 is the analytic sterile edge,
//!   never imposed here — see the GERM AXIS note below).
//! - income(soma) is read from `Telemetry::income_probe` (STEP-A0 addition, `sim-core/src/lib.rs`)
//!   — NOT `Telemetry::income_record`, which is drained by `std::mem::take` inside
//!   `stage_observe` (`stages.rs:1591`) before `step()` returns control to the caller.
//!   `income_probe` mirrors `income_record`'s content but is never taken (same pattern as
//!   `entity_contention_rate`), so it is safely readable after every `step()`. Accumulated across
//!   ALL bodies and ticks for the whole horizon (not a single end-of-run snapshot) — measured
//!   solely from this booked channel, never from a `demand·soma` closed form (the #391 mistake).
//!
//! **GERM AXIS (analytic, NOT measured):** under `dol_economy`, germ=0 is sterile by construction
//! and germ≥1 is a flat fertility step (`stages.rs:1419-1426`) — a step function, not a curve to
//! probe. This probe never imposes germ=0 (the soma sweep excludes body_size, i.e. germ excludes 0).
//!
//! **GOLDEN-NEUTRAL:** `Telemetry::income_probe` / `Sim::{soma_count_entity_probe,
//! impose_graph_probe, lock_repro_probe}` are purely additive/observational — `Telemetry` is never
//! folded into `state_hash` (folds only Position/Energy/Genome/BrainState/BrainOutput/Velocity/
//! MineralQuota, see `Sim::state_hash`), so no existing golden re-pins.
//!
//! Heavy (3 densities × 3 seeds × 8000 ticks) — `#[ignore]`d; run via `scripts/sim-run.sh`.

use cli::{build_sim, phase2_config};
use sim_core::{CellGraph, CellType, DetMap};
use std::collections::BTreeMap;

const GDEV: i64 = 4; // gdev_cap/phase2_config default — g_dev fixed for the whole probe (F5)
const BODY_SIZE: i64 = GDEV * GDEV; // 16
const DEFAULT_TICKS: u64 = 8000;
const SEEDS: [u64; 3] = [1, 2, 3];

/// Copied from `feat/topo-diff-rung0`'s `dol_germ_repro_interior_optimum_probe.rs` (verified
/// signature per #398's ТЗ; that branch's helper is not reachable from `main`). Builds a
/// hand-constructed body at a FIXED `body_size` with an arbitrary germ:soma split: a germ module
/// (type B, `germ_count` cells) + a soma module (type A, `body_size - germ_count` cells).
fn build_imposed_split_body(body_size: i64, germ_count: i64) -> CellGraph {
    let soma_count = (body_size - germ_count).max(0).min(body_size);
    let germ_count_i32 = (body_size - soma_count) as i32;
    let soma_count_i32 = soma_count as i32;

    let mut module_type = vec![];
    let mut module_cell_count = vec![];
    let mut module_is_germ = vec![];

    if germ_count_i32 > 0 {
        module_type.push(CellType::B);
        module_cell_count.push(germ_count_i32);
        module_is_germ.push(true);
    }
    if soma_count_i32 > 0 {
        module_type.push(CellType::A);
        module_cell_count.push(soma_count_i32);
        module_is_germ.push(false);
    }

    let n_modules = module_type.len();
    CellGraph {
        g_dev: GDEV as usize,
        module_type,
        module_cell_count,
        module_is_germ,
        module_reachable: vec![true; n_modules],
        module_consortium: (0..n_modules).collect(),
        cell_positions: Vec::new(),
    }
}

/// Classify income(soma) shape from binned (soma -> mean grant) points: BOUNDED if the marginal
/// Δgrant/Δsoma over the LATE third of the swept range has decayed to < 30% of the EARLY third's
/// marginal (clamps before the range saturates); LINEAR/UNBOUNDED otherwise (marginal stays
/// roughly constant across the whole range). Needs >= 4 points to have an early/late split.
fn classify_shape(curve: &[(i64, f64, u64)]) -> (&'static str, f64, f64) {
    if curve.len() < 4 {
        return ("INSUFFICIENT_DATA", 0.0, 0.0);
    }
    let slopes: Vec<f64> = curve.windows(2).map(|w| w[1].1 - w[0].1).collect();
    let third = (slopes.len() / 3).max(1);
    let early: f64 = slopes[..third].iter().sum::<f64>() / third as f64;
    let late: f64 = slopes[slopes.len() - third..].iter().sum::<f64>() / third as f64;
    let verdict = if early > 0.0 && late < early * 0.3 { "BOUNDED" } else { "LINEAR" };
    (verdict, early, late)
}

/// STEP A0: measure income(soma) under `dol_economy=true` from a live stepping population of
/// fixed-structure imposed-split bodies, at >= 3 densities, classify BOUNDED vs LINEAR per
/// density, and emit the pre-gate routing line.
#[test]
#[ignore]
fn step_a0_concavity_pregate() {
    let ticks = std::env::var("STEP_A0_TICKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_TICKS);

    // Densities (agents/cell) at world_dim=64 (4096 cells) — matches EXT-0a's shipped=0.02 anchor.
    let densities: [(&str, u64); 3] = [
        ("sparse", 41),  // 0.01 x 4096
        ("shipped", 82), // 0.02 x 4096 (production density)
        ("dense", 205),  // 0.05 x 4096
    ];

    println!(
        "\nSTEP-A0 concavity pre-gate: income(soma) under dol_economy=true, g_dev={GDEV} (body_size={BODY_SIZE}), ticks={ticks}"
    );
    println!(
        "GERM AXIS: analytic (germ=0 sterile by construction; dol_economy germ>=1 is a flat fertility step, stages.rs:1419-1426) — NOT measured here."
    );

    let mut verdict_by_density: BTreeMap<&str, &str> = BTreeMap::new();

    for (label, n_founders) in &densities {
        // soma -> (sum_booked_grant, n_samples), accumulated across all seeds and all ticks.
        let mut income_accum: BTreeMap<i64, (i64, u64)> = BTreeMap::new();

        for &seed in &SEEDS {
            let mut cfg = phase2_config(seed);
            cfg.econ.dol_economy = true; // STEP-A0: the socket under test (as in dr0_config)
            cfg.econ.d0_scaled = 0; // no repro-replacement over 8000 ticks — see module doc
            cfg.n_founders = *n_founders;

            let mut sim = build_sim(cfg);

            // Impose the fixed-structure population: sweep soma in {1..BODY_SIZE-1}, cycled over
            // founders in deterministic entity-id order (F5/F6: body_size fixed, structure imposed
            // once, held by disabling reproduction below).
            let entity_bits: Vec<u64> = sim.body_size_entity_probe().keys().copied().collect();
            let mut graphs: DetMap<u64, CellGraph> = DetMap::new();
            for (i, &bits) in entity_bits.iter().enumerate() {
                let soma = 1 + (i as i64 % (BODY_SIZE - 1));
                let germ = BODY_SIZE - soma;
                graphs.insert(bits, build_imposed_split_body(BODY_SIZE, germ));
            }
            sim.impose_graph_probe(&graphs);
            sim.lock_repro_probe();

            // Soma count is fixed for the whole run (no repro/mutation) — one read suffices.
            let soma_by_entity = sim.soma_count_entity_probe();
            for (i, &bits) in entity_bits.iter().enumerate() {
                let expected = 1 + (i as i64 % (BODY_SIZE - 1));
                assert_eq!(
                    soma_by_entity.get(&bits).copied().unwrap_or(-1),
                    expected,
                    "imposed soma split must be readable back via soma_count_entity_probe"
                );
            }

            for _ in 0..ticks {
                sim.step();
                // income(soma) is MEASURED from booked telemetry (income_probe), never from a
                // demand*soma closed form — see the module doc's #391 warning.
                let tel = sim.telemetry();
                for (bits, &(_photo, got)) in tel.income_probe.iter() {
                    if let Some(&soma) = soma_by_entity.get(bits) {
                        let e = income_accum.entry(soma).or_insert((0, 0));
                        e.0 += got;
                        e.1 += 1;
                    }
                }
            }
        }

        let curve: Vec<(i64, f64, u64)> = income_accum
            .iter()
            .map(|(&soma, &(sum, n))| (soma, sum as f64 / n.max(1) as f64, n))
            .collect();
        let (verdict, early_slope, late_slope) = classify_shape(&curve);
        verdict_by_density.insert(label, verdict);

        let points: String = curve
            .iter()
            .map(|(soma, mean, n)| format!("{}:{:.2}:{}", soma, mean, n))
            .collect::<Vec<_>>()
            .join(";");
        println!(
            "STEP-A0 density={:<8} verdict={:<9} early_slope={:.3} late_slope={:.3} points=[{}]",
            label, verdict, early_slope, late_slope, points
        );

        assert!(
            !curve.is_empty(),
            "density={label}: zero income samples — measurement pipeline broken (extinction or booked channel never populated)"
        );
    }

    // Pre-gate routing line (the deliverable PM reads): BOUNDED at EITHER shipped or dense density
    // is sufficient to route to topology; only LINEAR at BOTH runs the full Rung-0.
    let shipped_v = verdict_by_density.get("shipped").copied().unwrap_or("UNKNOWN");
    let dense_v = verdict_by_density.get("dense").copied().unwrap_or("UNKNOWN");
    let routing = if shipped_v == "BOUNDED" || dense_v == "BOUNDED" {
        "ROUTE: BOUNDED at shipped/dense density => Michod holds => germ/soma specialist predicts NULL => skip full Rung-0, route to Rung 1/2 (topology)"
    } else {
        "ROUTE: LINEAR at all densities => economy escapes concavity => run the full specialist-vs-generalist Rung-0"
    };
    println!("{routing}");
    println!("STEP-A0 complete. PM reads the per-density verdicts + routing line.");
}
