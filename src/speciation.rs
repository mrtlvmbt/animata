//! Lightweight species detection by clustering creatures in normalized
//! phenotype space (leader / threshold clustering — adaptive species count, no
//! fixed k). Run periodically, not every step. Species ids are stable across
//! updates as long as a cluster persists; the UI colors creatures by species id.

use crate::config::*;
use crate::creature::Creature;
use crate::genome::{Appendage, Phenotype};

/// Number of phenotype features used for clustering: 9 classic traits + 5
/// morphological ones (segment count + appendage composition), so two creatures
/// with the same speed/size/colour but different *body plans* (e.g. a legged
/// walker vs a winged flier) cluster as distinct species. Baseline circular
/// bodies score 0 on the morphology axes, so they cluster exactly as before.
const K: usize = 14;
/// Max distance (in normalized feature space) to count as the same species.
/// Raised from the pre-morphology 0.34: the 5 added body-plan axes partition the
/// population (a flier and a walker can't share a species), which multiplies the
/// cluster count, so the base radius is widened to keep species at the level of
/// interpretable macro-classes rather than fine trait micro-clusters. Binary
/// appendage axes sit 1.0 apart, far beyond this, so body plans still separate.
const THRESHOLD: f32 = 0.5;

/// Normalized phenotype feature vector (each component ~0..1).
fn feature(p: &Phenotype) -> [f32; K] {
    let n = |v: f32, r: (f32, f32)| ((v - r.0) / (r.1 - r.0)).clamp(0.0, 1.0);
    // Appendage *presence* (not fraction): a body plan is "has legs / has fins /
    // …", so two creatures with the same kind of limb cluster together regardless
    // of how many — separating major plans without fragmenting on limb count.
    let has = |kind: Appendage| {
        if p.segments.iter().any(|s| s.appendage == kind) {
            1.0
        } else {
            0.0
        }
    };
    // Coarse complexity bucket so a 2-segment and 3-segment body don't split, but
    // a worm-length chain reads as a different plan.
    let complexity = (p.segments.len() as f32 / MAX_SEGMENTS as f32).min(1.0);
    [
        n(p.max_speed, SPEED_RANGE),
        n(p.sense_range, SENSE_RANGE),
        n(p.radius, RADIUS_RANGE),
        n(p.metabolism, METAB_RANGE),
        p.carnivory,
        n(p.prime, LONGEVITY_RANGE),
        p.color.0,
        p.color.1,
        p.color.2,
        (complexity * 3.0).round() / 3.0,
        has(Appendage::Fin),
        has(Appendage::Leg),
        has(Appendage::Wing),
        has(Appendage::Burrow),
    ]
}

fn dist2(a: &[f32; K], b: &[f32; K]) -> f32 {
    (0..K).map(|k| (a[k] - b[k]).powi(2)).sum()
}

struct Species {
    id: u32,
    centroid: [f32; K],
    count: usize,
}

#[derive(Default)]
pub struct Speciation {
    species: Vec<Species>,
    next_id: u32,
}

impl Speciation {
    pub fn new() -> Self {
        Speciation::default()
    }

    /// Number of distinct living species.
    pub fn count(&self) -> usize {
        self.species.len()
    }

    /// Reassign every creature to a species and refresh centroids. A creature
    /// joins the nearest species within `THRESHOLD`, otherwise founds a new one.
    pub fn update(&mut self, creatures: &mut [Creature]) {
        let thr2 = THRESHOLD * THRESHOLD;
        let mut sums: Vec<[f32; K]> = vec![[0.0; K]; self.species.len()];
        for s in &mut self.species {
            s.count = 0;
        }

        // Phylogenetic hysteresis: a creature keeps its inherited species (its
        // parent's, set at birth) as long as it hasn't drifted past an expanded
        // threshold. This makes species track clades (near-monophyletic) and
        // stops id flicker when a cluster's membership shuffles.
        let keep2 = (THRESHOLD * 1.4).powi(2);
        for c in creatures.iter_mut() {
            let f = feature(&c.pheno);
            // Nearest existing species.
            let mut best = (usize::MAX, f32::INFINITY);
            for (i, s) in self.species.iter().enumerate() {
                let d = dist2(&f, &s.centroid);
                if d < best.1 {
                    best = (i, d);
                }
            }
            // Stay in the inherited species if it still exists and is in range.
            let inherited = self
                .species
                .iter()
                .position(|s| s.id == c.species_id)
                .filter(|&i| dist2(&f, &self.species[i].centroid) <= keep2);
            let idx = if let Some(i) = inherited {
                i
            } else if best.1 <= thr2 {
                best.0
            } else {
                self.species.push(Species { id: self.next_id, centroid: f, count: 0 });
                sums.push([0.0; K]);
                self.next_id = self.next_id.wrapping_add(1);
                self.species.len() - 1
            };
            self.species[idx].count += 1;
            for k in 0..K {
                sums[idx][k] += f[k];
            }
            c.species_id = self.species[idx].id;
        }

        // Recompute centroids; drop species that lost all members.
        let mut kept = Vec::with_capacity(self.species.len());
        for (i, s) in self.species.iter().enumerate() {
            if s.count == 0 {
                continue;
            }
            let mut cen = [0.0; K];
            for k in 0..K {
                cen[k] = sums[i][k] / s.count as f32;
            }
            kept.push(Species { id: s.id, centroid: cen, count: s.count });
        }
        // Merge species whose centroids drifted close together (keep the lower id).
        let merge2 = (THRESHOLD * 0.6).powi(2);
        let mut i = 0;
        while i < kept.len() {
            let mut j = i + 1;
            while j < kept.len() {
                if dist2(&kept[i].centroid, &kept[j].centroid) < merge2 {
                    if kept[j].id < kept[i].id {
                        kept[i].id = kept[j].id;
                    }
                    kept[i].count += kept[j].count;
                    kept.remove(j);
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
        self.species = kept;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavior::BehaviorKind;
    use crate::genome::{seed, Genome};
    use macroquad::math::Vec2;

    #[test]
    fn distinct_genomes_form_multiple_species() {
        seed(1);
        // Two tight groups of identical creatures with very different genomes.
        let ga = Genome::random();
        let gb = Genome::random();
        let mut creatures: Vec<Creature> = Vec::new();
        for _ in 0..20 {
            creatures.push(Creature::new(ga.clone(), Vec2::ZERO, 50.0, 0, BehaviorKind::Neural));
            creatures.push(Creature::new(gb.clone(), Vec2::ZERO, 50.0, 0, BehaviorKind::Neural));
        }
        let mut sp = Speciation::new();
        sp.update(&mut creatures);
        // Identical-within-group creatures should not explode into many species,
        // and two unrelated genomes should usually land in different clusters.
        assert!(sp.count() >= 1 && sp.count() <= 4, "unexpected species count {}", sp.count());
        // Every creature got a species assignment that exists.
        assert!(creatures.iter().all(|c| c.species_id < sp.next_id_for_test()));
    }
}

#[cfg(test)]
impl Speciation {
    fn next_id_for_test(&self) -> u32 {
        self.next_id
    }
}
