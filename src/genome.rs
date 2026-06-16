//! ACGT genome: random generation, mutation, and decoding into a phenotype.
//!
//! A genome is a fixed-length string of nucleotides (`0..=3` == A,C,G,T).
//! It is read in groups of [`config::NT_PER_GENE`] nucleotides; each group is a
//! base-4 number that decodes to `0..=255`, then mapped into a trait range or a
//! neural-network weight.

use crate::config::*;
use macroquad::rand::{gen_range, srand};

pub const NUCLEOTIDES: [char; 4] = ['A', 'C', 'G', 'T'];

#[derive(Clone)]
pub struct Genome {
    pub nt: Vec<u8>,
}

/// Decoded, ready-to-use traits plus raw neural-network weights.
#[derive(Clone)]
pub struct Phenotype {
    pub radius: f32,
    pub max_speed: f32,
    pub sense_range: f32,
    /// Metabolism multiplier applied to base upkeep.
    pub metabolism: f32,
    pub color: (f32, f32, f32),
    /// Age (steps) of "prime" before senescence sets in.
    pub prime: f32,
    /// Diet on a 0..1 herbivore→carnivore scale.
    pub carnivory: f32,
    /// Sexual-selection display trait (0..1) and the mate-preference for it.
    pub ornament: f32,
    pub preference: f32,
    /// Disease-resistance allele (matching-allele target, 0..1).
    pub resistance: f32,
    /// Diet niche: the food "flavor" (0..1) this creature digests best.
    pub diet_niche: f32,
    /// Memory-leak γ (LEAK_RANGE) for the leaky-integrator hidden state.
    pub leak: f32,
    pub weights: Vec<f32>,
}

impl Genome {
    pub fn random() -> Self {
        let nt = (0..GENOME_LEN).map(|_| gen_range(0u32, 4) as u8).collect();
        Genome { nt }
    }

    /// Return a child copy with per-nucleotide substitutions (at `mut_rate`) and
    /// indels. Insertions/deletions change the genome length and shift downstream
    /// genes (frameshift), so the same nucleotides can decode to very different
    /// traits.
    pub fn mutated(&self, mut_rate: f64) -> Self {
        let mut nt = Vec::with_capacity(self.nt.len() + 4);
        for &base in &self.nt {
            let r = gen_range(0.0f64, 1.0);
            if r < INDEL_RATE {
                // Deletion: drop this nucleotide.
                continue;
            } else if r < 2.0 * INDEL_RATE {
                // Insertion: a random nucleotide before keeping this one.
                nt.push(gen_range(0u32, 4) as u8);
                nt.push(base);
            } else if gen_range(0.0f64, 1.0) < mut_rate {
                // Substitution with a *different* nucleotide.
                let shift = gen_range(1u32, 4) as u8;
                nt.push((base + shift) % 4);
            } else {
                nt.push(base);
            }
        }
        Genome { nt: clamp_len(nt) }
    }

    /// Single-point crossover: head of `a` joined to the tail of `b`. Each
    /// parent is cut independently, so with variable-length genomes the child
    /// length varies too.
    pub fn crossover(a: &Genome, b: &Genome) -> Genome {
        let ka = cut_point(a.nt.len());
        let kb = cut_point(b.nt.len());
        let mut nt = Vec::with_capacity(ka + b.nt.len().saturating_sub(kb));
        nt.extend_from_slice(&a.nt[..ka]);
        nt.extend_from_slice(&b.nt[kb..]);
        Genome { nt: clamp_len(nt) }
    }

    /// Read the gene at `index` as a base-4 number in `0..=255`. Nucleotides
    /// past the end of a (possibly shortened) genome read as 0, so decoding
    /// always yields a full phenotype regardless of length.
    fn gene_u8(&self, index: usize) -> u8 {
        let start = index * NT_PER_GENE;
        let mut v: u32 = 0;
        for i in 0..NT_PER_GENE {
            v = v * 4 + self.nt.get(start + i).copied().unwrap_or(0) as u32;
        }
        // NT_PER_GENE == 4 -> 0..=255; clamp defensively for other configs.
        v.min(255) as u8
    }

    pub fn decode(&self) -> Phenotype {
        let g = |i| self.gene_u8(i) as f32 / 255.0;

        let radius = lerp(RADIUS_RANGE, g(0));
        let max_speed = lerp(SPEED_RANGE, g(1));
        let sense_range = lerp(SENSE_RANGE, g(2));
        let metabolism = lerp(METAB_RANGE, g(3));
        let color = (g(4), g(5), g(6));
        let prime = lerp(LONGEVITY_RANGE, g(7));
        let carnivory = g(8); // already 0..=1
        let ornament = g(9);
        let preference = g(10);
        let resistance = g(11);
        let diet_niche = g(12);
        let leak = lerp(LEAK_RANGE, g(13));

        let weights = (0..NN_WEIGHTS)
            .map(|w| {
                let v = g(BODY_GENES + w); // 0..1
                (v * 2.0 - 1.0) * WEIGHT_SCALE // -SCALE..SCALE
            })
            .collect();

        Phenotype {
            radius,
            max_speed,
            sense_range,
            metabolism,
            color,
            prime,
            carnivory,
            ornament,
            preference,
            resistance,
            diet_niche,
            leak,
            weights,
        }
    }

    /// Human-readable ACGT string (for debugging / inspection / saving).
    pub fn to_string(&self) -> String {
        self.nt.iter().map(|&b| NUCLEOTIDES[b as usize]).collect()
    }

    /// Parse an ACGT string back into a genome (any non-ACGT char is skipped).
    pub fn from_acgt(s: &str) -> Genome {
        let nt = s
            .bytes()
            .filter_map(|b| match b {
                b'A' => Some(0),
                b'C' => Some(1),
                b'G' => Some(2),
                b'T' => Some(3),
                _ => None,
            })
            .collect();
        Genome { nt }
    }
}

impl Phenotype {
    /// Recurrent-memory reliance: RMS magnitude of the brain's hidden->hidden
    /// (recurrent) weights, normalized to 0..1 by `WEIGHT_SCALE`. 0 == a purely
    /// feed-forward brain (no memory); a random genome sits near ~0.58. Tracking
    /// it shows whether selection favors carrying state between steps.
    /// How well this creature digests a pellet of the given `flavor`, 0..1.
    /// Gaussian falloff from its `diet_niche` with fixed width — the trade-off
    /// that makes specialists thrive locally and starve in foreign biomes.
    pub fn diet_efficiency(&self, flavor: f32) -> f32 {
        let d = flavor - self.diet_niche;
        (-(d * d) / (2.0 * DIET_WIDTH * DIET_WIDTH)).exp()
    }

    pub fn recurrent_gain(&self) -> f32 {
        let ih = NN_INPUTS * NN_HIDDEN;
        let hh = NN_HIDDEN * NN_HIDDEN;
        let slice = &self.weights[ih..ih + hh];
        let ss: f32 = slice.iter().map(|w| w * w).sum();
        ((ss / hh as f32).sqrt() / WEIGHT_SCALE).min(1.0)
    }
}

/// Map `t` in `0..=1` into the inclusive range `(lo, hi)`.
fn lerp(range: (f32, f32), t: f32) -> f32 {
    range.0 + (range.1 - range.0) * t
}

/// A crossover cut point in `1..len` (so both parents contribute), or `len`
/// for degenerate tiny genomes.
fn cut_point(len: usize) -> usize {
    if len > 1 {
        gen_range(1u32, len as u32) as usize
    } else {
        len
    }
}

/// Keep a genome length inside `[GENOME_MIN_LEN, GENOME_MAX_LEN]`: pad short
/// genomes with random nucleotides, truncate long ones.
fn clamp_len(mut nt: Vec<u8>) -> Vec<u8> {
    while nt.len() < GENOME_MIN_LEN {
        nt.push(gen_range(0u32, 4) as u8);
    }
    nt.truncate(GENOME_MAX_LEN);
    nt
}

/// Seed the global RNG (call once at startup / reset).
pub fn seed(s: u64) {
    srand(s);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genome_has_fixed_length() {
        seed(1);
        let g = Genome::random();
        assert_eq!(g.nt.len(), GENOME_LEN);
        assert!(g.nt.iter().all(|&b| b < 4));
    }

    #[test]
    fn decode_is_deterministic_and_in_range() {
        seed(42);
        let g = Genome::random();
        let p = g.decode();
        let p2 = g.decode();
        assert_eq!(p.weights.len(), NN_WEIGHTS);
        // Deterministic for same genome.
        assert_eq!(p.radius, p2.radius);
        assert_eq!(p.weights, p2.weights);
        // Traits within configured ranges.
        assert!(p.radius >= RADIUS_RANGE.0 && p.radius <= RADIUS_RANGE.1);
        assert!(p.max_speed >= SPEED_RANGE.0 && p.max_speed <= SPEED_RANGE.1);
        assert!(p.sense_range >= SENSE_RANGE.0 && p.sense_range <= SENSE_RANGE.1);
        assert!(p.metabolism >= METAB_RANGE.0 && p.metabolism <= METAB_RANGE.1);
        assert!(p.prime >= LONGEVITY_RANGE.0 && p.prime <= LONGEVITY_RANGE.1);
        assert!((0.0..=1.0).contains(&p.carnivory));
        for &w in &p.weights {
            assert!(w >= -WEIGHT_SCALE && w <= WEIGHT_SCALE);
        }
    }

    #[test]
    fn crossover_takes_head_of_a_and_tail_of_b() {
        seed(3);
        let a = Genome::random();
        let b = Genome::random();
        let child = Genome::crossover(&a, &b);
        // Length stays inside the clamp band; both parents contribute.
        assert!(child.nt.len() >= GENOME_MIN_LEN && child.nt.len() <= GENOME_MAX_LEN);
        let head = child.nt.iter().zip(&a.nt).take_while(|(c, x)| c == x).count();
        assert!(head > 0, "child should share a prefix with parent a");
        assert!(child.nt.iter().all(|&n| n < 4));
    }

    #[test]
    fn mutation_stays_valid_and_within_length_bounds() {
        seed(7);
        let parent = Genome::random();
        let mut any_diff = false;
        for _ in 0..40 {
            let child = parent.mutated(MUTATION_RATE);
            assert!(child.nt.iter().all(|&n| n < 4));
            assert!(child.nt.len() >= GENOME_MIN_LEN && child.nt.len() <= GENOME_MAX_LEN);
            if child.nt != parent.nt {
                any_diff = true;
            }
        }
        assert!(any_diff, "mutation never changed the genome");
    }

    #[test]
    fn indels_sometimes_change_length() {
        seed(11);
        let parent = Genome::random();
        let changed = (0..200).any(|_| parent.mutated(MUTATION_RATE).nt.len() != GENOME_LEN);
        assert!(changed, "indels never altered genome length");
    }

    #[test]
    fn decode_handles_short_genome() {
        // A genome far shorter than the canonical length still decodes fully:
        // missing nucleotides read as 0, weights count is exact.
        let g = Genome { nt: vec![1, 2, 3, 0, 2] };
        let p = g.decode();
        assert_eq!(p.weights.len(), NN_WEIGHTS);
        assert!(p.radius >= RADIUS_RANGE.0 && p.radius <= RADIUS_RANGE.1);
        assert!(p.max_speed >= SPEED_RANGE.0 && p.max_speed <= SPEED_RANGE.1);
    }
}
