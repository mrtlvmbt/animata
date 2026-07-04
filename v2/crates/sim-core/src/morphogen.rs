//! Phase-2 **E-2**: the morphogen SUBSTRATE — a standalone, deterministic, INTEGER
//! reaction-diffusion routine that will feed E-3's GRN. **Prod-inert**: nothing in this module is
//! called by `Genome::decode` or any spawn site (0 sites changed); it exists here proven by unit
//! tests over a production `MorphogenSpec` fixture, so E-3 reuses the type without a rewrite.
//!
//! The morphogen is INFORMATION, not conserved mass: it never touches `FieldStore`/`FieldScatter`/
//! the Σ-balance (R13/R15), and its output is never folded into `hash_contribution` — even once E-3
//! wires it in, it stays a cold derivative of the (already-hashed) genome (plan §2/R19).
//!
//! Determinism (plan §3, same resolution as the M3 fixed-point brain): pure integer arithmetic (this
//! module is float-free — see the lint below), a symmetric full-double-buffer stencil (every cell
//! reads the OLD grid and writes the NEW one, so the traversal order does not affect the result — we
//! still pin row-major for explicitness), a fixed step count OR an exact-integer stop predicate
//! (never a float-ε or wall-clock stop), and overflow that SATURATES (never wraps — a wrap would be
//! order-dependent/non-deterministic in spirit even though a single-threaded sum is not literally
//! order-dependent; more importantly a wrap silently aliases to a wrong value while a saturate stays
//! bounded and detectable).
// Guard: no float arithmetic in the morphogen substrate (mirrors genome.rs / energy.rs). This is a
// clippy lint — `tests.yml` runs `cargo nextest`, not `cargo clippy`, so CI does NOT gate on it; the
// PR notes a clean local `cargo clippy -p sim-core` run.
#![deny(clippy::float_arithmetic)]

use crate::Genome;

/// Boundary condition at the grid edge (plan §3). `Reflecting` (no-flux): a missing neighbor is
/// treated as equal to the cell itself (zero flux across the edge). `Absorbing`: a missing neighbor
/// is treated as concentration `0` (the edge drains toward zero).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boundary {
    Reflecting,
    Absorbing,
}

/// Production configuration for one morphogen solve — grid size, step policy, boundary, and the
/// integer diffusion/decay/seed constants. NOT `#[cfg(test)]`: E-2 instantiates this with a test
/// *value*; E-3 reuses the *type* unchanged when it wires the morphogen into `decode` (F9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MorphogenSpec {
    /// Grid side length `G_dev` (grid is `g_dev × g_dev`). Provisional perf ceiling: `≤ 4`.
    pub g_dev: usize,
    /// Maximum reaction-diffusion steps `N_dev`. Provisional perf ceiling: `≤ 16`.
    pub n_dev: u32,
    pub boundary: Boundary,
    /// Diffusion flux shift: each cell exchanges `(neighbor − self) >> diffuse_shift` with every
    /// present neighbor. CFL-safe (non-oscillating) requires `neighbor_count / 2^diffuse_shift ≤ 1`;
    /// for a 4-neighbor stencil `diffuse_shift ≥ 3` (4/8 = 1/2 ≤ 1), mirroring the conserved-field
    /// flux diffusion of M2 (`fields::CpuFieldStore`).
    pub diffuse_shift: u32,
    /// Multiplicative decay applied to non-source cells after diffusion: `new -= (new * decay_num) >>
    /// decay_shift`. Pure integer analogue of the M2 signal field's `u *= (1−λ)`.
    pub decay_num: i64,
    pub decay_shift: u32,
    /// Genome trait → seed concentration scale: `seed = genome.size * seed_scale`, clamped to
    /// `±VALUE_MAX`. The genome↔morphogen *semantic* mapping is E-3's job (Ф0 `size` is a stand-in
    /// so E-2 genuinely reads `&Genome`, per the required signature).
    pub seed_scale: i32,
    /// Exact-integer early-stop predicate: if `> 0`, the solve stops once `max|Δ| ≤ stop_threshold`
    /// (computed on the integer grid — never a float-ε compare). `0` disables early stop (always runs
    /// the full `n_dev` steps).
    pub stop_threshold: i32,
    /// **M7-b apoptosis gate.** `None` (default, every shipped spec) disables the pass entirely —
    /// `CellGraph::from_gradient` is byte-identical to M7-a. `Some(t)`: a per-cell death predicate,
    /// evaluated on gene 0 of the per-cell GRN-resolved `state` (`state[0] < t`), marks the cell dead
    /// BEFORE union-find labeling (F3, PINNED — integer-only, no float, no multi-term formula).
    pub apoptosis_threshold: Option<i32>,
    /// **M7-c germ/soma gate.** `None` (default, every shipped spec) disables the pass entirely —
    /// `CellGraph::from_gradient` leaves `module_is_germ` all-`false`, byte-identical to M7-b.
    /// `Some(t)` (test-only): a LIVE module (post Step-3 collection, dead cells already excluded) is
    /// classified GERM iff `module_cell_count[mid] <= t` (small=germ, large=soma) — a module-level
    /// integer predicate, PINNED, no float, no morphogen re-traversal.
    pub germ_threshold: Option<i32>,
}

/// The morphogen output: a local, position-indexed integer concentration field (row-major,
/// `g_dev × g_dev`). This is what E-3's GRN samples positionally — "the gradient" of plan §2/§3.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gradient {
    pub g_dev: usize,
    pub cells: Vec<i32>,
}

impl Gradient {
    #[inline]
    pub fn at(&self, x: usize, z: usize) -> i32 {
        self.cells[z * self.g_dev + x]
    }
}

/// Concentration ceiling (documented, i32-domain). The overflow guard below saturates the
/// per-step accumulator to this bound before casting to `i32` — never a raw `as i32` truncation
/// (which would silently WRAP on out-of-range input; see `overflow_saturates_not_wraps`).
const VALUE_MAX: i64 = 1_000_000;

/// One cell's diffusion+decay update given its OLD value, its (present) neighbors' OLD values, and
/// the spec. Pure integer; the sum is accumulated in `i64` (comfortably wide: worst case here is
/// `4 · 2 · VALUE_MAX + VALUE_MAX ≈ 9·VALUE_MAX ≈ 9e6`, `≪ i64::MAX` — the guard below is
/// belt-and-braces documentation of that margin, and the ceiling it enforces is what the overflow
/// unit test drives past on purpose).
fn diffuse_decay_cell(old_self: i32, neighbors: &[i32], spec: &MorphogenSpec) -> i32 {
    let mut acc: i64 = old_self as i64;
    for &n in neighbors {
        let flux = (n as i64 - old_self as i64) >> spec.diffuse_shift;
        acc += flux;
    }
    // Decay (post-diffusion), integer multiplicative shrink toward 0.
    let decay = (acc * spec.decay_num) >> spec.decay_shift;
    acc -= decay;

    // SATURATE (never wrap): clamp the wide accumulator into the documented i32-domain ceiling
    // BEFORE casting. A bare `acc as i32` on an out-of-range i64 truncates/wraps in Rust — exactly
    // the non-deterministic-looking aliasing this guard exists to prevent. For any realistic
    // genome-derived input (seed/decay/diffuse bounded as documented on `MorphogenSpec`) this clamp
    // never engages — see the module-level peak calc; `overflow_saturates_not_wraps` drives an
    // adversarial input straight into it on purpose.
    acc.clamp(-VALUE_MAX, VALUE_MAX) as i32
}

/// Gather up to 4 axis-neighbor OLD values for `(x, z)` per the boundary condition.
fn neighbors_of(grid: &[i32], g: usize, x: usize, z: usize, boundary: Boundary) -> Vec<i32> {
    let get = |xx: isize, zz: isize| -> i32 {
        if xx < 0 || zz < 0 || xx as usize >= g || zz as usize >= g {
            match boundary {
                Boundary::Reflecting => grid[z * g + x], // no-flux: neighbor == self
                Boundary::Absorbing => 0,                // edge drains to 0
            }
        } else {
            grid[zz as usize * g + xx as usize]
        }
    };
    let (xi, zi) = (x as isize, z as isize);
    vec![get(xi - 1, zi), get(xi + 1, zi), get(xi, zi - 1), get(xi, zi + 1)]
}

/// Run the morphogen solve and additionally return the number of steps actually taken (≤
/// `spec.n_dev`; `< n_dev` only if `stop_threshold > 0` and the grid converged early). Exposed for
/// the deterministic-termination test; [`morphogen`] is the primary entry point.
pub fn morphogen_steps(genome: &Genome, spec: &MorphogenSpec) -> (Gradient, u32) {
    let g = spec.g_dev.max(1);
    let seed = ((genome.size as i64) * (spec.seed_scale as i64)).clamp(-VALUE_MAX, VALUE_MAX) as i32;
    let source = (0usize, 0usize); // pinned source cell (plan §3: "a pinned source cell")

    let mut old = vec![0i32; g * g];
    old[source.1 * g + source.0] = seed;
    let mut new = vec![0i32; g * g];

    let mut steps_taken = 0u32;
    for _step in 0..spec.n_dev {
        let mut max_delta: i64 = 0;
        // Canonical traversal order: row-major (z outer, x inner) — pinned for explicitness (the
        // full-double-buffer stencil below is order-independent by construction: it only ever reads
        // `old`, never `new`).
        for z in 0..g {
            for x in 0..g {
                let i = z * g + x;
                if (x, z) == source {
                    new[i] = seed; // fixed (Dirichlet) source: reset every step, never decays
                } else {
                    let ns = neighbors_of(&old, g, x, z, spec.boundary);
                    new[i] = diffuse_decay_cell(old[i], &ns, spec);
                }
                let delta = (new[i] as i64 - old[i] as i64).abs();
                if delta > max_delta {
                    max_delta = delta;
                }
            }
        }
        std::mem::swap(&mut old, &mut new);
        steps_taken += 1;
        if spec.stop_threshold > 0 && max_delta <= spec.stop_threshold as i64 {
            break; // exact-integer stop predicate — never a float-ε/wall-clock stop
        }
    }
    (Gradient { g_dev: g, cells: old }, steps_taken)
}

/// Pure integer morphogen reaction-diffusion (plan §2/§3). Reads only `genome` + `spec` — no
/// `FieldStore`, no global sim state, no RNG, no clock. The grid is local to the call and discarded;
/// only the [`Gradient`] survives.
pub fn morphogen(genome: &Genome, spec: &MorphogenSpec) -> Gradient {
    morphogen_steps(genome, spec).0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Genome;

    /// E-2 fixture — respects the provisional perf ceiling `G_dev ≤ 4`, `N_dev ≤ 16` (PR states the
    /// chosen values). Shift ≥ 3 keeps the 4-neighbor stencil CFL-stable (mirrors M2's fields crate).
    fn fixture_spec() -> MorphogenSpec {
        MorphogenSpec {
            g_dev: 4,
            n_dev: 8,
            boundary: Boundary::Reflecting,
            diffuse_shift: 3,
            decay_num: 1,
            decay_shift: 4, // ~6.25% decay per step
            seed_scale: 4096,
            stop_threshold: 0, // always run the full 8 steps (deterministic fixed count)
            apoptosis_threshold: None,
            germ_threshold: None,
        }
    }

    fn fixture_genome() -> Genome {
        Genome::founder(1)
    }

    // ── determinism teeth ────────────────────────────────────────────────────────────────────────

    #[test]
    fn morphogen_is_deterministic_across_repeated_calls() {
        let (g, spec) = (fixture_genome(), fixture_spec());
        let a = morphogen(&g, &spec);
        let b = morphogen(&g, &spec);
        assert_eq!(a, b, "same (genome, spec) must produce byte-identical gradients");
    }

    #[test]
    fn morphogen_reproduces_bytewise_on_rerun() {
        // A second, independent re-run (fresh call stack / fresh Vec allocations) must reproduce the
        // exact same bytes — the pinned canonical cell order holds across runs, not just aliases.
        let (g, spec) = (fixture_genome(), fixture_spec());
        let run_a: Vec<u8> = morphogen(&g, &spec).cells.iter().flat_map(|v| v.to_le_bytes()).collect();
        let run_b: Vec<u8> = morphogen(&g, &spec).cells.iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(run_a, run_b, "re-run must reproduce byte-for-byte");
    }

    #[test]
    fn different_genome_diverges() {
        let spec = fixture_spec();
        let founder = morphogen(&Genome::founder(1), &spec);
        let mutated = morphogen(&Genome::founder(1).mutate(0xDEAD_BEEF, 1, false, 0, false, false), &spec);
        // Not asserting inequality unconditionally (mutation could no-op on `size`); assert the
        // function actually depends on the genome by varying `size` directly.
        let mut bigger = Genome::founder(1);
        bigger.size = founder_size_plus(bigger.size);
        let bigger_out = morphogen(&bigger, &spec);
        assert_ne!(bigger_out, founder, "seed derives from genome.size — a different size must diverge");
        let _ = mutated; // exercised for realism; not asserted (mutation may be a no-op on `size`)
    }

    fn founder_size_plus(size: i32) -> i32 {
        (size + 4).min(32)
    }

    // ── deterministic termination ────────────────────────────────────────────────────────────────

    #[test]
    fn fixed_step_count_runs_exactly_n_dev_when_stop_disabled() {
        let (g, mut spec) = (fixture_genome(), fixture_spec());
        spec.stop_threshold = 0;
        let (_grad, steps) = morphogen_steps(&g, &spec);
        assert_eq!(steps, spec.n_dev, "stop_threshold=0 must always run the full fixed step count");
    }

    #[test]
    fn exact_integer_stop_predicate_terminates_early_and_deterministically() {
        let (g, mut spec) = (fixture_genome(), fixture_spec());
        spec.n_dev = 100; // generous cap
        spec.stop_threshold = 1; // exact integer compare — never float-ε
        let (grad_a, steps_a) = morphogen_steps(&g, &spec);
        let (grad_b, steps_b) = morphogen_steps(&g, &spec);
        assert!(steps_a < spec.n_dev, "must converge and stop before the generous cap");
        assert_eq!(steps_a, steps_b, "early-stop step count must be deterministic");
        assert_eq!(grad_a, grad_b, "early-stop result must be deterministic");
    }

    // ── overflow: saturate, never wrap ───────────────────────────────────────────────────────────

    #[test]
    fn overflow_saturates_not_wraps() {
        let spec = MorphogenSpec { diffuse_shift: 0, decay_num: 0, decay_shift: 1, ..fixture_spec() };
        // Drive the raw pre-clamp accumulator far past VALUE_MAX by forcing a huge one-sided flux
        // (diffuse_shift=0 ⇒ no attenuation; a MAX-vs-MIN neighbor gap forces the accumulator past
        // the documented ceiling).
        let out = diffuse_decay_cell(0, &[i32::MAX, i32::MAX, i32::MAX, i32::MAX], &spec);
        assert_eq!(out, VALUE_MAX as i32, "must SATURATE to the positive ceiling, not wrap");
        let out_neg = diffuse_decay_cell(0, &[i32::MIN, i32::MIN, i32::MIN, i32::MIN], &spec);
        assert_eq!(out_neg, -(VALUE_MAX as i32), "must SATURATE to the negative ceiling, not wrap");
        // A naive `as i32` cast of the raw (unclamped) i64 sum would alias to a small/negative
        // number here (classic wraparound) — prove our result is nowhere near that aliased value.
        let raw_unclamped_would_be: i64 = 4 * (i32::MAX as i64);
        assert_ne!(out as i64, raw_unclamped_would_be as i32 as i64, "must not equal the wrapped alias");
    }

    // ── structural stencil property (F10 — not merely "not constant") ───────────────────────────

    #[test]
    fn concentration_decays_monotonically_from_source() {
        let (g, spec) = (fixture_genome(), fixture_spec());
        let grad = morphogen(&g, &spec);
        // Source is (0,0). A correct diffusion+decay stencil with a fixed positive source and a
        // small grid/step-count (well short of full equilibration) must not let a farther cell along
        // the source row/column exceed a nearer one — a stencil bug (sign error, wrong neighbor
        // lookup, order dependence) breaks this.
        for x in 0..grad.g_dev - 1 {
            assert!(
                grad.at(x, 0) >= grad.at(x + 1, 0),
                "row 0 must decay monotonically from the source: at({x},0)={} < at({},0)={}",
                grad.at(x, 0),
                x + 1,
                grad.at(x + 1, 0)
            );
        }
        for z in 0..grad.g_dev - 1 {
            assert!(
                grad.at(0, z) >= grad.at(0, z + 1),
                "column 0 must decay monotonically from the source: at(0,{z})={} < at(0,{})={}",
                grad.at(0, z),
                z + 1,
                grad.at(0, z + 1)
            );
        }
        assert!(grad.at(0, 0) > grad.at(grad.g_dev - 1, grad.g_dev - 1), "source must dominate the far corner");
    }

    // ── golden vector — byte-exact regression lock (F10) ─────────────────────────────────────────

    #[test]
    fn golden_vector_matches_pinned_gradient() {
        let grad = morphogen(&fixture_genome(), &fixture_spec());
        assert_eq!(grad.g_dev, 4);
        // Pinned on this implementation (integer, deterministic — re-derive by running this test
        // with the assertion temporarily replaced by a print if the stencil is intentionally changed).
        const GOLDEN: [i32; 16] = [
            16384, 5382, 1506, 383, 5382, 2391, 736, 183, 1506, 736, 227, 50, 383, 183, 50, 8,
        ];
        assert_eq!(grad.cells, GOLDEN, "morphogen golden vector drifted — stencil/arithmetic regression");
    }

    // ── purity: no hidden state, reads only its inputs ───────────────────────────────────────────

    #[test]
    fn morphogen_ignores_unrelated_genome_fields() {
        // Two genomes differing only in a trait NOT used by the seed derivation (move_speed) must
        // produce identical gradients — proves the function reads only what it documents reading.
        let mut a = Genome::founder(1);
        let mut b = Genome::founder(1);
        a.move_speed = 1;
        b.move_speed = 7;
        assert_eq!(morphogen(&a, &fixture_spec()), morphogen(&b, &fixture_spec()));
    }
}
