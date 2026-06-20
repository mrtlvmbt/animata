use super::*;
use crate::rng::Rng;

/// The continuity keystone: an empty-GRN founder develops to EXACTLY one structural cell —
/// biomass 1, no specialisation — i.e. the C0 organism, by construction.
#[test]
fn founder_develops_to_one_structural_cell() {
    let mut rng = Rng::new(1);
    let g = Genome::founder(&mut rng);
    let p = g.develop();
    assert_eq!(p, Phenotype { n_cells: 1, structural: 1, ..Default::default() });
    assert_eq!(p.complexity(), 0);
}

/// Development is bounded and deterministic for any genome (cost + replay).
#[test]
fn development_is_bounded_and_deterministic() {
    for seed in 0..200u64 {
        let mut rng = Rng::new(seed);
        // A mutated genome (non-empty GRN) — may grow a body.
        let g = Genome::founder(&mut rng).mutate(&mut rng, 0.3, 0.8);
        let p1 = g.develop();
        let p2 = g.develop();
        assert_eq!(p1, p2, "development not deterministic");
        assert!(p1.n_cells >= 1 && p1.n_cells as usize <= MAX_CELLS, "cell count out of range: {}", p1.n_cells);
        let typed = p1.effector + p1.storage + p1.sensor + p1.predator + p1.flight + p1.burrow + p1.photo;
        assert_eq!(typed + p1.structural, p1.n_cells);
    }
}

/// Mutation can grow multicellular AND specialised bodies (the mechanism isn't stuck at 1
/// cell) — over many random GRNs we see >1-cell bodies and ≥2-type complex ones.
#[test]
fn mutation_can_grow_complex_bodies() {
    let (mut multi, mut complex, mut maxn) = (0, 0, 0u32);
    for seed in 0..2000u64 {
        let mut rng = Rng::new(seed ^ 0xABCD);
        // Several mutation steps so the GRN drifts well away from empty.
        let mut g = Genome::founder(&mut rng);
        for _ in 0..5 {
            g = g.mutate(&mut rng, 0.3, 0.9);
        }
        let p = g.develop();
        if p.n_cells > 1 {
            multi += 1;
        }
        if p.complexity() == 2 {
            complex += 1;
        }
        maxn = maxn.max(p.n_cells);
    }
    eprintln!("of 2000 drifted GRNs: {multi} multicellular, {complex} complex, max cells {maxn}");
    assert!(multi > 50, "almost no multicellular bodies emerge: {multi}");
    assert!(complex > 5, "no complex (multi-type) bodies emerge: {complex}");
}
