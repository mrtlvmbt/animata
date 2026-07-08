//! V-1: the three heritability teeth (issue's explicit vacuity guard — "≥2 CellTypes appear" is
//! NOT sufficient on its own; a razor-edge `initial` would pass that check via pure noise, not a
//! heritable program). Together (a)+(b)+(c) prove the repositioned `phase2_config` GRN spec
//! (`weights=[32,-32,-32,32]`, `initial=[144,112]`) produces genuinely HERITABLE, STABLE
//! phenotypic variation under the real `Genome::mutate` operator:
//!   (a) a mutated spec decodes to the SAME `CellType` on re-run (determinism — holds by
//!       construction, since `decode`/`morphogen`/`grn` are pure; asserted directly).
//!   (b) heritable + NOT a coin-flip: (i) a child inheriting the parent's spec with no NEW
//!       mutation keeps the parent's EXACT fate, and (ii) the fate-FLIP-RATE under the real
//!       mutation operator is a STRICT MINORITY (`0 < rate < 50%`, comfortably below a razor-edge
//!       ~100% coin-flip).
//!   (c) variation genuinely exists — a decode-level sweep of the real mutation operator produces
//!       ≥2 of {A,B,Mixed} (distinct from `phase2_liveness.rs`, which tests population BOUNDS, not
//!       fate diversity).
//! (b-ii) and (c) share the same sample (streams 0..N through the REAL `Genome::mutate`), per the
//! issue's explicit instruction — one measures the flip-RATE, the other the fate-SET.
//!
//! Runs on x86 (arch-independent: decode-level over a fixed sample, no pinned constant, no live
//! multi-generation `Sim` needed — `Genome::mutate`/`decode` are pure functions of
//! `(genome, stream)`/`(genome, econ)`).

use cli::phase2_config;
use sim_core::{CellType, Genome};
use std::sync::Arc;

const SEED: u64 = 0xA11A_2A11;
/// Sample size for the flip-rate/fate-set sweep — large enough that the measured rate is stable
/// (not a one-off), small enough to run instantly (decode-level, no field/ECS machinery).
const SAMPLE: u64 = 4000;

/// Reconstruct the phase2 founder genome with its heritable spec attached — the SAME production
/// values `Sim::new` seeds onto every founder at spawn (V-1's founder-spawn seam), rebuilt here at
/// the decode level so these teeth don't need a live multi-generation `Sim` run.
fn phase2_founder_genome() -> Genome {
    let econ = phase2_config(SEED).econ;
    Genome::founder(2).with_specs(econ.grn.clone().map(Arc::new), econ.morphogen)
}

/// One (spec_changed, resulting_fate) sample: mutate the founder with `stream`, then reset `size`
/// back to the founder's own value before decoding. `size` also seeds the morphogen gradient
/// (`morphogen.rs`) — resetting it isolates the GRN-spec-mutation effect from an unrelated Ф0
/// trait drift (`mutate` perturbs `size` independently, gated by the same `mutation_rate`), so a
/// measured flip is attributable to the spec, not a coincidental size change.
fn mutant_sample(founder: &Genome, stream: u64) -> (bool, Option<CellType>) {
    let econ = phase2_config(SEED).econ;
    let mut child = founder.mutate(stream, 2, false, 0, false, false, false, false, false, false, 0, 0, 0, 0);
    child.size = founder.size;
    let spec_changed = child.grn_spec != founder.grn_spec;
    let fate = child.decode(&econ).and_then(|ph| ph.cell_type);
    (spec_changed, fate)
}

/// (a) Determinism: decoding the SAME mutated genome twice yields byte-identical `cell_type`.
/// Holds by construction (`decode`/`morphogen`/`grn` are pure, no RNG/clock/thread-dependence —
/// E-2/E-3 determinism holds transitively) — asserted directly, not merely assumed.
#[test]
fn v1_mutated_genome_decode_is_bit_deterministic() {
    let founder = phase2_founder_genome();
    let econ = phase2_config(SEED).econ;
    for stream in [0x1234_5678u64, 0xDEAD_BEEF, 0xA11A_2A11, 42, 1_000_003] {
        let mut child = founder.mutate(stream, 2, false, 0, false, false, false, false, false, false, 0, 0, 0, 0);
        child.size = founder.size;
        let a = child.decode(&econ);
        let b = child.decode(&econ);
        assert_eq!(a, b, "decode(mutated genome, stream={stream}) must be byte-identical across repeated calls");
    }
}

/// (b-i) Heritability: a child inheriting the parent's spec with NO new mutation (forced by
/// setting `mutation_rate=0` so `mutate`'s gated draws never fire — the GRN-spec `Arc` stays
/// shared, unmutated) decodes to the parent's EXACT fate.
#[test]
fn v1_no_mutation_child_inherits_parents_exact_fate() {
    let econ = phase2_config(SEED).econ;
    let founder = phase2_founder_genome();
    let founder_fate = founder.decode(&econ).expect("phase2 founder must be viable").cell_type;

    let mut no_mutate_parent = founder.clone();
    no_mutate_parent.mutation_rate = 0;
    for stream in [0x1234_5678u64, 0xDEAD_BEEF, 7, 999] {
        let child = no_mutate_parent.mutate(stream, 2, false, 0, false, false, false, false, false, false, 0, 0, 0, 0);
        assert_eq!(
            child.grn_spec, no_mutate_parent.grn_spec,
            "mutation_rate=0 must leave the GRN spec byte-identical (no draw ever fires) at stream={stream}"
        );
        let child_fate = child.decode(&econ).expect("unmutated child must stay viable").cell_type;
        assert_eq!(
            child_fate, founder_fate,
            "a child inheriting the parent's spec with no new mutation must decode to the parent's EXACT fate (stream={stream})"
        );
    }
}

/// (b-ii) The concrete flip-rate gate: over a sample of real `Genome::mutate` outcomes (the
/// production heritable `mutation_rate=32`, not a hand-picked single mutation), among children
/// whose GRN spec actually changed vs the parent, the fraction whose fate differs from the
/// parent's must be a STRICT MINORITY — `0 < rate < 50%` (a razor-edge coin-flip is ~100%; this
/// is what actually fails on the `[128,128]` alternative the issue explicitly forbids). `> 0`
/// guarantees variation is reachable; `< 50%` (checked here at a stricter `< 0.4` margin) rules
/// out the coin-flip.
#[test]
fn v1_flip_rate_under_real_mutation_is_strict_minority() {
    let founder = phase2_founder_genome();
    let founder_fate = founder.decode(&phase2_config(SEED).econ).expect("founder must be viable").cell_type;

    let mut total_mutated = 0u32;
    let mut flips = 0u32;
    for stream in 0..SAMPLE {
        let (spec_changed, fate) = mutant_sample(&founder, stream);
        if spec_changed {
            total_mutated += 1;
            if fate != founder_fate {
                flips += 1;
            }
        }
    }
    assert!(
        total_mutated > 0,
        "no sampled stream (0..{SAMPLE}) produced a GRN-spec mutation — mutation_rate/salts drifted from what this test expects"
    );
    let rate = f64::from(flips) / f64::from(total_mutated);
    assert!(
        rate > 0.0,
        "flip rate must be > 0 (variation must be reachable) — got 0/{total_mutated} over {SAMPLE} streams"
    );
    assert!(
        rate < 0.4,
        "flip rate must be a STRICT MINORITY (< 40%, well below a razor-edge ~100% coin-flip) — \
         got {flips}/{total_mutated} = {:.1}%",
        rate * 100.0
    );
}

/// (c) Variation genuinely exists: across the SAME sample as (b-ii) (per the issue's explicit
/// instruction — one sample, two measurements), the real mutation operator reaches ≥2 of
/// {A,B,Mixed} through the real `morphogen→grn→classify` chain. Distinct from
/// `phase2_liveness.rs` (population BOUNDS) — this is fate DIVERSITY, decode-level, no live sim.
#[test]
fn v1_mutant_sweep_reaches_at_least_two_fates() {
    let founder = phase2_founder_genome();
    let mut fates: Vec<CellType> = Vec::new();
    for stream in 0..SAMPLE {
        let (_, fate) = mutant_sample(&founder, stream);
        if let Some(ct) = fate {
            fates.push(ct);
        }
    }
    assert!(!fates.is_empty(), "sweep must produce at least one viable, decoded mutant over {SAMPLE} streams");
    let first = fates[0];
    assert!(
        fates.iter().any(|ct| *ct != first),
        "the repositioned spec must produce ≥2 distinct fates across a real-mutation sweep — got only {first:?} for all {} viable samples",
        fates.len()
    );
}

/// E-6 non-regression (critic F13): the reposition touches ONLY `phase2_config`'s `initial`/
/// `weights` — `input_weights` stays `[0,0]` (drive dead), so phase2 remains structurally distinct
/// from E-6's drive-coupled test-only fixture (`input_weights=[8,0]`, `genome.rs`'s
/// `e6_fixture_spec_is_structurally_distinct_from_shipped_monomorphic_shape`), which asserts
/// exactly that inequality and must stay green (verified separately — `cargo test -p sim-core
/// --lib e6_` — this test pins the OTHER side of that same invariant from phase2's perspective).
#[test]
fn v1_phase2_spec_stays_drive_dead_after_reposition() {
    let econ = phase2_config(SEED).econ;
    let gspec = econ.grn.expect("phase2_config must carry a GRN spec");
    assert_eq!(
        gspec.input_weights, vec![0, 0],
        "phase2_config's input_weights must stay [0,0] (drive dead) after the V-1 reposition — \
         only initial/weights moved; a nonzero input_weights would collapse phase2 toward E-6's \
         drive-coupled test-only shape"
    );
}
