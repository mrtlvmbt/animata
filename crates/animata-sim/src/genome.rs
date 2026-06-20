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

// Gene roles (gene 9 is GENE_ADHESION; none free now at G=10).
const GENE_DIVIDE: usize = 0; // > THETA ⇒ the cell divides this step
const GENE_POLARITY: usize = 1; // negated in the daughter so sisters can differentiate
const GENE_EFFECTOR: usize = 2; // expressed ⇒ contractile/locomotor cell (also fins in water)
const GENE_STORAGE: usize = 3; // expressed ⇒ energy-storage cell
const GENE_SENSOR: usize = 4; // expressed ⇒ sensory cell
const GENE_PREDATOR: usize = 5; // expressed ⇒ predatory/meat-digesting cell (C2)
const GENE_FLIGHT: usize = 6; // expressed ⇒ wing/lift cell — access to the AIR stratum (C3)
const GENE_BURROW: usize = 7; // expressed ⇒ digging cell — access to the UNDERGROUND stratum (C3)
const GENE_PHOTO: usize = 8; // expressed ⇒ photosynthetic cell — makes energy from light (C3)
const GENE_ADHESION: usize = 9; // how strongly the cell sticks to its own type (differential adhesion)
/// Differential-adhesion sorting (PR-B). `GENE_ADHESION` is binned into `0..=ADH_Q` integer tiers at
/// GRN exit (the one float read); the whole sort is then i32 ⇒ deterministic WITHIN a profile (it
/// inherits the GRN's per-profile FMA reality, like the golden — no cross-profile claim). The sort
/// permutes which cell sits at which lattice slot to cluster same-type cells (tissues); it preserves
/// the cell MULTISET, so `develop()`'s type counts — and the golden — are unchanged.
const ADH_Q: i32 = 4;
const SORT_SWEEPS: usize = 6;
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
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Phenotype {
    pub n_cells: u32,
    pub effector: u32,
    pub storage: u32,
    pub sensor: u32,
    pub predator: u32,
    pub flight: u32,
    pub burrow: u32,
    pub photo: u32,
    pub structural: u32,
}

impl Phenotype {
    /// A coarse complexity tier from the developed body (the single→multi→complex axis).
    /// 0 = unicellular, 1 = multicellular (≤1 specialised type), 2 = complex (≥2 types).
    pub fn complexity(&self) -> u8 {
        if self.n_cells <= 1 {
            return 0;
        }
        let types = [self.effector, self.storage, self.sensor, self.predator, self.flight, self.burrow, self.photo]
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

    /// Fraction of the body that is photosynthetic cells (the autotroph investment; C3).
    pub fn photo_frac(&self) -> f32 {
        self.frac(self.photo)
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
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Genome {
    grn_w: Vec<f32>, // G×G, row-major
    grn_b: Vec<f32>, // G
    pub brain: Vec<f32>,
    pub thermal_pref: f32,
    /// Body coloration in `[0,1]` (dark .. light) — a heritable appearance trait. A predator
    /// detects prey with a probability rising with the CONTRAST between this and the local
    /// ground tone, so matching the background (crypsis / camouflage) lowers predation (C3).
    pub coloration: f32,
    /// Tolerance `[0,1]` to environmental toxicity (heavy metals / pollutants in the ground). A
    /// creature standing on ground more toxic than its tolerance suffers an extra death hazard, so
    /// toxic regions select for resistant lineages (a habitat filter on a new abiotic axis).
    pub toxin_resistance: f32,
}

impl Genome {
    /// Founder genome: empty GRN (develops to one cell) + random brain weights + random thermal
    /// preference + random coloration (deterministic from the threaded `rng`).
    pub fn founder(rng: &mut Rng) -> Self {
        Genome {
            grn_w: vec![0.0; G * G],
            grn_b: vec![0.0; G],
            brain: (0..BRAIN_WEIGHTS).map(|_| rng.signed()).collect(),
            thermal_pref: rng.unit(),
            coloration: rng.unit(),
            toxin_resistance: rng.unit(),
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
            coloration: (self.coloration + rng.signed() * grn_std).clamp(0.0, 1.0),
            toxin_resistance: (self.toxin_resistance + rng.signed() * grn_std).clamp(0.0, 1.0),
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

    /// The shared morphogenesis core (the SINGLE source of truth for body structure): grow the body
    /// from one seed cell by running the GRN for `DEV_STEPS`, dividing cells whose divide gene fires
    /// (daughter gets a polarity flip so sisters can differentiate), capped at `MAX_CELLS`. Returns
    /// each cell's final GRN state PLUS its integer lattice position. The GRN growth is byte-identical
    /// to the pre-morphogenesis loop — positions are assigned ALONGSIDE and feed nothing back, so the
    /// cell set (and therefore `develop()`'s tallies) is unchanged. Deterministic; depends only on the
    /// genome. **Empty GRN ⇒ exactly one structural cell at the origin (C0).**
    fn grow(&self) -> (Vec<[f32; G]>, Vec<(i16, i16)>) {
        let mut seed = [0.0f32; G];
        seed[0] = 1.0; // a maternal factor to bootstrap a non-empty GRN (ignored by W=0)
        let mut states: Vec<[f32; G]> = vec![seed];
        let mut pos: Vec<(i16, i16)> = vec![(0, 0)];
        for _ in 0..DEV_STEPS {
            let cur = states.len(); // fixed during this step (newborns go to a side buffer)
            let mut newborn: Vec<[f32; G]> = Vec::new();
            let mut newborn_pos: Vec<(i16, i16)> = Vec::new();
            for i in 0..cur {
                let ns = self.regulate(&states[i]);
                states[i] = ns;
                if ns[GENE_DIVIDE] > DIVIDE_THETA && cur + newborn.len() < MAX_CELLS {
                    let mut child = ns;
                    child[GENE_POLARITY] = -child[GENE_POLARITY];
                    // Place the daughter on a free lattice neighbour, preferred direction from the
                    // parent's polarity (render metadata only — never fed back into the GRN).
                    let p = place_cell(pos[i], ns[GENE_POLARITY], &pos, &newborn_pos);
                    newborn.push(child);
                    newborn_pos.push(p);
                }
            }
            if newborn.is_empty() {
                break; // settled — no more growth
            }
            states.extend(newborn);
            pos.extend(newborn_pos);
            if states.len() >= MAX_CELLS {
                break;
            }
        }
        // Differential adhesion: cluster same-type cells into tissues by permuting which cell sits at
        // which lattice slot (positions fixed). Preserves the cell multiset ⇒ type counts unchanged.
        adhesion_sort(&mut states, &pos);
        (states, pos)
    }

    /// Develop the body and tally cell-type COUNTS (the per-tick stat inputs). Reduces the shared
    /// [`grow`](Self::grow) core to the same `Phenotype` counts as before morphogenesis.
    pub fn develop(&self) -> Phenotype {
        let (states, _pos) = self.grow();
        let mut p = Phenotype { n_cells: states.len() as u32, ..Default::default() };
        for s in &states {
            match cell_type(s) {
                1 => p.effector += 1,
                2 => p.storage += 1,
                3 => p.sensor += 1,
                4 => p.predator += 1,
                5 => p.flight += 1,
                6 => p.burrow += 1,
                7 => p.photo += 1,
                _ => p.structural += 1, // 0 = structural (no function gene beats the baseline)
            }
        }
        p
    }

    /// The developed body as `(x, y, cell_type)` on the lattice — for RENDER ONLY (drawing the
    /// organism's shape at close zoom). Same shared [`grow`](Self::grow) core `develop()` uses, so the
    /// drawn body always matches the stats. `cell_type`: 0 = structural, 1..=7 = effector / storage /
    /// sensor / predator / flight / burrow / photo. Re-derived on demand; nothing is stored per-creature.
    pub fn body_layout(&self) -> Vec<(i16, i16, u8)> {
        let (states, pos) = self.grow();
        states.iter().zip(pos).map(|(s, p)| (p.0, p.1, cell_type(s))).collect()
    }
}

/// A cell's type from its final GRN state: argmax over the 7 function genes if any beats the
/// specialise baseline (first gene reaching the max wins the tie — deterministic), else structural.
/// 0 = structural, 1..=7 = effector / storage / sensor / predator / flight / burrow / photo. The
/// single classifier both `develop()` (counts) and `body_layout()` (render) share.
fn cell_type(s: &[f32; G]) -> u8 {
    const FUNCS: [usize; 7] =
        [GENE_EFFECTOR, GENE_STORAGE, GENE_SENSOR, GENE_PREDATOR, GENE_FLIGHT, GENE_BURROW, GENE_PHOTO];
    let best = FUNCS.iter().map(|&g| s[g]).fold(f32::MIN, f32::max);
    if best < SPECIALISE_THETA {
        return 0;
    }
    for (idx, &g) in FUNCS.iter().enumerate() {
        if s[g] == best {
            return (idx + 1) as u8;
        }
    }
    0
}

/// Place a dividing cell's daughter on a free 4-neighbour lattice site, preferring the direction set
/// by the parent's polarity (a fixed-threshold bin into N/E/S/W — render metadata, so the float read
/// here never feeds the GRN). If all four neighbours are taken, spiral outward in a fixed scan order
/// until a free site is found. Deterministic for ≤ `MAX_CELLS` cells. `taken`/`pending` are the
/// already-placed coords (this step's parents + this step's newborns).
fn place_cell(parent: (i16, i16), polarity: f32, taken: &[(i16, i16)], pending: &[(i16, i16)]) -> (i16, i16) {
    const DIRS: [(i16, i16); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)]; // N, E, S, W
    let free = |c: (i16, i16)| !taken.contains(&c) && !pending.contains(&c);
    // Preferred starting direction from polarity (one fixed-threshold bin into 0..=3).
    let start = (((polarity.clamp(-1.0, 1.0) + 1.0) * 0.5 * 3.999) as usize).min(3);
    for k in 0..4 {
        let d = DIRS[(start + k) % 4];
        let c = (parent.0 + d.0, parent.1 + d.1);
        if free(c) {
            return c;
        }
    }
    // All four neighbours taken: expand square rings around the parent, fixed scan order.
    for r in 2i16.. {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx.abs().max(dy.abs()) != r {
                    continue; // ring perimeter only
                }
                let c = (parent.0 + dx, parent.1 + dy);
                if free(c) {
                    return c;
                }
            }
        }
    }
    parent // unreachable for ≤ MAX_CELLS, but a total function
}

/// `GENE_ADHESION` binned to an integer tier `0..=ADH_Q` (the single float read of the sort).
fn adhesion_tier(s: &[f32; G]) -> i32 {
    (((s[GENE_ADHESION].clamp(-1.0, 1.0) + 1.0) * 0.5 * ADH_Q as f32) as i32).clamp(0, ADH_Q)
}

/// Two lattice sites are bonded iff 4-adjacent (Manhattan distance 1).
fn is_adjacent(a: (i16, i16), b: (i16, i16)) -> bool {
    (a.0 - b.0).abs() + (a.1 - b.1).abs() == 1
}

/// Differential-adhesion cell sorting (Steinberg): permute the cell CONTENTS across the fixed lattice
/// slots so same-type cells become 4-adjacent, maximising the integer "bond" energy
/// `E = Σ over same-type adjacent pairs of (adh_a + adh_b)`. Greedy, fixed index order, strict
/// improvement only (tie ⇒ no move) ⇒ deterministic within a profile. The cell multiset is preserved
/// (only the slot assignment changes), so `develop()`'s type counts are unchanged. O(n²·sweeps) with
/// n ≤ `MAX_CELLS=32` — negligible per birth.
fn adhesion_sort(states: &mut [[f32; G]], pos: &[(i16, i16)]) {
    let n = states.len();
    if n < 3 {
        return; // nothing to cluster
    }
    let neighbors: Vec<Vec<usize>> = (0..n)
        .map(|a| (0..n).filter(|&b| b != a && is_adjacent(pos[a], pos[b])).collect())
        .collect();
    // Bonded energy of the cell currently in slot `a` with its same-type neighbours.
    let local_e = |states: &[[f32; G]], a: usize| -> i32 {
        let (ta, aa) = (cell_type(&states[a]), adhesion_tier(&states[a]));
        neighbors[a]
            .iter()
            .filter(|&&b| cell_type(&states[b]) == ta)
            .map(|&b| aa + adhesion_tier(&states[b]))
            .sum()
    };
    for _ in 0..SORT_SWEEPS {
        for a in 0..n {
            for b in (a + 1)..n {
                let before = local_e(states, a) + local_e(states, b);
                states.swap(a, b);
                let after = local_e(states, a) + local_e(states, b);
                if after <= before {
                    states.swap(a, b); // strict-improvement only; revert ties/regressions
                }
            }
        }
    }
}

impl Genome {
    /// Fold the FULL heritable state into a determinism checksum (PR1 lock): every GRN weight,
    /// bias and brain weight (bit-reinterpreted, never float-add — F2) plus the scalar niche
    /// traits. Hashing the whole genome (not a subset) keeps the lock from leaking when a
    /// "silent" gene is perturbed (F7). (Used by the determinism-checksum; metrics in PR5.)
    #[allow(dead_code)]
    pub fn checksum(&self) -> u64 {
        let mut h = crate::rng::FNV_OFFSET;
        for &w in &self.grn_w {
            crate::rng::fnv_fold_u32(&mut h, w.to_bits());
        }
        for &b in &self.grn_b {
            crate::rng::fnv_fold_u32(&mut h, b.to_bits());
        }
        for &w in &self.brain {
            crate::rng::fnv_fold_u32(&mut h, w.to_bits());
        }
        crate::rng::fnv_fold_u32(&mut h, self.thermal_pref.to_bits());
        crate::rng::fnv_fold_u32(&mut h, self.coloration.to_bits());
        crate::rng::fnv_fold_u32(&mut h, self.toxin_resistance.to_bits());
        h
    }
}

#[cfg(test)]
#[path = "genome_tests.rs"]
mod tests;
