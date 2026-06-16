//! ACGT genome: random generation, mutation, and decoding into a phenotype.
//!
//! A genome is a fixed-length string of nucleotides (`0..=3` == A,C,G,T).
//! It is read in groups of [`config::NT_PER_GENE`] nucleotides; each group is a
//! base-4 number that decodes to `0..=255`, then mapped into a trait range or a
//! neural-network weight.

use crate::config::*;
use macroquad::math::Vec2;
use macroquad::rand::{gen_range, srand};

pub const NUCLEOTIDES: [char; 4] = ['A', 'C', 'G', 'T'];

#[derive(Clone)]
pub struct Genome {
    pub nt: Vec<u8>,
}

/// One decoded synapse. Ports are *tags* (indices into the port space), not
/// array offsets, so body-grown ports can be added later without disturbing
/// existing connections.
///
/// `src` indexes the source port space (`0..SRC_PORTS`): values below
/// `NN_INPUTS` are input ports, the rest are hidden units (`src - NN_INPUTS`).
/// `dst` indexes the destination space (`0..DST_PORTS`): values below
/// `NN_HIDDEN` are hidden units, the rest are output ports (`dst - NN_HIDDEN`).
#[derive(Clone, Copy)]
pub struct Synapse {
    pub src: u8,
    pub dst: u8,
    pub w: f32,
}

/// An appendage on a body segment. Drives medium locomotion and layer access in
/// later Phase-2 sub-steps (fins → swim, wings → fly, legs → walk, burrow → dig).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Appendage {
    None,
    Fin,
    Wing,
    Leg,
    Burrow,
}

impl Appendage {
    fn from_tag(tag: u8) -> Self {
        match tag % APPENDAGE_KINDS as u8 {
            0 => Appendage::None,
            1 => Appendage::Fin,
            2 => Appendage::Wing,
            3 => Appendage::Leg,
            _ => Appendage::Burrow,
        }
    }
}

/// One body segment in the chain (head → tail).
#[derive(Clone, Copy)]
pub struct Segment {
    pub length: f32,
    pub width: f32,
    pub appendage: Appendage,
    /// How freely this segment bends relative to the previous one (0..1).
    pub flexibility: f32,
}

/// Decoded, ready-to-use traits plus the brain's synapse list.
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
    /// Brain wiring: marker-decoded synapses (variable count), with src/dst
    /// already resolved against `n_hidden`.
    pub synapses: Vec<Synapse>,
    /// Hidden-layer width, evolved via neuron records (clamped to the brain-size
    /// bounds). Founders are `FOUNDER_HIDDEN`.
    pub n_hidden: usize,
    /// Body plan: marker-decoded segment chain (empty == a single implicit
    /// segment sized by the radius gene, i.e. the original circular body).
    pub segments: Vec<Segment>,
}

impl Genome {
    /// A founder genome: a random body-gene block followed by a *constructed*
    /// dense brain — one synapse record for every input->hidden, hidden->hidden
    /// and hidden->output connection, each with a random weight. Random nucleotides
    /// alone would rarely contain enough start codons to wire a working brain, so
    /// founders are built explicitly; mutation then sparsifies/rewires from there.
    pub fn random() -> Self {
        let mut nt = Vec::with_capacity(GENOME_LEN);
        // Body-gene block (the world later pokes specific body genes by index).
        for _ in 0..BODY_GENES * NT_PER_GENE {
            nt.push(gen_range(0u32, 4) as u8);
        }
        // Hidden layer: one neuron record per founder hidden unit (type gene in
        // the neuron band). These define the brain width the synapses below wire.
        for _ in 0..FOUNDER_HIDDEN {
            nt.extend_from_slice(&RECORD_START);
            nt.extend_from_slice(&gene_nt(NEURON_TYPE_MIN as u32));
        }
        let mut emit = |src: usize, dst: usize| {
            nt.extend_from_slice(&RECORD_START);
            nt.extend_from_slice(&gene_nt(0)); // type gene below SEGMENT_TYPE_MIN -> synapse
            nt.extend_from_slice(&gene_nt(src as u32));
            nt.extend_from_slice(&gene_nt(dst as u32));
            nt.extend_from_slice(&gene_nt(gen_range(0u32, 256)));
        };
        for i in 0..NN_INPUTS {
            for h in 0..NN_HIDDEN {
                emit(i, h); // input i -> hidden h
            }
        }
        for p in 0..NN_HIDDEN {
            for h in 0..NN_HIDDEN {
                emit(NN_INPUTS + p, h); // hidden p -> hidden h (recurrent)
            }
        }
        for p in 0..NN_HIDDEN {
            for o in 0..NN_OUTPUTS {
                emit(NN_INPUTS + p, NN_HIDDEN + o); // hidden p -> output o
            }
        }
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

    /// Read the body gene at `index` as a base-4 number in `0..=255`.
    fn gene_u8(&self, index: usize) -> u8 {
        gene_at(&self.nt, index * NT_PER_GENE)
    }

    /// Scan the genome's record stream once into raw synapse genes, the segment
    /// chain, and a hidden-neuron count. The scan is at nt granularity (any
    /// reading frame); after a record matches it advances past the whole record,
    /// so a record's interior can never spawn a nested record, and an indel only
    /// adds, drops or shifts whole records. Synapse src/dst genes are kept *raw*
    /// here because resolving them to port indices needs the hidden width, which
    /// is only known once all neuron records are counted (see `decode`).
    fn scan_records(&self) -> (Vec<(u8, u8, f32)>, Vec<Segment>, usize) {
        let nt = &self.nt;
        let mut syn = Vec::new();
        let mut seg = Vec::new();
        let mut neurons = 0usize;
        let mut i = 0usize;
        // The start codon + type gene are common; need at least that much to read.
        while i + 3 + NT_PER_GENE <= nt.len() {
            if !(nt[i] == RECORD_START[0]
                && nt[i + 1] == RECORD_START[1]
                && nt[i + 2] == RECORD_START[2])
            {
                i += 1;
                continue;
            }
            // Field reader, relative to the first field gene (after start+type).
            let f = |k: usize| gene_at(nt, i + 3 + NT_PER_GENE + k * NT_PER_GENE);
            let tg = gene_at(nt, i + 3);
            if tg >= SEGMENT_TYPE_MIN {
                if i + SEGMENT_RECORD_NT <= nt.len() && seg.len() < MAX_SEGMENTS {
                    seg.push(Segment {
                        length: lerp(SEG_LEN_RANGE, f(0) as f32 / 255.0),
                        width: lerp(SEG_WIDTH_RANGE, f(1) as f32 / 255.0),
                        appendage: Appendage::from_tag(f(2)),
                        flexibility: f(3) as f32 / 255.0,
                    });
                    i += SEGMENT_RECORD_NT;
                } else {
                    i += 1; // record runs off the end, or segment cap reached
                }
            } else if tg >= NEURON_TYPE_MIN {
                // Neuron record: its presence adds one hidden unit (no payload).
                neurons += 1;
                i += NEURON_RECORD_NT;
            } else if i + SYNAPSE_RECORD_NT <= nt.len() {
                let wv = f(2) as f32 / 255.0;
                syn.push((f(0), f(1), (wv * 2.0 - 1.0) * WEIGHT_SCALE));
                i += SYNAPSE_RECORD_NT;
            } else {
                i += 1;
            }
        }
        (syn, seg, neurons)
    }

    pub fn decode(&self) -> Phenotype {
        let g = |i| self.gene_u8(i) as f32 / 255.0;

        let radius_gene = lerp(RADIUS_RANGE, g(0));
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

        let (raw_syn, segments, n_neurons) = self.scan_records();
        // Hidden width = neuron-record count, clamped to the brain-size bounds.
        let n_hidden = n_neurons.clamp(MIN_HIDDEN, MAX_HIDDEN);
        // Resolve each synapse's raw src/dst genes against this brain's port space:
        // sources are the inputs then the hidden units; destinations the hidden
        // units then the outputs.
        let src_ports = NN_INPUTS + n_hidden;
        let dst_ports = n_hidden + NN_OUTPUTS;
        let synapses: Vec<Synapse> = raw_syn
            .into_iter()
            .map(|(sg, dg, w)| Synapse {
                src: (sg as usize % src_ports) as u8,
                dst: (dg as usize % dst_ports) as u8,
                w,
            })
            .collect();
        let radius = body_radius(&segments, radius_gene);

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
            synapses,
            n_hidden,
            segments,
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

    /// World-space layout of the body segments for rendering: `(center, radius,
    /// appendage)` per segment, with the head at `pos` and the chain trailing
    /// opposite `heading`, gently curved by each joint's flexibility. Empty when
    /// the body is a single implicit segment (drawn as the original shape).
    pub fn segment_layout(&self, pos: Vec2, heading: f32) -> Vec<(Vec2, f32, Appendage)> {
        let mut out = Vec::with_capacity(self.segments.len());
        let mut cur = pos;
        let mut dir = heading + std::f32::consts::PI; // tail points behind the head
        for s in &self.segments {
            let step = Vec2::new(dir.cos(), dir.sin());
            cur += step * (s.length * 0.5);
            out.push((cur, s.width * 0.5, s.appendage));
            cur += step * (s.length * 0.5);
            dir += (s.flexibility - 0.5) * 0.6; // flex curves the chain
        }
        out
    }

    /// The vertical stratum this body lives in, from its appendages: the air if
    /// it has wings, else underground if it has a burrow appendage, else the
    /// surface. (Wings win ties — a flier doesn't also dig.)
    pub fn primary_layer(&self) -> u8 {
        let mut wings = false;
        let mut burrow = false;
        for s in &self.segments {
            match s.appendage {
                Appendage::Wing => wings = true,
                Appendage::Burrow => burrow = true,
                _ => {}
            }
        }
        if wings {
            LAYER_AIR
        } else if burrow {
            LAYER_UNDERGROUND
        } else {
            LAYER_SURFACE
        }
    }

    pub fn recurrent_gain(&self) -> f32 {
        // RMS magnitude of the recurrent (hidden->hidden) synapses, normalized.
        let mut ss = 0.0f32;
        let mut n = 0usize;
        for s in &self.synapses {
            if s.src as usize >= NN_INPUTS && (s.dst as usize) < self.n_hidden {
                ss += s.w * s.w;
                n += 1;
            }
        }
        if n == 0 {
            0.0
        } else {
            ((ss / n as f32).sqrt() / WEIGHT_SCALE).min(1.0)
        }
    }
}

/// Read `NT_PER_GENE` nucleotides starting at nt offset `pos` as a base-4 number
/// in `0..=255`. Reads past the end yield 0, so decoding tolerates short genomes.
fn gene_at(nt: &[u8], pos: usize) -> u8 {
    let mut v: u32 = 0;
    for k in 0..NT_PER_GENE {
        v = v * 4 + nt.get(pos + k).copied().unwrap_or(0) as u32;
    }
    v.min(255) as u8
}

/// Encode `v` as `NT_PER_GENE` base-4 nucleotides (big-endian), the inverse of
/// [`gene_at`] for values in `0..=255`. Used to construct founder genomes.
fn gene_nt(v: u32) -> [u8; NT_PER_GENE] {
    let mut out = [0u8; NT_PER_GENE];
    let mut x = v;
    for k in (0..NT_PER_GENE).rev() {
        out[k] = (x % 4) as u8;
        x /= 4;
    }
    out
}

/// Bounding radius used for collision / sensing / metabolism. With no segments
/// it's just the radius gene (the original circle); with a segment chain it's
/// derived from the body's extent, clamped so long worms stay tractable.
fn body_radius(segments: &[Segment], radius_gene: f32) -> f32 {
    if segments.is_empty() {
        return radius_gene;
    }
    // Width drives the bounding radius (used for collision / sensing / eating
    // reach), not chain length — otherwise a long body would win free eating
    // reach and segments would run away to the cap regardless of locomotion.
    let max_w = segments.iter().map(|s| s.width).fold(0.0, f32::max);
    max_w.clamp(RADIUS_RANGE.0, RADIUS_RANGE.1)
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
        // A founder constructs the dense connection set (a stray start codon in
        // the random body block can decode to a few extra synapses — harmless).
        assert!(p.synapses.len() >= FOUNDER_SYNAPSES);
        // Deterministic for same genome.
        assert_eq!(p.radius, p2.radius);
        assert_eq!(p.synapses.len(), p2.synapses.len());
        // Traits within configured ranges.
        assert!(p.radius >= RADIUS_RANGE.0 && p.radius <= RADIUS_RANGE.1);
        assert!(p.max_speed >= SPEED_RANGE.0 && p.max_speed <= SPEED_RANGE.1);
        assert!(p.sense_range >= SENSE_RANGE.0 && p.sense_range <= SENSE_RANGE.1);
        assert!(p.metabolism >= METAB_RANGE.0 && p.metabolism <= METAB_RANGE.1);
        assert!(p.prime >= LONGEVITY_RANGE.0 && p.prime <= LONGEVITY_RANGE.1);
        assert!((0.0..=1.0).contains(&p.carnivory));
        assert_eq!(p.n_hidden, FOUNDER_HIDDEN); // founders emit FOUNDER_HIDDEN neurons
        for s in &p.synapses {
            assert!(s.w >= -WEIGHT_SCALE && s.w <= WEIGHT_SCALE);
            assert!((s.src as usize) < NN_INPUTS + p.n_hidden);
            assert!((s.dst as usize) < p.n_hidden + NN_OUTPUTS);
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
    fn segment_record_decodes_and_sizes_body() {
        // A body-gene block of zeros (no stray start codon) plus one segment
        // record decodes to exactly one segment, and the body radius derives
        // from it rather than the (zero) radius gene.
        let mut nt = vec![0u8; BODY_GENES * NT_PER_GENE];
        nt.extend_from_slice(&RECORD_START);
        nt.extend_from_slice(&gene_nt(255)); // type gene >= SEGMENT_TYPE_MIN -> segment
        nt.extend_from_slice(&gene_nt(255)); // length
        nt.extend_from_slice(&gene_nt(128)); // width
        nt.extend_from_slice(&gene_nt(1)); // appendage tag -> Fin
        nt.extend_from_slice(&gene_nt(200)); // flexibility
        let g = Genome { nt };
        let p = g.decode();
        assert_eq!(p.segments.len(), 1);
        assert_eq!(p.segments[0].appendage, Appendage::Fin);
        assert!(p.segments[0].length >= SEG_LEN_RANGE.0 && p.segments[0].length <= SEG_LEN_RANGE.1);
        assert!(p.radius >= RADIUS_RANGE.0);
    }

    #[test]
    fn decode_handles_short_genome() {
        // A genome far shorter than the canonical length still decodes fully:
        // body traits read (missing nt as 0); too short to hold any synapse
        // record, so the brain is simply empty rather than a decode failure.
        let g = Genome { nt: vec![1, 2, 3, 0, 2] };
        let p = g.decode();
        assert!(p.synapses.is_empty());
        assert!(p.radius >= RADIUS_RANGE.0 && p.radius <= RADIUS_RANGE.1);
        assert!(p.max_speed >= SPEED_RANGE.0 && p.max_speed <= SPEED_RANGE.1);
    }
}
