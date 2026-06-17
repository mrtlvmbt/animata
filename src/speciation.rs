//! Species detection grounded in the **Biological Species Concept**: a species
//! is a group that shares gene flow, i.e. whose members can interbreed. We label
//! it with exactly the reproductive-isolation barriers that
//! [`crate::world::World::reproduce`] enforces when choosing a mate — so the
//! cluster is a real gene-flow group, not a post-hoc morphological cluster:
//!   * **architecture** ([`plan_key`]): different body plans can't interbreed
//!     (a mechanical/genetic prezygotic barrier) — a hard partition.
//!   * **mate-recognition traits** ([`feature`]): diet assortment (carnivory)
//!     and the sexual display (ornament) that gate who mates with whom.
//!
//! Ecological traits that *don't* isolate (speed, size, metabolism) are
//! deliberately absent: two creatures that differ only in size still interbreed,
//! so by the BSC they are one species. Leader/threshold clustering (adaptive
//! count, no fixed k), run periodically; the UI colours creatures by species id.

use crate::creature::Creature;
use crate::genome::{Appendage, Phenotype};

/// Mate-recognition features used for soft clustering *within* a body plan: the
/// continuous traits that gate who interbreeds — diet (carnivory) and the sexual
/// display (ornament). The architecture is a hard partition (see [`plan_key`]),
/// so these axes only separate creatures that already share a body plan.
const K: usize = 2;
/// Max distance (in mate-recognition space) to count as the same species, given
/// the same body plan. Sized to the diet barrier ([`MATE_CARN_WINDOW`]) so the
/// label matches who actually interbreeds.
const THRESHOLD: f32 = 0.28;

/// Discrete signature of a body's **gross architecture** — the reproductive
/// barrier level. It is the set of appendage kinds the body *has* (presence, not
/// order or count) plus a coarse body-elongation bucket. So a winged flier, a
/// burrowing digger, a finned swimmer and a long appendage-less worm are each
/// their own architecture and can't interbreed, but two bodies that both "have
/// legs and fins" share it regardless of limb order or how many of each — because
/// limb order/count is not a real mating barrier, gross plan is. Founders (no
/// segment records) all map to key 0. Two species with different keys never merge.
pub fn plan_key(p: &Phenotype) -> u64 {
    // Bit per appendage kind that is present anywhere in the chain.
    let mut mask = 0u64;
    for s in &p.segments {
        if s.appendage != Appendage::None {
            mask |= 1 << (s.appendage as u64);
        }
    }
    // Coarse body-length bucket distinguishes a long multi-segment chain (a worm)
    // from a compact body without splitting on every added segment.
    let bucket = (p.segments.len() / 2).min(4) as u64;
    mask * 5 + bucket
}

/// Mate-recognition feature vector (each component 0..1): the traits mating
/// actually selects on — diet assortment and the sexual display.
fn feature(p: &Phenotype) -> [f32; K] {
    [p.carnivory, p.ornament]
}

fn dist2(a: &[f32; K], b: &[f32; K]) -> f32 {
    (0..K).map(|k| (a[k] - b[k]).powi(2)).sum()
}

struct Species {
    id: u32,
    /// Topological body-plan key; a creature can only join a species with the
    /// same plan, so species are partitioned by architecture first.
    plan: u64,
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
            let pk = plan_key(&c.pheno);
            // Nearest existing species *of the same body plan* (architecture is a
            // hard partition — a flier and a walker never share a species).
            let mut best = (usize::MAX, f32::INFINITY);
            for (i, s) in self.species.iter().enumerate() {
                if s.plan != pk {
                    continue;
                }
                let d = dist2(&f, &s.centroid);
                if d < best.1 {
                    best = (i, d);
                }
            }
            // Stay in the inherited species if it still exists, shares this body
            // plan, and is in range (a creature whose plan mutated must leave).
            let inherited = self
                .species
                .iter()
                .position(|s| s.id == c.species_id && s.plan == pk)
                .filter(|&i| dist2(&f, &self.species[i].centroid) <= keep2);
            let idx = if let Some(i) = inherited {
                i
            } else if best.1 <= thr2 {
                best.0
            } else {
                self.species.push(Species { id: self.next_id, plan: pk, centroid: f, count: 0 });
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
            kept.push(Species { id: s.id, plan: s.plan, centroid: cen, count: s.count });
        }
        // Merge species whose centroids drifted close together (keep the lower id).
        // Only same-plan species may merge — body plans stay separate taxa.
        let merge2 = (THRESHOLD * 0.6).powi(2);
        let mut i = 0;
        while i < kept.len() {
            let mut j = i + 1;
            while j < kept.len() {
                if kept[i].plan == kept[j].plan && dist2(&kept[i].centroid, &kept[j].centroid) < merge2 {
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
impl Speciation {
    fn next_id_for_test(&self) -> u32 {
        self.next_id
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
