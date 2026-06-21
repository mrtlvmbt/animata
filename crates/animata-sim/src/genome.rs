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

use crate::config::ORGAN_BONUS;
use crate::rng::Rng;

/// Morphogen genes per cell (the GRN state width).
pub const G: usize = 10;
/// Development steps (bounded → cheap + deterministic).
pub const DEV_STEPS: usize = 10;
/// Hard cap on cells per body (bounds dev cost AND the per-tick brain/biomass cost).
pub const MAX_CELLS: usize = 32;

/// Morphogen SIGNALLING channels (Phase 2 / PR-D) — cell-cell diffusion fields the GRN READS, NOT
/// regulated genes. D1 = 1 (the axis morphogen); D3 adds a symmetry channel. Each channel evolves by a
/// position-anchored SOURCE + lattice DIFFUSION + DECAY (the PR-D0 spike proved decay is required —
/// source+diffusion alone homogenise to a flat field; decay gives the screened-Poisson `exp(−r/λ)`
/// gradient), and feeds INTO `regulate` as an extra input so `W` can couple a cell's TYPE to its
/// POSITION → emergent body axes. **C0 continuity by meaning:** founder `W=0` never reads the
/// morphogen, so it changes no differentiation and a founder still develops to one cell — even though
/// the rates are armed (non-zero) at birth.
pub const N_MORPH: usize = 1;
/// Founder morphogen kinetics — the PR-D0 spike's proven gradient (`d≈0.5`, `k≈0.3`). Armed at birth so
/// only the READ weights (`morph_w`) must evolve to couple type↔position — the gradient is already
/// there to read (no triple-trait fitness valley of diffusion+decay+read together). Founder `morph_w=0`
/// ⇒ the armed gradient is NOT read ⇒ a founder develops byte-identically to the pre-morphogen body
/// (C0 continuity). The read weights evolve (PR-D2) on a SEPARATE RNG stream from the existing genes
/// (see `mutate`), so the existing genes' mutation stream is untouched and the morphogen perturbs the
/// ecosystem only as `morph_w` slowly drifts off 0 under selection.
const FOUNDER_DIFF: f32 = 0.5;
const FOUNDER_DECAY: f32 = 0.3;

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

/// The grown body: the cell-type counts (cell count = integer biomass) PLUS, per function type, the
/// size of its largest CONNECTED cluster (`organ`) — coherent tissue, from the differential-adhesion
/// layout. The type tallies + organ coherence drive the emergent stats (see [`organ_power`]).
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
    /// Largest connected same-type cluster per function type, index 0..=6 = effector / storage /
    /// sensor / predator / flight / burrow / photo. `0` or `1` ⇒ no coherent organ (no bonus).
    pub organ: [u8; 7],
    /// AXIS ORDER (Phase 2 / PR-D1), `0..=255`: how strongly cell TYPE varies with POSITION along the
    /// body's radial axis (distance from the morphogen source) — a scale-invariant η² (between-type /
    /// total variance of radial position), computed on the PRE-`adhesion_sort` layout so the gradient's
    /// type↔position map is read before the cosmetic sort scrambles it. `0` for a founder / no axial
    /// patterning; high when types segregate along the axis (an emergent body plan).
    pub axis_order: u8,
}

impl Phenotype {
    /// The effective power of a function type: its cell count plus a coherence bonus for clustering
    /// those cells into one organ — `count + ORGAN_BONUS·max(0, largest_cluster − 1)`. Monotone in
    /// both count and coherence; at ≤1 cell the bonus is 0 (founder/small-body stats unchanged).
    /// `type_idx` 0..=6 = effector / storage / sensor / predator / flight / burrow / photo.
    pub fn organ_power(&self, type_idx: usize) -> f32 {
        let count = [
            self.effector, self.storage, self.sensor, self.predator, self.flight, self.burrow, self.photo,
        ][type_idx];
        count as f32 + ORGAN_BONUS * self.organ[type_idx].saturating_sub(1) as f32
    }
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

/// The mutually-exclusive **food (trophic) niches** a body can occupy — the energy-income strategies
/// the sim actually models (photosynthesis / predation / grazing; see `Sim::step`). This is the ONE
/// place a creature is classified by diet: the population panel iterates [`TrophicNiche::ALL`] and the
/// inspector classifies through [`TrophicNiche::classify`], so adding a variant here makes a new bar
/// appear automatically. `#[non_exhaustive]` forces the UI's colour map to carry a neutral fallback,
/// so a new niche renders (with a placeholder colour) before anyone wires a colour for it.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrophicNiche {
    Autotroph,
    Carnivore,
    Herbivore,
}

impl TrophicNiche {
    /// Every niche, in display order. Iterate this to render one bar per niche.
    pub const ALL: &'static [TrophicNiche] = &[
        TrophicNiche::Autotroph,
        TrophicNiche::Carnivore,
        TrophicNiche::Herbivore,
    ];

    /// Classify a body by its cell mix. Precedence (matches the energy model): a photosynthetic body
    /// feeds itself, so it's an autotroph even with predator cells; otherwise a sufficiently predatory
    /// body is a carnivore; everything else grazes. Mutually exclusive ⇒ population fractions sum to 1.
    pub fn classify(pheno: &Phenotype) -> TrophicNiche {
        if pheno.photo_frac() > crate::config::PHOTO_THETA {
            TrophicNiche::Autotroph
        } else if pheno.carnivory() > crate::config::CARNIVORE_THRESHOLD {
            TrophicNiche::Carnivore
        } else {
            TrophicNiche::Herbivore
        }
    }

    /// Lower-case bar label (matches the existing `multicellular` complexity label style).
    pub fn label(self) -> &'static str {
        match self {
            TrophicNiche::Autotroph => "autotrophy",
            TrophicNiche::Carnivore => "carnivory",
            TrophicNiche::Herbivore => "herbivory",
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
    grn_w: Vec<f32>, // G×G, row-major (unchanged — the gene→gene regulatory weights)
    grn_b: Vec<f32>, // G
    /// Morphogen READ weights, `G×N_MORPH` row-major (PR-D): how strongly each regulated gene responds
    /// to each morphogen channel. Founder `0` ⇒ the morphogen is not read ⇒ C0-identical development.
    morph_w: Vec<f32>,
    /// Morphogen kinetics, `N_MORPH` each, `[0,1]` (PR-D): per-channel diffusion + decay rates that
    /// drive the position-anchored signalling field the GRN reads (not regulated genes themselves).
    diff_rate: Vec<f32>,
    decay_rate: Vec<f32>,
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
    /// Tolerance `[0,1]` to dissolved OXYGEN (gas cycle Phase 1). Founder `0.0` ⇒ SENSITIVE: O2 is a
    /// poison to the unadapted (reactive-oxygen damage), recapitulating the Great Oxygenation Event.
    /// Excess local O2 above this tolerance is a per-tick death hazard, so O2-rich zones (which dense
    /// autotrophs create as a photosynthesis byproduct) select for tolerant lineages AND brake the
    /// autotroph density that produced the O2. Evolves UP on its own mutation stream.
    pub oxygen_tolerance: f32,
}

impl Genome {
    /// Founder genome: empty GRN (develops to one cell) + random brain weights + random thermal
    /// preference + random coloration (deterministic from the threaded `rng`).
    pub fn founder(rng: &mut Rng) -> Self {
        Genome {
            grn_w: vec![0.0; G * G],
            grn_b: vec![0.0; G],
            morph_w: vec![0.0; G * N_MORPH],
            diff_rate: vec![FOUNDER_DIFF; N_MORPH],
            decay_rate: vec![FOUNDER_DECAY; N_MORPH],
            brain: (0..BRAIN_WEIGHTS).map(|_| rng.signed()).collect(),
            thermal_pref: rng.unit(),
            coloration: rng.unit(),
            toxin_resistance: rng.unit(),
            // Founder SENSITIVE to O2 (tolerance 0) — the anoxic ancestor. A constant (no `rng` draw),
            // so founders are byte-identical to the pre-feature sim; tolerance evolves up in `mutate`.
            oxygen_tolerance: 0.0,
        }
    }

    /// A mutated child genome: every gene (GRN weights, GRN bias, brain weights, thermal pref)
    /// is nudged by `±std` noise. GRN mutations grow/shrink/retype the body; brain mutations tune
    /// behaviour; the thermal pref drifts to track the climate it lives in. `grn_std` is kept smaller
    /// so body plans change by rarer, gentler steps than behaviour. The morphogen READ weights evolve
    /// on a SEPARATE stream (`morph_rng`) — see below.
    pub fn mutate(&self, rng: &mut Rng, morph_rng: &mut Rng, gas_rng: &mut Rng, brain_std: f32, grn_std: f32) -> Self {
        let m = |v: &[f32], std: f32, rng: &mut Rng| -> Vec<f32> {
            v.iter().map(|&w| w + rng.signed() * std).collect()
        };
        // The pre-morphogen genes are drawn from `rng` in their original order — a byte-identical RNG
        // stream to the pre-morphogen sim (so the child's spawn pos/heading, drawn from this same `rng`
        // AFTER mutate returns in `sim::step`, are unperturbed too).
        let grn_w = m(&self.grn_w, grn_std, rng);
        let grn_b = m(&self.grn_b, grn_std, rng);
        let brain = m(&self.brain, brain_std, rng);
        let thermal_pref = (self.thermal_pref + rng.signed() * grn_std).clamp(0.0, 1.0);
        let coloration = (self.coloration + rng.signed() * grn_std).clamp(0.0, 1.0);
        let toxin_resistance = (self.toxin_resistance + rng.signed() * grn_std).clamp(0.0, 1.0);
        // PR-D2 — the morphogen coupling is now LIVE: the READ weights `morph_w` EVOLVE (founder 0 ⇒
        // they drift up under selection, coupling a cell's TYPE to the local morphogen concentration,
        // i.e. to its POSITION → emergent body axes). They are drawn from an INDEPENDENT stream
        // (`morph_rng`, salted apart in `sim::step`) so activating the coupling consumes NO draws from
        // `rng`: every existing gene AND the child's pos/heading stay byte-identical to the inert sim,
        // and the trajectory shift is attributable purely to the morphogen mechanism — not an RNG
        // reshuffle. The kinetics (`diff_rate`/`decay_rate`) stay ARMED-FROZEN at the founder gradient
        // (`d≈0.5`, `k≈0.3`, the PR-D0-proven `exp(−r/λ)`): only the read weights must evolve to couple
        // type↔position, so there is no triple-trait (diffuse+decay+read) fitness valley and the
        // gradient is already there to read the instant `morph_w` lifts off 0 (C0-continuity preserved:
        // a founder still has `morph_w=0` ⇒ the armed gradient is never read ⇒ develops to one cell).
        let morph_w = m(&self.morph_w, grn_std, morph_rng);
        let diff_rate = self.diff_rate.clone();
        let decay_rate = self.decay_rate.clone();
        // O2 tolerance evolves on its OWN independent stream (`gas_rng`, salted apart in `sim::step`),
        // consuming ZERO draws from `rng` — so adding this gene leaves every existing gene AND the
        // child's pos/heading (drawn from `rng` after mutate returns) byte-identical (gas-cycle F9,
        // the morph_w pattern). NOT a draw appended to `rng`, NOT a reuse of `morph_rng`.
        let oxygen_tolerance = (self.oxygen_tolerance + gas_rng.signed() * grn_std).clamp(0.0, 1.0);
        Genome {
            grn_w,
            grn_b,
            morph_w,
            diff_rate,
            decay_rate,
            brain,
            thermal_pref,
            coloration,
            toxin_resistance,
            oxygen_tolerance,
        }
    }

    /// One GRN update of a cell: `s' = tanh(W·[s ; morph] + b)`. The regulated genes read both the `G`
    /// gene states AND the `N_MORPH` morphogen channels (the extra `W` columns), so a cell's
    /// differentiation can depend on the local morphogen concentration — i.e. on its POSITION (PR-D).
    fn regulate(&self, s: &[f32; G], morph: &[f32; N_MORPH]) -> [f32; G] {
        let mut out = [0.0f32; G];
        for (i, o) in out.iter_mut().enumerate() {
            let mut sum = self.grn_b[i];
            for (j, &sj) in s.iter().enumerate() {
                sum += self.grn_w[i * G + j] * sj;
            }
            for (c, &mc) in morph.iter().enumerate() {
                sum += self.morph_w[i * N_MORPH + c] * mc; // morphogen read (position → differentiation)
            }
            *o = sum.tanh();
        }
        out
    }

    /// The shared morphogenesis core (the SINGLE source of truth for body structure): grow the body
    /// from one seed cell by running the GRN for `DEV_STEPS`, dividing cells whose divide gene fires
    /// (daughter gets a polarity flip so sisters can differentiate), capped at `MAX_CELLS`. Returns
    /// each cell's final GRN state, its integer lattice position, and the body's `axis_order` (computed
    /// PRE-`adhesion_sort`, F6). A MORPHOGEN field co-evolves with the cells (PR-D): a position-anchored
    /// source + lattice diffusion + decay; `regulate` READS it, so a cell's type can depend on its
    /// position — this is why the old "positions feed nothing back into the GRN" invariant is now
    /// intentionally lifted. Deterministic (within a profile); depends only on the genome.
    /// **Empty GRN ⇒ exactly one structural cell at the origin (C0): with `W=0` the morphogen is never
    /// read, so the armed rates change nothing.**
    fn grow(&self) -> (Vec<[f32; G]>, Vec<(i16, i16)>, u8) {
        let mut seed = [0.0f32; G];
        seed[0] = 1.0; // a maternal factor to bootstrap a non-empty GRN (ignored by W=0)
        let mut states: Vec<[f32; G]> = vec![seed];
        let mut pos: Vec<(i16, i16)> = vec![(0, 0)];
        let mut morph: Vec<[f32; N_MORPH]> = vec![[0.0; N_MORPH]];
        // The morphogen field only matters if SOME gene reads it (`morph_w ≠ 0`). When it doesn't —
        // every founder, and every body in PR-D1 where the read genes are held inert — skip the
        // diffusion entirely: it would be computed and never read, a pure waste (this is also why
        // PR-D1 needs no render-layout cache yet — `grow`'s cost is unchanged until PR-D2 switches the
        // coupling on). Determinism is unaffected: with `morph_w = 0` the skipped field was unused.
        let reads_morph = self.morph_w.iter().any(|&w| w != 0.0);
        for _ in 0..DEV_STEPS {
            let cur = states.len(); // fixed during this step (newborns go to a side buffer)
            let mut newborn: Vec<[f32; G]> = Vec::new();
            let mut newborn_pos: Vec<(i16, i16)> = Vec::new();
            let mut newborn_morph: Vec<[f32; N_MORPH]> = Vec::new();
            for i in 0..cur {
                let ns = self.regulate(&states[i], &morph[i]); // reads this cell's morphogen
                states[i] = ns;
                if ns[GENE_DIVIDE] > DIVIDE_THETA && cur + newborn.len() < MAX_CELLS {
                    let mut child = ns;
                    child[GENE_POLARITY] = -child[GENE_POLARITY];
                    // Place the daughter on a free lattice neighbour, preferred direction from the
                    // parent's polarity. The daughter inherits the parent's current morphogen level.
                    let p = place_cell(pos[i], ns[GENE_POLARITY], &pos, &newborn_pos);
                    newborn.push(child);
                    newborn_pos.push(p);
                    newborn_morph.push(morph[i]);
                }
            }
            let settled = newborn.is_empty();
            states.extend(newborn);
            pos.extend(newborn_pos);
            morph.extend(newborn_morph);
            // Morphogen signalling on the (grown) lattice: diffuse + decay + re-pin the origin source.
            // Runs every growth step so `regulate` reads a progressively sharper gradient (one-step lag).
            if reads_morph {
                diffuse_morphogen(&mut morph, &pos, &self.diff_rate, &self.decay_rate);
            }
            if settled || states.len() >= MAX_CELLS {
                break; // body shape settled (no division) — same stop as before morphogenesis
            }
        }
        // AXIS ORDER (F6): measured on the PRE-sort type↔position map the gradient built, before the
        // cosmetic `adhesion_sort` permutes cells across slots.
        let axis_order = axis_order_metric(&states, &pos);
        // Differential adhesion: cluster same-type cells into tissues by permuting which cell sits at
        // which lattice slot (positions fixed). Preserves the cell multiset ⇒ type counts unchanged.
        adhesion_sort(&mut states, &pos);
        (states, pos, axis_order)
    }

    /// Develop the body and tally cell-type COUNTS (the per-tick stat inputs). Reduces the shared
    /// [`grow`](Self::grow) core to the same `Phenotype` counts as before morphogenesis.
    pub fn develop(&self) -> Phenotype {
        let (states, pos, axis_order) = self.grow();
        let mut p = Phenotype { n_cells: states.len() as u32, axis_order, ..Default::default() };
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
        p.organ = largest_organs(&states, &pos); // coherent-tissue size per function type (PR-C)
        p
    }

    /// The developed body as `(x, y, cell_type)` on the lattice — for RENDER ONLY (drawing the
    /// organism's shape at close zoom). Same shared [`grow`](Self::grow) core `develop()` uses, so the
    /// drawn body always matches the stats. `cell_type`: 0 = structural, 1..=7 = effector / storage /
    /// sensor / predator / flight / burrow / photo. Re-derived on demand; nothing is stored per-creature.
    pub fn body_layout(&self) -> Vec<(i16, i16, u8)> {
        let (states, pos, _axis) = self.grow();
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

/// One morphogen signalling step on the lattice (PR-D1). Per channel: DIFFUSION — relax each cell
/// toward the mean of its 4-neighbours from a SNAPSHOT (synchronous ⇒ traversal order can't matter);
/// DECAY — degrade by `1−k`; SOURCE — re-pin the origin cell to `1.0` (a position-anchored boundary).
/// Decay is essential: the PR-D0 spike showed source+diffusion ALONE homogenise to a flat field; decay
/// yields the screened-Poisson `c(r) ∝ exp(−r/λ)` gradient. The neighbour sum is a serial fixed index
/// order ⇒ within-profile deterministic (it inherits the GRN's per-profile FMA, like the golden).
fn diffuse_morphogen(morph: &mut [[f32; N_MORPH]], pos: &[(i16, i16)], diff: &[f32], decay: &[f32]) {
    let n = morph.len();
    for c in 0..N_MORPH {
        let snap: Vec<f32> = morph.iter().map(|m| m[c]).collect();
        for a in 0..n {
            let (mut sum, mut cnt) = (0.0f32, 0u32);
            for b in 0..n {
                if is_adjacent(pos[a], pos[b]) {
                    sum += snap[b];
                    cnt += 1;
                }
            }
            if cnt > 0 {
                morph[a][c] += diff[c] * (sum / cnt as f32 - snap[a]);
            }
            morph[a][c] *= 1.0 - decay[c];
        }
        for a in 0..n {
            if pos[a] == (0, 0) {
                morph[a][c] = 1.0; // SOURCE (origin)
            }
        }
    }
}

/// AXIS ORDER (PR-D1): how strongly cell TYPE varies with RADIAL position (Manhattan distance from the
/// origin source) — η² = (between-type variance of radial distance) / (total variance), mapped to
/// `0..=255`. A RATIO ⇒ scale-invariant: it does NOT reward sheer body size (the F1 trap), only genuine
/// type↔position structure. `0` for a founder, a single type, or equidistant cells; high when the
/// morphogen gradient has segregated types along the axis (an emergent body plan). Computed PRE-sort.
fn axis_order_metric(states: &[[f32; G]], pos: &[(i16, i16)]) -> u8 {
    let n = states.len();
    if n < 2 {
        return 0;
    }
    let d: Vec<f32> = pos.iter().map(|&(x, y)| (x.abs() + y.abs()) as f32).collect();
    let mean_all = d.iter().sum::<f32>() / n as f32;
    let ss_total: f32 = d.iter().map(|&v| (v - mean_all).powi(2)).sum();
    if ss_total <= 0.0 {
        return 0; // all cells equidistant from the source ⇒ no axis to speak of
    }
    let types: Vec<u8> = states.iter().map(cell_type).collect();
    let mut ss_between = 0.0f32;
    for t in 0u8..=7 {
        let members: Vec<f32> = (0..n).filter(|&i| types[i] == t).map(|i| d[i]).collect();
        if members.is_empty() {
            continue;
        }
        let mg = members.iter().sum::<f32>() / members.len() as f32;
        ss_between += members.len() as f32 * (mg - mean_all).powi(2);
    }
    ((ss_between / ss_total).clamp(0.0, 1.0) * 255.0) as u8
}

/// Largest CONNECTED same-type cluster size per function type (4-adjacency on the lattice), index
/// 0..=6 = effector..photo. This is the "organ" coherence the differential-adhesion sort builds up —
/// the input to [`Phenotype::organ_power`]. Structural cells (type 0) are ignored. O(n²), n ≤ 32.
fn largest_organs(states: &[[f32; G]], pos: &[(i16, i16)]) -> [u8; 7] {
    let n = states.len();
    let types: Vec<u8> = states.iter().map(cell_type).collect();
    let mut organ = [0u8; 7];
    let mut visited = vec![false; n];
    for start in 0..n {
        let t = types[start];
        if t == 0 || visited[start] {
            continue; // structural, or already part of a counted component
        }
        let mut stack = vec![start];
        visited[start] = true;
        let mut size = 0u32;
        while let Some(a) = stack.pop() {
            size += 1;
            for b in 0..n {
                if !visited[b] && types[b] == t && is_adjacent(pos[a], pos[b]) {
                    visited[b] = true;
                    stack.push(b);
                }
            }
        }
        let idx = (t - 1) as usize;
        organ[idx] = organ[idx].max(size.min(255) as u8);
    }
    organ
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
        for &w in &self.morph_w {
            crate::rng::fnv_fold_u32(&mut h, w.to_bits());
        }
        for &r in &self.diff_rate {
            crate::rng::fnv_fold_u32(&mut h, r.to_bits());
        }
        for &r in &self.decay_rate {
            crate::rng::fnv_fold_u32(&mut h, r.to_bits());
        }
        for &w in &self.brain {
            crate::rng::fnv_fold_u32(&mut h, w.to_bits());
        }
        crate::rng::fnv_fold_u32(&mut h, self.thermal_pref.to_bits());
        crate::rng::fnv_fold_u32(&mut h, self.coloration.to_bits());
        crate::rng::fnv_fold_u32(&mut h, self.toxin_resistance.to_bits());
        crate::rng::fnv_fold_u32(&mut h, self.oxygen_tolerance.to_bits());
        h
    }
}

/// Frozen ANM2 `Genome` shape (pre-`oxygen_tolerance`) for save migration ([`crate::persist`] v2).
/// NEVER edit — it must reproduce the EXACT ANM2 bincode layout (`Genome` minus the trailing
/// `oxygen_tolerance`). Field order/types mirror ANM2 `Genome` verbatim.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct GenomeV2 {
    grn_w: Vec<f32>,
    grn_b: Vec<f32>,
    morph_w: Vec<f32>,
    diff_rate: Vec<f32>,
    decay_rate: Vec<f32>,
    brain: Vec<f32>,
    thermal_pref: f32,
    coloration: f32,
    toxin_resistance: f32,
}

impl GenomeV2 {
    /// ANM2 → current. The only added gene is `oxygen_tolerance`, filled with its CONTINUITY value
    /// `0.0` — a pre-feature (anoxic) save's lineages never faced O2 selection, so they resume sensitive.
    pub(crate) fn migrate(self) -> Genome {
        Genome {
            grn_w: self.grn_w,
            grn_b: self.grn_b,
            morph_w: self.morph_w,
            diff_rate: self.diff_rate,
            decay_rate: self.decay_rate,
            brain: self.brain,
            thermal_pref: self.thermal_pref,
            coloration: self.coloration,
            toxin_resistance: self.toxin_resistance,
            oxygen_tolerance: 0.0,
        }
    }
}

#[cfg(test)]
impl Genome {
    /// Down-convert to the frozen ANM2 shape (drops `oxygen_tolerance`) — migration-test support only.
    pub(crate) fn to_v2(&self) -> GenomeV2 {
        GenomeV2 {
            grn_w: self.grn_w.clone(),
            grn_b: self.grn_b.clone(),
            morph_w: self.morph_w.clone(),
            diff_rate: self.diff_rate.clone(),
            decay_rate: self.decay_rate.clone(),
            brain: self.brain.clone(),
            thermal_pref: self.thermal_pref,
            coloration: self.coloration,
            toxin_resistance: self.toxin_resistance,
        }
    }
}

#[cfg(test)]
#[path = "genome_tests.rs"]
mod tests;
