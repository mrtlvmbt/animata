//! Developmental / gene-regulatory encoding (phase C1).
//!
//! A creature's genome does NOT describe its body directly — it describes a small
//! **gene-regulatory network** (GRN) that *grows* the body from one seed cell. Development
//! runs the GRN over a few steps: each cell updates a morphogen state vector by the rule
//! `s' = tanh(W·s + b)`, and thresholds on that state trigger cell **division** (with a
//! polarity flip so daughters can diverge) and **differentiation** (a cell's type = which
//! function gene it expresses most). So body size and cell-type mix are EMERGENT from the
//! dynamics, not chosen from a menu — the literature's answer to "simple low-level traits →
//! complex unpredictable bodies" (CPPN/GRN/artificial-embryogeny; protoevo multicellularity).
//!
//! **C0 continuity by construction:** the founder's GRN is all-zero, so `tanh(W·s+b) = 0`
//! every step, the divide gene never fires, and development yields exactly ONE structural
//! cell — biomass 1, no stat boosts — i.e. the C0 organism, driven by the (still-present,
//! evolvable) brain weights. Mutation grows the GRN away from there.

use crate::rng::Rng;

/// Morphogen genes per cell (the GRN state width).
pub const G: usize = 10;
/// Development steps (bounded → cheap + deterministic).
pub const DEV_STEPS: usize = 10;
/// Hard cap on cells per body (bounds dev cost AND the per-tick brain/biomass cost).
pub const MAX_CELLS: usize = 32;

// Gene roles (the rest, 8..G, are free regulatory genes the GRN can use as it likes).
const GENE_DIVIDE: usize = 0; // > THETA ⇒ the cell divides this step
const GENE_POLARITY: usize = 1; // negated in the daughter so sisters can differentiate
const GENE_EFFECTOR: usize = 2; // expressed ⇒ contractile/locomotor cell (also fins in water)
const GENE_STORAGE: usize = 3; // expressed ⇒ energy-storage cell
const GENE_SENSOR: usize = 4; // expressed ⇒ sensory cell
const GENE_PREDATOR: usize = 5; // expressed ⇒ predatory/meat-digesting cell (C2)
const GENE_FLIGHT: usize = 6; // expressed ⇒ wing/lift cell — access to the AIR stratum (C3)
const GENE_BURROW: usize = 7; // expressed ⇒ digging cell — access to the UNDERGROUND stratum (C3)
/// Division fires when the divide gene exceeds this. Low enough that a few accumulated GRN
/// mutations can reach it from the empty founder (so multicellularity is evolutionarily
/// reachable, not stranded behind an unmutatable threshold).
const DIVIDE_THETA: f32 = 0.35;
/// A function gene must beat this baseline to specialise the cell (else it stays structural).
const SPECIALISE_THETA: f32 = 0.3;

/// Brain (controller) weights for the fixed 11→6→2 topology (C2 added prey/threat senses to
/// the C0/C1 plant-field + interoception inputs), carried in the genome and evolved. Must equal
/// `sim::N_INPUTS*N_HIDDEN + N_HIDDEN*N_OUTPUTS` (a test guards this).
pub const BRAIN_WEIGHTS: usize = 11 * 6 + 6 * 2;

/// The grown body: just the counts C1 needs (cell positions/adhesion come later). Cell count
/// is the integer biomass; the type tallies drive the emergent stats.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Phenotype {
    pub n_cells: u32,
    pub effector: u32,
    pub storage: u32,
    pub sensor: u32,
    pub predator: u32,
    pub flight: u32,
    pub burrow: u32,
    pub structural: u32,
}

impl Phenotype {
    /// A coarse complexity tier from the developed body (the single→multi→complex axis).
    /// 0 = unicellular, 1 = multicellular (≤1 specialised type), 2 = complex (≥2 types).
    pub fn complexity(&self) -> u8 {
        if self.n_cells <= 1 {
            return 0;
        }
        let types = [self.effector, self.storage, self.sensor, self.predator, self.flight, self.burrow]
            .iter()
            .filter(|&&c| c > 0)
            .count();
        if types >= 2 {
            2
        } else {
            1
        }
    }

    /// Carnivory in `[0,1]` — the fraction of the body that is predatory cells. Drives how
    /// well the creature digests meat vs plants (a body with no predator cells is a pure
    /// herbivore; an all-predator body is a pure carnivore).
    pub fn carnivory(&self) -> f32 {
        self.frac(self.predator)
    }

    /// Fraction of the body that is flight cells (gates access to the AIR stratum).
    pub fn flight_frac(&self) -> f32 {
        self.frac(self.flight)
    }

    /// Fraction of the body that is burrow cells (gates access to the UNDERGROUND stratum).
    pub fn burrow_frac(&self) -> f32 {
        self.frac(self.burrow)
    }

    /// Fraction of the body that is effector/fin cells (gates the WATER stratum in water biomes).
    pub fn fin_frac(&self) -> f32 {
        self.frac(self.effector)
    }

    fn frac(&self, count: u32) -> f32 {
        if self.n_cells == 0 {
            0.0
        } else {
            count as f32 / self.n_cells as f32
        }
    }
}

/// The heritable genome: the developmental GRN (weight matrix + bias), the controller weights,
/// and the climate-niche trait. A "founder" has a zero GRN (→ single cell == C0), random brain
/// weights, and a random thermal preference. `thermal_pref` in `[0,1]` is the temperature this
/// lineage is adapted to (0 cold .. 1 hot); living far from it costs extra metabolism (C3), so
/// different climate bands favour different prefs → habitats / allopatry.
#[derive(Clone)]
pub struct Genome {
    grn_w: Vec<f32>, // G×G, row-major
    grn_b: Vec<f32>, // G
    pub brain: Vec<f32>,
    pub thermal_pref: f32,
}

impl Genome {
    /// Founder genome: empty GRN (develops to one cell) + random brain weights + random thermal
    /// preference, all from `rng` (threaded so founders are deterministic from the world seed).
    pub fn founder(rng: &mut Rng) -> Self {
        Genome {
            grn_w: vec![0.0; G * G],
            grn_b: vec![0.0; G],
            brain: (0..BRAIN_WEIGHTS).map(|_| rng.signed()).collect(),
            thermal_pref: rng.unit(),
        }
    }

    /// A mutated child genome: every gene (GRN weights, GRN bias, brain weights, thermal pref)
    /// is nudged by `±std` noise from `rng`. GRN mutations grow/shrink/retype the body; brain
    /// mutations tune behaviour; the thermal pref drifts to track the climate it lives in.
    /// `grn_std` is kept smaller so body plans change by rarer, gentler steps than behaviour.
    pub fn mutate(&self, rng: &mut Rng, brain_std: f32, grn_std: f32) -> Self {
        let m = |v: &[f32], std: f32, rng: &mut Rng| -> Vec<f32> {
            v.iter().map(|&w| w + rng.signed() * std).collect()
        };
        Genome {
            grn_w: m(&self.grn_w, grn_std, rng),
            grn_b: m(&self.grn_b, grn_std, rng),
            brain: m(&self.brain, brain_std, rng),
            thermal_pref: (self.thermal_pref + rng.signed() * grn_std).clamp(0.0, 1.0),
        }
    }

    /// One GRN update of a cell state: `s' = tanh(W·s + b)`.
    fn regulate(&self, s: &[f32; G]) -> [f32; G] {
        let mut out = [0.0f32; G];
        for (i, o) in out.iter_mut().enumerate() {
            let mut sum = self.grn_b[i];
            for (j, &sj) in s.iter().enumerate() {
                sum += self.grn_w[i * G + j] * sj;
            }
            *o = sum.tanh();
        }
        out
    }

    /// Grow the body from one seed cell by running the GRN for `DEV_STEPS`, dividing cells
    /// whose divide gene fires (daughter gets a polarity flip so sisters can differentiate),
    /// capped at `MAX_CELLS`. Deterministic — depends only on the genome. Then tally cell
    /// types from the final states. **Empty GRN ⇒ exactly one structural cell (C0).**
    pub fn develop(&self) -> Phenotype {
        let mut seed = [0.0f32; G];
        seed[0] = 1.0; // a maternal factor to bootstrap a non-empty GRN (ignored by W=0)
        let mut states: Vec<[f32; G]> = vec![seed];
        for _ in 0..DEV_STEPS {
            let cur = states.len(); // fixed during this step (newborns go to a side buffer)
            let mut newborn: Vec<[f32; G]> = Vec::new();
            for s in states.iter_mut() {
                let ns = self.regulate(s);
                *s = ns;
                if ns[GENE_DIVIDE] > DIVIDE_THETA && cur + newborn.len() < MAX_CELLS {
                    let mut child = ns;
                    child[GENE_POLARITY] = -child[GENE_POLARITY];
                    newborn.push(child);
                }
            }
            if newborn.is_empty() {
                break; // settled — no more growth
            }
            states.extend(newborn);
            if states.len() >= MAX_CELLS {
                break;
            }
        }
        let mut p = Phenotype { n_cells: states.len() as u32, ..Default::default() };
        for s in &states {
            // The cell takes the identity of its most-expressed function gene (if any beats the
            // baseline; else it's structural). One arg-max over all the function genes.
            let funcs = [
                (GENE_EFFECTOR, &mut p.effector),
                (GENE_STORAGE, &mut p.storage),
                (GENE_SENSOR, &mut p.sensor),
                (GENE_PREDATOR, &mut p.predator),
                (GENE_FLIGHT, &mut p.flight),
                (GENE_BURROW, &mut p.burrow),
            ];
            let best = funcs.iter().map(|&(g, _)| s[g]).fold(f32::MIN, f32::max);
            if best < SPECIALISE_THETA {
                p.structural += 1;
            } else {
                // First gene reaching the max wins the tie (deterministic).
                for (g, count) in funcs {
                    if s[g] == best {
                        *count += 1;
                        break;
                    }
                }
            }
        }
        p
    }
}

#[cfg(test)]
mod tests {
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
            let typed = p1.effector + p1.storage + p1.sensor + p1.predator + p1.flight + p1.burrow;
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
}
