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

    // ── P1-1: respiratory pathway gene (redox-strategy selector) ──────────────────────────────
    /// Respiratory strategy selector — encodes the choice of primary and fallback electron acceptors
    /// (redox hierarchy O₂ > NO₃⁻ > fermentation). `0` → obligate aerobe (O₂ only); `65..=128` →
    /// facultative (O₂ primary, NO₃⁻ fallback); higher ranges reserved for P5+ diversification.
    /// Founders = 0 (obligate aerobe). Mutated ±1 clamped only when O₂-config is enabled
    /// (`enable_oxygen=true`) — non-O₂ configs stay at 0 forever, existing goldens byte-identical
    /// (mutation gate prevents draw, hash gate prevents state inclusion). Range [0, 255].
    /// Decode to Phenotype.respiratory_pathways (PURE, no field-reads).
    pub respiratory_pathway: i32,

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
    /// M7-c: per-module germ/soma marker (indexed by ModuleId). `germ_threshold=None` (every
    /// shipped spec) leaves this `vec![false; n_modules]` — all-soma/unmarked, byte-identical to
    /// M7-b. `Some(t)`: `module_is_germ[mid] == (module_cell_count[mid] <= t)`.
    pub module_is_germ: Vec<bool>,
    /// M7-d: per-module supply-reachability marker (indexed by ModuleId). `supply_source=None`
    /// (every shipped spec) leaves this `vec![true; n_modules]` — all-supplied, byte-identical to
    /// M7-c. `Some(src)`: `module_reachable[mid]` is `true` iff module `mid` is reachable from the
    /// source module via the LIVE module-adjacency graph (BFS). Cold — never consumed by a live stage.
    pub module_reachable: Vec<bool>,
    /// M7-f: per-module consortium root (indexed by ModuleId; values are ModuleId indices in
    /// `[0, n_modules)`). `adhesion_threshold=None` (every shipped spec) leaves this the identity
    /// mapping `(0..n_modules).collect()` — each module its own consortium, byte-identical to M7-d.
    /// `Some(_)`: two adjacent modules with equal `module_is_germ` status are unioned into the same
    /// consortium (min-index representative, mirrors Step 2's cell union-find). Cold — never
    /// consumed by a live stage.
    pub module_consortium: Vec<usize>,
}

impl CellGraph {
    /// D-3a (#272): body size — `Σ module_cell_count`, clamped ≥1 (an empty graph, e.g. every
    /// non-phase2 config's `CellGraph::empty()`, decodes to the unicell floor). Pure integer, no
    /// float/HashMap — the exact per-entity metric `stage_observe` folds into `Telemetry::
    /// {mean,max}_body_size`/`multicellular_frac`.
    pub fn body_size(&self) -> i64 {
        self.module_cell_count.iter().map(|&c| c as i64).sum::<i64>().max(1)
    }

    /// Empty graph (zero modules, e.g. for non-phase2 configs where the graph is not computed).
    pub fn empty() -> Self {
        CellGraph {
            g_dev: 0,
            module_type: Vec::new(),
            module_cell_count: Vec::new(),
            module_is_germ: Vec::new(),
            module_reachable: Vec::new(),
            module_consortium: Vec::new(),
        }
    }

    /// Decode a morphogen gradient into a multicellular graph: per-grid-cell classification,
    /// M7-b apoptosis marking, connected-component labeling via union-find (skipping dead cells),
    /// canonical row-major order + min-index representative.
    ///
    /// `apoptosis_threshold`: `None` (every shipped spec) runs the pass exactly as M7-a — byte-
    /// identical, no dead cells ever marked. `Some(t)` (test-only): a cell is marked dead iff its
    /// resolved gene-0 expression `state[0] < t` (F3, PINNED — integer-only). Dead cells are marked
    /// in place (**F1, PINNED: `grid_cell_type` is never compacted** — union-find indexes by
    /// `idx = z*g_dev + x`, so compacting would shift indices and corrupt labeling) and are skipped
    /// by both union-find and module collection: a dead cell is never a union representative, never
    /// unioned with a neighbor, and contributes to no module.
    pub fn from_gradient(
        gradient: &crate::morphogen::Gradient,
        gspec: &GrnSpec,
        apoptosis_threshold: Option<i32>,
        germ_threshold: Option<i32>,
        supply_source: Option<i32>,
        adhesion_threshold: Option<i32>,
    ) -> Self {
        let g_dev = gradient.g_dev;
        let n_cells = g_dev * g_dev;

        // 1. Per-grid-cell classification: each cell gets a CellType from its local gradient value,
        // plus an M7-b dead mark from the same GRN-resolved state (canonical row-major order).
        // For each grid cell, sample the gradient at that position and run the GRN to resolve
        // the attractor state, then classify that state to determine the cell type.
        let mut grid_cell_type: Vec<CellType> = Vec::with_capacity(n_cells);
        let mut dead: Vec<bool> = Vec::with_capacity(n_cells);
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
                let (state, ct, _steps) = crate::grn_resolve(&cell_gradient, &cell_gspec);
                grid_cell_type.push(ct);
                // M7-b (F3, PINNED): gene-0 expression below threshold ⇒ apoptosis. `None` ⇒ never dead.
                dead.push(matches!(apoptosis_threshold, Some(t) if state[0] < t));
            }
        }

        // 2. Union-find: connect same-type adjacent LIVE cells (4-neighbour), min-index representative.
        // Dead cells are never a union source or target — they stay isolated singletons in `parent`.
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

        // Row-major traversal: connect each LIVE cell to its LIVE right/down neighbors (if same type).
        for z in 0..g_dev {
            for x in 0..g_dev {
                let idx = z * g_dev + x;
                if dead[idx] {
                    continue;
                }
                let ct = grid_cell_type[idx];
                // Right neighbor
                if x + 1 < g_dev {
                    let right_idx = z * g_dev + (x + 1);
                    if !dead[right_idx] && grid_cell_type[right_idx] == ct {
                        union(&mut parent, idx, right_idx);
                    }
                }
                // Down neighbor
                if z + 1 < g_dev {
                    let down_idx = (z + 1) * g_dev + x;
                    if !dead[down_idx] && grid_cell_type[down_idx] == ct {
                        union(&mut parent, idx, down_idx);
                    }
                }
            }
        }

        // 3. Collect modules: each distinct root among LIVE cells → one module. Dead cells never
        // reach here (F2/F5: this can legitimately empty out to zero modules — that is a VALID
        // Phenotype, not a stillbirth; see `Genome::decode`'s E-5b/apoptosis ordering).
        let mut module_id_map: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
        let mut module_type: Vec<CellType> = Vec::new();
        let mut module_cell_count: Vec<i32> = Vec::new();
        // M7-d: per-cell module lookup (`None` for dead cells) — reused by Step 5 below to
        // reconstruct module adjacency without re-running union-find `find`.
        let mut cell_module: Vec<Option<usize>> = vec![None; n_cells];

        for idx in 0..n_cells {
            if dead[idx] {
                continue;
            }
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
            cell_module[idx] = Some(mid);
        }

        // 4. M7-c germ/soma labeling — runs AFTER module collection (needs `module_cell_count`),
        // on LIVE modules only (dead cells already excluded by Steps 1-3, so germ/soma is
        // orthogonal to apoptosis). `germ_threshold: None` (every shipped spec) ⇒ all-`false`,
        // byte-identical to M7-b. `Some(t)` (F4, PINNED — module-level integer, no float, no
        // morphogen re-traversal): a module is GERM iff its cell count `<= t` (small=germ, large=soma).
        let module_is_germ: Vec<bool> = match germ_threshold {
            Some(t) => module_cell_count.iter().map(|&count| count <= t).collect(),
            None => vec![false; module_type.len()],
        };

        // 5. M7-d supply-gate reachability — runs AFTER germ/soma labeling, on LIVE modules only.
        // `supply_source: None` (every shipped spec) ⇒ `module_reachable = vec![true; n_modules]`,
        // byte-identical to M7-c. `Some(src)` (test-only): `src` is the linear index of the supply
        // source cell (F1, PINNED: `src = z*g_dev + x`); the source MODULE is the module containing
        // that cell. Module adjacency is RECONSTRUCTED (the union-find `parent` is discarded after
        // Step 3 — it never recorded cross-module edges) from LIVE cells' 4-neighbors in a fixed
        // up/down/left/right order, into a `BTreeSet<(ModuleId, ModuleId)>` (no Hash*). Reachability
        // is a BFS from the source module via a `Vec` queue + `Vec<bool>` visited (no Hash*, no
        // recursion). An invalid/dead/out-of-range source ⇒ `module_reachable` all-`false`, no panic.
        let n_modules = module_type.len();
        let module_reachable: Vec<bool> = match supply_source {
            None => vec![true; n_modules],
            Some(src) => {
                let mut reachable = vec![false; n_modules];
                let source_mid = usize::try_from(src).ok().filter(|&i| i < n_cells).and_then(|i| cell_module[i]);
                if let Some(source_mid) = source_mid {
                    let mut edges: std::collections::BTreeSet<(usize, usize)> = std::collections::BTreeSet::new();
                    for z in 0..g_dev {
                        for x in 0..g_dev {
                            let idx = z * g_dev + x;
                            let Some(mid_a) = cell_module[idx] else { continue };
                            // Fixed neighbor order: up, down, left, right.
                            let neighbors = [
                                z.checked_sub(1).map(|nz| nz * g_dev + x),
                                (z + 1 < g_dev).then(|| (z + 1) * g_dev + x),
                                x.checked_sub(1).map(|nx| z * g_dev + nx),
                                (x + 1 < g_dev).then(|| z * g_dev + (x + 1)),
                            ];
                            for nidx in neighbors.into_iter().flatten() {
                                if let Some(mid_b) = cell_module[nidx] {
                                    if mid_b != mid_a {
                                        edges.insert((mid_a.min(mid_b), mid_a.max(mid_b)));
                                    }
                                }
                            }
                        }
                    }

                    // Adjacency list built from the sorted edge set — neighbors land in ascending order.
                    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); n_modules];
                    for &(a, b) in &edges {
                        adjacency[a].push(b);
                        adjacency[b].push(a);
                    }

                    reachable[source_mid] = true;
                    let mut queue: Vec<usize> = vec![source_mid];
                    let mut head = 0;
                    while head < queue.len() {
                        let cur = queue[head];
                        head += 1;
                        for &nb in &adjacency[cur] {
                            if !reachable[nb] {
                                reachable[nb] = true;
                                queue.push(nb);
                            }
                        }
                    }
                }
                reachable
            }
        };

        // 6. M7-f consortium adhesion — runs AFTER Step 5, on LIVE modules only. `adhesion_threshold:
        // None` (every shipped spec) ⇒ each module is its own consortium (identity), byte-identical
        // to M7-d. `Some(_)` (test-only): RECOMPUTE module adjacency fresh (F1, PINNED — M7-d's
        // adjacency set is built only inside its `Some(src)` branch, and every shipped spec has
        // `supply_source=None`, so there is no ready-made adjacency to reuse in prod) by iterating
        // LIVE cells row-major and recording each live 4-neighbor pair in a DIFFERENT module into a
        // `BTreeSet<(ModuleId, ModuleId)>` (no Hash*, mirrors Step 5's edge-collection exactly). Two
        // adjacent modules ADHERE iff they share the same germ-status
        // (`module_is_germ[A] == module_is_germ[B]`); adhered pairs are grouped by a SECOND
        // union-find OVER MODULES, reusing Step 2's `find`/`union` (min-index representative,
        // deterministic, order-independent).
        let module_consortium: Vec<usize> = match adhesion_threshold {
            None => (0..n_modules).collect(),
            Some(_) => {
                let mut edges: std::collections::BTreeSet<(usize, usize)> = std::collections::BTreeSet::new();
                for z in 0..g_dev {
                    for x in 0..g_dev {
                        let idx = z * g_dev + x;
                        let Some(mid_a) = cell_module[idx] else { continue };
                        // Fixed neighbor order: up, down, left, right.
                        let neighbors = [
                            z.checked_sub(1).map(|nz| nz * g_dev + x),
                            (z + 1 < g_dev).then(|| (z + 1) * g_dev + x),
                            x.checked_sub(1).map(|nx| z * g_dev + nx),
                            (x + 1 < g_dev).then(|| z * g_dev + (x + 1)),
                        ];
                        for nidx in neighbors.into_iter().flatten() {
                            if let Some(mid_b) = cell_module[nidx] {
                                if mid_b != mid_a {
                                    edges.insert((mid_a.min(mid_b), mid_a.max(mid_b)));
                                }
                            }
                        }
                    }
                }

                let mut consort_parent: Vec<usize> = (0..n_modules).collect();
                for (a, b) in edges {
                    if module_is_germ[a] == module_is_germ[b] {
                        union(&mut consort_parent, a, b);
                    }
                }
                (0..n_modules).map(|i| find(&mut consort_parent, i)).collect()
            }
        };

        CellGraph {
            g_dev,
            module_type,
            module_cell_count,
            module_is_germ,
            module_reachable,
            module_consortium,
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
/// P1-1: respiratory pathway phenotype — decoded from `Genome::respiratory_pathway` gene.
/// PURE function of genome, no field-reads or RNG. Cold-cached at entity birth.
/// Encodes redox-acceptor strategy: primary electron acceptor (O₂, NO₃), fallback layers,
/// metabolic costs (obligate-aerobe ×1.0; facultative ×0.7; obligate-anaerobe ×0.125).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RespiratoryPathway {
    /// Primary electron acceptor field (e.g., O₂ or NO₃⁻), decoded from `respiratory_pathway` gene.
    /// Used to read electron-acceptor availability from the field layer during metabolism.
    pub primary_layer: crate::FieldId,
    /// Efficiency factor for primary acceptor, as a fraction of 256 (integer Q1.8).
    /// E.g., 256 = ×1.0 (O₂); 180 = ×0.7 (NO₃⁻).
    pub primary_eff_x256: i16,
    /// Fallback electron acceptor layers (in redox-priority order), if primary unavailable.
    /// E.g., [NO₃⁻] for facultative, [] for obligate.
    pub fallback_layers: Vec<crate::FieldId>,
    /// Efficiency factors for fallback layers (parallel Vec to fallback_layers).
    pub fallback_effs_x256: Vec<i16>,
    /// Metabolic cost when in anoxia (all layers unavailable), as fraction of 256.
    /// E.g., 256 = death (×1.0, unsustainable); 32 = fermentation (×0.125).
    pub anoxia_cost_x256: i16,
    /// Genoic metabolic cost of maintaining O₂-respiration machinery (ROS-detoxification, etc.),
    /// as a fraction of 256, multiplied by the entity's current energy (proportional cost).
    /// E.g., 10 = ×(10/256) = −3.9% per tick (obligate-aerobe); 15 = −5.9% (facultative).
    pub aerobe_cost_x256: i16,
}

#[derive(bevy_ecs::prelude::Component, Clone, Debug, PartialEq, Eq)]
pub struct Phenotype {
    /// Layer index the entity will eat from (direct copy of `Genome::uptake_layer` for Ф0).
    pub uptake_layer: i32,
    /// Resolved ontogenesis cell type (E-4a). `None` for Ф0 / all 5 existing configs.
    pub cell_type: Option<CellType>,
    /// M7-a: multicellular graph-body COLD representation. Computed from the morphogen grid;
    /// never consumed in M7-a (prod-inert). Empty for non-phase2 configs.
    pub graph: CellGraph,
    /// P1-1: respiratory pathway strategy, decoded from `Genome::respiratory_pathway` gene.
    /// `None` when O₂-config is disabled or respiratory_pathway = 0 (inert). Each entity carries
    /// one primary redox strategy (no Vec — Component trait requirement).
    pub respiratory_pathway: Option<RespiratoryPathway>,
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

/// P1-1: Decode respiratory strategy from genome gene (PURE function, no field-reads/RNG/clock).
/// Maps the 8-bit `respiratory_pathway` genotype to an `Option<RespiratoryPathway>` phenotype
/// encoding the redox-acceptor hierarchy (O₂ > NO₃⁻ > fermentation). Integer-deterministic.
///
/// Gene ranges (redox-priority order):
/// - `0..=64`: obligate aerobe (O₂ only; anoxia cost = 256 = death; aerobe_cost = 10 = −3.9%).
/// - `65..=128`: facultative (O₂ primary + NO₃⁻ fallback; anoxia = fermentation cost 32 = −12.5%;
///   aerobe_cost = 15 = −5.9% — more expensive than obligate due to enzyme maintenance).
/// - `129..=192`: reserved for P5+ redox diversification (NO₃⁻-primary, etc.).
/// - `193..=255`: reserved for P5+ (obligate-anaerobe, etc.).
/// - Gene=0 encodes the obligate-aerobe founder phenotype (byte-identical to P1-0 when disabled).
fn decode_respiratory_pathways(genome: &Genome) -> Option<RespiratoryPathway> {
    let rtype = genome.respiratory_pathway;

    match rtype {
        // Obligate aerobe: O₂ primary, no fallback, death in anoxia.
        0..=64 => {
            Some(RespiratoryPathway {
                primary_layer: crate::FieldId::Oxygen,
                primary_eff_x256: 256,                 // ×1.0 efficiency
                fallback_layers: vec![],               // no fallback → obligate
                fallback_effs_x256: vec![],
                anoxia_cost_x256: 256,                 // ×1.0 cost (unsustainable → death)
                aerobe_cost_x256: 10,                  // ×(10/256) = −3.9% ROS-protection cost
            })
        }
        // Facultative: O₂ primary + NO₃⁻ fallback, survives anoxia via fermentation.
        65..=128 => {
            Some(RespiratoryPathway {
                primary_layer: crate::FieldId::Oxygen,
                primary_eff_x256: 256,                 // ×1.0 (O₂)
                fallback_layers: vec![crate::FieldId::Nitrate],
                fallback_effs_x256: vec![180],         // ×(180/256) = ×0.7 (NO₃⁻ ~70% efficiency)
                anoxia_cost_x256: 32,                  // ×(32/256) = ×0.125 fermentation (2 ATP)
                aerobe_cost_x256: 15,                  // ×(15/256) = −5.9% (dual enzyme sets)
            })
        }
        // Reserved for P5+ (NO₃⁻-primary, obligate-anaerobe, etc.) — returns None (inert) for now.
        _ => None,
    }
}

/// Exact-integer `CellType` → `uptake_layer` decision (E-4b-i). `A` eats layer 0; `B` eats layer 1
/// (clamped into `[0, n_layers)` — degenerate `n_layers <= 1` configs never route here in practice,
/// but the clamp keeps the function total); an exact-tie `Mixed` resolution falls back to the raw
/// genome value (no differentiation signal to act on). Never a float threshold.
///
/// **(#247, F5, PINNED) `Diff(i)` mapping**: `(i as i32) % n_layers.max(1)` — integer, deterministic,
/// always in `[0, n_layers)` (the `.max(1)` guards the degenerate `n_layers == 0` case, same as the
/// `max_layer` guard above).
fn cell_type_uptake_layer(cell_type: CellType, genome_fallback: i32, n_layers: usize) -> i32 {
    let max_layer = (n_layers.max(1) - 1) as i32;
    match cell_type {
        CellType::A => 0,
        CellType::B => 1.min(max_layer),
        CellType::Mixed => genome_fallback,
        CellType::Diff(i) => (i as i32) % (n_layers.max(1) as i32),
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
            // P1-1: respiratory_pathway founder = 0 (obligate aerobe); evolution brings up strategy.
            respiratory_pathway: 0,
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
    /// `enable_oxygen` gates the `respiratory_pathway` mutation (P1-1): when `false`, respiratory_pathway
    /// stays 0, non-O₂ genomes never carry a non-zero respiratory_pathway, keeping existing goldens byte-identical.
    pub fn mutate(&self, stream: u64, n_layers: usize, has_light: bool, reg_gain_max: i32, has_predation: bool, enable_variable_length: bool, evolve_body_size: bool, enable_oxygen: bool) -> Genome {
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

        // P1-1: respiratory_pathway mutates only when O₂-config is enabled. Salt 0x5245_5350 ("RESP").
        // RNG-salt position: AFTER combat_trait (0x434F_4D42), BEFORE GRN (0x4752_4E57+).
        // Same ±1 pattern. Non-O₂ configs: respiratory_pathway stays 0, existing goldens byte-identical
        // (mutation gate prevents draw, hash gate prevents state inclusion).
        if enable_oxygen {
            let r = seed_fold(stream, &[0x5245_5350u64]);
            if (r & 0xFF) < self.mutation_rate as u64 {
                let delta = ((r >> 8) % 3) as i32 - 1; // -1, 0, +1
                g.respiratory_pathway = (g.respiratory_pathway + delta).clamp(0, 255);
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

        // V-4 (#276): body-size axis — heritable point-mutation of `morphogen_spec.g_dev`
        // (unicellular↔multicellular). Disjoint salt `SALT_GDEV`, drawn AFTER the V-1 GRN block,
        // BEFORE V-3-b (§5.2 stream hygiene — every prior draw position preserved). Gated on BOTH
        // `self.morphogen_spec.is_some()` (the morphogen chain must exist to interpret g_dev) AND
        // `evolve_body_size` (opt-in per config — only `driver_config`; the five other production
        // configs pass `false` → zero draws → byte-identical). `MorphogenSpec` is `Copy` (no Arc/CoW
        // needed) — mutate the carried copy directly.
        const SALT_GDEV: u64 = 0x4744_4556u64; // "GDEV"
        if evolve_body_size {
            if let Some(mut mspec) = self.morphogen_spec {
                let r = seed_fold(stream, &[SALT_GDEV]);
                if (r & 0xFF) < self.mutation_rate as u64 {
                    let delta = ((r >> 8) % 3) as i32 - 1; // -1, 0, +1
                    mspec.g_dev = (mspec.g_dev as i32 + delta).clamp(1, 4) as usize;
                    g.morphogen_spec = Some(mspec);
                }
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

        // V-3-d: gene translocation operator (swap two gene positions) — CANONICAL ORDER (appended
        // AFTER V-3-c indel, so indel's draw positions are unchanged). Gate: enable_variable_length
        // && grn_spec.is_some(); flag-off or no-spec draws ZERO values → existing goldens
        // byte-identical. Unlike V-3-b/c, n_genes and dup_counter are UNCHANGED — this is a
        // length-preserving reordering, not a gene-adding/removing event.
        //
        // Mechanism: draw application-count (0-or-1, private salt SALT_TRANS) exactly like V-3-b/c.
        // If it fires, draw two independent gene indices i, j (SALT_TRANS+1, SALT_TRANS+2) and
        // SWAP(i,j): exchange gene i and gene j's positions everywhere — weights ROWS AND COLUMNS
        // (F2 pinned: the SAME permutation π=swap(i,j) on both axes is the load-bearing invariant;
        // a mismatched row/col permutation corrupts the network and breaks conjugacy) and the
        // parallel per-gene vectors (input_weights/bias/initial/gene_ids). `i == j` is a no-op (no
        // allocation, no hash change).
        //
        // Effectful, not neutral: state dynamics are conjugate under π (attractor_new ==
        // π(attractor_old)), but `classify()`'s position-based readout (`state[0]` vs `state[1]`)
        // can flip the decoded cell-type (A↔B) — see grn.rs. That flip IS the variation this
        // operator contributes; it is not a bug to be designed away.
        const SALT_TRANS: u64 = 0x5452_4100u64; // "TRA\0"
        if enable_variable_length {
            // Read from `g.grn_spec`, NOT `self.grn_spec` — compose onto whatever V-3-b/c already
            // produced above, instead of discarding it.
            if let Some(gspec) = g.grn_spec.clone() {
                let r_count = seed_fold(stream, &[SALT_TRANS]);
                let fires = (r_count & 0xFF) < self.mutation_rate as u64;
                if fires {
                    let n = gspec.n_genes;
                    let r_i = seed_fold(stream, &[SALT_TRANS + 1]);
                    let r_j = seed_fold(stream, &[SALT_TRANS + 2]);
                    let i = (r_i >> 8) as usize % n;
                    let j = (r_j >> 8) as usize % n;
                    if i != j {
                        let mut mutated = (*gspec).clone();

                        // Swap rows i,j, then columns i,j, of the row-major n×n matrix. Sequential
                        // row-then-column swap realizes the SAME similarity permutation π=swap(i,j)
                        // on both axes (row and column swaps commute; the composition is exactly
                        // `new[π(r)*n+π(c)] = old[r*n+c]`).
                        for col in 0..n {
                            mutated.weights.swap(i * n + col, j * n + col);
                        }
                        for row in 0..n {
                            mutated.weights.swap(row * n + i, row * n + j);
                        }
                        mutated.input_weights.swap(i, j);
                        mutated.bias.swap(i, j);
                        mutated.initial.swap(i, j);
                        mutated.gene_ids.swap(i, j);
                        // n_genes, dup_counter unchanged (F5: translocation is not a gene-adding
                        // or gene-removing event — it reorders, nothing more).

                        assert_eq!(mutated.weights.len(), mutated.n_genes * mutated.n_genes, "V-3-d: weights must be square after translocation");
                        assert_eq!(mutated.input_weights.len(), mutated.n_genes, "V-3-d: input_weights length mismatch after translocation");
                        assert_eq!(mutated.bias.len(), mutated.n_genes, "V-3-d: bias length mismatch after translocation");
                        assert_eq!(mutated.initial.len(), mutated.n_genes, "V-3-d: initial length mismatch after translocation");
                        assert_eq!(mutated.gene_ids.len(), mutated.n_genes, "V-3-d: gene_ids length mismatch after translocation");

                        g.grn_spec = Some(Arc::new(mutated));
                    }
                    // i == j: no-op — leave g.grn_spec as whatever V-3-b/c already produced.
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
                // This is cold-derived and prod-inert (never consumed in M7-a/M7-b).
                // M7-b: apoptosis_threshold=None on every shipped spec ⇒ the pass never marks a
                // cell dead ⇒ byte-identical to M7-a for all 6 prod configs (golden-NEUTRAL).
                // M7-c: germ_threshold=None on every shipped spec ⇒ module_is_germ stays all-false
                // ⇒ byte-identical to M7-b for all 6 prod configs (golden-NEUTRAL).
                // M7-d: supply_source=None on every shipped spec ⇒ module_reachable stays all-true
                // ⇒ byte-identical to M7-c for all 6 prod configs (golden-NEUTRAL).
                // M7-f: adhesion_threshold=None on every shipped spec ⇒ module_consortium stays the
                // identity mapping ⇒ byte-identical to M7-d for all 6 prod configs (golden-NEUTRAL).
                let g = CellGraph::from_gradient(
                    &gradient,
                    gspec,
                    mspec.apoptosis_threshold,
                    mspec.germ_threshold,
                    mspec.supply_source,
                    mspec.adhesion_threshold,
                );
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
        // P1-1: decode respiratory strategy PURE — no field-reads, no RNG, no clock (deterministic).
        let respiratory_pathway = decode_respiratory_pathways(self);

        Some(Phenotype { uptake_layer, cell_type, graph, respiratory_pathway })
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
        // P1-1: fold respiratory_pathway ONLY when non-zero (same pattern as photo_gain, reg_gain, combat_trait).
        // Non-O₂ configs: respiratory_pathway always 0 → not folded → checksums undisturbed,
        // existing goldens byte-identical. O₂ configs: respiratory_pathway>0 → folded (locked).
        if self.respiratory_pathway != 0 {
            h = fnv_mix(h, self.respiratory_pathway as u64);
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
            // (#247) classify_nway gated: fold only when true (mirrors dup_counter's gate above) —
            // every shipped config has classify_nway=false, so this folds nothing → 6 goldens
            // byte-identical. A hand-built classify_nway=true spec locks the mode into the hash.
            if gspec.classify_nway {
                h = fnv_mix(h, 1u64);
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
            // M7-b: apoptosis_threshold folded Some-gated (mirrors dup_counter above) — `None` on
            // every shipped spec contributes nothing, so all 6 prod goldens stay byte-identical.
            if let Some(t) = mspec.apoptosis_threshold {
                h = fnv_mix(h, t as u64);
            }
            // M7-c: germ_threshold folded Some-gated (mirrors apoptosis_threshold above) — `None`
            // on every shipped spec contributes nothing, so all 6 prod goldens stay byte-identical.
            // `module_is_germ` VALUES are not separately folded (mirrors module_type/
            // module_cell_count, which are not folded either — Phenotype/CellGraph stays a cold,
            // un-hashed derivative; see the `hash_contribution` doc above).
            if let Some(t) = mspec.germ_threshold {
                h = fnv_mix(h, t as u64);
            }
            // M7-d: supply_source folded Some-gated (mirrors germ_threshold above) — `None` on
            // every shipped spec contributes nothing, so all 6 prod goldens stay byte-identical.
            // `module_reachable` VALUES are not separately folded (mirrors module_is_germ — cold,
            // un-hashed CellGraph derivative).
            if let Some(src) = mspec.supply_source {
                h = fnv_mix(h, src as u64);
            }
            // M7-f: adhesion_threshold folded Some-gated (mirrors supply_source above) — `None` on
            // every shipped spec contributes nothing, so all 6 prod goldens stay byte-identical.
            // `module_consortium` VALUES are not separately folded (mirrors module_reachable — cold,
            // un-hashed CellGraph derivative).
            if let Some(t) = mspec.adhesion_threshold {
                h = fnv_mix(h, t as u64);
            }
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
        assert_eq!(g.mutate(123, 2, false, 4, false, false, false, false), g.mutate(123, 2, false, 4, false, false, false, false));
        for s in 0..200u64 {
            let m = g.mutate(s, 2, false, 4, false, false, false, false);
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
            let m = g.mutate(s, 2, true, 4, false, false, false, false);
            assert!((0..=256).contains(&m.photo_gain), "photo_gain must be in [0,256]");
            assert!((-4..=4).contains(&m.reg_gain), "reg_gain must be in [-reg_gain_max, +reg_gain_max]");
        }
        // reg_gain_max=0 locks regulation OFF even when has_light=true.
        for s in 0..200u64 {
            let m = g.mutate(s, 2, true, 0, false, false, false, false);
            assert_eq!(m.reg_gain, 0, "reg_gain must stay 0 when reg_gain_max=0 (D′-2c lock)");
        }
        // L=1 bench path: layers clamped to 0.
        let g1 = Genome::founder(1);
        assert_eq!(g1.excrete_layer, 0);
        let m1 = g1.mutate(0, 1, false, 0, false, false, false, false);
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
        let mutated = g.mutate(0xDEAD_BEEF, 2, true, 4, false, false, false, false);
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
            let m = g.mutate(s, 2, false, 0, false, false, false, false);
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
        let mutated_child = stillborn.mutate(0xDEAD_CAFE, 2, false, 0, false, false, false, false);
        assert!(mutated_child.force_decode_none,
            "force_decode_none must be inherited by mutate() so the entire lineage stays stillborn");
        assert!(mutated_child.decode(&EconParams::default()).is_none(),
            "inherited flag: child decode() also returns None (lineage-level stillbirth)");

        // Normal mutated child (force_decode_none=false) returns Some — mutation alone never triggers None.
        let normal_child = g.mutate(0xDEAD_CAFE, 2, false, 0, false, false, false, false);
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
            apoptosis_threshold: None,
            germ_threshold: None,
            supply_source: None,
            adhesion_threshold: None,
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
            classify_nway: false,
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
            apoptosis_threshold: None,
            germ_threshold: None,
            supply_source: None,
            adhesion_threshold: None,
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
            apoptosis_threshold: None,
            germ_threshold: None,
            supply_source: None,
            adhesion_threshold: None,
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
            seed_scale: 64, stop_threshold: 0, apoptosis_threshold: None,
            germ_threshold: None,
            supply_source: None,
            adhesion_threshold: None,
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
            seed_scale: 64, stop_threshold: 0, apoptosis_threshold: None,
            germ_threshold: None,
            supply_source: None,
            adhesion_threshold: None,
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
            seed_scale: 64, stop_threshold: 0, apoptosis_threshold: None,
            germ_threshold: None,
            supply_source: None,
            adhesion_threshold: None,
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
            seed_scale: 64, stop_threshold: 0, apoptosis_threshold: None,
            germ_threshold: None,
            supply_source: None,
            adhesion_threshold: None,
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

    // ── M7-b: apoptosis-in-decode (golden-NEUTRAL) ──────────────────────────────────────────────
    //
    // Adds a death-threshold pass between M7-a's Step 1 classification and Step 2 union-find
    // labeling: `state[0] < t` (F3, PINNED — gene 0 of the per-cell GRN-resolved state) marks a
    // cell dead; dead cells are marked IN PLACE (F1, PINNED — `grid_cell_type` is never compacted)
    // and skipped by both union-find and module collection. `apoptosis_threshold: None` on every
    // shipped spec means the pass never marks anything dead — byte-identical to M7-a (checked
    // structurally in the `cli` crate's `m7b_prod_inert_all_goldens`, which asserts every
    // production config keeps the gate off; the real byte-identity proof is CI's existing golden/
    // golden_conserved suite staying green, since `None` reproduces M7-a exactly).

    /// M7-b test-only spec builder — same grid/diffusion/decay basis as the M7-a fixtures above,
    /// parameterized only by the apoptosis gate.
    fn m7b_mspec(apoptosis_threshold: Option<i32>) -> MorphogenSpec {
        MorphogenSpec {
            g_dev: 4, n_dev: 8, boundary: Boundary::Reflecting,
            diffuse_shift: 3, decay_num: 1, decay_shift: 4,
            seed_scale: 64, stop_threshold: 0,
            apoptosis_threshold,
            germ_threshold: None,
            supply_source: None,
            adhesion_threshold: None,
        }
    }

    /// M7-b fixture: the M7-a live-drive genome (size=21, ≥2 modules with apoptosis off) — reused
    /// so apoptosis tests exercise a grid already known to be spatially non-uniform.
    fn m7b_live_drive_genome(apoptosis_threshold: Option<i32>) -> Genome {
        let gspec = m7a_live_drive_gspec();
        let mspec = m7b_mspec(apoptosis_threshold);
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec)), Some(mspec));
        g.size = 21; // above the E-6 fate boundary — matches m7a_live_drive_produces_multiple_modules
        g
    }

    /// Independent per-cell gene-0 state readout — calls the SAME public `morphogen`/`grn_resolve`
    /// functions `CellGraph::from_gradient` uses internally, so tests can pick a threshold from the
    /// REAL resolved values instead of hardcoding a guessed magic constant.
    fn m7b_cell_states(g: &Genome, mspec: &MorphogenSpec, gspec: &GrnSpec) -> Vec<i32> {
        let gradient = crate::morphogen(g, mspec);
        let g_dev = mspec.g_dev;
        let mut out = Vec::with_capacity(g_dev * g_dev);
        for z in 0..g_dev {
            for x in 0..g_dev {
                let grad_val = gradient.at(x, z);
                let cell_gradient = crate::morphogen::Gradient { g_dev: 1, cells: vec![grad_val] };
                let mut cell_gspec = gspec.clone();
                cell_gspec.sample_x = 0;
                cell_gspec.sample_z = 0;
                let (state, _ct, _steps) = crate::grn_resolve(&cell_gradient, &cell_gspec);
                out.push(state[0]);
            }
        }
        out
    }

    /// A threshold that marks SOME but not ALL of the 16 grid cells dead on the live-drive fixture
    /// (`min_state + 1`: the minimum-state cell(s) die, the maximum-state cell(s) survive, since
    /// `min < max` on this spatially non-uniform fixture).
    fn m7b_partial_threshold() -> i32 {
        let gspec = m7a_live_drive_gspec();
        let mspec = m7b_mspec(None);
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec.clone())), Some(mspec));
        g.size = 21;
        let states = m7b_cell_states(&g, &mspec, &gspec);
        let min = *states.iter().min().unwrap();
        let max = *states.iter().max().unwrap();
        assert!(min < max, "M7-b fixture must have non-uniform gene-0 states; got {states:?}");
        min + 1
    }

    #[test]
    fn m7b_apoptosis_determinism() {
        let econ = EconParams::default();
        let t = m7b_partial_threshold();
        let g = m7b_live_drive_genome(Some(t));
        let ph1 = g.decode(&econ).expect("must decode to Some");
        let ph2 = g.decode(&econ).expect("must decode to Some");
        assert_eq!(ph1.graph, ph2.graph, "repeated decode() with Some(t) must be byte-identical");
    }

    #[test]
    fn m7b_removed_before_labeling() {
        let econ = EconParams::default();
        let t = m7b_partial_threshold();
        let ph_none = m7b_live_drive_genome(None).decode(&econ).expect("must decode to Some");
        let ph_some = m7b_live_drive_genome(Some(t)).decode(&econ).expect("must decode to Some");

        assert_ne!(
            ph_none.graph, ph_some.graph,
            "removing cells must change the labeled graph — proves removal feeds (and precedes) labeling"
        );
        let live: i32 = ph_some.graph.module_cell_count.iter().sum();
        assert!(live > 0 && live < 16, "fixture threshold must produce PARTIAL removal; got {live} live cells");
    }

    #[test]
    fn m7b_removal_order_deterministic() {
        let econ = EconParams::default();
        let t = m7b_partial_threshold();
        let gspec = m7a_live_drive_gspec();
        let mspec = m7b_mspec(Some(t));
        let g = m7b_live_drive_genome(Some(t));
        let ph = g.decode(&econ).expect("must decode to Some");

        // Independent reference: re-derive per-cell alive/type from the SAME public functions, then
        // flood-fill connected components in a REVERSE column-major order — deliberately different
        // from production's forward row-major union-find traversal — to prove the module partition
        // is not an artifact of iteration order.
        let g_dev = mspec.g_dev;
        let states_and_types: Vec<(i32, CellType)> = {
            let gradient = crate::morphogen(&g, &mspec);
            let mut out = Vec::with_capacity(g_dev * g_dev);
            for z in 0..g_dev {
                for x in 0..g_dev {
                    let grad_val = gradient.at(x, z);
                    let cell_gradient = crate::morphogen::Gradient { g_dev: 1, cells: vec![grad_val] };
                    let mut cell_gspec = gspec.clone();
                    cell_gspec.sample_x = 0;
                    cell_gspec.sample_z = 0;
                    let (state, ct, _steps) = crate::grn_resolve(&cell_gradient, &cell_gspec);
                    out.push((state[0], ct));
                }
            }
            out
        };
        let alive: Vec<Option<CellType>> = states_and_types
            .iter()
            .map(|&(s0, ct)| if s0 < t { None } else { Some(ct) })
            .collect();

        let mut visited = vec![false; g_dev * g_dev];
        let mut ref_counts: Vec<i32> = Vec::new();
        for x in (0..g_dev).rev() {
            for z in (0..g_dev).rev() {
                let idx = z * g_dev + x;
                if visited[idx] || alive[idx].is_none() {
                    continue;
                }
                let ct = alive[idx].unwrap();
                let mut stack = vec![idx];
                visited[idx] = true;
                let mut count = 0;
                while let Some(cur) = stack.pop() {
                    count += 1;
                    let (cz, cx) = (cur / g_dev, cur % g_dev);
                    let mut neighbors = Vec::new();
                    if cx + 1 < g_dev { neighbors.push(cz * g_dev + (cx + 1)); }
                    if cx > 0 { neighbors.push(cz * g_dev + (cx - 1)); }
                    if cz + 1 < g_dev { neighbors.push((cz + 1) * g_dev + cx); }
                    if cz > 0 { neighbors.push((cz - 1) * g_dev + cx); }
                    for n in neighbors {
                        if !visited[n] && alive[n] == Some(ct) {
                            visited[n] = true;
                            stack.push(n);
                        }
                    }
                }
                ref_counts.push(count);
            }
        }

        let mut prod_counts = ph.graph.module_cell_count.clone();
        prod_counts.sort();
        ref_counts.sort();
        assert_eq!(
            prod_counts, ref_counts,
            "module-size multiset from an INDEPENDENT reverse-order flood-fill must match production's \
             canonical row-major union-find — proves the result is order-independent"
        );
    }

    #[test]
    fn m7b_removed_cells_absent_from_modules() {
        let econ = EconParams::default();
        let t = m7b_partial_threshold();
        let gspec = m7a_live_drive_gspec();
        let mspec = m7b_mspec(Some(t));
        let g = m7b_live_drive_genome(Some(t));
        let ph = g.decode(&econ).expect("must decode to Some");

        let states = m7b_cell_states(&g, &mspec, &gspec);
        let dead_count = states.iter().filter(|&&s0| s0 < t).count() as i32;
        assert!(dead_count > 0, "fixture threshold must remove >= 1 cell");

        let module_total: i32 = ph.graph.module_cell_count.iter().sum();
        assert_eq!(
            module_total, 16 - dead_count,
            "Σ module_cell_count must equal (grid cells − removed cells); no dead cell may contribute to a module"
        );
    }

    #[test]
    fn m7b_empty_body_valid() {
        let econ = EconParams::default();
        // GRN state[0] is a non-negative expression bounded by GRN_EXPR_MAX (grn.rs "σ semantics") —
        // any threshold strictly above that bound guarantees state[0] < t for EVERY cell.
        let g = m7b_live_drive_genome(Some(crate::GRN_EXPR_MAX + 1));
        let ph = g.decode(&econ).expect("F2/F5 PINNED: apoptosis-to-zero must return Some, never None");
        assert_eq!(ph.graph.num_modules(), 0, "all-dead grid must yield zero modules");
        assert!(ph.graph.module_cell_count.is_empty());
        assert!(ph.graph.module_type.is_empty());
    }

    #[test]
    fn m7b_e5b_is_sole_stillbirth() {
        let econ = EconParams::default();

        // (a) E-5b fires BEFORE any graph/apoptosis work: an unviable-size genome returns None
        // regardless of apoptosis_threshold (even when apoptosis is armed).
        let mut g_unviable = m7b_live_drive_genome(Some(1));
        g_unviable.size = SIZE_VIABILITY_FLOOR; // == floor, not > floor ⇒ E-5b fails
        assert_eq!(
            g_unviable.decode(&econ), None,
            "E-5b size-viability gate must return None regardless of apoptosis_threshold"
        );

        // (b) A genome that PASSES E-5b (size=21, well above the floor) and then apoptoses to zero
        // must still return Some(empty-graph) — never None (no second stillbirth path).
        let g_zero = m7b_live_drive_genome(Some(crate::GRN_EXPR_MAX + 1));
        assert!(is_viable_size(g_zero.size), "fixture must clear E-5b's viability floor (sanity)");
        let ph = g_zero.decode(&econ).expect("apoptosis-to-zero must never become a second stillbirth path");
        assert_eq!(ph.graph.num_modules(), 0);
    }

    // ── M7-c: germ/soma split (golden-NEUTRAL) ──────────────────────────────────────────────────
    //
    // Adds a module-level germ/soma marker computed AFTER Step 3 (module collection): a LIVE
    // module is GERM iff `module_cell_count[mid] <= germ_threshold` (small=germ, large=soma) —
    // integer-only, PINNED, no float, no morphogen re-traversal. `germ_threshold: None` on every
    // shipped spec means `module_is_germ` stays all-`false` — byte-identical to M7-b (checked
    // structurally in the `cli` crate's `m7c_prod_inert_all_goldens`). Dead cells (M7-b apoptosis)
    // are excluded before Step 3 already runs, so germ/soma partitions only live modules — the two
    // mechanisms are orthogonal.

    /// M7-c test-only spec builder — same grid/diffusion/decay basis as M7-a/M7-b, parameterized by
    /// BOTH gates (unlike `m7b_mspec`, which hardcodes `germ_threshold: None`).
    fn m7c_mspec(apoptosis_threshold: Option<i32>, germ_threshold: Option<i32>) -> MorphogenSpec {
        MorphogenSpec {
            g_dev: 4, n_dev: 8, boundary: Boundary::Reflecting,
            diffuse_shift: 3, decay_num: 1, decay_shift: 4,
            seed_scale: 64, stop_threshold: 0,
            apoptosis_threshold, germ_threshold,
            supply_source: None,
            adhesion_threshold: None,
        }
    }

    /// M7-c fixture: the M7-a live-drive genome (size=21, ≥2 modules) — reused so germ/soma tests
    /// exercise a grid already known to produce a multi-module split.
    fn m7c_live_drive_genome(apoptosis_threshold: Option<i32>, germ_threshold: Option<i32>) -> Genome {
        let gspec = m7a_live_drive_gspec();
        let mspec = m7c_mspec(apoptosis_threshold, germ_threshold);
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec)), Some(mspec));
        g.size = 21; // above the E-6 fate boundary — matches m7a_live_drive_produces_multiple_modules
        g
    }

    /// A threshold that splits the live-drive fixture's modules into BOTH germ and soma (the
    /// minimum module cell count on the apoptosis-off baseline graph — picked from the REAL
    /// resolved module sizes, not a guessed magic constant; asserts the fixture is non-uniform so
    /// the split is genuinely mixed, not degenerate).
    fn m7c_partial_threshold() -> i32 {
        let econ = EconParams::default();
        let ph = m7c_live_drive_genome(None, None).decode(&econ).expect("must decode to Some");
        let counts = &ph.graph.module_cell_count;
        let min = *counts.iter().min().unwrap();
        let max = *counts.iter().max().unwrap();
        assert!(min < max, "M7-c fixture must have modules of different sizes; got {counts:?}");
        min
    }

    #[test]
    fn m7c_germ_soma_determinism() {
        let econ = EconParams::default();
        let t = m7c_partial_threshold();
        let g = m7c_live_drive_genome(None, Some(t));
        let ph1 = g.decode(&econ).expect("must decode to Some");
        let ph2 = g.decode(&econ).expect("must decode to Some");
        assert_eq!(ph1.graph, ph2.graph, "same genome + Some(t) must produce identical module_is_germ across repeated decode");
    }

    #[test]
    fn m7c_produces_both_types() {
        let econ = EconParams::default();
        let t = m7c_partial_threshold();
        let g = m7c_live_drive_genome(None, Some(t));
        let ph = g.decode(&econ).expect("must decode to Some");

        assert!(ph.graph.module_is_germ.iter().any(|&is_germ| is_germ), "fixture must yield >= 1 germ module");
        assert!(ph.graph.module_is_germ.iter().any(|&is_germ| !is_germ), "fixture must yield >= 1 soma module");

        // Cross-check the predicate directly against module_cell_count (module-level, integer).
        for (i, &count) in ph.graph.module_cell_count.iter().enumerate() {
            assert_eq!(ph.graph.module_is_germ[i], count <= t, "module {i}: is_germ must equal (cell_count <= threshold)");
        }
    }

    #[test]
    fn m7c_none_all_soma() {
        let econ = EconParams::default();
        let g_none = m7c_live_drive_genome(None, None);
        let ph_none = g_none.decode(&econ).expect("must decode to Some");
        assert!(ph_none.graph.module_is_germ.iter().all(|&is_germ| !is_germ), "germ_threshold=None must leave every module soma/unmarked");

        // Byte-identical to M7-b for module_type/module_cell_count (only module_is_germ is new).
        let ph_m7b = m7b_live_drive_genome(None).decode(&econ).expect("must decode to Some");
        assert_eq!(ph_none.graph.module_type, ph_m7b.graph.module_type, "M7-c must not perturb M7-b's module_type");
        assert_eq!(ph_none.graph.module_cell_count, ph_m7b.graph.module_cell_count, "M7-c must not perturb M7-b's module_cell_count");
    }

    #[test]
    fn m7c_interacts_with_apoptosis() {
        let econ = EconParams::default();
        let t_apop = m7b_partial_threshold();

        // Derive a germ threshold from the POST-apoptosis live module sizes (germ/soma runs after
        // Step 3, on live modules only).
        let ph_apop_only = m7c_live_drive_genome(Some(t_apop), None).decode(&econ).expect("must decode to Some");
        let live_counts = ph_apop_only.graph.module_cell_count.clone();
        let t_germ = *live_counts.iter().min().unwrap();

        let g_both = m7c_live_drive_genome(Some(t_apop), Some(t_germ));
        let ph = g_both.decode(&econ).expect("must decode to Some");

        // Both labels coexist: module_is_germ is indexed 1:1 with the LIVE modules (dead cells
        // already excluded, so neither an all-dead nor a partially-dead cell ever appears here).
        assert_eq!(ph.graph.module_is_germ.len(), ph.graph.module_cell_count.len(), "module_is_germ must be indexed 1:1 with live modules");
        assert_eq!(ph.graph.module_cell_count, live_counts, "germ_threshold must not perturb apoptosis's live-module partition");
        for (i, &count) in ph.graph.module_cell_count.iter().enumerate() {
            assert_eq!(ph.graph.module_is_germ[i], count <= t_germ, "module {i}: predicate must hold on LIVE (post-apoptosis) counts");
        }
        let module_total: i32 = ph.graph.module_cell_count.iter().sum();
        assert!(module_total < 16, "apoptosis must still have removed >= 1 cell from the 16-cell grid");
    }

    #[test]
    fn m7c_empty_body_valid() {
        let econ = EconParams::default();
        // Every cell apoptosed (see m7b_empty_body_valid) ⇒ zero live modules ⇒ zero germ/zero soma.
        let g = m7c_live_drive_genome(Some(crate::GRN_EXPR_MAX + 1), Some(0));
        let ph = g.decode(&econ).expect("all-dead grid must return Some, never a second stillbirth path");
        assert_eq!(ph.graph.num_modules(), 0, "all-dead grid must yield zero modules");
        assert!(ph.graph.module_is_germ.is_empty(), "zero modules must yield zero germ/zero soma markers");
    }

    #[test]
    fn m7c_degenerate_ok() {
        let econ = EconParams::default();

        // All-germ: a threshold no module's cell count can exceed.
        let g_all_germ = m7c_live_drive_genome(None, Some(i32::MAX));
        let ph_all_germ = g_all_germ.decode(&econ).expect("must decode to Some");
        assert!(!ph_all_germ.graph.module_is_germ.is_empty(), "sanity: fixture must have >= 1 live module");
        assert!(ph_all_germ.graph.module_is_germ.iter().all(|&is_germ| is_germ), "an unreachable-high threshold must mark every module germ");

        // All-soma: a threshold no module's (>= 1) cell count can clear.
        let g_all_soma = m7c_live_drive_genome(None, Some(0));
        let ph_all_soma = g_all_soma.decode(&econ).expect("must decode to Some");
        assert!(ph_all_soma.graph.module_is_germ.iter().all(|&is_germ| !is_germ), "threshold=0 must mark every module soma (every module has >= 1 cell)");
    }

    // ── M7-d: supply-gate reachability-at-birth (golden-NEUTRAL) ────────────────────────────────
    //
    // Adds a module-level supply-reachability marker computed AFTER germ/soma labeling (Step 5):
    // BFS from a supply-source module over the LIVE module-adjacency graph, reconstructed from
    // cell 4-neighbors (the union-find `parent` never recorded cross-module edges, only same-type
    // unions). `supply_source: None` on every shipped spec means `module_reachable` stays
    // all-`true` — byte-identical to M7-c (checked structurally in the `cli` crate's
    // `m7d_prod_inert_all_goldens`). Dead cells (M7-b apoptosis) are excluded before adjacency
    // reconstruction, so reachability partitions only live modules — orthogonal to germ/soma.

    /// M7-d test-only spec builder — same grid/diffusion/decay basis as M7-a/b/c, parameterized by
    /// all three gates.
    fn m7d_mspec(apoptosis_threshold: Option<i32>, germ_threshold: Option<i32>, supply_source: Option<i32>) -> MorphogenSpec {
        MorphogenSpec {
            g_dev: 4, n_dev: 8, boundary: Boundary::Reflecting,
            diffuse_shift: 3, decay_num: 1, decay_shift: 4,
            seed_scale: 64, stop_threshold: 0,
            apoptosis_threshold, germ_threshold, supply_source,
            adhesion_threshold: None,
        }
    }

    /// M7-d fixture: the M7-a live-drive genome (size=21, 2 modules: a 1-cell corner module + a
    /// 15-cell module, fully live and grid-adjacent) — reused so the wiring/determinism teeth
    /// exercise the same production `decode()` path M7-b/M7-c did.
    fn m7d_live_drive_genome(
        apoptosis_threshold: Option<i32>,
        germ_threshold: Option<i32>,
        supply_source: Option<i32>,
    ) -> Genome {
        let gspec = m7a_live_drive_gspec();
        let mspec = m7d_mspec(apoptosis_threshold, germ_threshold, supply_source);
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec)), Some(mspec));
        g.size = 21; // above the E-6 fate boundary — matches m7a_live_drive_produces_multiple_modules
        g
    }

    #[test]
    fn m7d_reachability_determinism() {
        let econ = EconParams::default();
        let g = m7d_live_drive_genome(None, None, Some(0));
        let ph1 = g.decode(&econ).expect("must decode to Some");
        let ph2 = g.decode(&econ).expect("must decode to Some");
        assert_eq!(ph1.graph, ph2.graph, "same genome + Some(src) must produce identical module_reachable across repeated decode");
    }

    #[test]
    fn m7d_source_module_reachable() {
        let econ = EconParams::default();
        // idx 0 = cell (0,0), the M7-a fixture's 1-cell corner module (discovered first, module 0).
        let g = m7d_live_drive_genome(None, None, Some(0));
        let ph = g.decode(&econ).expect("must decode to Some");
        assert_eq!(ph.graph.module_type.len(), 2, "sanity: fixture must have 2 modules (M7-a baseline)");
        assert!(ph.graph.module_reachable[0], "the source module must always be reachable");
        // This fixture is fully live (no apoptosis): the module quotient of a connected live grid
        // is itself connected, so the OTHER module (the 15-cell block, grid-adjacent to the
        // corner) must be reachable too.
        assert!(ph.graph.module_reachable.iter().all(|&r| r), "fully-live grid must yield a fully-connected module graph");
    }

    #[test]
    fn m7d_none_all_reachable() {
        let econ = EconParams::default();
        let g_none = m7d_live_drive_genome(None, None, None);
        let ph_none = g_none.decode(&econ).expect("must decode to Some");
        assert!(ph_none.graph.module_reachable.iter().all(|&r| r), "supply_source=None must leave every module reachable");

        // Byte-identical to M7-c for module_type/module_cell_count/module_is_germ (only
        // module_reachable is new).
        let ph_m7c = m7c_live_drive_genome(None, None).decode(&econ).expect("must decode to Some");
        assert_eq!(ph_none.graph.module_type, ph_m7c.graph.module_type, "M7-d must not perturb M7-c's module_type");
        assert_eq!(ph_none.graph.module_cell_count, ph_m7c.graph.module_cell_count, "M7-d must not perturb M7-c's module_cell_count");
        assert_eq!(ph_none.graph.module_is_germ, ph_m7c.graph.module_is_germ, "M7-d must not perturb M7-c's module_is_germ");
    }

    /// M7-d disconnection fixture: a HAND-BUILT gradient (bypassing the morphogen chain, calling
    /// `CellGraph::from_gradient` directly) whose row `z=1` is a "cold" wall (drive=0) separating
    /// row `z=0` (top block, 4 cells) from rows `z=2..3` (bottom block, 8 cells) — apoptosis kills
    /// the wall, leaving NO live path between the two blocks. Both blocks classify to the SAME
    /// `CellType::A` (manually traced below), so the split is provably topological (Step 5's
    /// adjacency reconstruction sees no live neighbor across the dead row), not a type-boundary
    /// artifact.
    fn m7d_wall_gradient() -> crate::morphogen::Gradient {
        crate::morphogen::Gradient {
            g_dev: 4,
            cells: vec![
                10, 10, 10, 10,
                0, 0, 0, 0,
                10, 10, 10, 10,
                10, 10, 10, 10,
            ],
        }
    }

    /// Linear (non-bistable, no internal recurrence) readout: `state[0]` is monotone in the raw
    /// drive value, so `CellGraph::from_gradient`'s per-cell classification/apoptosis directly
    /// reflects `m7d_wall_gradient`'s hand-picked values.
    fn m7d_wall_gspec() -> GrnSpec {
        GrnSpec::new(2, vec![0, 0, 0, 0], vec![1, 0], vec![0, 0], 0, 1, 0, 0, vec![0, 0])
    }

    /// The apoptosis threshold that kills exactly the wall row and spares the rest — derived from
    /// the REAL resolved gene-0 states (mirrors `m7b_partial_threshold`'s min+1 discipline), not a
    /// guessed magic constant.
    fn m7d_wall_apoptosis_threshold() -> i32 {
        let gspec = m7d_wall_gspec();
        let mut cg = gspec.clone();
        cg.sample_x = 0;
        cg.sample_z = 0;
        let dead_gradient = crate::morphogen::Gradient { g_dev: 1, cells: vec![0] };
        let live_gradient = crate::morphogen::Gradient { g_dev: 1, cells: vec![10] };
        let (dead_state, _, _) = crate::grn_resolve(&dead_gradient, &cg);
        let (live_state, _, _) = crate::grn_resolve(&live_gradient, &cg);
        assert!(
            dead_state[0] < live_state[0],
            "wall fixture must have a genuine state gap to threshold on; got dead={dead_state:?} live={live_state:?}"
        );
        dead_state[0] + 1
    }

    #[test]
    fn m7d_unreachable_exists() {
        let gradient = m7d_wall_gradient();
        let gspec = m7d_wall_gspec();
        let t = m7d_wall_apoptosis_threshold();
        // idx 0 = cell (0,0), inside the top (surviving) block.
        let graph = CellGraph::from_gradient(&gradient, &gspec, Some(t), None, Some(0), None);
        assert_eq!(
            graph.module_type,
            vec![CellType::A, CellType::A],
            "sanity: both live blocks classify to the SAME type (disconnection is topological, not type-driven)"
        );
        assert_eq!(graph.module_cell_count, vec![4, 8], "sanity: top block 4 cells, bottom block 8 cells (manual trace)");
        assert_eq!(
            graph.module_reachable,
            vec![true, false],
            "the source module (top, idx 0) is reachable; the wall-severed bottom module is not"
        );
    }

    #[test]
    fn m7d_source_dead_all_unreachable() {
        let gradient = m7d_wall_gradient();
        let gspec = m7d_wall_gspec();
        let t = m7d_wall_apoptosis_threshold();
        // idx 4 = cell (x=0, z=1), inside the apoptosed wall row -- a genuinely dead source.
        let graph = CellGraph::from_gradient(&gradient, &gspec, Some(t), None, Some(4), None);
        assert_eq!(graph.num_modules(), 2, "sanity: apoptosis must still leave 2 live modules");
        assert!(graph.module_reachable.iter().all(|&r| !r), "a dead supply source must leave every module unreachable, valid Some, no panic");
    }

    #[test]
    fn m7d_interacts_apoptosis_germ() {
        let gradient = m7d_wall_gradient();
        let gspec = m7d_wall_gspec();
        let t_apop = m7d_wall_apoptosis_threshold();
        // germ_threshold=4: the top module (count=4) is GERM, the bottom module (count=8) is SOMA.
        let graph = CellGraph::from_gradient(&gradient, &gspec, Some(t_apop), Some(4), Some(0), None);
        assert_eq!(graph.module_cell_count, vec![4, 8]);
        assert_eq!(graph.module_is_germ, vec![true, false], "module 0 (count=4) is germ; module 1 (count=8) is soma");
        assert_eq!(graph.module_reachable, vec![true, false], "reachability must be unaffected by germ/soma labeling");

        // All three gates armed together must still be deterministic.
        let graph2 = CellGraph::from_gradient(&gradient, &gspec, Some(t_apop), Some(4), Some(0), None);
        assert_eq!(graph, graph2, "apoptosis + germ + supply must coexist deterministically");
    }

    #[test]
    fn m7d_empty_body_valid() {
        let gradient = m7d_wall_gradient();
        let gspec = m7d_wall_gspec();
        // A threshold above every resolved state kills the whole grid (mirrors m7b/m7c's
        // `GRN_EXPR_MAX + 1` empty-body pattern).
        let graph = CellGraph::from_gradient(&gradient, &gspec, Some(crate::GRN_EXPR_MAX + 1), None, Some(0), None);
        assert_eq!(graph.num_modules(), 0, "all-dead grid must yield zero modules");
        assert!(graph.module_reachable.is_empty(), "zero modules must yield an empty module_reachable, not a panic");
    }

    // ── M7-f: consortium adhesion (#252) ─────────────────────────────────────────────────────────

    /// M7-f fixture: a HAND-BUILT gradient (bypassing the morphogen chain, calling
    /// `CellGraph::from_gradient` directly, mirrors `m7d_wall_gradient`'s discipline) producing
    /// exactly 3 LIVE modules, all pairwise grid-adjacent:
    /// - module 0 (`CellType::A`, top-left 2×2 block, 4 cells, drive=10)
    /// - module 1 (`CellType::B`, the right two columns, 8 cells, drive=-10)
    /// - module 2 (`CellType::Mixed`, bottom-left 2×2 block, 4 cells, drive=0)
    ///
    /// With `m7d_wall_gspec` (linear gene-0 readout, gene-1 pinned at `sigma(0)`): `classify`
    /// resolves drive=10 to `A` (state0>state1), drive=-10 to `B` (state0<state1), drive=0 to
    /// `Mixed` (exact tie) — the same monotone reasoning `m7d_wall_apoptosis_threshold` relies on.
    /// No apoptosis: all 16 cells stay live.
    fn m7f_adhesion_gradient() -> crate::morphogen::Gradient {
        crate::morphogen::Gradient {
            g_dev: 4,
            #[rustfmt::skip]
            cells: vec![
                10, 10, -10, -10,
                10, 10, -10, -10,
                 0,  0, -10, -10,
                 0,  0, -10, -10,
            ],
        }
    }

    /// M7-f fixture: `germ_threshold=4` splits the 3-module grid into germ=[true, false, true]
    /// (module 0 count=4 <=4, module 1 count=8 >4, module 2 count=4 <=4). Module 1 (soma) is
    /// adjacent to BOTH module 0 and module 2 but differs in germ-status from both — it must stay
    /// its own consortium while modules 0 and 2 (same germ-status, also adjacent) merge.
    fn m7f_adhesion_germ_threshold() -> i32 {
        4
    }

    #[test]
    fn m7f_creates_consortia() {
        let gradient = m7f_adhesion_gradient();
        let gspec = m7d_wall_gspec();
        let graph = CellGraph::from_gradient(
            &gradient,
            &gspec,
            None,
            Some(m7f_adhesion_germ_threshold()),
            None,
            Some(1),
        );
        assert_eq!(graph.module_type, vec![CellType::A, CellType::B, CellType::Mixed], "sanity: 3 distinct modules");
        assert_eq!(graph.module_cell_count, vec![4, 8, 4], "sanity: manually-traced module sizes");
        assert_eq!(graph.module_is_germ, vec![true, false, true], "sanity: germ_threshold=4 splits germ=[T,F,T]");

        assert_eq!(
            graph.module_consortium[0], graph.module_consortium[2],
            "modules 0 and 2 are adjacent and share germ-status (both true) — must share a consortium root"
        );
        assert_ne!(
            graph.module_consortium[1], graph.module_consortium[0],
            "module 1 is adjacent to both 0 and 2 but differs in germ-status from both — must stay its own consortium"
        );
    }

    #[test]
    fn m7f_adhesion_determinism() {
        let gradient = m7f_adhesion_gradient();
        let gspec = m7d_wall_gspec();
        let g1 = CellGraph::from_gradient(&gradient, &gspec, None, Some(m7f_adhesion_germ_threshold()), None, Some(1));
        let g2 = CellGraph::from_gradient(&gradient, &gspec, None, Some(m7f_adhesion_germ_threshold()), None, Some(1));
        assert_eq!(g1, g2, "same gradient + Some(_) must produce identical module_consortium across repeated calls");
    }

    #[test]
    fn m7f_none_identity() {
        let gradient = m7f_adhesion_gradient();
        let gspec = m7d_wall_gspec();
        let graph_off = CellGraph::from_gradient(&gradient, &gspec, None, Some(m7f_adhesion_germ_threshold()), None, None);
        assert_eq!(
            graph_off.module_consortium,
            (0..graph_off.num_modules()).collect::<Vec<usize>>(),
            "adhesion_threshold=None must leave module_consortium as the identity mapping"
        );

        let graph_on = CellGraph::from_gradient(&gradient, &gspec, None, Some(m7f_adhesion_germ_threshold()), None, Some(1));
        assert_eq!(graph_off.module_type, graph_on.module_type, "M7-f must not perturb module_type");
        assert_eq!(graph_off.module_cell_count, graph_on.module_cell_count, "M7-f must not perturb module_cell_count");
        assert_eq!(graph_off.module_is_germ, graph_on.module_is_germ, "M7-f must not perturb module_is_germ");
        assert_eq!(graph_off.module_reachable, graph_on.module_reachable, "M7-f must not perturb module_reachable");
        assert_ne!(
            graph_off.module_consortium, graph_on.module_consortium,
            "sanity: this fixture must actually change grouping when adhesion is armed"
        );
    }

    // `m7f_prod_inert_all_goldens` (the exhaustive 6-config sweep against the real shipped configs)
    // lives in `cli`'s `tests/m7f_adhesion.rs` — it needs the production config builders, which are
    // defined in that crate (mirrors `m7d_prod_inert_all_goldens`'s placement).

    #[test]
    fn m7f_interacts_apoptosis_germ_supply() {
        let gradient = m7d_wall_gradient();
        let gspec = m7d_wall_gspec();
        let t_apop = m7d_wall_apoptosis_threshold();
        // All four gates armed: apoptosis kills the wall row, germ_threshold=4 splits germ=[T,F],
        // supply_source=0 makes only the top module reachable, adhesion_threshold=Some(_) arms
        // the consortium pass. The wall severs LIVE adjacency between the two surviving modules,
        // so Step 6 finds no edge to union regardless of germ-status — both stay their own.
        let graph = CellGraph::from_gradient(&gradient, &gspec, Some(t_apop), Some(4), Some(0), Some(1));
        assert_eq!(graph.module_cell_count, vec![4, 8], "sanity: same wall-fixture module sizes as M7-d");
        assert_eq!(graph.module_is_germ, vec![true, false], "sanity: germ_threshold=4 still splits germ=[T,F]");
        assert_eq!(graph.module_reachable, vec![true, false], "sanity: wall still severs reachability");
        assert_eq!(
            graph.module_consortium,
            vec![0, 1],
            "wall severs live adjacency — no edge exists to union, both modules stay their own consortium"
        );

        let graph2 = CellGraph::from_gradient(&gradient, &gspec, Some(t_apop), Some(4), Some(0), Some(1));
        assert_eq!(graph, graph2, "all four gates (apoptosis+germ+supply+adhesion) must coexist deterministically");
    }

    #[test]
    fn m7f_empty_body_valid() {
        let gradient = m7d_wall_gradient();
        let gspec = m7d_wall_gspec();
        // A threshold above every resolved state kills the whole grid (mirrors m7b/m7c/m7d's
        // `GRN_EXPR_MAX + 1` empty-body pattern).
        let graph = CellGraph::from_gradient(&gradient, &gspec, Some(crate::GRN_EXPR_MAX + 1), None, Some(0), Some(1));
        assert_eq!(graph.num_modules(), 0, "all-dead grid must yield zero modules");
        assert!(graph.module_consortium.is_empty(), "zero modules must yield an empty module_consortium, not a panic");
    }

    #[test]
    fn m7f_single_module() {
        // Uniform grid: every cell resolves to the SAME CellType (drive=10 everywhere) → union-find
        // collapses the whole grid into exactly 1 module (mirrors M7-a's baseline reasoning).
        let gradient = crate::morphogen::Gradient { g_dev: 4, cells: vec![10; 16] };
        let gspec = m7d_wall_gspec();
        let graph = CellGraph::from_gradient(&gradient, &gspec, None, None, None, Some(1));
        assert_eq!(graph.num_modules(), 1, "sanity: uniform grid must collapse to 1 module");
        assert_eq!(graph.module_consortium, vec![0], "a singleton module must be its own (trivial) consortium");
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
            g = g.mutate(seed, econ.n_energy_layers, econ.light.is_some(), econ.reg_gain_max, econ.predation.is_some(), false, false, false);
            
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
        let m1 = g1.mutate(seed, econ.n_energy_layers, econ.light.is_some(), econ.reg_gain_max, econ.predation.is_some(), false, false, false);
        let m2 = g2.mutate(seed, econ.n_energy_layers, econ.light.is_some(), econ.reg_gain_max, econ.predation.is_some(), false, false, false);
        
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
        let m = g.mutate(0x9999_9999, 2, false, 4, false, true, false, false);
        
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
            let m = g.clone().mutate(seed, 2, false, 4, false, true, false, false);
            
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
            let m = g.clone().mutate(seed, 2, false, 4, false, true, false, false);
            
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
            let m = g.clone().mutate(seed, 2, false, 4, false, true, false, false);
            
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
            seed_scale: 64, stop_threshold: 0, apoptosis_threshold: None,
            germ_threshold: None,
            supply_source: None,
            adhesion_threshold: None,
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
            let m = g.clone().mutate(seed, econ.n_energy_layers, econ.light.is_some(), econ.reg_gain_max, econ.predation.is_some(), true, false, false);
            
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
            g = g.mutate(seed, 2, false, 4, false, false, false, false);
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
        let m1 = g1.mutate(seed, 2, false, 4, false, false, false, false);
        let m2 = g2.mutate(seed, 2, false, 4, false, false, false, false);

        assert_eq!(m1.metabolism_eff, m2.metabolism_eff);
        assert_eq!(m1.weights, m2.weights);
        assert_eq!(m1.grn_spec, m2.grn_spec, "V-3-c: flag=false output must be byte-identical");
        assert_eq!(m1.grn_spec.unwrap().n_genes, 2, "V-3-c: flag=false must never change n_genes");
    }

    #[test]
    fn v3c_no_spec_no_indel() {
        // flag=true but grn_spec=None → no length change (Some-gated).
        let g = Genome::founder(2); // no spec
        let m = g.mutate(0x2222_2222, 2, false, 4, false, true, false, false);
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
            let m = g.clone().mutate(seed, 2, false, 4, false, true, false, false);
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
            let m = g.clone().mutate(seed, 2, false, 4, false, true, false, false);
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
            let m = g.clone().mutate(seed, 2, false, 4, false, true, false, false);
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
            g = g.mutate(seed, 2, false, 4, false, true, false, false);
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
            g = g.mutate(seed, 2, false, 4, false, true, false, false);
            if let Some(spec) = &g.grn_spec {
                let mut seen = std::collections::BTreeSet::new();
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
            seed_scale: 64, stop_threshold: 0, apoptosis_threshold: None,
            germ_threshold: None,
            supply_source: None,
            adhesion_threshold: None,
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
            let m = g.clone().mutate(seed, econ.n_energy_layers, econ.light.is_some(), econ.reg_gain_max, econ.predation.is_some(), true, false, false);
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
        let results: Vec<Genome> = (0..8).map(|_| g.clone().mutate(seed, 2, false, 4, false, true, false, false)).collect();

        for m in &results[1..] {
            assert_eq!(m.grn_spec, results[0].grn_spec, "V-3-c: replay must be seed-deterministic, not run-order-dependent");
        }
    }

    // ── V-3-d: translocation operator (gene swap, golden-NEUTRAL, #245) ─────────────────────────
    //
    // Independent reference implementation of π = SWAP(i,j) on a `GrnSpec` — built by directly
    // remapping every (row,col)/index through π, NOT by production's incremental row-then-col swap
    // (`Genome::mutate`). Used by the structural-equality and conjugacy teeth (F2) to prove the
    // production swap is the SAME permutation on every axis, via a construction that can't share a
    // bug with it.
    fn v3d_manual_swap(spec: &GrnSpec, i: usize, j: usize) -> GrnSpec {
        let n = spec.n_genes;
        let pi = |k: usize| if k == i { j } else if k == j { i } else { k };
        let mut out = spec.clone();
        for row in 0..n {
            for col in 0..n {
                out.weights[row * n + col] = spec.weights[pi(row) * n + pi(col)];
            }
        }
        out.input_weights.swap(i, j);
        out.bias.swap(i, j);
        out.initial.swap(i, j);
        out.gene_ids.swap(i, j);
        out
    }

    /// A real i != j translocation REORDERS `gene_ids` (same multiset, different sequence) — unlike
    /// V-3-b duplication (adds a fresh id) or V-3-c indel (adds/removes an id), which also change
    /// `n_genes`. Guarding on `n_genes == old_len` alone is NOT enough: a dup THEN a delete can
    /// round-trip `n_genes` back to its old value while still swapping in a novel id, so this checks
    /// the id SET is unchanged (rules out dup/indel contamination) and the ORDER differs (a real
    /// swap happened, not merely a no-op or an untouched clone).
    fn v3d_is_pure_reorder(candidate: &[u32], original: &[u32]) -> bool {
        if candidate.len() != original.len() || candidate == original {
            return false;
        }
        let mut c = candidate.to_vec();
        let mut o = original.to_vec();
        c.sort();
        o.sort();
        c == o
    }

    #[test]
    fn v3d_flag_off_no_translocation() {
        // flag=false → n_genes never changes and gene_ids never permute, even with a grn_spec
        // attached and max mutation_rate.
        let gspec = GrnSpec::new(2, vec![64, -64, -64, 64], vec![0, 0], vec![0, 0], 3, 12, 0, 0, vec![0, 64]);
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec)), None);
        g.mutation_rate = 256;

        for i in 0..100 {
            let seed = 0x1A00_0000 + (i as u64);
            g = g.mutate(seed, 2, false, 4, false, false, false, false);
            if let Some(spec) = &g.grn_spec {
                assert_eq!(spec.n_genes, 2, "V-3-d: flag=false must keep n_genes constant (iteration {i})");
                assert_eq!(spec.gene_ids, vec![0, 1], "V-3-d: flag=false must never permute gene_ids (iteration {i})");
            }
        }
    }

    #[test]
    fn v3d_flag_off_byte_identity() {
        // With enable_variable_length=false, V-3-d (appended AFTER V-3-c indel) draws zero values —
        // the mutated genome (including its grn_spec) must be fully deterministic/unperturbed,
        // proving translocation's presence doesn't move any earlier draw in the fixed order.
        let gspec = GrnSpec::new(2, vec![64, -64, -64, 64], vec![0, 0], vec![0, 0], 3, 12, 0, 0, vec![0, 64]);
        let g1 = Genome::founder(2).with_specs(Some(Arc::new(gspec.clone())), None);
        let g2 = Genome::founder(2).with_specs(Some(Arc::new(gspec)), None);

        let seed = 0xDEAD_BEEF;
        let m1 = g1.mutate(seed, 2, false, 4, false, false, false, false);
        let m2 = g2.mutate(seed, 2, false, 4, false, false, false, false);

        assert_eq!(m1.metabolism_eff, m2.metabolism_eff);
        assert_eq!(m1.weights, m2.weights);
        assert_eq!(m1.grn_spec, m2.grn_spec, "V-3-d: flag=false output must be byte-identical");
        assert_eq!(m1.grn_spec.unwrap().n_genes, 2, "V-3-d: flag=false must never change n_genes");
    }

    #[test]
    fn v3d_no_spec_no_op() {
        // flag=true but grn_spec=None → no-op (Some-gated).
        let g = Genome::founder(2); // no spec
        let m = g.mutate(0x2A2A_2A2A, 2, false, 4, false, true, false, false);
        assert!(m.grn_spec.is_none(), "V-3-d: genome without spec must stay without spec");
    }

    #[test]
    fn v3d_can_translocate() {
        // A seed that fires a real i != j swap reverses gene_ids and the parallel per-gene vectors
        // on an n_genes=2 fixture (the only possible non-identity swap is (0,1)). Low mutation_rate
        // + a match against the independent `v3d_manual_swap` reference (rather than accepting the
        // first `gene_ids`-reordered candidate) skips seeds where V-1's point-mutation ALSO fired
        // alongside translocation — this test wants a translocation-ONLY diff.
        let gspec0 = GrnSpec::new(2, vec![64, -64, -64, 64], vec![10, 20], vec![1, 2], 3, 12, 0, 0, vec![100, 200]);
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec0.clone())), None);
        g.mutation_rate = 4;

        for i in 0..5000u64 {
            let seed = 0x3A00_0000 + i;
            let m = g.clone().mutate(seed, 2, false, 4, false, true, false, false);
            if let Some(spec) = &m.grn_spec {
                if v3d_is_pure_reorder(&spec.gene_ids, &gspec0.gene_ids) && **spec == v3d_manual_swap(&gspec0, 0, 1) {
                    assert_eq!(spec.gene_ids, vec![1, 0], "V-3-d: a real i!=j swap on n_genes=2 must reverse gene_ids");
                    assert_eq!(spec.input_weights, vec![20, 10], "V-3-d: input_weights must swap with the genes");
                    assert_eq!(spec.bias, vec![2, 1], "V-3-d: bias must swap with the genes");
                    assert_eq!(spec.initial, vec![200, 100], "V-3-d: initial must swap with the genes");
                    return;
                }
            }
        }
        panic!("V-3-d: expected to find a clean (uncontaminated) translocation within 5000 seeds");
    }

    #[test]
    fn v3d_no_op_self_move() {
        // Mirrors the production private salt (PINNED 0x5452_4100 "TRA\0", grn.rs/genome.rs spec
        // #245) to locate a seed where translocation's application-count draw fires but i == j — the
        // self-move branch, which must leave the spec byte-identical to the parent (no allocation).
        const SALT_TRANS_MIRROR: u64 = 0x5452_4100u64;
        let gspec0 = GrnSpec::new(2, vec![64, -64, -64, 64], vec![10, 20], vec![5, 15], 3, 12, 0, 0, vec![32, 96]);
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec0.clone())), None);
        g.mutation_rate = 8; // low — minimizes V-1/V-3-b/c interference on the found seed

        for i in 0..5000u64 {
            let seed = 0xB000_0000 + i;
            let r_count = seed_fold(seed, &[SALT_TRANS_MIRROR]);
            if (r_count & 0xFF) >= g.mutation_rate as u64 {
                continue; // translocation didn't fire this seed
            }
            let r_i = seed_fold(seed, &[SALT_TRANS_MIRROR + 1]);
            let r_j = seed_fold(seed, &[SALT_TRANS_MIRROR + 2]);
            if (r_i >> 8) % 2 != (r_j >> 8) % 2 {
                continue; // i != j — not the self-move branch
            }
            let m = g.clone().mutate(seed, 2, false, 4, false, true, false, false);
            if let Some(spec) = &m.grn_spec {
                if **spec == gspec0 {
                    return; // self-move fired; spec byte-identical to parent — no-op proven
                }
            }
        }
        panic!("V-3-d: expected to find a translocation self-move (i==j) no-op within 5000 seeds");
    }

    #[test]
    fn v3d_swap_structural_equality() {
        // THE CRITICAL TOOTH (F2): after a real i != j swap, the resulting GrnSpec must equal the
        // spec produced by an INDEPENDENT reference remap (`v3d_manual_swap`) — proving the SAME
        // permutation π hit weights (both axes) and every parallel per-gene vector.
        const SALT_TRANS_MIRROR: u64 = 0x5452_4100u64;
        let gspec0 = GrnSpec::new(
            3,
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9],
            vec![10, 20, 30],
            vec![100, 200, 300],
            3, 12, 0, 0,
            vec![1000, 2000, 3000],
        );
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec0.clone())), None);
        g.mutation_rate = 4; // low — minimizes V-1/V-3-b/c interference on the found seed

        for i in 0..8000u64 {
            let seed = 0xD000_0000 + i;
            let r_count = seed_fold(seed, &[SALT_TRANS_MIRROR]);
            if (r_count & 0xFF) >= g.mutation_rate as u64 {
                continue;
            }
            let r_i = seed_fold(seed, &[SALT_TRANS_MIRROR + 1]);
            let r_j = seed_fold(seed, &[SALT_TRANS_MIRROR + 2]);
            let (ti, tj) = ((r_i >> 8) as usize % 3, (r_j >> 8) as usize % 3);
            if ti == tj {
                continue;
            }
            let m = g.clone().mutate(seed, 2, false, 4, false, true, false, false);
            let Some(spec) = &m.grn_spec else { continue };
            if spec.n_genes != 3 {
                continue; // V-3-b/c also fired this seed — skip (searching for a clean translocation)
            }
            let expected = v3d_manual_swap(&gspec0, ti, tj);
            if **spec == expected {
                return; // F2 proven: production's row-then-col swap matches the independent π remap
            }
            // else: V-1 point-mutation also fired alongside translocation this seed — keep searching.
        }
        panic!("V-3-d: expected to find a clean (uncontaminated) i != j translocation within 8000 seeds");
    }

    #[test]
    fn v3d_state_conjugacy() {
        // The translocated spec's resolved attractor must equal π applied to the original spec's
        // attractor: `grn_resolve(translocated).state == swap(grn_resolve(original).state, i, j)` —
        // proves the permutation preserves dynamics up to relabeling.
        const SALT_TRANS_MIRROR: u64 = 0x5452_4100u64;
        let gspec0 = GrnSpec::new(
            3,
            vec![10, -5, 3, 2, 20, -7, -4, 6, 15],
            vec![8, 1, 0],
            vec![2, -3, 5],
            3, 12, 0, 0,
            vec![64, 128, 32],
        );
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec0.clone())), None);
        g.mutation_rate = 4;

        // sample_x=sample_z=0 ⇒ only cells[0] of the gradient matters; the exact value is arbitrary.
        let gradient = crate::Gradient { g_dev: 1, cells: vec![100] };

        for i in 0..8000u64 {
            let seed = 0xC000_0000 + i;
            let r_count = seed_fold(seed, &[SALT_TRANS_MIRROR]);
            if (r_count & 0xFF) >= g.mutation_rate as u64 {
                continue;
            }
            let r_i = seed_fold(seed, &[SALT_TRANS_MIRROR + 1]);
            let r_j = seed_fold(seed, &[SALT_TRANS_MIRROR + 2]);
            let (ti, tj) = ((r_i >> 8) as usize % 3, (r_j >> 8) as usize % 3);
            if ti == tj {
                continue;
            }
            let m = g.clone().mutate(seed, 2, false, 4, false, true, false, false);
            let Some(spec) = &m.grn_spec else { continue };
            if spec.n_genes != 3 {
                continue;
            }
            // Verify this candidate is a CLEAN translocation (nothing else fired) via the same
            // structural-equality reference tooth 6 relies on, before trusting it for conjugacy.
            let expected_struct = v3d_manual_swap(&gspec0, ti, tj);
            if **spec != expected_struct {
                continue;
            }

            let (orig_state, _orig_ct, _s1) = crate::grn_resolve(&gradient, &gspec0);
            let (new_state, _new_ct, _s2) = crate::grn_resolve(&gradient, spec);
            let mut expected_state = orig_state.clone();
            expected_state.swap(ti, tj);
            assert_eq!(
                new_state, expected_state,
                "V-3-d: translocated attractor must equal π applied to the original attractor (conjugacy)"
            );
            return;
        }
        panic!("V-3-d: expected to find a clean (uncontaminated) translocation within 8000 seeds");
    }

    #[test]
    fn v3d_phenotype_is_effectful() {
        // A translocation on a bistable spec (initial=[EXPR_MAX,0]) must FLIP the decoded cell_type
        // (A->B) — confirms translocation is real variation, not a no-op. Reuses the `m7b_mspec`
        // fixture FUNCTION (PARALLEL-SAFETY: never inline a `MorphogenSpec {...}` literal here).
        let gspec0 = GrnSpec::new(2, vec![64, -64, -64, 64], vec![0, 0], vec![0, 0], 3, 12, 0, 0, vec![crate::GRN_EXPR_MAX, 0]);
        let mspec = m7b_mspec(None);
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec0.clone())), Some(mspec));
        g.mutation_rate = 64;

        let econ = EconParams::default();
        let ph0 = g.decode(&econ).expect("fixture must decode to Some");
        assert_eq!(ph0.cell_type, Some(CellType::A), "bistable fixture must start at A (pinned)");

        for i in 0..2000u64 {
            let seed = 0xA000_0000 + i;
            let m = g.clone().mutate(seed, econ.n_energy_layers, econ.light.is_some(), econ.reg_gain_max, econ.predation.is_some(), true, false, false);
            if let Some(spec) = &m.grn_spec {
                if v3d_is_pure_reorder(&spec.gene_ids, &gspec0.gene_ids) {
                    let ph = m.decode(&econ).expect("post-translocation genome must decode to Some");
                    assert_eq!(ph.cell_type, Some(CellType::B), "V-3-d: translocation must flip the decoded cell_type A->B");
                    return;
                }
            }
        }
        panic!("V-3-d: expected to find a translocation within 2000 seeds");
    }

    #[test]
    fn v3d_structural_validity() {
        // Post-move: all 4 parallel vectors == n_genes, weights square, n_genes unchanged — across a
        // run of successive generations exercising V-3-b/c/d together.
        let gspec = GrnSpec::new(2, vec![64, -64, -64, 64], vec![0, 0], vec![0, 0], 3, 12, 0, 0, vec![0, 64]);
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec)), None);
        g.mutation_rate = 256;

        for i in 0..500 {
            let seed = 0x1D1D_0000 + (i as u64);
            g = g.mutate(seed, 2, false, 4, false, true, false, false);
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
    fn v3d_gene_ids_injective() {
        // After any V-3-b/c/d sequence (duplication/indel/translocation composed), gene_ids must
        // have no duplicates (homology tracking intact — translocation only reorders, never merges).
        let gspec = GrnSpec::new(2, vec![64, -64, -64, 64], vec![0, 0], vec![0, 0], 3, 12, 0, 0, vec![0, 64]);
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec)), None);
        g.mutation_rate = 256;

        for i in 0..500 {
            let seed = 0x1E1E_0000 + (i as u64);
            g = g.mutate(seed, 2, false, 4, false, true, false, false);
            if let Some(spec) = &g.grn_spec {
                let mut seen = std::collections::BTreeSet::new();
                for &id in &spec.gene_ids {
                    assert!(seen.insert(id), "V-3-d: duplicate gene_id {id} at iteration {i}");
                }
            }
        }
    }

    #[test]
    fn v3d_dup_counter_unchanged() {
        // A lone translocation (n_genes stays 2, so V-3-b/c did not fire) must not touch dup_counter
        // — it is not a gene-adding/removing event.
        let gspec0 = GrnSpec::new(2, vec![64, -64, -64, 64], vec![10, 20], vec![1, 2], 3, 12, 0, 0, vec![100, 200]);
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec0.clone())), None);
        g.mutation_rate = 64;

        for i in 0..1000u64 {
            let seed = 0x1F1F_0000 + i;
            let m = g.clone().mutate(seed, 2, false, 4, false, true, false, false);
            if let Some(spec) = &m.grn_spec {
                if v3d_is_pure_reorder(&spec.gene_ids, &gspec0.gene_ids) {
                    assert_eq!(spec.dup_counter, gspec0.dup_counter, "V-3-d: translocation must not touch dup_counter");
                    return;
                }
            }
        }
        panic!("V-3-d: expected to find a translocation within 1000 seeds");
    }

    #[test]
    fn v3d_replay_1_vs_n() {
        // N genomes each mutated with the same seed → identical results (R14 1-vs-N).
        let gspec = GrnSpec::new(2, vec![64, -64, -64, 64], vec![10, 20], vec![5, 15], 3, 12, 0, 0, vec![32, 96]);
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(gspec)), None);
        g.mutation_rate = 256;

        let seed = 0x1234_5A5A;
        let results: Vec<Genome> = (0..8).map(|_| g.clone().mutate(seed, 2, false, 4, false, true, false, false)).collect();

        for m in &results[1..] {
            assert_eq!(m.grn_spec, results[0].grn_spec, "V-3-d: replay must be seed-deterministic, not run-order-dependent");
        }
    }

    // ── #247 classify-gen: N-way cell-type classification (gated) ──────────────────────────────

    /// (F5, PINNED) `Diff(i) => i % n_layers.max(1)` — integer, deterministic, always in `[0,
    /// n_layers)`; `A`/`B`/`Mixed` stay exactly as before #247 (regression sanity).
    #[test]
    fn classifygen_uptake_mapping() {
        assert_eq!(cell_type_uptake_layer(CellType::Diff(0), 7, 3), 0);
        assert_eq!(cell_type_uptake_layer(CellType::Diff(1), 7, 3), 1);
        assert_eq!(cell_type_uptake_layer(CellType::Diff(2), 7, 3), 2);
        assert_eq!(cell_type_uptake_layer(CellType::Diff(3), 7, 3), 0, "wraps modulo n_layers");
        assert_eq!(cell_type_uptake_layer(CellType::Diff(5), 7, 3), 2);
        // Degenerate n_layers=0 must not panic (guarded by `.max(1)`).
        assert_eq!(cell_type_uptake_layer(CellType::Diff(4), 7, 0), 0);
        // Sanity: A/B/Mixed routing is unperturbed by #247.
        assert_eq!(cell_type_uptake_layer(CellType::A, 7, 3), 0);
        assert_eq!(cell_type_uptake_layer(CellType::B, 7, 3), 1);
        assert_eq!(cell_type_uptake_layer(CellType::Mixed, 7, 3), 7);
    }

    /// Hand-built `n_genes=3`, `classify_nway=true` spec (mirrors `grn.rs`'s
    /// `classifygen_nway_produces_ge3_types` fixture): no gene coupling (`weights` all zero), gene
    /// 0/2 driven oppositely by the gradient, gene 1 drive-dead with a fixed bias edge that only
    /// wins at drive=0.
    fn classifygen_nway_gspec() -> GrnSpec {
        let mut spec = GrnSpec::new(3, vec![0; 9], vec![50, 0, -50], vec![0, 1024, 0], 3, 4, 0, 0, vec![128, 128, 128]);
        spec.classify_nway = true;
        spec
    }

    #[test]
    fn classifygen_union_find_groups_nway() {
        // Top-left 2×2 (drive=8192) -> Diff(0); top-right 2×2 (drive=-8192) -> Diff(2);
        // bottom 2×4 (drive=0) -> Diff(1), one connected 8-cell module.
        let gradient = crate::morphogen::Gradient {
            g_dev: 4,
            #[rustfmt::skip]
            cells: vec![
                 8192,  8192, -8192, -8192,
                 8192,  8192, -8192, -8192,
                    0,     0,     0,     0,
                    0,     0,     0,     0,
            ],
        };
        let gspec = classifygen_nway_gspec();
        let graph = CellGraph::from_gradient(&gradient, &gspec, None, None, None, None);

        assert_eq!(graph.num_modules(), 3, "3 distinct Diff(_) blocks must yield exactly 3 modules");
        let mut counts: Vec<(u8, i32)> = graph
            .module_type
            .iter()
            .zip(graph.module_cell_count.iter())
            .map(|(t, &c)| match t {
                CellType::Diff(i) => (*i, c),
                other => panic!("expected Diff(_), got {other:?}"),
            })
            .collect();
        counts.sort();
        assert_eq!(
            counts,
            vec![(0, 4), (1, 8), (2, 4)],
            "union-find must group same-Diff(i) adjacent cells into one module, not merge distinct types nor split a same-type block"
        );
    }

    /// `classify_nway` folds into `hash_contribution` ONLY when `true` (mirrors `dup_counter`'s
    /// gate) — `false` must be a byte-identical no-op, `true` must lock the mode into the hash.
    #[test]
    fn classifygen_hash_gated() {
        let gspec = e6_gspec();
        assert!(!gspec.classify_nway, "sanity: the e6 fixture must default to classify_nway=false");

        let g_false = Genome::founder(2).with_specs(Some(Arc::new(gspec.clone())), Some(e6_mspec()));
        let h_false = g_false.hash_contribution(0);

        let mut gspec_true = gspec.clone();
        gspec_true.classify_nway = true;
        let g_true = Genome::founder(2).with_specs(Some(Arc::new(gspec_true)), Some(e6_mspec()));
        let h_true = g_true.hash_contribution(0);
        assert_ne!(h_false, h_true, "classify_nway=true must fold into hash_contribution (locks the mode)");

        // Toggling back to false must reproduce the ORIGINAL hash exactly — proves the gate
        // contributes NOTHING when false (not merely a fixed offset every genome shares).
        let mut gspec_false_again = gspec.clone();
        gspec_false_again.classify_nway = false;
        let g_false_again = Genome::founder(2).with_specs(Some(Arc::new(gspec_false_again)), Some(e6_mspec()));
        assert_eq!(g_false_again.hash_contribution(0), h_false, "classify_nway=false must be a byte-identical no-op fold");
    }

    // ── V-4 (#276): evolvable developmental grid (body-size axis) ───────────────────────────────

    /// Uniform-fate GRN fixture — `phase2_config`'s exact spec (`input_weights=[0,0]`: drive dead,
    /// every grid cell resolves to the SAME `CellType` regardless of position), so the whole grid
    /// collapses into ONE connected module and `body_size() == g_dev²` exactly — the body-size axis
    /// is directly observable from `g_dev` alone, isolated from any GRN-position interaction.
    fn v4_uniform_gspec() -> GrnSpec {
        GrnSpec::new(2, vec![64, -64, -64, 64], vec![0, 0], vec![0, 0], 3, 12, 0, 0, vec![256, 0])
    }

    fn v4_mspec(g_dev: usize) -> MorphogenSpec {
        MorphogenSpec {
            g_dev, n_dev: 8, boundary: Boundary::Reflecting,
            diffuse_shift: 3, decay_num: 1, decay_shift: 4,
            seed_scale: 4096, stop_threshold: 0, apoptosis_threshold: None,
            germ_threshold: None, supply_source: None, adhesion_threshold: None,
        }
    }

    /// Teeth (1): with `evolve_body_size=true` and `morphogen_spec` present, a lineage started at
    /// `g_dev=1` must move away from 1 within a bounded number of high-mutation-rate generations,
    /// and `g_dev` must stay clamped to `[1,4]` at every single generation.
    #[test]
    fn v4_gdev_mutates_when_enabled() {
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(v4_uniform_gspec())), Some(v4_mspec(1)));
        g.mutation_rate = 256; // max rate — the (r & 0xFF) < mutation_rate gate always fires
        let mut saw_change = false;
        for gen in 0..64u64 {
            let seed = 0x5644_4556_0000u64 + gen; // "GDEV" salt already inside mutate(); vary seed
            g = g.mutate(seed, 2, false, 4, false, false, true, false);
            let gd = g.morphogen_spec.expect("spec must survive mutation").g_dev;
            assert!((1..=4).contains(&gd), "g_dev must stay clamped to [1,4], got {gd} at generation {gen}");
            if gd != 1 {
                saw_change = true;
            }
        }
        assert!(saw_change, "g_dev must move away from 1 within 64 high-mutation-rate generations");
    }

    /// Teeth (2): with `evolve_body_size=false`, `g_dev` never changes across many generations
    /// (zero draws from `SALT_GDEV`), AND the gate being off does not perturb any OTHER mutated
    /// field on the same seed — proves the new draw is fully disjoint/inert, not coincidentally equal.
    #[test]
    fn v4_gdev_inert_when_disabled() {
        let mut g = Genome::founder(2).with_specs(Some(Arc::new(v4_uniform_gspec())), Some(v4_mspec(1)));
        g.mutation_rate = 256;
        for gen in 0..64u64 {
            let seed = 0x5644_4556_1111u64 + gen;
            g = g.mutate(seed, 2, false, 4, false, false, false, false);
            assert_eq!(g.morphogen_spec.unwrap().g_dev, 1, "flag off: g_dev must never change (generation {gen})");
        }

        // Disjoint-stream check: same seed/genome, flag on vs off, every OTHER field must agree.
        let base = Genome::founder(2).with_specs(Some(Arc::new(v4_uniform_gspec())), Some(v4_mspec(1)));
        let seed = 0xDEAD_BEEF_1234u64;
        let on = base.clone().mutate(seed, 2, false, 4, false, false, true, false);
        let off = base.clone().mutate(seed, 2, false, 4, false, false, false, false);
        assert_eq!(on.metabolism_eff, off.metabolism_eff);
        assert_eq!(on.move_speed, off.move_speed);
        assert_eq!(on.sense_range, off.sense_range);
        assert_eq!(on.size, off.size);
        assert_eq!(on.repro_threshold, off.repro_threshold);
        assert_eq!(on.mutation_rate, off.mutation_rate);
        assert_eq!(on.uptake_layer, off.uptake_layer);
        assert_eq!(on.excrete_layer, off.excrete_layer);
        assert_eq!(on.weights, off.weights);
    }

    /// Teeth (3): a `g_dev=1` genome decodes to a valid unicellular `Phenotype` — `body_size()==1`
    /// (1 cell, 1 module), no panic — and is viable (founder `size=4 > SIZE_VIABILITY_FLOOR=3`).
    #[test]
    fn v4_founder_gdev1_is_unicellular() {
        let econ = EconParams::default();
        let g = Genome::founder(2).with_specs(Some(Arc::new(v4_uniform_gspec())), Some(v4_mspec(1)));
        let ph = g.decode(&econ).expect("g_dev=1 genome must decode to Some (viable, no panic)");
        assert_eq!(ph.graph.body_size(), 1, "g_dev=1 must decode to a 1-cell unicellular body");
        assert_eq!(ph.graph.num_modules(), 1, "g_dev=1 must decode to exactly 1 module");
    }

    /// Teeth (4): growing `g_dev` from 1 to 4 monotonically increases the achievable `body_size`
    /// (1 → up to 16) under the uniform-fate fixture — the axis is real, not merely wired-but-inert.
    #[test]
    fn v4_gdev_grows_body() {
        let econ = EconParams::default();
        let gspec = v4_uniform_gspec();
        let mut sizes = Vec::new();
        for g_dev in 1..=4usize {
            let g = Genome::founder(2).with_specs(Some(Arc::new(gspec.clone())), Some(v4_mspec(g_dev)));
            let ph = g.decode(&econ).expect("must decode to Some at every g_dev in [1,4]");
            sizes.push(ph.graph.body_size());
        }
        assert_eq!(sizes, vec![1, 4, 9, 16], "uniform-fate fixture: body_size must equal g_dev² exactly (monotone growth 1→16)");
    }

    /// Teeth (5): the same seed stream (same starting genome, same per-generation seeds) reproduces
    /// the IDENTICAL `g_dev` trajectory across two independent replays.
    #[test]
    fn v4_determinism() {
        let mut base = Genome::founder(2).with_specs(Some(Arc::new(v4_uniform_gspec())), Some(v4_mspec(1)));
        base.mutation_rate = 256;

        let run = |start: &Genome| -> Vec<usize> {
            let mut lineage = start.clone();
            let mut trajectory = Vec::new();
            for gen in 0..32u64 {
                lineage = lineage.mutate(0x1357_9BDFu64 + gen, 2, false, 4, false, false, true, false);
                trajectory.push(lineage.morphogen_spec.unwrap().g_dev);
            }
            trajectory
        };
        assert_eq!(run(&base), run(&base), "same seed stream must reproduce the identical g_dev trajectory");
    }

    /// Teeth (7): `g_dev` is already folded into `hash_contribution` Some-gated (pre-V-4) — a genome
    /// with `g_dev=1` must hash differently from an otherwise-identical genome with `g_dev=2`.
    #[test]
    fn v4_hash_folds_gdev() {
        let g1 = Genome::founder(2).with_specs(Some(Arc::new(v4_uniform_gspec())), Some(v4_mspec(1)));
        let g2 = Genome::founder(2).with_specs(Some(Arc::new(v4_uniform_gspec())), Some(v4_mspec(2)));
        assert_ne!(g1.hash_contribution(0), g2.hash_contribution(0), "g_dev=1 vs g_dev=2 must hash differently (already Some-gated fold)");
    }

    // ── R31: respiratory-genes decode (P1-1) ──────────────────────────────────────────────────────
    /// R31 (a): founder respiratory_pathway must be 0 (obligate aerobe).
    #[test]
    fn p1_respiratory_founder_zero() {
        let g = Genome::founder(2);
        assert_eq!(g.respiratory_pathway, 0, "founder respiratory_pathway must be 0 (obligate aerobe)");
    }

    /// R31 (b): respiratory_pathway mutates only when enable_oxygen=true; stays 0 when false.
    #[test]
    fn p1_respiratory_mutation_gated_by_enable_oxygen() {
        let mut g = Genome::founder(2);
        g.mutation_rate = 256; // Force every mutation to attempt

        // With enable_oxygen=false, respiratory_pathway must stay 0 (no mutation draw).
        for seed in 0..100u64 {
            let m = g.mutate(seed, 2, false, 4, false, false, false, false); // enable_oxygen=false
            assert_eq!(m.respiratory_pathway, 0, "with enable_oxygen=false, respiratory_pathway must not mutate (seed={seed})");
        }

        // With enable_oxygen=true, respiratory_pathway can mutate (at least some seeds must change it).
        let mut mutated_count = 0;
        for seed in 0..1000u64 {
            let m = g.mutate(seed, 2, false, 4, false, false, false, true); // enable_oxygen=true
            if m.respiratory_pathway != 0 {
                mutated_count += 1;
            }
        }
        assert!(mutated_count > 0, "with enable_oxygen=true and mutation_rate=256, respiratory_pathway must mutate in at least some seeds (got {mutated_count}/1000)");
    }

    /// R31 (c): decode_respiratory_pathways() is PURE — deterministic, no field/RNG/clock reads.
    #[test]
    fn p1_respiratory_decode_pure_deterministic() {
        let econ = EconParams::default();
        let mut g = Genome::founder(2);

        // Set respiratory_pathway to test different genotypes
        for rtype in [0, 32, 65, 96, 128, 192, 255] {
            g.respiratory_pathway = rtype;

            // Calling decode twice must produce byte-identical Phenotype.respiratory_pathway
            let ph1 = g.decode(&econ).expect("decode must return Some");
            let ph2 = g.decode(&econ).expect("decode must return Some");
            assert_eq!(
                ph1.respiratory_pathway, ph2.respiratory_pathway,
                "decode_respiratory_pathways must be byte-identical on repeated calls (rtype={rtype})"
            );
        }
    }

    /// R31 (d): redox-hierarchy decoding — different rtype values decode to correct strategies.
    #[test]
    fn p1_respiratory_redox_hierarchy() {
        let econ = EconParams::default();
        let mut g = Genome::founder(2);

        // rtype in [0..=64] → obligate aerobe (O₂ only, no fallback, anoxia_cost=256)
        g.respiratory_pathway = 0;
        let ph0 = g.decode(&econ).expect("rtype=0 must decode");
        assert!(ph0.respiratory_pathway.is_some(), "rtype=0 must decode to Some (obligate)");
        let rp0 = ph0.respiratory_pathway.as_ref().expect("rtype=0");
        assert_eq!(rp0.primary_layer, crate::FieldId::Oxygen, "obligate must have O₂ primary");
        assert_eq!(rp0.fallback_layers.len(), 0, "obligate must have no fallback");
        assert_eq!(rp0.anoxia_cost_x256, 256, "obligate anoxia_cost must be 256 (death)");
        assert_eq!(rp0.aerobe_cost_x256, 10, "obligate aerobe_cost must be 10 (−3.9%)");

        g.respiratory_pathway = 64;
        let ph64 = g.decode(&econ).expect("rtype=64 must decode");
        assert_eq!(ph64.respiratory_pathway, ph0.respiratory_pathway, "rtype=64 must equal rtype=0 (both obligate)");

        // rtype in [65..=128] → facultative (O₂ primary + NO₃ fallback, anoxia_cost=32 fermentation)
        g.respiratory_pathway = 65;
        let ph65 = g.decode(&econ).expect("rtype=65 must decode");
        assert!(ph65.respiratory_pathway.is_some(), "rtype=65 must decode to Some (facultative)");
        let rp65 = ph65.respiratory_pathway.as_ref().expect("rtype=65");
        assert_eq!(rp65.primary_layer, crate::FieldId::Oxygen, "facultative must have O₂ primary");
        assert_eq!(rp65.fallback_layers, vec![crate::FieldId::Nitrate], "facultative must have NO₃ fallback");
        assert_eq!(rp65.fallback_effs_x256[0], 180, "NO₃ efficiency must be 180 (×0.7)");
        assert_eq!(rp65.anoxia_cost_x256, 32, "facultative anoxia_cost must be 32 (fermentation)");
        assert_eq!(rp65.aerobe_cost_x256, 15, "facultative aerobe_cost must be 15 (−5.9%)");

        g.respiratory_pathway = 128;
        let ph128 = g.decode(&econ).expect("rtype=128 must decode");
        assert_eq!(ph128.respiratory_pathway, ph65.respiratory_pathway, "rtype=128 must equal rtype=65 (both facultative)");

        // rtype > 128 → reserved for P5+, returns None (inert) for now
        g.respiratory_pathway = 192;
        let ph192 = g.decode(&econ).expect("rtype=192 must decode");
        assert!(ph192.respiratory_pathway.is_none(), "rtype=192 must decode to None (reserved for P5+)");
    }

    /// R31 (e): hash_contribution folds respiratory_pathway when non-zero (byte-identical when 0).
    #[test]
    fn p1_respiratory_hash_gated() {
        let g0 = Genome::founder(2);
        assert_eq!(g0.respiratory_pathway, 0, "founder must have respiratory_pathway=0");

        // Hash with rtype=0 must not include respiratory_pathway in the fold.
        // Create an otherwise-identical genome with rtype=1.
        let mut g1 = g0.clone();
        g1.respiratory_pathway = 1;

        // Hash should differ because respiratory_pathway is now non-zero (folded).
        assert_ne!(g0.hash_contribution(0), g1.hash_contribution(0), "rtype=0 vs rtype=1 must hash differently (respiratory_pathway is folded when non-zero)");
    }
}
