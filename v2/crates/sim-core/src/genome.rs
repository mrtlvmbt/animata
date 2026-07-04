//! Direct-encoded Ф0 genome — **8 integer traits + photo-regulation gene (D′-2b)**. Integer
//! everywhere: mutation is an integer perturbation, the metabolic cost is an integer function of
//! size, and the genome folds into the deterministic state hash. No float in the genetics layer.
// Guard: no float arithmetic in the conserved layer (M0/F2). Complements the token-grep in
// no_float_guard.rs: `float_arithmetic` catches operations on inferred-float types that the grep
// misses (e.g. `let x = 1.5; x + 1.0` where no `f32`/`f64` keyword appears).
#![deny(clippy::float_arithmetic)]

use crate::{
    brain_w_ho, brain_w_ih, fnv_mix, grn, morphogen, seed_fold, Boundary, CellType, EconParams,
    GrnSpec, MorphogenSpec, BRAIN_WEIGHTS, GRN_EXPR_MAX,
};
use bevy_ecs::prelude::Component;
use std::sync::Arc;

/// Integer square root (floor), Newton's method. Deterministic, arch-independent.
pub fn isqrt(n: i64) -> i64 {
    if n <= 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// Integer `size^(3/4) = sqrt(sqrt(size^3))` — Kleiber metabolic scaling (economy/01 §6) as a pure
/// integer function (two `isqrt`s). Arch-independent ⇒ the metabolic cost (a conserved-layer
/// quantity) never depends on float.
pub fn size_pow_three_quarters(size: i32) -> i64 {
    let s = (size.max(1)) as i64;
    isqrt(isqrt(s * s * s))
}

/// The six Ф0 traits + two B-2 layer-targeting traits (research/13 §2). Ranges are clamped on
/// mutation; all integer.
///
/// **`Clone`, NOT `Copy`** (V-1): `grn_spec` carries a `GrnSpec` (`Vec<i32>` fields), so
/// `Option<Arc<GrnSpec>>` cannot be `Copy`. Mirrors the `EconParams` Copy→Clone ripple from E-4a —
/// every implicit-copy call site was audited and converted to an explicit `.clone()` (compiler-
/// enforced: a missed site is a compile-time move error, not a silent runtime bug).
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct Genome {
    /// Resource→energy conversion efficiency, as a fraction of 256 (0..=256).
    pub metabolism_eff: i32,
    /// Cells moved per tick (movement is metabolically priced).
    pub move_speed: i32,
    /// Gradient-sensing radius in cells (sensing is priced).
    pub sense_range: i32,
    /// Body size → metabolism ∝ size^(3/4).
    pub size: i32,
    /// Energy threshold to divide.
    pub repro_threshold: i32,
    /// Heritable mutation rate, as a fraction of 256 (probability scale).
    pub mutation_rate: i32,
    /// Conserved layer to eat from and sense (0..=n_layers-1). Founder eats layer 0 (substrate).
    pub uptake_layer: i32,
    /// Conserved layer to excrete to (0..=n_layers-1). Founder excretes to layer 1 at L≥2 (seeds
    /// cross-feeding gradient); at L=1 this is 0 (closed mono-layer loop, no out-of-bounds).
    pub excrete_layer: i32,
    /// **Test-only injection flag** for the E-1/E-4 decode-gate plumbing (never set in Ф0 production).
    /// When `true`, `decode()` returns `None` — exercises the skip path in `stage_birth_death` without
    /// introducing a real viability filter (that is E-4). The flag is heritable: `mutate()` copies
    /// `*self`, so children of a poisoned parent also carry it, making the entire lineage stillborn.
    /// `#[cfg(test)]` → zero cost, zero size, zero impact outside test builds.
    #[cfg(test)]
    pub(crate) force_decode_none: bool,
    /// Evolved brain weights for the FIXED topology (D-Brain-1) — `int8` Q1.7, packed `W_ih·W_hh·W_ho`
    /// (layout = the shared [`crate::brain_w_ih`]/`brain_w_hh`/`brain_w_ho` indices). Inherited and
    /// mutated exactly like the six Ф0 traits; the `brain` crate reads this vector during inference.
    /// Resident here (genome-SoA in the ECS) so no genome→weights repack happens on a Brain tick.
    pub weights: [i8; BRAIN_WEIGHTS],
    /// Photo-energy absorption capacity (D′-1). `0` → no phototrophy; higher → more light energy
    /// per tick via `U_photo(L) = photo_gain · L / (km_photo + L)`. Mutated only when the light
    /// field is present (`EconParams.light.is_some()`) — non-dprime genomes always carry 0, so the
    /// existing arm64 goldens stay byte-identical un-re-pinned. Range: 0..=256.
    pub photo_gain: i32,

    // ── D′-2b: photo-GRN regulation gene (reuses D-slice setpoint+gain pattern on L(t)) ──────────
    /// Light-signal setpoint for photo-expression regulation (D′-2b). Compared to `L(t)` by the
    /// `expressed_capacity` rule. Calibrated to `l_max / 2 = 50` (equidistant from day=100 and
    /// night=0 in `dprime_config`) so both positive and negative `reg_gain` polarities are viable
    /// from the founder. Range [0, 256]. Mutates only when `has_light`.
    pub reg_setpoint: i32,
    /// Photo-expression signed gain (D′-2b). **Explicit disabled encoding**: `0` = INERT (founder /
    /// regulation OFF) — the cell expresses photo constitutively, behaving exactly as D′-2a.
    ///   `> 0`: express by DAY  (`L ≥ reg_setpoint` → full `photo_gain`; `L < reg_setpoint` → 0).
    ///   `< 0`: express by NIGHT (`L < reg_setpoint` → full `photo_gain`; `L ≥ reg_setpoint` → 0).
    ///
    /// **Encoding (declared F3 — binary threshold).** The gain MAGNITUDE is dead weight on the
    /// expression function — only `sign(reg_gain)` affects `expressed_capacity`. The trait is
    /// effectively 3-state: neg / 0 / pos. `reg_gain_max` controls the evolvable range
    /// `[−reg_gain_max, +reg_gain_max]` and LOCKS regulation OFF when 0 (the D′-2c control line).
    /// D′-2c must account for this: the constitutive control is `reg_gain_max = 0`, not a specific
    /// gain value. All non-zero gains produce identical binary expression behaviour.
    ///
    /// Founder = 0 (INERT). Mutates only when `has_light` (same gate as `photo_gain`) so non-dprime
    /// genomes carry it at 0 forever → 4 existing goldens byte-identical. Range `[−max, +max]`.
    pub reg_gain: i32,

    // ── P-2a: predation combat trait (heritable niche) ─────────────────────────────────────────
    /// Combat strength locus — the real predation trait decoupled from `size` (P-2a). Range [0, 32];
    /// founders = 0 (predation is emergent, not ancestral). Mutated ±1 clamped under a DISTINCT salt
    /// (`SALT_COMBAT`), drawn only when `config.predation.is_some()` — non-predation configs stay
    /// byte-identical (mutation gate prevents draw, hash gate prevents state inclusion).
    /// Read by P-1 substrate `resolve_encounter` via `PredationSpec` mapping (P-2a wires it in).
    pub combat_trait: i32,

    // ── V-1: heritable + point-mutable indirect genome (the differentiation PROGRAM) ────────────
    /// The GRN regulatory spec — heritable, per-individual, point-mutated by [`Genome::mutate`].
    /// `None` for the five non-phase2 configs (`EconParams.grn` stays `None` there too — `decode`
    /// takes the trivial `_ => None` cell_type projection, byte-identical to pre-V-1 behavior).
    /// `Arc` for copy-on-write (RnD 37 §1): an unmutated child SHARES the parent's spec (a cheap
    /// refcount bump via `Genome::clone`); only a spec-mutating division allocates a fresh clone
    /// (`mutate`, below). Folded into [`Genome::hash_contribution`] by INTEGER CONTENTS, Some-gated
    /// — the `Arc` pointer itself is never hashed (CoW sharing is transparent to the hash).
    pub grn_spec: Option<Arc<GrnSpec>>,
    /// The morphogen spec — heritable (carried per-individual like `grn_spec`) but NOT mutated in
    /// V-1 (scope: the fate-relevant lever on the dead-drive phase2 spec is the GRN spec; morphogen-
    /// param mutation is a trivial later addition, out of V-1 scope). `MorphogenSpec` is small and
    /// `Copy` — no `Arc` needed here (unlike `grn_spec`'s `Vec<i32>` fields).
    pub morphogen_spec: Option<MorphogenSpec>,
}

/// M7-a: graph-body COLD representation — the multicellular module lattice derived from
/// per-grid-cell classification of the morphogen gradient. Integer-only, canonical labeling,
/// deterministic, prod-inert (never consumed in M7-a). `ModuleId` is row-major order + union-find
/// minimum index (order-independent, architecture-stable). Each module records its cell type and
/// total cell count (per-module adjacency deferred to M7-f).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellGraph {
    /// Grid size (square grid is `g_dev × g_dev`, same as the morphogen spec).
    pub g_dev: usize,
    /// Per-module cell type (indexed by ModuleId = module's representative index in row-major traversal).
    pub module_type: Vec<CellType>,
    /// Per-module cell count (indexed by ModuleId).
    pub module_cell_count: Vec<i32>,
}

impl CellGraph {
    /// Empty graph (zero modules, e.g. for non-phase2 configs where the graph is not computed).
    pub fn empty() -> Self {
        CellGraph {
            g_dev: 0,
            module_type: Vec::new(),
            module_cell_count: Vec::new(),
        }
    }

    /// Decode a morphogen gradient into a multicellular graph: per-grid-cell classification,
    /// connected-component labeling via union-find, canonical row-major order + min-index representative.
    pub fn from_gradient(gradient: &crate::morphogen::Gradient, gspec: &GrnSpec) -> Self {
        let g_dev = gradient.g_dev;
        let n_cells = g_dev * g_dev;

        // 1. Per-grid-cell classification: each cell gets a CellType from its local gradient value.
        // For each grid cell, sample the gradient at that position and run the GRN to resolve
        // the attractor state, then classify that state to determine the cell type.
        let mut grid_cell_type: Vec<CellType> = Vec::with_capacity(n_cells);
        for z in 0..g_dev {
            for x in 0..g_dev {
                // Create a singleton gradient with the value at this cell
                let grad_val = gradient.at(x, z);
                let cell_gradient = crate::morphogen::Gradient {
                    g_dev: 1,
                    cells: vec![grad_val],
                };
                // Create a modified GrnSpec that samples at (0, 0) (the only cell in the singleton gradient)
                let mut cell_gspec = gspec.clone();
                cell_gspec.sample_x = 0;
                cell_gspec.sample_z = 0;
                // Run the GRN with this gradient value and resolve to attractor
                let (_state, ct, _steps) = crate::grn_resolve(&cell_gradient, &cell_gspec);
                grid_cell_type.push(ct);
            }
        }

        // 2. Union-find: connect same-type adjacent cells (4-neighbour), min-index representative.
        let mut parent: Vec<usize> = (0..n_cells).collect();

        fn find(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]]; // path compression
                i = parent[i];
            }
            i
        }

        fn union(parent: &mut [usize], a: usize, b: usize) {
            let ra = find(parent, a);
            let rb = find(parent, b);
            if ra != rb {
                // Union by min-index representative (deterministic, order-independent).
                if ra < rb {
                    parent[rb] = ra;
                } else {
                    parent[ra] = rb;
                }
            }
        }

        // Row-major traversal: connect each cell to its right and down neighbors (if same type).
        for z in 0..g_dev {
            for x in 0..g_dev {
                let idx = z * g_dev + x;
                let ct = grid_cell_type[idx];
                // Right neighbor
                if x + 1 < g_dev {
                    let right_idx = z * g_dev + (x + 1);
                    if grid_cell_type[right_idx] == ct {
                        union(&mut parent, idx, right_idx);
                    }
                }
                // Down neighbor
                if z + 1 < g_dev {
                    let down_idx = (z + 1) * g_dev + x;
                    if grid_cell_type[down_idx] == ct {
                        union(&mut parent, idx, down_idx);
                    }
                }
            }
        }

        // 3. Collect modules: each distinct root → one module.
        let mut module_id_map: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
        let mut module_type: Vec<CellType> = Vec::new();
        let mut module_cell_count: Vec<i32> = Vec::new();

        for idx in 0..n_cells {
            let root = find(&mut parent, idx);
            let ct = grid_cell_type[idx];

            if !module_id_map.contains_key(&root) {
                let mid = module_type.len();
                module_id_map.insert(root, mid);
                module_type.push(ct);
                module_cell_count.push(0);
            }
            let mid = module_id_map[&root];
            module_cell_count[mid] += 1;
        }

        CellGraph {
            g_dev,
            module_type,
            module_cell_count,
        }
    }

    /// Number of modules (connected components).
    pub fn num_modules(&self) -> usize {
        self.module_type.len()
    }
}

/// Phase-2 E-1: cold, lean cache of the decoded genome traits consumed by hot-path stages.
///
/// Attached at every spawn site (founders + children) so a `&Phenotype` query is REQUIRED
/// (not optional) — a missed spawn site makes that entity invisible to the consumer stage,
/// which is detectable via a shifted golden (the correct detection signal, not a runtime panic).
///
/// **Ф0 content**: `uptake_layer` — the raw integer field consumed by `stage_interactions`.
///
/// **E-4a**: `cell_type` — the resolved ontogenesis attractor when `EconParams.morphogen` +
/// `EconParams.grn` are both `Some` (E-1's trivial Ф0 projection otherwise, `None`). Pinned as
/// `Option<CellType>`, NOT a new `CellType::Undifferentiated` variant (critic F5): `CellType` is
/// the GRN's own attractor enum (`grn.rs`) and must not carry a value `grn_resolve` never
/// produces; `Option` is the same proven gate `EconParams.light`/`.mineral_layer` already use.
/// **No consumer reads this field in E-4a** — it is behaviourally inert this slice (E-4b adds the
/// consumer); growing this archetype column is what this slice proves neutral (see `Genome::decode`).
///
/// **M7-a**: `graph` — the multicellular graph-body COLD representation. `CellGraph` is computed
/// from the morphogen grid, never consumed in M7-a (prod-inert), and causes `Phenotype` to lose
/// `Copy` (must be `Clone`). All hot-path copy sites have been audited and updated to use
/// `.clone()` explicitly (copy propagates through reference, no performance impact).
///
/// NOT folded into `hash_contribution`: phenotype is a deterministic cold derivative of the
/// genome that is already in the hash; double-hashing is redundant (plan §2/§6, R19).
#[derive(bevy_ecs::prelude::Component, Clone, Debug, PartialEq, Eq)]
pub struct Phenotype {
    /// Layer index the entity will eat from (direct copy of `Genome::uptake_layer` for Ф0).
    pub uptake_layer: i32,
    /// Resolved ontogenesis cell type (E-4a). `None` for Ф0 / all 5 existing configs.
    pub cell_type: Option<CellType>,
    /// M7-a: multicellular graph-body COLD representation. Computed from the morphogen grid;
    /// never consumed in M7-a (prod-inert). Empty for non-phase2 configs.
    pub graph: CellGraph,
}


/// E-5b viability criterion (plan §4.1): the minimum `size` an embryo must exceed to survive to
/// materialize. `size`-threshold-based (NOT `cell_type == Mixed` — critic history/E-4b-ii shelving:
/// a cell_type-value criterion is NULL-prone, `Mixed` collapses under selection). `size` is
/// continuously regenerated by mutation (`genome.rs` mutate: unit-step ±1, reflecting wall at 1,
/// range `[1,32]`, founder=4), so stillbirths RECUR over the horizon instead of dying out once.
///
/// Calibrated against `phase2_config(0xA11A_2A11)` (`cli/tests/phase2_viability.rs`): the founder's
/// own `size` is 4, and `Sim::new` unconditionally `.expect()`s the founder's own `decode()` to
/// return `Some` — so the floor MUST be `< 4` or founders themselves miscarry at spawn (an
/// immediate panic, not merely an extinction risk). `3` is therefore the highest floor this
/// mechanism structurally permits, and it is also empirically the best-calibrated choice: the
/// first real stillbirth lands at tick 35 (inside the 384-tick golden window, well clear of
/// `GOLDEN_LAST_TICK=383`), phase2 stays bounded (`population.min() > 0` from tick 0 to 1200+), and
/// the criterion genuinely RECURS (1 stillbirth by tick 384, 5 by tick 1200) rather than firing once
/// and going silent. A floor at `size <= 2` was tried first and observed to be too rare — zero
/// stillbirths over 400 ticks for this seed (a lineage must drift down TWO generations, 4→3→2,
/// compounding an already-~4% per-division event).
pub(crate) const SIZE_VIABILITY_FLOOR: i32 = 3;

/// Pure integer viability predicate — `true` iff `size` clears the floor. Scoped to the `(Some,
/// Some)` chain arm of `decode` (only `phase2_config` enters it today); the five existing configs
/// take `_ => None` for `cell_type` and never call this, so they can never produce a stillbirth.
fn is_viable_size(size: i32) -> bool {
    size > SIZE_VIABILITY_FLOOR
}

/// Exact-integer `CellType` → `uptake_layer` decision (E-4b-i). `A` eats layer 0; `B` eats layer 1
/// (clamped into `[0, n_layers)` — degenerate `n_layers <= 1` configs never route here in practice,
/// but the clamp keeps the function total); an exact-tie `Mixed` resolution falls back to the raw
/// genome value (no differentiation signal to act on). Never a float threshold.
fn cell_type_uptake_layer(cell_type: CellType, genome_fallback: i32, n_layers: usize) -> i32 {
    let max_layer = (n_layers.max(1) - 1) as i32;
    match cell_type {
        CellType::A => 0,
        CellType::B => 1.min(max_layer),
        CellType::Mixed => genome_fallback,
    }
}

impl Genome {
    /// The founder phenotype — viable (feeds more than it burns at abundance). The founder brain is a
    /// minimal **resource-chemotaxis reflex** so the M3 population starts behaviourally viable (it
    /// climbs the resource gradient, as M1's hard-coded Act did) and evolution tunes the net from
    /// there: hidden 0 ← resource-gradient-x, hidden 1 ← resource-gradient-z, output 0 (vx) ← hidden 0,
    /// output 1 (vz) ← hidden 1, every other weight zero. Inputs 2..6 (local resource, energy, bias,
    /// reserved) start with zero weight — emergence wires them in.
    /// The founder phenotype (config-derived for B-2). `n_layers` determines `excrete_layer`:
    /// at L=1 excretes to layer 0 (closed loop, bench-safe); at L≥2 excretes to layer 1
    /// (seeds the producer half of the cross-feeding gradient).
    pub fn founder(n_layers: usize) -> Self {
        let mut weights = [0i8; BRAIN_WEIGHTS];
        weights[brain_w_ih(0, 0)] = 127; // hidden 0 ← input 0 (grad x)
        weights[brain_w_ih(1, 1)] = 127; // hidden 1 ← input 1 (grad z)
        weights[brain_w_ho(0, 0)] = 127; // output 0 (vx) ← hidden 0
        weights[brain_w_ho(1, 1)] = 127; // output 1 (vz) ← hidden 1
        Genome {
            metabolism_eff: 200,
            move_speed: 1,
            sense_range: 1,
            size: 4,
            repro_threshold: 1500,
            mutation_rate: 32,
            uptake_layer: 0,
            excrete_layer: (n_layers.saturating_sub(1)).min(1) as i32,
            weights,
            photo_gain: 0,  // D′-1: founders carry zero photo capacity; evolution brings it up
            // D′-2b: regulation gene INERT at founding (reg_gain=0 explicit disabled encoding).
            // reg_setpoint calibrated to l_max/2=50 so both polarities (+gain=day, -gain=night)
            // are equidistant from the founder; evolution discovers direction (F7 — no hardcode).
            reg_setpoint: 50,
            reg_gain: 0,
            // P-2a: combat_trait founder = 0, predation is emergent (not ancestral).
            combat_trait: 0,
            // Test-only E-1/E-4 injection flag — always false in production.
            #[cfg(test)]
            force_decode_none: false,
            // V-1: no heritable spec by default — attach one via `with_specs` at founder-spawn
            // (production: `Sim::new`, seeded from `EconParams.grn`/`.morphogen`).
            grn_spec: None,
            morphogen_spec: None,
        }
    }

    /// V-1: attach a heritable GRN/morphogen spec to this genome. Used ONCE, at founder-spawn
    /// (`Sim::new`), to seed the per-lineage differentiation program from `EconParams`'s founder
    /// template — `EconParams.grn`/`.morphogen` are consulted ONLY there; `decode` reads `self`
    /// instead (below), never `econ.grn`/`econ.morphogen` directly.
    pub fn with_specs(mut self, grn_spec: Option<Arc<GrnSpec>>, morphogen_spec: Option<MorphogenSpec>) -> Self {
        self.grn_spec = grn_spec;
        self.morphogen_spec = morphogen_spec;
        self
    }

    /// Integer metabolic cost units `size^(3/4)`.
    pub fn metab_units(&self) -> i64 {
        size_pow_three_quarters(self.size)
    }

    /// Deterministic mutated clone. `stream` is a per-birth seeded value; each trait draws a disjoint
    /// integer perturbation in `{-1,0,+1}` gated by `mutation_rate`, then is clamped to range.
    /// `n_layers` clamps layer traits to `0..=n_layers-1` — must equal the field's actual layer
    /// count (guaranteed by `build_sim` setting `econ.n_layers = config.n_layers`).
    /// `has_light` gates the `photo_gain` and reg-gene mutations (D′-1/D′-2b): when `false`, both
    /// stay at their founder values — non-dprime genomes never carry a non-zero photo or reg gene,
    /// keeping existing goldens byte-identical.
    /// `reg_gain_max` clamps the reg-gain range to `[−reg_gain_max, +reg_gain_max]` (D′-2b).
    /// Set `reg_gain_max = 0` to lock regulation OFF — reg_gain stays 0 (the D′-2c control line).
    /// `has_predation` gates the `combat_trait` mutation (P-2a): when `false`, combat_trait stays 0,
    /// non-predation genomes never carry a non-zero combat_trait, keeping existing goldens byte-identical.
    pub fn mutate(&self, stream: u64, n_layers: usize, has_light: bool, reg_gain_max: i32, has_predation: bool, enable_variable_length: bool) -> Genome {
        let mut g = self.clone();
        let max_layer = n_layers.saturating_sub(1) as i32;
        let traits: [(&mut i32, i32, i32); 8] = [
            (&mut g.metabolism_eff, 0, 256),
            (&mut g.move_speed, 0, 8),
            (&mut g.sense_range, 0, 8),
            (&mut g.size, 1, 32),
            (&mut g.repro_threshold, 200, 5000),
            (&mut g.mutation_rate, 0, 256),
            (&mut g.uptake_layer, 0, max_layer),
            (&mut g.excrete_layer, 0, max_layer),
        ];
        for (i, (slot, lo, hi)) in traits.into_iter().enumerate() {
            let r = seed_fold(stream, &[0x6D75_7400 + i as u64]); // "mut" + trait index
            // Gate the mutation by mutation_rate/256, then a signed unit step.
            if (r & 0xFF) < self.mutation_rate as u64 {
                let delta = ((r >> 8) % 3) as i32 - 1; // -1,0,+1
                *slot = (*slot + delta).clamp(lo, hi);
            }
        }
        // Brain weights mutate the same way — but their RNG draws come LAST (disjoint salt stream), so
        // the six Ф0 traits above keep their exact historical draw sequence (skill §5.2 hygiene).
        for (wi, w) in g.weights.iter_mut().enumerate() {
            let r = seed_fold(stream, &[0x7700_0000 + wi as u64]); // "w" + weight index
            if (r & 0xFF) < self.mutation_rate as u64 {
                let delta = ((r >> 8) % 3) as i64 - 1; // -1,0,+1
                *w = (*w as i64 + delta).clamp(-127, 127) as i8;
            }
        }
        // D′-1/D′-2b: photo_gain and reg gene mutate only when light is present.
        // Salts are disjoint from trait (0x6D757400+) and weight (0x77000000+) salts → uncorrelated
        // draw streams. Come AFTER weights so prior draws are undisturbed (§5.2 stream hygiene).
        if has_light {
            // photo_gain — salt 0x5048_4700 ("PHG\0")
            let r = seed_fold(stream, &[0x5048_4700u64]);
            if (r & 0xFF) < self.mutation_rate as u64 {
                let delta = ((r >> 8) % 3) as i32 - 1; // -1, 0, +1
                g.photo_gain = (g.photo_gain + delta).clamp(0, 256);
            }
            // D′-2b: reg_setpoint — salt 0x5253_5000 ("RSP\0")
            let r_sp = seed_fold(stream, &[0x5253_5000u64]);
            if (r_sp & 0xFF) < self.mutation_rate as u64 {
                let delta = ((r_sp >> 8) % 3) as i32 - 1;
                g.reg_setpoint = (g.reg_setpoint + delta).clamp(0, 256);
            }
            // D′-2b: reg_gain — salt 0x5247_4E00 ("RGN\0").
            // When reg_gain_max=0: clamp(-0,0) always yields 0 → regulation locked OFF (D′-2c line).
            let r_gn = seed_fold(stream, &[0x5247_4E00u64]);
            if (r_gn & 0xFF) < self.mutation_rate as u64 {
                let delta = ((r_gn >> 8) % 3) as i32 - 1;
                g.reg_gain = (g.reg_gain + delta).clamp(-reg_gain_max, reg_gain_max);
            }
        }

        // P-2a: combat_trait mutates only when predation is enabled. Salt 0x434F_4D42 ("COMB").
        // Same ±1 pattern as the six Ф0 traits. Non-predation configs: combat_trait stays 0,
        // existing goldens byte-identical (mutation gate prevents draw, hash gate prevents state).
        if has_predation {
            let r = seed_fold(stream, &[0x434F_4D42u64]);
            if (r & 0xFF) < self.mutation_rate as u64 {
                let delta = ((r >> 8) % 3) as i32 - 1; // -1, 0, +1
                g.combat_trait = (g.combat_trait + delta).clamp(0, 32);
            }
        }

        // V-1: point-mutation of the heritable GRN spec — FIXED-LENGTH only (n_genes==2 invariant
        // holds: no gene duplication/indel/translocation, those are deferred). A known 10 integer
        // draws (4 weights + 2 input_weights + 2 bias + 2 initial), disjoint salts from every
        // prior draw stream above, drawn AFTER them (§5.2 stream hygiene — backward-compatible
        // draw positions preserved). `g.grn_spec` already shares the parent's `Arc` (from
        // `self.clone()` above) — CoW: only overwritten with a fresh `Arc::new` if a draw actually
        // fires, so an unmutated child (the common case) never allocates. Morphogen spec is
        // heritable but NOT mutated in V-1 (`g.morphogen_spec` already carried over by
        // `self.clone()`, unchanged — `MorphogenSpec` is `Copy`).
        //
        // Step magnitude `GRN_SPEC_MUT_STEP=4` (NOT `±1` like the six Ф0 traits): this GRN toggle
        // family self-amplifies over its iterated steps (grn.rs) into a hard either/or attractor —
        // empirically (throwaway probe, mirrors E-6's calibration process) a `±1` step on the
        // repositioned spec (`weights=[32,-32,-32,32]`, `initial=[144,112]`, `cli::phase2_config`)
        // NEVER flips the fate (0/20 single-field perturbations); `±4` gives a measured 2/20 (10%)
        // — a real but strict-minority flip rate, comfortably inside `0 < rate < 50%` (see the
        // heritability teeth in `phase2_viability.rs`).
        const GRN_SPEC_MUT_STEP: i32 = 4;
        if let Some(gspec) = &self.grn_spec {
            let mut mutated = (**gspec).clone();
            let mut any_change = false;

            for (i, w) in mutated.weights.iter_mut().enumerate() {
                let r = seed_fold(stream, &[0x4752_4E57 + i as u64]); // "GRNW" + index
                if (r & 0xFF) < self.mutation_rate as u64 {
                    let delta = (((r >> 8) % 3) as i32 - 1) * GRN_SPEC_MUT_STEP; // -4,0,+4
                    *w = (*w + delta).clamp(-1024, 1024);
                    any_change = true;
                }
            }
            for (i, w) in mutated.input_weights.iter_mut().enumerate() {
                let r = seed_fold(stream, &[0x4752_4E49 + i as u64]); // "GRNI" + index
                if (r & 0xFF) < self.mutation_rate as u64 {
                    let delta = (((r >> 8) % 3) as i32 - 1) * GRN_SPEC_MUT_STEP;
                    *w = (*w + delta).clamp(-1024, 1024);
                    any_change = true;
                }
            }
            for (i, w) in mutated.bias.iter_mut().enumerate() {
                let r = seed_fold(stream, &[0x4752_4E42 + i as u64]); // "GRNB" + index
                if (r & 0xFF) < self.mutation_rate as u64 {
                    let delta = (((r >> 8) % 3) as i32 - 1) * GRN_SPEC_MUT_STEP;
                    *w = (*w + delta).clamp(-1024, 1024);
                    any_change = true;
                }
            }
            for (i, w) in mutated.initial.iter_mut().enumerate() {
                // "GRNL" ("initiaL") + index — clamped to [0, GRN_EXPR_MAX]: `initial` is a gene
                // EXPRESSION level (grn.rs), not a free coefficient like weights/input_weights/bias.
                let r = seed_fold(stream, &[0x4752_4E4C + i as u64]);
                if (r & 0xFF) < self.mutation_rate as u64 {
                    let delta = (((r >> 8) % 3) as i32 - 1) * GRN_SPEC_MUT_STEP;
                    *w = (*w + delta).clamp(0, GRN_EXPR_MAX);
                    any_change = true;
                }
            }

            if any_change {
                g.grn_spec = Some(Arc::new(mutated));
            }
        }
        // V-3-b: variable-length genome duplication operator (CANONICAL ORDER — appended AFTER all V-1 draws)
        // Gate: when enable_variable_length == false, the operator draws ZERO values and n_genes stays constant
        // → existing goldens byte-identical (neutrality contract).
        // Mechanism: draw application-count from new private salt (SALT_DUP), then per-application:
        // - Draw which gene to duplicate (0..n_genes)
        // - Copy that gene's row+col in weights
        // - Append entries to input_weights, bias, initial
        // - Assign new gene id = ((parent_id+1)<<16) | dup_index
        // - Increment dup_counter and n_genes
        // Determinism: private stream + fixed draw order → stream position = pure fn of (seed, input genome).
        const SALT_DUP: u64 = 0x4455_5000u64; // "DUP\0"
        if enable_variable_length {
            if let Some(gspec) = &self.grn_spec {
                // Draw application-count (like other operators)
                let r_dup = seed_fold(stream, &[SALT_DUP]);
                let dup_count = if (r_dup & 0xFF) < self.mutation_rate as u64 { 1 } else { 0 };
                
                if dup_count > 0 {
                    let mut mutated = (**gspec).clone();
                    let old_n_genes = mutated.n_genes;
                    
                    for dup_idx in 0..dup_count {
                        // Draw which gene to duplicate
                        let r_which = seed_fold(stream, &[SALT_DUP + 1 + dup_idx as u64]);
                        let gene_to_dup = (r_which >> 8) as usize % old_n_genes;
                        let parent_id = mutated.gene_ids[gene_to_dup];
                        
                        // Expand weights matrix to (n+1)×(n+1) by inserting a new gene at the end.
                        // Row-major: w_ij = influence of gene j on gene i.
                        // For duplicating source_gene: copy its row and column to the new gene's position.
                        let n = mutated.n_genes;
                        let source = gene_to_dup;
                        let mut new_weights = Vec::with_capacity((n + 1) * (n + 1));

                        // Copy existing n×n matrix, but expand each row to include the new gene
                        for row in 0..n {
                            // Copy existing row values
                            for col in 0..n {
                                let idx = row * n + col;
                                new_weights.push(mutated.weights[idx]);
                            }
                            // Add value for influence of new gene on this gene (copy from source gene)
                            new_weights.push(mutated.weights[row * n + source]);
                        }

                        // Add the new row (source gene's row copied, including self-interaction)
                        for col in 0..n {
                            let idx = source * n + col;
                            new_weights.push(mutated.weights[idx]);
                        }
                        // Add self-interaction for new gene
                        new_weights.push(mutated.weights[source * n + source]);

                        mutated.weights = new_weights;
                        
                        // Append copies of input_weights, bias, initial
                        mutated.input_weights.push(mutated.input_weights[gene_to_dup]);
                        mutated.bias.push(mutated.bias[gene_to_dup]);
                        mutated.initial.push(mutated.initial[gene_to_dup]);
                        
                        // New gene id = ((parent_id+1)<<16) | dup_index
                        let new_id = ((parent_id + 1) << 16) | (mutated.dup_counter as u32);
                        mutated.gene_ids.push(new_id);
                        
                        // Increment counters
                        mutated.dup_counter += 1;
                        mutated.n_genes += 1;
                    }
                    
                    // Structural validity: verify the spec is still valid
                    assert_eq!(mutated.weights.len(), mutated.n_genes * mutated.n_genes, "V-3-b: weights must be square");
                    assert_eq!(mutated.input_weights.len(), mutated.n_genes, "V-3-b: input_weights length mismatch");
                    assert_eq!(mutated.bias.len(), mutated.n_genes, "V-3-b: bias length mismatch");
                    assert_eq!(mutated.initial.len(), mutated.n_genes, "V-3-b: initial length mismatch");
                    assert_eq!(mutated.gene_ids.len(), mutated.n_genes, "V-3-b: gene_ids length mismatch");
                    assert!(mutated.n_genes >= 2, "V-3-b: n_genes must stay >= 2");
                    
                    g.grn_spec = Some(Arc::new(mutated));
                }
            }
        }

        // V-3-c: variable-length genome indel operator (insert-gene / delete-gene) — CANONICAL ORDER
        // (appended AFTER V-3-b duplication, so duplication's draw positions are unchanged — §5.2
        // stream hygiene). Gate: enable_variable_length && grn_spec.is_some(); flag-off or no-spec
        // draws ZERO values → existing goldens byte-identical (neutrality contract).
        // Mechanism: draw application-count (0-or-1, private salt SALT_INDEL) exactly like V-3-b's
        // SALT_DUP. If it fires, a bit of that SAME draw picks insert vs delete (no extra draw spent
        // on the coin, so the flag-off/no-spec paths still cost zero draws):
        // - Insert (grow n→n+1): append a NOVEL, ZERO-initialized gene at position n (F1 pinned — no
        //   content draw at all, unlike duplication's paralog copy). New id =
        //   `NOVEL_GENE_ID_TAG | dup_counter` — high-bit-tagged, deliberately NOT duplication's
        //   `(parent_id+1)<<16` shift (F2 note: avoids duplication's shallow-chain overflow risk by
        //   construction). `dup_counter += 1` (a gene-ADDING event), `n_genes += 1`.
        // - Delete (shrink n→n-1): draw `gene_to_delete = (r_which >> 8) % n_genes` from a SECOND draw
        //   at `SALT_INDEL + 1` (F3/F6 pinned — fixed position immediately after the application-count
        //   draw). Remove row/col `gene_to_delete` from `weights` (row-major stride preserved) and the
        //   matching entry from `input_weights`/`bias`/`initial`/`gene_ids`. `n_genes -= 1`;
        //   `dup_counter` UNCHANGED (F5 note — the counter tracks gene-ADDING events only, so it
        //   stays a monotonic lineage-history flag even across later shrinkage).
        //   Floor: `n_genes >= 2` (the `grn.rs` classify floor) — a delete drawn at `n_genes == 2` is
        //   a no-op (the second draw is skipped entirely; the genome stays the parent's clone).
        const SALT_INDEL: u64 = 0x494E_4445u64; // "INDE"
        const NOVEL_GENE_ID_TAG: u32 = 0x8000_0000;
        if enable_variable_length {
            // Read from `g.grn_spec`, NOT `self.grn_spec` — `g` already carries V-3-b's result (if
            // duplication fired above), so indel composes onto it instead of silently discarding it.
            if let Some(gspec) = g.grn_spec.clone() {
                let gspec = &gspec;
                let r_count = seed_fold(stream, &[SALT_INDEL]);
                let fires = (r_count & 0xFF) < self.mutation_rate as u64;
                if fires {
                    let is_delete = ((r_count >> 8) & 1) == 1;
                    let n = gspec.n_genes;
                    if is_delete {
                        if n > 2 {
                            let r_which = seed_fold(stream, &[SALT_INDEL + 1]);
                            let gene_to_delete = (r_which >> 8) as usize % n;
                            let mut mutated = (**gspec).clone();

                            // Remove row/col `gene_to_delete` from the row-major n×n matrix,
                            // preserving stride for the remaining (n-1)×(n-1).
                            let mut new_weights = Vec::with_capacity((n - 1) * (n - 1));
                            for row in 0..n {
                                if row == gene_to_delete {
                                    continue;
                                }
                                for col in 0..n {
                                    if col == gene_to_delete {
                                        continue;
                                    }
                                    new_weights.push(mutated.weights[row * n + col]);
                                }
                            }
                            mutated.weights = new_weights;
                            mutated.input_weights.remove(gene_to_delete);
                            mutated.bias.remove(gene_to_delete);
                            mutated.initial.remove(gene_to_delete);
                            mutated.gene_ids.remove(gene_to_delete);
                            mutated.n_genes -= 1;
                            // dup_counter unchanged (F5): delete is not a gene-adding event.

                            assert_eq!(mutated.weights.len(), mutated.n_genes * mutated.n_genes, "V-3-c: weights must be square after delete");
                            assert_eq!(mutated.input_weights.len(), mutated.n_genes, "V-3-c: input_weights length mismatch after delete");
                            assert_eq!(mutated.bias.len(), mutated.n_genes, "V-3-c: bias length mismatch after delete");
                            assert_eq!(mutated.initial.len(), mutated.n_genes, "V-3-c: initial length mismatch after delete");
                            assert_eq!(mutated.gene_ids.len(), mutated.n_genes, "V-3-c: gene_ids length mismatch after delete");
                            assert!(mutated.n_genes >= 2, "V-3-c: n_genes must stay >= 2 after delete");

                            g.grn_spec = Some(Arc::new(mutated));
                        }
                        // n <= 2: floor no-op — leave g.grn_spec as the parent's cloned Arc, untouched.
                    } else {
                        let mut mutated = (**gspec).clone();

                        // Append a zero row + zero column at position n (F1 pinned — no content draw).
                        let mut new_weights = Vec::with_capacity((n + 1) * (n + 1));
                        for row in 0..n {
                            for col in 0..n {
                                new_weights.push(mutated.weights[row * n + col]);
                            }
                            new_weights.push(0); // new gene's influence on existing gene `row`
                        }
                        // New gene's own row: influenced by all genes (including itself) = 0.
                        new_weights.extend(std::iter::repeat_n(0, n + 1));
                        mutated.weights = new_weights;

                        mutated.input_weights.push(0);
                        mutated.bias.push(0);
                        mutated.initial.push(0);

                        // Novel (parentless) gene id — high-bit-tagged, NOT duplication's `(parent_id+1)<<16`
                        // shift (F2): disjoint from both the base range `0..n_genes` and every duplication id.
                        let new_id = NOVEL_GENE_ID_TAG | mutated.dup_counter;
                        mutated.gene_ids.push(new_id);

                        mutated.dup_counter += 1; // gene-ADDING event (F5).
                        mutated.n_genes += 1;

                        assert_eq!(mutated.weights.len(), mutated.n_genes * mutated.n_genes, "V-3-c: weights must be square after insert");
                        assert_eq!(mutated.input_weights.len(), mutated.n_genes, "V-3-c: input_weights length mismatch after insert");
                        assert_eq!(mutated.bias.len(), mutated.n_genes, "V-3-c: bias length mismatch after insert");
                        assert_eq!(mutated.initial.len(), mutated.n_genes, "V-3-c: initial length mismatch after insert");
                        assert_eq!(mutated.gene_ids.len(), mutated.n_genes, "V-3-c: gene_ids length mismatch after insert");
                        assert!(mutated.n_genes >= 2, "V-3-c: n_genes must stay >= 2 after insert");

                        g.grn_spec = Some(Arc::new(mutated));
                    }
                }
            }
        }

        g
    }

    /// Brain-weight L1 genetic distance — the speciation metric (M5/criterion 2).
    /// Protected by the `deny(float_arithmetic)` guard on this file. Integer, arch-independent.
    pub fn brain_weight_l1(&self, other: &Genome) -> i64 {
        self.weights.iter().zip(other.weights.iter())
            .map(|(a, b)| (*a as i64 - *b as i64).abs())
            .sum()
    }

    /// Decode this genome to a `Phenotype` (Phase-2 E-1 seam entry point; E-4a adds the ontogenesis
    /// chain opt-in).
    ///
    /// **E-4a/V-1:** when `self.morphogen_spec` and `self.grn_spec` are BOTH `Some`, runs the full
    /// ontogenesis chain — `morphogen(self, &mspec)` → `grn(&gradient, &gspec)` — and caches the
    /// resolved `CellType` on `Phenotype.cell_type`. The chain is a PURE function of `self` (spec
    /// included): no RNG/clock/thread-dependence (E-2/E-3 determinism holds transitively). **V-1**:
    /// the spec is read from `self`, NOT `econ.morphogen`/`econ.grn` (those fields are consulted
    /// ONLY once, at founder-spawn, to seed `self.grn_spec`/`self.morphogen_spec` via
    /// [`Genome::with_specs`] — `decode` never reads them directly). When either per-individual
    /// spec is absent (all 5 non-phase2 configs — their founders are never seeded with one),
    /// `decode` is the E-1 trivial Ф0 projection with `cell_type: None` — byte-identical to before
    /// V-1/E-4a.
    ///
    /// Returns `Some` for every valid Ф0 genome, and for every genome under the five existing
    /// configs (no spec there, so the viability gate below is unreachable).
    /// **E-5b**: under `phase2_config` (the only config whose founders are seeded with both specs),
    /// an embryo whose `size` does not clear [`SIZE_VIABILITY_FLOOR`] returns `None` — a real,
    /// production-reachable stillbirth. `stage_birth_death` (E-5a) already books the conservation-
    /// correct `None` branch; this slice makes that branch reachable, it adds no new conservation
    /// code.
    ///
    /// `econ` is still consulted for `econ.n_layers` (the `uptake_layer` clamp) — only the
    /// morphogen/GRN spec moved onto `self` (V-1).
    ///
    /// Pure and deterministic: no RNG, no clock, no thread-dependent work.
    /// Phenotype is NOT folded into `hash_contribution` (it is a cold derivative of Genome;
    /// genome IS in the hash, decode is deterministic ⟹ phenotype is fully determined — plan §2/R19).
    pub fn decode(&self, econ: &EconParams) -> Option<Phenotype> {
        // E-1/E-4 test injection: when force_decode_none=true, the gate fires the skip path.
        // In Ф0 production this branch is compiled OUT entirely (#[cfg(test)]).
        #[cfg(test)]
        if self.force_decode_none {
            return None;
        }
        let (cell_type, graph) = match (&self.morphogen_spec, &self.grn_spec) {
            (Some(mspec), Some(gspec)) => {
                let gradient = morphogen(self, mspec);
                let ct = grn(&gradient, gspec);
                // E-5b: the viability gate — scoped to this arm only (see is_viable_size docs).
                if !is_viable_size(self.size) {
                    return None;
                }
                // M7-a: compute the multicellular graph from the morphogen gradient.
                // This is cold-derived and prod-inert (never consumed in M7-a).
                let g = CellGraph::from_gradient(&gradient, gspec);
                (Some(ct), g)
            }
            _ => (None, CellGraph::empty()), // E-1 trivial projection: no non-phase2 founder is ever seeded with both specs
        };
        // E-4b-i: when the chain ran, cell_type DRIVES uptake_layer (the live hot-path consumer —
        // stage_sense and stage_interactions both read Phenotype.uptake_layer, never Genome's raw
        // field, so this single derivation point keeps both stages consistent — critic F3/F11).
        // When cell_type is None (every non-Phase-2 config, always, until a Phase-2 config exists),
        // uptake_layer stays the raw 1:1 genome projection — BYTE-IDENTICAL to E-1/E-4a.
        let uptake_layer = match cell_type {
            Some(ct) => cell_type_uptake_layer(ct, self.uptake_layer, econ.n_layers),
            None => self.uptake_layer,
        };
        Some(Phenotype { uptake_layer, cell_type, graph })
    }

    /// E-5b: `true` iff a `decode(econ)` call on this genome returns `None` because of the REAL
    /// `size`-viability criterion (as opposed to the `#[cfg(test)]` `force_decode_none` injection).
    /// Reuses the exact same predicate `decode` checks — no duplicated conservation code, just the
    /// attribution `stage_birth_death` needs to increment the criterion-triggered stillbirth counter
    /// without conflating it with a test injection (critic requirement: the two must be
    /// distinguishable at the count site). `#[cfg(test)] force_decode_none` fires BEFORE this
    /// condition is ever reached in `decode`, so a genome with both flags set is attributed to the
    /// real criterion here — callers must not mix the two in the same probe run (see
    /// `phase2_viability.rs`'s "clean run" requirement).
    ///
    /// **V-1**: reads `self.morphogen_spec`/`self.grn_spec` (no longer `econ.morphogen`/`econ.grn`
    /// — those are the founder-spawn template only, per `decode`'s doc), so no `econ` param needed.
    pub(crate) fn is_stillbirth_by_size_criterion(&self) -> bool {
        matches!((&self.morphogen_spec, &self.grn_spec), (Some(_), Some(_))) && !is_viable_size(self.size)
    }

    /// Fold all six traits into the per-entity state-hash contribution.
    pub fn hash_contribution(&self, mut h: u64) -> u64 {
        for v in [
            self.metabolism_eff,
            self.move_speed,
            self.sense_range,
            self.size,
            self.repro_threshold,
            self.mutation_rate,
            self.uptake_layer,
            self.excrete_layer,
        ] {
            h = fnv_mix(h, v as u64);
        }
        // Fold the evolved brain weights too (F9 — a new genome field must enter the determinism lock).
        for &w in &self.weights {
            h = fnv_mix(h, w as u64);
        }
        // D′-1 F9 trade-off: fold photo_gain ONLY when non-zero. `fnv_mix(h, 0) = h * FNV_PRIME ≠ h`,
        // so naively folding 0 would shift the checksum for every non-dprime cell. Gating preserves
        // byte-identity for default_config/l3_config/cprime_config (photo_gain always 0 there).
        // A dprime cell that evolves photo_gain > 0 IS locked. A dprime cell staying at 0 is not
        // folded — mild weakening, safe because its other traits ARE folded and mutation is gated.
        if self.photo_gain != 0 {
            h = fnv_mix(h, self.photo_gain as u64);
        }
        // D′-2b (critic F2): fold BOTH reg_setpoint AND reg_gain when reg_gain != 0.
        // Gated on reg_gain ≠ 0 (same pattern as photo_gain) — non-dprime genomes always have
        // reg_gain=0, so their checksums are undisturbed → 4 existing goldens byte-identical.
        // Folding both together catches a regression where only setpoint changes (F2).
        // Accepted mild weakening: two dprime cells both with reg_gain=0 but differing setpoints
        // collide in the hash — acceptable because gain-0 cells are behaviourally identical
        // regardless of setpoint (the gene is inert at gain=0; setpoint only matters when active).
        if self.reg_gain != 0 {
            h = fnv_mix(h, self.reg_setpoint as u64);
            h = fnv_mix(h, self.reg_gain as u64);
        }
        // P-2a: fold combat_trait ONLY when non-zero (same pattern as photo_gain and reg_gain).
        // Non-predation configs: combat_trait always 0 → not folded → checksums undisturbed,
        // existing goldens byte-identical. Predation configs: combat_trait>0 → folded (locked).
        if self.combat_trait != 0 {
            h = fnv_mix(h, self.combat_trait as u64);
        }
        // V-1: fold the heritable GRN/morphogen spec by INTEGER CONTENTS, in a FIXED field order —
        // exactly like the existing i32 traits above, never the `Arc` pointer (CoW sharing is
        // transparent to the hash; two lineages with byte-identical spec CONTENTS hash the same
        // regardless of whether they share one `Arc` or hold independent clones).
        //
        // Some-GATED: a `None` spec contributes NOTHING (`if let Some(...) { fold }`), NOT
        // `fnv_mix(h, hash_of_option)` (which would fold a value even for `None` and shift the
        // checksum for every one of the five non-phase2 configs, whose founders are never seeded
        // with a spec). `n_genes` is fixed at 2 in V-1 (no variable-length operator yet), so every
        // `Vec` length here is constant — no length-encoding ambiguity to worry about.
        if let Some(gspec) = &self.grn_spec {
            for &v in &gspec.weights {
                h = fnv_mix(h, v as u64);
            }
            for &v in &gspec.input_weights {
                h = fnv_mix(h, v as u64);
            }
            for &v in &gspec.bias {
                h = fnv_mix(h, v as u64);
            }
            h = fnv_mix(h, gspec.shift as u64);
            h = fnv_mix(h, gspec.max_steps as u64);
            h = fnv_mix(h, gspec.sample_x as u64);
            h = fnv_mix(h, gspec.sample_z as u64);
            for &v in &gspec.initial {
                h = fnv_mix(h, v as u64);
            }
            h = fnv_mix(h, gspec.n_genes as u64);
            // V-3-a: dup_counter gated (fold only when non-zero). In V-3-a, all genomes have
            // dup_counter=0 (no duplication operator yet), so this folds nothing → existing goldens
            // byte-identical. V-3-b will fold dup_counter>0 to lock mutations that incremented it.
            // gene_ids are recomputable from n_genes for base genes (0..n_genes-1), so not folded
            // (would waste hash budget; the lineage-derived ids are infrastructure for future
            // homology-driven operators, not a separate state to lock).
            if gspec.dup_counter != 0 {
                h = fnv_mix(h, gspec.dup_counter as u64);
            }
        }
        if let Some(mspec) = &self.morphogen_spec {
            h = fnv_mix(h, mspec.g_dev as u64);
            h = fnv_mix(h, mspec.n_dev as u64);
            h = fnv_mix(h, match mspec.boundary { Boundary::Reflecting => 0u64, Boundary::Absorbing => 1u64 });
            h = fnv_mix(h, mspec.diffuse_shift as u64);
            h = fnv_mix(h, mspec.decay_num as u64);
            h = fnv_mix(h, mspec.decay_shift as u64);
            h = fnv_mix(h, mspec.seed_scale as u64);
            h = fnv_mix(h, mspec.stop_threshold as u64);
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isqrt_floor() {
        assert_eq!(isqrt(0), 0);
        assert_eq!(isqrt(15), 3);
        assert_eq!(isqrt(16), 4);
        assert_eq!(isqrt(4096), 64);
    }

    #[test]
    fn size34_monotone() {
        assert!(size_pow_three_quarters(1) <= size_pow_three_quarters(8));
        assert!(size_pow_three_quarters(8) <= size_pow_three_quarters(32));
        assert_eq!(size_pow_three_quarters(16), 8); // sqrt(sqrt(4096)) = sqrt(64) = 8
    }

    #[test]
    fn mutation_is_deterministic_and_clamped() {
        let g = Genome::founder(2);
        assert_eq!(g.mutate(123, 2, false, 4, false, false), g.mutate(123, 2, false, 4, false, false));
        for s in 0..200u64 {
            let m = g.mutate(s, 2, false, 4, false, false);
            assert!((0..=256).contains(&m.metabolism_eff));
            assert!((1..=32).contains(&m.size));
            assert!((0..=1).contains(&m.uptake_layer));
            assert!((0..=1).contains(&m.excrete_layer));
            // Without light, photo_gain and reg gene must stay at founder values.
            assert_eq!(m.photo_gain, 0, "photo_gain must not mutate when has_light=false");
            assert_eq!(m.reg_gain, 0, "reg_gain must not mutate when has_light=false");
        }
        // With light, photo_gain can mutate (starts at 0, may go to 1 or stay 0).
        for s in 0..200u64 {
            let m = g.mutate(s, 2, true, 4, false, false);
            assert!((0..=256).contains(&m.photo_gain), "photo_gain must be in [0,256]");
            assert!((-4..=4).contains(&m.reg_gain), "reg_gain must be in [-reg_gain_max, +reg_gain_max]");
        }
        // reg_gain_max=0 locks regulation OFF even when has_light=true.
        for s in 0..200u64 {
            let m = g.mutate(s, 2, true, 0, false, false);
            assert_eq!(m.reg_gain, 0, "reg_gain must stay 0 when reg_gain_max=0 (D′-2c lock)");
        }
        // L=1 bench path: layers clamped to 0.
        let g1 = Genome::founder(1);
        assert_eq!(g1.excrete_layer, 0);
        let m1 = g1.mutate(0, 1, false, 0, false, false);
        assert_eq!(m1.uptake_layer, 0);
        assert_eq!(m1.excrete_layer, 0);
    }

    // ── E-1: decode-surface seam unit tests (Phase-2 foundation) ─────────────────────────────

    /// Decode is bit-identical across repeated calls on the same genome (determinism gate).
    /// Seeds the §3 determinism contract extended by later slices.
    #[test]
    fn decode_is_deterministic_across_calls() {
        for n_layers in [1usize, 2, 3] {
            let g = Genome::founder(n_layers);
            let a = g.decode(&EconParams::default());
            let b = g.decode(&EconParams::default());
            assert_eq!(a, b, "decode must be deterministic: same genome → same Phenotype");
        }
        // Also holds for a mutated genome.
        let g = Genome::founder(2);
        let mutated = g.mutate(0xDEAD_BEEF, 2, true, 4, false, false);
        assert_eq!(mutated.decode(&EconParams::default()), mutated.decode(&EconParams::default()), "decode deterministic on mutated genome");
    }

    /// Every Ф0 genome decodes to Some — Ф0 viability is unconditional.
    #[test]
    fn decode_some_for_all_phi0_founders() {
        for n_layers in [1usize, 2, 3] {
            let g = Genome::founder(n_layers);
            assert!(g.decode(&EconParams::default()).is_some(), "founder genome must decode to Some (Ф0 trivial case)");
        }
    }

    /// Ф0 decode is a 1:1 projection: phenotype.uptake_layer == genome.uptake_layer.
    /// Proves the consumer's field is bit-exact — no computed quantity or truncation.
    #[test]
    fn phenotype_uptake_layer_matches_genome() {
        let g = Genome::founder(2);
        let ph = g.decode(&EconParams::default()).expect("Ф0 must decode to Some");
        assert_eq!(ph.uptake_layer, g.uptake_layer,
            "Phenotype::uptake_layer must equal Genome::uptake_layer for Ф0");
        // Also for mutated genome — projection stays 1:1 regardless of trait value.
        for s in 0..50u64 {
            let m = g.mutate(s, 2, false, 0, false, false);
            let mph = m.decode(&EconParams::default()).expect("mutated Ф0 must decode to Some");
            assert_eq!(mph.uptake_layer, m.uptake_layer,
                "1:1 projection must hold after mutation (seed={s})");
        }
    }

    /// None-gate wiring test: proves the REAL `Genome::decode()` — the function `stage_birth_death`
    /// calls (`let Some(child_phenotype) = child_genome.decode(&econ) else { continue; }`) —
    /// returns `None` when the E-4 injection flag is set, and `Some` otherwise.
    ///
    /// This is NOT a tautology on `Option::is_some()`: it injects `force_decode_none=true`
    /// into the SAME `decode()` that production calls; the prior `phenotype_gate` wrapper
    /// was a separate function NOT wired to production (critic finding F1). Removed.
    ///
    /// Point (a) — non-materialization: the `let Some(...) else { continue }` in stage_birth_death
    /// fires `continue`, skipping BOTH mineral and non-mineral spawn sites. The integration test
    /// `e1_none_gate_suppresses_births_end_to_end` (`sim-core/src/lib.rs`) proves this end-to-end.
    ///
    /// Point (b) — other newborns deterministic: 5 goldens byte-identical (force_decode_none is
    /// always `false` in Ф0; `#[cfg(test)]` compiles the branch out in release, and even in test
    /// builds the false branch is a no-op that leaves decode() deterministic for all normal genomes).
    #[test]
    fn none_gate_calls_real_decode_and_skips() {
        // Normal genome (force_decode_none=false): decode() returns Some → gate passes.
        let g = Genome::founder(2);
        assert!(!g.force_decode_none, "founder must have force_decode_none=false");
        assert!(g.decode(&EconParams::default()).is_some(), "Ф0 genome must decode to Some (gate passes)");

        // E-4 injection: set force_decode_none=true → THE SAME decode() returns None.
        // This is the identical function stage_birth_death calls on child_genome.
        let mut stillborn = Genome::founder(2);
        stillborn.force_decode_none = true;
        assert!(stillborn.decode(&EconParams::default()).is_none(),
            "force_decode_none=true must make decode() return None (gate fires → spawn skipped)");

        // Mutated children inherit the flag (mutate copies *self) → entire lineage stays stillborn.
        let mutated_child = stillborn.mutate(0xDEAD_CAFE, 2, false, 0, false, false);
        assert!(mutated_child.force_decode_none,
            "force_decode_none must be inherited by mutate() so the entire lineage stays stillborn");
        assert!(mutated_child.decode(&EconParams::default()).is_none(),
            "inherited flag: child decode() also returns None (lineage-level stillbirth)");

        // Normal mutated child (force_decode_none=false) returns Some — mutation alone never triggers None.
        let normal_child = g.mutate(0xDEAD_CAFE, 2, false, 0, false, false);
        assert!(!normal_child.force_decode_none, "normal child must NOT inherit false as true");
        assert!(normal_child.decode(&EconParams::default()).is_some(), "normal child decode() must return Some");
    }

    // ── E-4a: chain-in-decode wiring (INJECTED test config, not a production path) ────────────

    /// Proves the PRODUCTION `decode()` genuinely runs `morphogen → grn` when both specs are
    /// `Some` — via an injected `EconParams` (E-1's inject-and-test pattern), not a dead branch.
    /// Golden-vector on the resolved `cell_type`: catches any regression in the chain wiring
    /// (wrong spec threaded through, chain skipped, wrong gradient/spec paired).
    #[test]
    fn decode_runs_ontogenesis_chain_when_both_specs_present() {
        use crate::{Boundary, GrnSpec, MorphogenSpec};

        let mspec = MorphogenSpec {
            g_dev: 4,
            n_dev: 8,
            boundary: Boundary::Reflecting,
            diffuse_shift: 3,
            decay_num: 1,
            decay_shift: 4,
            seed_scale: 4096,
            stop_threshold: 0,
        };
        let gspec = GrnSpec {
            n_genes: 2,
            weights: vec![64, -64, -64, 64],
            input_weights: vec![0, 0],
            bias: vec![0, 0],
            shift: 3,
            max_steps: 12,
            sample_x: 0,
            sample_z: 0,
            initial: vec![256, 0],
            gene_ids: vec![0, 1],
            dup_counter: 0,
        };
        let econ = EconParams::default();

        // V-1: decode reads the spec from `self` (the genome), not `econ` — attach it here via
        // `with_specs` (production seeds this ONCE at founder-spawn from `EconParams`'s template;
        // this test attaches it directly, mirroring that seam).
        let g = Genome::founder(2).with_specs(Some(Arc::new(gspec.clone())), Some(mspec));
        let ph = g.decode(&econ).expect("Ф0 genome must still decode to Some with the chain enabled");

        // The SAME chain, called directly, must agree exactly (proves decode wires the REAL
        // morphogen()/grn() functions, not a stand-in).
        let gradient = crate::morphogen(&g, &mspec);
        let expected = crate::grn(&gradient, &gspec);
        assert_eq!(ph.cell_type, Some(expected), "decode's cached cell_type must equal the direct chain result");

        // Golden vector (pinned on this implementation — founder genome, the fixtures above):
        // catches a stencil/arithmetic/wiring regression even if the direct-chain comparison above
        // were accidentally also wrong in the same way.
        assert_eq!(ph.cell_type, Some(CellType::A), "pinned chain-in-decode golden");

        // Determinism: repeated decode() calls with the SAME injected config agree.
        let ph2 = g.decode(&econ).expect("must decode to Some again");
        assert_eq!(ph.cell_type, ph2.cell_type, "chain-in-decode must be deterministic across calls");
    }

    /// E-4b-i: `cell_type` DRIVES `uptake_layer` (the live hot-path consumer) when the chain runs —
    /// exact-integer decision, not a float threshold. Genome's `uptake_layer` defaults to 0 for the
    /// founder, so a `CellType::A` result would be indistinguishable from "chain didn't run"; this
    /// test forces `CellType::B` (via the flipped-corner bistable fixture) to prove the derivation
    /// actually OVERRIDES the raw genome value, not just happens to agree with it.
    #[test]
    fn decode_cell_type_drives_uptake_layer() {
        use crate::{Boundary, GrnSpec, MorphogenSpec};

        let mspec = MorphogenSpec {
            g_dev: 4,
            n_dev: 8,
            boundary: Boundary::Reflecting,
            diffuse_shift: 3,
            decay_num: 1,
            decay_shift: 4,
            seed_scale: 4096,
            stop_threshold: 0,
        };
        // Flipped-corner bistable fixture (mirrors grn.rs's, initial swapped) → resolves to B.
        let gspec = GrnSpec::new(2, vec![64, -64, -64, 64], vec![0, 0], vec![0, 0], 3, 12, 0, 0, vec![0, 256]);
        let econ = EconParams { n_layers: 2, ..EconParams::default() };

        // V-1: attach the spec to the genome (decode reads `self`, not `econ` — see above).
        let g = Genome::founder(2).with_specs(Some(Arc::new(gspec.clone())), Some(mspec));
        assert_eq!(g.uptake_layer, 0, "founder's raw genome uptake_layer is 0 (sanity)");
        let ph = g.decode(&econ).expect("Ф0 must decode to Some");

        let gradient = crate::morphogen(&g, &mspec);
        let expected_ct = crate::grn(&gradient, &gspec);
        assert_eq!(ph.cell_type, Some(CellType::B), "fixture must resolve to B (pinned)");
        assert_eq!(ph.cell_type, Some(expected_ct));
        assert_eq!(ph.uptake_layer, 1, "CellType::B must route uptake_layer to 1, overriding genome's raw 0");
    }

    /// When only ONE spec is present, decode() must NOT run the chain (both are required) —
    /// stays the E-1 trivial projection with `cell_type: None`.
    #[test]
    fn decode_stays_trivial_when_only_one_spec_present() {
        use crate::{Boundary, MorphogenSpec};

        let mspec = MorphogenSpec {
            g_dev: 4,
            n_dev: 8,
            boundary: Boundary::Reflecting,
            diffuse_shift: 3,
            decay_num: 1,
            decay_shift: 4,
            seed_scale: 4096,
            stop_threshold: 0,
        };
        // grn_spec stays None — only morphogen_spec attached.
        let g = Genome::founder(2).with_specs(None, Some(mspec));
        let ph = g.decode(&EconParams::default()).expect("Ф0 must decode to Some");
        assert_eq!(ph.cell_type, None, "chain must NOT run when only one of morphogen/grn is Some");
    }

    /// All 5 existing configs carry `morphogen: None, grn: None` — decode's `cell_type` stays
    /// `None`, the E-1 trivial projection. The direct proof the archetype-growth is neutral.
    #[test]
    fn decode_cell_type_is_none_for_default_econ() {
        let g = Genome::founder(2);
        let ph = g.decode(&EconParams::default()).expect("Ф0 must decode to Some");
        assert_eq!(ph.cell_type, None, "default EconParams must never enable the ontogenesis chain");
    }

    // ── E-6: differentiation-verdict capstone (golden-NEUTRAL) ──────────────────────────────────
    //
    // Proves the ASSEMBLED chain (genome.size → morphogen seed → gradient → GRN drive → attractor
    // → classify) differentiates end-to-end: two genomes differing ONLY in `size` decode to
    // DISTINCT `CellType`s, and the fate boundary is crossable by the real `size` mutation
    // operator (a `±1` step). **This verdicts the engine's CAPABILITY — reachable and
    // bit-deterministic — NOT that differentiation emerges or is selectively maintained in the
    // live shipped population.** The shipped `phase2_config` (`cli` crate) stays monomorphic
    // (`input_weights: [0,0]`, `bias: [0,0]` — the morphogen drive has zero effect there, verified
    // structurally distinct below); live emergence / selective maintenance is deferred to the
    // driver work (#37 heritable programs / #42 predation economics) — verdicting it now, on
    // monomorphic founders with no driver, would repeat the E-4b-ii shelved NULL.
    //
    // Mechanism: the morphogen's Dirichlet seed `drive = genome.size · seed_scale` is
    // POSITIVE-only and monotonic in `size` (morphogen.rs) — a sign-flip GRN readout can never
    // differentiate on it. The only topology that flips a fate on a positive-only, monotonic
    // drive is a bistable toggle (`weights=[64,-64,-64,64]`, the same matrix `phase2_config`'s own
    // spec uses) seeded INSIDE one basin (`initial=[0, EXPR_MAX]` = basin B) with the drive
    // biasing the OTHER gene (`input_weights=[8,0]`): below a size-dependent threshold the basin
    // holds; above it the drive overpowers the cross-inhibition and the basin flips to A.
    // PM-verified anchor (`seed_scale=64, input_weights=[8,0]`): clean monotonic B→A step at
    // size 20|21, no `Mixed`, no oscillation, across the whole `[1,32]` range.

    use crate::{Boundary, GrnSpec, MorphogenSpec};

    fn e6_mspec() -> MorphogenSpec {
        MorphogenSpec {
            g_dev: 4, n_dev: 8, boundary: Boundary::Reflecting,
            diffuse_shift: 3, decay_num: 1, decay_shift: 4,
            seed_scale: 64, stop_threshold: 0,
        }
    }

    /// Production-shaped (via the validated `GrnSpec::new`), drive-coupled spec — a TEST fixture
    /// only, structurally distinct from `phase2_config`'s spec (`input_weights != [0,0]`), so it
    /// can never collapse into the shipped monomorphic config.
    fn e6_gspec() -> GrnSpec {
        GrnSpec::new(2, vec![64, -64, -64, 64], vec![8, 0], vec![0, 0], 3, 12, 0, 0, vec![0, crate::GRN_EXPR_MAX])
    }

    /// V-1: decode reads the spec from `self` (the genome), not `econ` — build a genome with the
    /// E-6 fixture spec attached (production seeds this ONCE at founder-spawn from `EconParams`'s
    /// template via `Genome::with_specs`; this helper mirrors that seam directly).
    fn e6_genome(size: i32) -> Genome {
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(e6_gspec())), Some(e6_mspec()));
        g.size = size;
        g
    }

    /// The capstone assertion: two genomes differing ONLY in `size` decode to DISTINCT, exact,
    /// pinned `CellType`s through the full assembled chain — driven by the SIZE difference
    /// propagating through the morphogen seed, not a hand-set GRN initial state (both genomes
    /// decode under the identical spec; only `genome.size` differs between the two calls).
    #[test]
    fn e6_end_to_end_size_only_divergence_crosses_fate_boundary() {
        let econ = EconParams::default();
        let g_lo = e6_genome(20);
        let g_hi = e6_genome(21);

        // The two genomes differ ONLY in `size` (both otherwise the untouched founder + same spec).
        assert_eq!(g_lo.metabolism_eff, g_hi.metabolism_eff);
        assert_eq!(g_lo.weights, g_hi.weights);
        assert_eq!(g_lo.grn_spec, g_hi.grn_spec);
        assert_ne!(g_lo.size, g_hi.size);

        let ph_lo = g_lo.decode(&econ).expect("size=20 must be viable (> SIZE_VIABILITY_FLOOR)");
        let ph_hi = g_hi.decode(&econ).expect("size=21 must be viable (> SIZE_VIABILITY_FLOOR)");

        // Pinned exact fates (not just "differ") — catches a future perturbation of the chain.
        assert_eq!(ph_lo.cell_type, Some(CellType::B), "size=20 must resolve to the pinned fate B");
        assert_eq!(ph_hi.cell_type, Some(CellType::A), "size=21 must resolve to the pinned fate A");
        assert_ne!(ph_lo.cell_type, ph_hi.cell_type, "size-only difference must flip the resolved fate");

        // The divergence flows through the morphogen seed itself.
        let grad_lo = crate::morphogen(&g_lo, &e6_mspec());
        let grad_hi = crate::morphogen(&g_hi, &e6_mspec());
        assert_ne!(
            grad_lo.at(0, 0), grad_hi.at(0, 0),
            "the morphogen seed itself must differ by size (drive = size · seed_scale)"
        );
    }

    /// The reachability proof: sweep `size` across the FULL `[1,32]` mutation range (the real
    /// `mutate()` clamp — `(&mut g.size, 1, 32)` in the trait table above), decode each through
    /// the real chain, and prove (a) ≥2 distinct `CellType`s occur and (b) some adjacent pair
    /// `(s, s+1)` straddles the boundary — the fate flip is reachable by a single real `±1`
    /// mutation step, not just a hand-picked far-apart pair. Sizes `<= SIZE_VIABILITY_FLOOR` (E-5b)
    /// decode to `None` (a stillbirth) under this drive-coupled spec too — expected (E-5b's
    /// criterion lives in the SAME `(Some, Some)` chain arm E-6 exercises) and excluded from the
    /// fate-distinctness count (a stillbirth has no `CellType`), not a failure of this test.
    #[test]
    fn e6_size_sweep_is_mutation_reachable_and_multi_fate() {
        let econ = EconParams::default();
        let mut fates: Vec<(i32, CellType)> = Vec::new();
        for size in 1..=32i32 {
            let g = e6_genome(size);
            if let Some(ph) = g.decode(&econ) {
                let ct = ph.cell_type.expect("(Some, Some) arm must always resolve a CellType when viable");
                fates.push((size, ct));
            }
        }

        assert!(!fates.is_empty(), "sweep must produce at least one viable, decoded genome");
        let first_ct = fates[0].1;
        assert!(
            fates.iter().any(|(_, ct)| *ct != first_ct),
            "the engine must differentiate SOMEWHERE across size∈[1,32] — got only {first_ct:?} for every viable size: {fates:?}"
        );

        let boundary = fates.windows(2).find(|w| w[0].1 != w[1].1);
        assert!(
            boundary.is_some(),
            "no adjacent (size, size+1) pair straddles a fate boundary — the flip would not be \
             reachable by a single real ±1 mutation step; fates={fates:?}"
        );
        let w = boundary.unwrap();
        let (s_lo, ct_lo) = w[0];
        let (s_hi, ct_hi) = w[1];
        assert_eq!(s_hi, s_lo + 1, "the straddling pair must be mutation-adjacent (±1), got {s_lo}|{s_hi}");
        assert_ne!(ct_lo, ct_hi);
        // Pinned to the same PM-verified anchor as the capstone test above (not a coincidence).
        assert_eq!((s_lo, ct_lo, s_hi, ct_hi), (20, CellType::B, 21, CellType::A));
    }

    /// Determinism: repeated decode of the SAME genome (on either side of the boundary) yields
    /// byte-identical `cell_type` and resolved GRN state — no float, no RNG, no clock, no
    /// thread-dependence (E-2/E-3 determinism holds transitively). Arch-stable: `morphogen`/`grn`
    /// are called DIRECTLY here (never through a live `build_sim` field, which would reintroduce
    /// float-noise arch-dependence) — the fate decision is fully integer.
    #[test]
    fn e6_decode_is_bit_deterministic_across_repeated_calls() {
        let econ = EconParams::default();
        for size in [4, 20, 21, 32] {
            let g = e6_genome(size);
            let a = g.decode(&econ);
            let b = g.decode(&econ);
            assert_eq!(a, b, "decode(size={size}) must be byte-identical across repeated calls");

            let (mspec, gspec) = (e6_mspec(), e6_gspec());
            let grad_a = crate::morphogen(&g, &mspec);
            let grad_b = crate::morphogen(&g, &mspec);
            assert_eq!(grad_a, grad_b, "morphogen(size={size}) must be byte-identical across repeated calls");
            let (state_a, ct_a, _) = crate::grn_resolve(&grad_a, &gspec);
            let (state_b, ct_b, _) = crate::grn_resolve(&grad_b, &gspec);
            assert_eq!((state_a, ct_a), (state_b, ct_b), "grn_resolve(size={size}) must be byte-identical across repeated calls");
        }
    }

    /// Golden-neutrality guard (belt-and-braces — the real proof is CI's byte-identical goldens):
    /// this test's fixture spec is structurally distinct from `phase2_config`'s shipped spec, so
    /// it can never be mistaken for, or accidentally collapse into, a production coupling.
    #[test]
    fn e6_fixture_spec_is_structurally_distinct_from_shipped_monomorphic_shape() {
        let gspec = e6_gspec();
        assert_ne!(
            gspec.input_weights, vec![0, 0],
            "E-6's fixture spec must be drive-COUPLED (input_weights != [0,0]) — phase2_config's \
             shipped spec has input_weights=[0,0] (monomorphic, zero morphogen effect); the two \
             must never collapse into the same shape"
        );
    }

    // ── M7-a: multicellular graph-body COLD representation (golden-NEUTRAL) ────────────────────
    //
    // Proves the CellGraph computation (per-grid-cell classification + module labeling) is
    // (a) BYTE-IDENTICAL: the 6 production goldens are unchanged (the existing cell_type/uptake_layer
    //     derivation is untouched — CellGraph is purely additive, never consumed in M7-a);
    // (b) MULTI-MODULE: a drive-coupled genome (input_weights≠[0,0]) produces ≥2 modules;
    // (c) DEAD-DRIVE: the shipped monomorphic genome (input_weights=[0,0]) produces 1 module;
    // (d) DETERMINISTIC: repeated decodes give byte-identical CellGraph, canonical module labeling
    //     is order-independent (min-index representative via union-find).

    /// M7-a: dead-drive fixture (monomorphic, 1 module). Reuses `e6_gspec` matrix but with
    /// input_weights=[0,0] (zero morphogen drive) — the shipped Phase-2 config topology.
    /// Despite the GRN drive being zero, the grid still classifies every cell identically
    /// (all cells see gradient=0, all resolve to `Mixed` or the same attractor), yielding 1 module.
    fn m7a_dead_drive_gspec() -> GrnSpec {
        GrnSpec::new(2, vec![64, -64, -64, 64], vec![0, 0], vec![0, 0], 3, 12, 0, 0, vec![0, crate::GRN_EXPR_MAX])
    }

    /// M7-a: live-drive fixture (drive-coupled, ≥2 modules). Input weights [8, 0] biases the
    /// input toward gene 0, allowing a gradient-driven fate flip across the [1,32] size range.
    fn m7a_live_drive_gspec() -> GrnSpec {
        GrnSpec::new(2, vec![64, -64, -64, 64], vec![8, 0], vec![0, 0], 3, 12, 0, 0, vec![0, crate::GRN_EXPR_MAX])
    }

    #[test]
    fn m7a_dead_drive_produces_one_module() {
        let econ = EconParams::default();
        let mspec = MorphogenSpec {
            g_dev: 4, n_dev: 8, boundary: Boundary::Reflecting,
            diffuse_shift: 3, decay_num: 1, decay_shift: 4,
            seed_scale: 64, stop_threshold: 0,
        };
        let gspec = m7a_dead_drive_gspec();

        // Dead-drive genome (input_weights=[0,0] → zero morphogen effect).
        // Every cell classifies identically → 1 module.
        let g = Genome::founder(2).with_specs(Some(Arc::new(gspec)), Some(mspec));
        let ph = g.decode(&econ).expect("dead-drive genome must decode to Some");

        assert_eq!(
            ph.graph.num_modules(), 1,
            "dead-drive (input_weights=[0,0]) must produce exactly 1 module (uniform classification)"
        );
    }

    #[test]
    fn m7a_live_drive_produces_multiple_modules() {
        let econ = EconParams::default();
        let mspec = MorphogenSpec {
            g_dev: 4, n_dev: 8, boundary: Boundary::Reflecting,
            diffuse_shift: 3, decay_num: 1, decay_shift: 4,
            seed_scale: 64, stop_threshold: 0,
        };
        let gspec = m7a_live_drive_gspec();

        // Live-drive genome (input_weights=[8,0] → morphogen biases gene 0).
        // Use size=21 (above the boundary from E-6) to maximize the gradient effect.
        // The gradient varies spatially → different cells classify to different types → ≥2 modules.
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec)), Some(mspec));
        g.size = 21; // Above the fate boundary (size=20|21 in E-6)
        let ph = g.decode(&econ).expect("live-drive genome must decode to Some");

        assert!(
            ph.graph.num_modules() >= 2,
            "live-drive (input_weights=[8,0]) with size=21 must produce ≥2 modules; got {}",
            ph.graph.num_modules()
        );
    }

    #[test]
    fn m7a_cell_graph_determinism() {
        let econ = EconParams::default();
        let mspec = MorphogenSpec {
            g_dev: 4, n_dev: 8, boundary: Boundary::Reflecting,
            diffuse_shift: 3, decay_num: 1, decay_shift: 4,
            seed_scale: 64, stop_threshold: 0,
        };
        let gspec = m7a_live_drive_gspec();

        let g = Genome::founder(2).with_specs(Some(Arc::new(gspec)), Some(mspec));
        let ph1 = g.decode(&econ).expect("must decode to Some");
        let ph2 = g.decode(&econ).expect("must decode to Some");

        assert_eq!(
            ph1.graph, ph2.graph,
            "repeated decode() calls must produce byte-identical CellGraph (determinism)"
        );
        assert_eq!(
            ph1.graph.module_type, ph2.graph.module_type,
            "module types must be byte-identical"
        );
        assert_eq!(
            ph1.graph.module_cell_count, ph2.graph.module_cell_count,
            "module cell counts must be byte-identical"
        );
    }

    #[test]
    fn m7a_graph_empty_for_non_phase2_config() {
        let econ = EconParams::default();
        let g = Genome::founder(2); // No specs attached → non-phase2
        let ph = g.decode(&econ).expect("Ф0 must decode to Some");

        assert_eq!(
            ph.graph, CellGraph::empty(),
            "non-phase2 genome (no specs) must have empty CellGraph"
        );
        assert_eq!(ph.graph.num_modules(), 0, "empty graph must have zero modules");
    }

    #[test]
    fn v3b_flag_off_no_duplication() {
        // When enable_variable_length=false, duplication operator is inert.
        // The genome should never grow.
        let mut g = Genome::founder(2);
        let econ = EconParams::default(); // enable_variable_length = false (default)
        
        // Mutate many times with a high mutation rate to trigger duplication if possible
        g.mutation_rate = 256; // Max mutation rate
        
        for i in 0..100 {
            let seed = 0x1234_5678 + (i as u64);
            g = g.mutate(seed, econ.n_energy_layers, econ.light.is_some(), econ.reg_gain_max, econ.predation.is_some(), false);
            
            // If grn_spec exists, n_genes should stay at 2
            if let Some(spec) = &g.grn_spec {
                assert_eq!(spec.n_genes, 2, "V-3-b: with flag=false, n_genes must stay constant at 2 (iteration {i})");
            }
        }
    }

    #[test]
    fn v3b_flag_off_preserves_golden_byte_identity() {
        // With enable_variable_length=false, the operator draws zero values from stream.
        // Byte-identical to non-V-3-b genomes. We verify that a known genome
        // mutates deterministically regardless of the flag (since it's gated).
        let g1 = Genome::founder(2);
        let g2 = Genome::founder(2);
        
        let seed = 0xDEAD_BEEF;
        let econ = EconParams::default();
        
        // Mutate both with flag=false
        let m1 = g1.mutate(seed, econ.n_energy_layers, econ.light.is_some(), econ.reg_gain_max, econ.predation.is_some(), false);
        let m2 = g2.mutate(seed, econ.n_energy_layers, econ.light.is_some(), econ.reg_gain_max, econ.predation.is_some(), false);
        
        // They should be byte-identical (determinism check)
        assert_eq!(m1.metabolism_eff, m2.metabolism_eff);
        assert_eq!(m1.mutation_rate, m2.mutation_rate);
        assert_eq!(m1.size, m2.size);
    }

    #[test]
    fn v3b_flag_on_with_no_spec_no_growth() {
        // When enable_variable_length=true but genome has no grn_spec,
        // duplication still doesn't happen (gated by Some check).
        let g = Genome::founder(2); // No spec
        
        // Mutate with flag=true
        let m = g.mutate(0x9999_9999, 2, false, 4, false, true);
        
        // Should still have no spec
        assert!(m.grn_spec.is_none(), "V-3-b: genome without spec should stay without spec");
    }

    #[test]
    fn v3b_flag_on_can_grow() {
        // With enable_variable_length=true and a genome with grn_spec,
        // duplication can happen (though probability depends on mutation_rate).
        
        // Create a genome with grn_spec
        let gspec = GrnSpec::new(
            2,
            vec![64, -64, -64, 64],    // weights: 2x2
            vec![0, 0],                // input_weights
            vec![0, 0],                // bias
            3,                         // shift
            12,                        // max_steps
            0,                         // sample_x
            0,                         // sample_z
            vec![0, 64],               // initial
        );
        
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec)), None);
        g.mutation_rate = 256; // Max rate to increase duplication probability
        
        // Try many seeds to find one that triggers duplication
        let mut found_growth = false;
        for i in 0..1000 {
            let seed = 0xABCD_0000 + (i as u64);
            let m = g.clone().mutate(seed, 2, false, 4, false, true);
            
            if let Some(spec) = &m.grn_spec {
                if spec.n_genes > 2 {
                    found_growth = true;
                    break;
                }
            }
        }
        
        // At least one seed should trigger growth with mutation_rate=256
        assert!(found_growth, "V-3-b: with high mutation_rate, duplication should occur in 1000 seeds");
    }

    #[test]
    fn v3b_duplication_structural_validity() {
        // When duplication occurs, the resulting spec must be structurally valid:
        // - weights: square matrix of size n_genes x n_genes
        // - input_weights, bias, initial: length n_genes
        // - gene_ids: length n_genes
        // - n_genes >= 2
        
        let gspec = GrnSpec::new(
            2,
            vec![64, -64, -64, 64],
            vec![10, 20],
            vec![5, 15],
            3,
            12,
            0,
            0,
            vec![32, 96],
        );
        
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec)), None);
        g.mutation_rate = 256;
        
        // Find a seed that triggers duplication
        for i in 0..1000 {
            let seed = 0xFEED_0000 + (i as u64);
            let m = g.clone().mutate(seed, 2, false, 4, false, true);
            
            if let Some(spec) = &m.grn_spec {
                if spec.n_genes > 2 {
                    // Verify structural validity
                    let n = spec.n_genes;
                    assert_eq!(spec.weights.len(), n * n, "weights must be n_genes^2");
                    assert_eq!(spec.input_weights.len(), n, "input_weights must be n_genes");
                    assert_eq!(spec.bias.len(), n, "bias must be n_genes");
                    assert_eq!(spec.initial.len(), n, "initial must be n_genes");
                    assert_eq!(spec.gene_ids.len(), n, "gene_ids must be n_genes");
                    assert!(spec.n_genes >= 2, "n_genes must be >= 2");
                    return;
                }
            }
        }
        
        panic!("V-3-b: expected to find duplication within 1000 seeds");
    }

    #[test]
    fn v3b_duplication_counter_increments() {
        // After duplication, dup_counter should increment.
        let gspec = GrnSpec::new(
            2,
            vec![64, -64, -64, 64],
            vec![0, 0],
            vec![0, 0],
            3,
            12,
            0,
            0,
            vec![0, 64],
        );
        
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec)), None);
        g.mutation_rate = 256;
        
        for i in 0..1000 {
            let seed = 0xBEEF_0000 + (i as u64);
            let m = g.clone().mutate(seed, 2, false, 4, false, true);
            
            if let Some(spec) = &m.grn_spec {
                if spec.n_genes > 2 {
                    // dup_counter should be > 0 after duplication
                    assert!(spec.dup_counter > 0, "V-3-b: dup_counter must increment after duplication");
                    return;
                }
            }
        }
        
        panic!("V-3-b: expected to find duplication within 1000 seeds");
    }

    #[test]
    fn v3b_duplicated_genome_can_decode() {
        // After duplication, the genome should still be able to decode
        // (if phase2 specs are present).
        let gspec = GrnSpec::new(
            2,
            vec![64, -64, -64, 64],
            vec![0, 0],
            vec![0, 0],
            3,
            12,
            0,
            0,
            vec![0, 64],
        );
        
        let mspec = MorphogenSpec {
            g_dev: 4, n_dev: 8, boundary: Boundary::Reflecting,
            diffuse_shift: 3, decay_num: 1, decay_shift: 4,
            seed_scale: 64, stop_threshold: 0,
        };
        
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec)), Some(mspec));
        g.mutation_rate = 256;
        
        let econ = EconParams {
            morphogen: Some(mspec),
            grn: Some(GrnSpec::new(2, vec![64, -64, -64, 64], vec![0, 0], vec![0, 0], 3, 12, 0, 0, vec![0, 64])),
            ..EconParams::default()
        };
        
        // Find a seed that triggers duplication
        for i in 0..1000 {
            let seed = 0xCAFE_0000 + (i as u64);
            let m = g.clone().mutate(seed, econ.n_energy_layers, econ.light.is_some(), econ.reg_gain_max, econ.predation.is_some(), true);
            
            if let Some(spec) = &m.grn_spec {
                if spec.n_genes > 2 {
                    // Try to decode — should not panic
                    let _ph = m.decode(&econ);
                    // decode may return None (E-5b stillbirth) or Some, but should not panic
                    return;
                }
            }
        }
        
        panic!("V-3-b: expected to find duplication within 1000 seeds");
    }

    #[test]
    fn v3c_flag_off_no_indel() {
        // flag=false → n_genes must never grow/shrink, even with a grn_spec attached and max
        // mutation_rate.
        let gspec = GrnSpec::new(2, vec![64, -64, -64, 64], vec![0, 0], vec![0, 0], 3, 12, 0, 0, vec![0, 64]);
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec)), None);
        g.mutation_rate = 256;

        for i in 0..100 {
            let seed = 0x1111_0000 + (i as u64);
            g = g.mutate(seed, 2, false, 4, false, false);
            if let Some(spec) = &g.grn_spec {
                assert_eq!(spec.n_genes, 2, "V-3-c: flag=false must keep n_genes constant (iteration {i})");
            }
        }
    }

    #[test]
    fn v3c_flag_off_byte_identity() {
        // With enable_variable_length=false, V-3-c (appended AFTER V-3-b) draws zero values —
        // the mutated genome (including its grn_spec) must be fully deterministic/unperturbed,
        // proving indel's presence doesn't move any earlier draw in the fixed order.
        let gspec = GrnSpec::new(2, vec![64, -64, -64, 64], vec![0, 0], vec![0, 0], 3, 12, 0, 0, vec![0, 64]);
        let g1 = Genome::founder(2).with_specs(Some(Arc::new(gspec.clone())), None);
        let g2 = Genome::founder(2).with_specs(Some(Arc::new(gspec)), None);

        let seed = 0xDEAD_BEEF;
        let m1 = g1.mutate(seed, 2, false, 4, false, false);
        let m2 = g2.mutate(seed, 2, false, 4, false, false);

        assert_eq!(m1.metabolism_eff, m2.metabolism_eff);
        assert_eq!(m1.weights, m2.weights);
        assert_eq!(m1.grn_spec, m2.grn_spec, "V-3-c: flag=false output must be byte-identical");
        assert_eq!(m1.grn_spec.unwrap().n_genes, 2, "V-3-c: flag=false must never change n_genes");
    }

    #[test]
    fn v3c_no_spec_no_indel() {
        // flag=true but grn_spec=None → no length change (Some-gated).
        let g = Genome::founder(2); // no spec
        let m = g.mutate(0x2222_2222, 2, false, 4, false, true);
        assert!(m.grn_spec.is_none(), "V-3-c: genome without spec must stay without spec");
    }

    #[test]
    fn v3c_can_insert() {
        // A seed that fires insert grows n_genes by exactly 1; structural invariant holds.
        let gspec = GrnSpec::new(2, vec![64, -64, -64, 64], vec![0, 0], vec![0, 0], 3, 12, 0, 0, vec![0, 64]);
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec)), None);
        // NOT 256 ("always fires"): V-3-b duplication shares this same mutation_rate gate, so a
        // maxed rate makes duplication fire on EVERY call too, chaining with indel and never
        // leaving n_genes at exactly 3 from insert alone. A partial rate gives seeds where
        // duplication skips and indel-insert alone fires.
        g.mutation_rate = 64;

        for i in 0..1000 {
            let seed = 0x3333_0000 + (i as u64);
            let m = g.clone().mutate(seed, 2, false, 4, false, true);
            if let Some(spec) = &m.grn_spec {
                // n_genes==3 alone doesn't prove indel-insert fired — V-3-b duplication also grows
                // n_genes by 1 (with a NON-zero paralog copy). Distinguish by the novel gene's id:
                // indel-insert tags it with the high bit (0x8000_0000), duplication never does.
                if spec.n_genes == 3 && spec.gene_ids[2] & 0x8000_0000 != 0 {
                    // Grew by exactly 1; the appended gene (last row/col) must be all-zero (F1).
                    let n = spec.n_genes;
                    assert_eq!(spec.weights.len(), n * n);
                    assert_eq!(spec.input_weights.len(), n);
                    assert_eq!(spec.bias.len(), n);
                    assert_eq!(spec.initial.len(), n);
                    assert_eq!(spec.gene_ids.len(), n);
                    for col in 0..n {
                        assert_eq!(spec.weights[(n - 1) * n + col], 0, "V-3-c: new gene's row must be zero");
                    }
                    for row in 0..n {
                        assert_eq!(spec.weights[row * n + (n - 1)], 0, "V-3-c: new gene's column must be zero");
                    }
                    assert_eq!(spec.input_weights[n - 1], 0);
                    assert_eq!(spec.bias[n - 1], 0);
                    assert_eq!(spec.initial[n - 1], 0);
                    return;
                }
            }
        }
        panic!("V-3-c: expected to find an insert within 1000 seeds");
    }

    #[test]
    fn v3c_can_delete() {
        // A seed that fires delete shrinks n_genes by exactly 1; structural invariant holds.
        let gspec = GrnSpec::new(
            3,
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9],
            vec![10, 20, 30],
            vec![1, 2, 3],
            3, 12, 0, 0,
            vec![40, 50, 60],
        );
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec)), None);
        // Partial rate — see v3c_can_insert's comment: at 256 duplication would also always fire,
        // so n_genes would never settle at exactly 2 from a lone delete.
        g.mutation_rate = 64;

        for i in 0..1000 {
            let seed = 0x4444_0000 + (i as u64);
            let m = g.clone().mutate(seed, 2, false, 4, false, true);
            if let Some(spec) = &m.grn_spec {
                if spec.n_genes == 2 {
                    let n = spec.n_genes;
                    assert_eq!(spec.weights.len(), n * n);
                    assert_eq!(spec.input_weights.len(), n);
                    assert_eq!(spec.bias.len(), n);
                    assert_eq!(spec.initial.len(), n);
                    assert_eq!(spec.gene_ids.len(), n);
                    return;
                }
            }
        }
        panic!("V-3-c: expected to find a delete within 1000 seeds");
    }

    #[test]
    fn v3c_delete_floor() {
        // At n_genes==2 (the classify floor), a drawn delete is a no-op — n_genes must never
        // drop below 2, across any seed.
        let gspec = GrnSpec::new(2, vec![64, -64, -64, 64], vec![0, 0], vec![0, 0], 3, 12, 0, 0, vec![0, 64]);
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec)), None);
        g.mutation_rate = 256;

        for i in 0..1000 {
            let seed = 0x5555_0000 + (i as u64);
            let m = g.clone().mutate(seed, 2, false, 4, false, true);
            if let Some(spec) = &m.grn_spec {
                assert!(spec.n_genes >= 2, "V-3-c: n_genes must never breach the floor (iteration {i})");
            }
        }
    }

    #[test]
    fn v3c_structural_validity() {
        // Post-indel: all 5 vectors == n_genes, weights square, no panic — across a run of
        // successive generations exercising both insert and delete.
        let gspec = GrnSpec::new(2, vec![64, -64, -64, 64], vec![0, 0], vec![0, 0], 3, 12, 0, 0, vec![0, 64]);
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec)), None);
        g.mutation_rate = 256;

        for i in 0..500 {
            let seed = 0x6666_0000 + (i as u64);
            g = g.mutate(seed, 2, false, 4, false, true);
            if let Some(spec) = &g.grn_spec {
                let n = spec.n_genes;
                assert_eq!(spec.weights.len(), n * n, "iteration {i}");
                assert_eq!(spec.input_weights.len(), n, "iteration {i}");
                assert_eq!(spec.bias.len(), n, "iteration {i}");
                assert_eq!(spec.initial.len(), n, "iteration {i}");
                assert_eq!(spec.gene_ids.len(), n, "iteration {i}");
                assert!(n >= 2, "iteration {i}");
            }
        }
    }

    #[test]
    fn v3c_gene_ids_injective() {
        // After any indel sequence, gene_ids must have no duplicates (homology tracking intact).
        let gspec = GrnSpec::new(2, vec![64, -64, -64, 64], vec![0, 0], vec![0, 0], 3, 12, 0, 0, vec![0, 64]);
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec)), None);
        g.mutation_rate = 256;

        for i in 0..500 {
            let seed = 0x7777_0000 + (i as u64);
            g = g.mutate(seed, 2, false, 4, false, true);
            if let Some(spec) = &g.grn_spec {
                let mut seen = std::collections::HashSet::new();
                for &id in &spec.gene_ids {
                    assert!(seen.insert(id), "V-3-c: duplicate gene_id {id} at iteration {i}");
                }
            }
        }
    }

    #[test]
    fn v3c_post_indel_decodes() {
        // A post-indel genome must still decode() to Some(Phenotype) or a clean None — never panic.
        let gspec = GrnSpec::new(2, vec![64, -64, -64, 64], vec![0, 0], vec![0, 0], 3, 12, 0, 0, vec![0, 64]);
        let mspec = MorphogenSpec {
            g_dev: 4, n_dev: 8, boundary: Boundary::Reflecting,
            diffuse_shift: 3, decay_num: 1, decay_shift: 4,
            seed_scale: 64, stop_threshold: 0,
        };
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec)), Some(mspec));
        g.mutation_rate = 256;

        let econ = EconParams {
            morphogen: Some(mspec),
            grn: Some(GrnSpec::new(2, vec![64, -64, -64, 64], vec![0, 0], vec![0, 0], 3, 12, 0, 0, vec![0, 64])),
            ..EconParams::default()
        };

        for i in 0..1000 {
            let seed = 0x8888_0000 + (i as u64);
            let m = g.clone().mutate(seed, econ.n_energy_layers, econ.light.is_some(), econ.reg_gain_max, econ.predation.is_some(), true);
            if let Some(spec) = &m.grn_spec {
                if spec.n_genes != 2 {
                    let _ph = m.decode(&econ); // must not panic; Some or None both fine
                    return;
                }
            }
        }
        panic!("V-3-c: expected to find an indel within 1000 seeds");
    }

    #[test]
    fn v3c_replay_1_vs_n() {
        // N genomes each mutated with the same seed → identical results (R14 1-vs-N).
        let gspec = GrnSpec::new(2, vec![64, -64, -64, 64], vec![10, 20], vec![5, 15], 3, 12, 0, 0, vec![32, 96]);
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec)), None);
        g.mutation_rate = 256;

        let seed = 0x9999_9999;
        let results: Vec<Genome> = (0..8).map(|_| g.clone().mutate(seed, 2, false, 4, false, true)).collect();

        for m in &results[1..] {
            assert_eq!(m.grn_spec, results[0].grn_spec, "V-3-c: replay must be seed-deterministic, not run-order-dependent");
        }
    }
}
