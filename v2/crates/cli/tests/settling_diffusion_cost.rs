//! P4/SL-2: settling + O₂-diffusion cost — live-regime guard (F2).
//!
//! **Purpose**: verify that the rebase of settling_config onto phase2_oxygen_config (static-O₂
//! deficit regime) preserves the falsified conditions (c):
//! - (a) at least one entity reaches N>1 (the settling-selected size intermediate is REACHABLE)
//! - (b) mean O₂ level sits in a scarce, non-saturating band (hypoxia cost actually BITES)
//!
//! **Design**: criterion (c) is proven by unit tests (hypoxia factor presence, monotonicity, independence).
//! This test guards that the LIVE REGIME (settling_config under g_dev=1 base change) still supports
//! the conditions that make those unit tests relevant (entities grow, O₂ stays deficit).
//! A failure here is a BLOCKER — do NOT silently re-tune hypoxia_base or the O₂ cap.
//!
//! **Determinism**: double-run check with identical seed → identical results (R33).

use cli::{build_sim, settling_config};

const SEED: u64 = 0xD1FF_C05T;

/// (F2-a) Reachability guard: at least one entity reaches N>1 under settling selection.
/// This verifies that the g_dev=1 (unicellular founder) base still allows growth to N>1
/// (the size intermediate should be SELECTED FOR by settling and hypoxia balance).
/// BLOCKER if this fails: settling up-pressure or the hypoxia penalty regime is broken.
#[test]
fn settling_diffusion_cost_reachable_intermediate() {
    if cfg!(debug_assertions) {
        return;
    }

    let mut sim = build_sim(settling_config(SEED));
    let horizon = 512; // Same golden horizon as settling_golden.

    let mut max_body_size = 0i64;
    for _ in 0..horizon {
        sim.step();
        // Find the largest body in the population at this tick.
        for entity in sim.entities.read() {
            let body_size = entity.genome.body_size() as i64;
            if body_size > max_body_size {
                max_body_size = body_size;
            }
        }
    }

    assert!(
        max_body_size > 1,
        "at least one entity must reach N>1 (body_size > 1) under settling+O₂-cost regime; \
         max observed body_size = {} — BLOCKER: settlement selection or hypoxia regime is broken",
        max_body_size
    );
}

/// (F2-b) O₂ scarcity guard: mean O₂ level sits in a scarce, non-saturating band.
/// This verifies that under g_dev=1's lower consumption (smaller founders), the O₂ deficit
/// regime (inherited from phase2_oxygen_config) still creates SCARCITY (hypoxia > 0 for N>1).
/// If O₂ becomes saturated under the new base, hypoxia cost vanishes → BLOCKER.
/// Scarce band: mean O₂ < cap_o2 * 0.5 (less than half of cap — shows genuine deficit).
#[test]
fn settling_diffusion_cost_o2_scarcity_band() {
    if cfg!(debug_assertions) {
        return;
    }

    let cfg = settling_config(SEED);
    // Extract O₂ layer cap from the config (phase2_oxygen_config uses L1_O2_SPEC).
    let cap_o2 = cfg.layer_specs[2].flat_cap; // Layer 2 is O₂ in phase2_oxygen_config.
    assert!(cap_o2 > 0, "O₂ cap must be set (phase2_oxygen_config regime)");

    let mut sim = build_sim(cfg);
    let horizon = 512;

    let mut tick_count = 0;
    let mut sum_o2 = 0i64;

    for _ in 0..horizon {
        sim.step();
        tick_count += 1;

        // Sample mean O₂ from the field.
        let o2_total = sim.field.conserved_total(2); // Layer 2 = O₂.
        let world_cells = (64 * 64) as i64; // ProcgenWorld cell count (standard size).
        let mean_o2 = o2_total / world_cells.max(1);
        sum_o2 += mean_o2;
    }

    let avg_mean_o2 = sum_o2 / tick_count;

    // Scarcity band: mean O₂ < 50% of cap (shows deficit, not saturated).
    let scarcity_threshold = cap_o2 / 2;
    assert!(
        avg_mean_o2 < scarcity_threshold,
        "O₂ must remain scarce (avg_mean_o2 < cap/2) so hypoxia cost bites; \
         avg_mean_o2 = {}, cap_o2 = {}, threshold = {} — BLOCKER: O₂ regime not deficit",
        avg_mean_o2, cap_o2, scarcity_threshold
    );
}

/// (F2-c) Determinism: double-run with same seed → identical state hashes (R33).
#[test]
fn settling_diffusion_cost_determinism() {
    if cfg!(debug_assertions) {
        return;
    }

    let mut sim1 = build_sim(settling_config(SEED));
    let mut sim2 = build_sim(settling_config(SEED));

    let horizon = 256;
    for tick in 0..horizon {
        sim1.step();
        sim2.step();
        assert_eq!(
            sim1.state_hash(),
            sim2.state_hash(),
            "R33 determinism failed at tick {}",
            tick
        );
    }
}
