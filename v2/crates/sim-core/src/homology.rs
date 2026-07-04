//! V-3-e: genome-distance metric (#37 indirect-variation) — a pure, integer, NEAT-style
//! compatibility distance between two GRN genomes. Used ONLY as read-only diversity telemetry
//! (`Telemetry::genome_diversity`, populated by `stages::stage_observe`); no sim decision consumes
//! it — the reproductive-barrier/speciation consumer is deferred to a later phase.
//!
//! **Alignment is by lineage `gene_id`, NOT array position (F2/F6).** V-3-b/c/d (duplication /
//! indel / translocation) reorder genes within a lineage, so the same `gene_id` can sit at
//! different positions in the two genomes being compared. Each genome's own `id -> position` map
//! is built once (`BTreeMap`, O(n) — never `HashMap`, `no_float_guard` bans it) and used to look
//! up THAT genome's own array entries for every matched id.

use crate::GrnSpec;
use std::collections::BTreeMap;

/// Scales the disjoint-gene-count term. Pinned (issue #246): C1=10000 gives headroom so
/// `C1*D/N >= 1` for `D>=1` up to `N<=10000`, covering realistic variable-length growth without
/// the integer division truncating a real disjoint-gene difference down to invisibility.
const C1: i64 = 10000;
/// Weights the per-matched-gene content term (`W_sum`).
const C2: i64 = 1;

/// `gene_id -> position` map for one genome's gene arrays.
fn id_positions(spec: &GrnSpec) -> BTreeMap<u32, usize> {
    spec.gene_ids.iter().enumerate().map(|(pos, &id)| (id, pos)).collect()
}

/// NEAT-style compatibility distance between two GRN genomes, aligned by lineage `gene_id`.
///
/// `distance = (C1 * D) / N + C2 * W_sum`, where:
/// - `D` = disjoint gene count = `(n_a - |matched|) + (n_b - |matched|)`.
/// - `N` = `max(n_a, n_b)` (0 if both genomes are empty — guarded, though `GrnSpec::new` enforces
///   `n_genes >= 2` in practice).
/// - `W_sum` = over every MATCHED gene (present in both, by `gene_id`), the sum of integer `|Δ|`
///   across `input_weights`/`bias`/`initial` (each read at EACH genome's own position for that
///   id) plus the gene's `weights` ROW restricted to columns whose `gene_id` is ALSO matched in
///   both genomes (homologous edges only).
///
/// Pure integer (no float, no truncation-to-0 short-circuit); `d(a,a) == 0`; symmetric by
/// construction (set-based id matching + `|Δ|` is commutative).
pub fn genome_distance(a: &GrnSpec, b: &GrnSpec) -> i64 {
    let n_a = a.n_genes;
    let n_b = b.n_genes;
    let n = n_a.max(n_b) as i64;
    if n == 0 {
        return 0;
    }

    let pos_a = id_positions(a);
    let pos_b = id_positions(b);
    let matched: Vec<u32> = pos_a.keys().filter(|id| pos_b.contains_key(id)).copied().collect();
    let d = (n_a - matched.len()) as i64 + (n_b - matched.len()) as i64;

    let mut w_sum: i64 = 0;
    for &id in &matched {
        let ia = pos_a[&id];
        let ib = pos_b[&id];
        w_sum += (a.input_weights[ia] as i64 - b.input_weights[ib] as i64).abs();
        w_sum += (a.bias[ia] as i64 - b.bias[ib] as i64).abs();
        w_sum += (a.initial[ia] as i64 - b.initial[ib] as i64).abs();
        for &col_id in &matched {
            let ja = pos_a[&col_id];
            let jb = pos_b[&col_id];
            let wa = a.weights[ia * n_a + ja] as i64;
            let wb = b.weights[ib * n_b + jb] as i64;
            w_sum += (wa - wb).abs();
        }
    }

    (C1 * d) / n + C2 * w_sum
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a `GrnSpec` directly (struct literal, not `GrnSpec::new`) so tests can set custom
    /// `gene_ids` — needed for the disjoint-gene and reordered-pair fixtures below.
    fn spec(n_genes: usize, weights: Vec<i32>, input_weights: Vec<i32>, bias: Vec<i32>, initial: Vec<i32>, gene_ids: Vec<u32>) -> GrnSpec {
        GrnSpec {
            n_genes,
            weights,
            input_weights,
            bias,
            shift: 3,
            max_steps: 12,
            sample_x: 0,
            sample_z: 0,
            initial,
            gene_ids,
            dup_counter: 0,
        }
    }

    fn zero2(gene_ids: Vec<u32>) -> GrnSpec {
        spec(2, vec![0, 0, 0, 0], vec![0, 0], vec![0, 0], vec![0, 0], gene_ids)
    }

    // ── Tooth 1: identity ────────────────────────────────────────────────────────────────────────

    #[test]
    fn v3e_distance_identity() {
        let fixtures = [
            zero2(vec![0, 1]),
            spec(3, vec![1, 2, 3, 4, 5, 6, 7, 8, 9], vec![10, 20, 30], vec![1, 2, 3], vec![100, 150, 200], vec![0, 1, 2]),
            spec(2, vec![32, -32, -32, 32], vec![0, 0], vec![0, 0], vec![144, 112], vec![0, 1]),
        ];
        for f in &fixtures {
            assert_eq!(genome_distance(f, f), 0, "d(a,a) must be 0 for {:?}", f.gene_ids);
        }
    }

    // ── Tooth 2: symmetry ────────────────────────────────────────────────────────────────────────

    #[test]
    fn v3e_distance_symmetry() {
        let fixtures = [
            zero2(vec![0, 1]),
            spec(3, vec![1, 2, 3, 4, 5, 6, 7, 8, 9], vec![10, 20, 30], vec![1, 2, 3], vec![100, 150, 200], vec![0, 1, 2]),
            spec(3, vec![0; 9], vec![0, 0, 0], vec![0, 0, 0], vec![0, 0, 0], vec![0, 1, 99]),
            spec(2, vec![32, -32, -32, 32], vec![0, 0], vec![0, 0], vec![144, 112], vec![0, 1]),
        ];
        for a in &fixtures {
            for b in &fixtures {
                assert_eq!(
                    genome_distance(a, b), genome_distance(b, a),
                    "d(a,b) must equal d(b,a) for {:?} vs {:?}", a.gene_ids, b.gene_ids
                );
            }
        }
    }

    // ── Tooth 3: known fixture, hand-computed ───────────────────────────────────────────────────

    /// One disjoint gene, everything else zero (isolates the `D` term). `a`: ids [0,1,2]; `b`:
    /// ids [0,1,99] — gene id 2/99 don't match. matched={0,1} → D=(3-2)+(3-2)=2, N=max(3,3)=3.
    /// C1*D/N = 10000*2/3 = 6666 (integer truncation). W_sum=0 (all matched entries are zero).
    /// distance = 6666.
    #[test]
    fn v3e_known_fixture_disjoint_gene() {
        let a = spec(3, vec![0; 9], vec![0, 0, 0], vec![0, 0, 0], vec![0, 0, 0], vec![0, 1, 2]);
        let b = spec(3, vec![0; 9], vec![0, 0, 0], vec![0, 0, 0], vec![0, 0, 0], vec![0, 1, 99]);
        assert_eq!(genome_distance(&a, &b), 6666);
    }

    /// All comparable matched-gene entries differ by exactly 1 (isolates the `W_sum` term, D=0).
    /// 2 matched genes × (input_weights + bias + initial = 3, + weights row over 2 matched cols =
    /// 2) = 5 diffs/gene × 2 genes = 10. distance = 0 + 10 = 10.
    #[test]
    fn v3e_known_fixture_uniform_weight_diff() {
        let a = zero2(vec![0, 1]);
        let b = spec(2, vec![1, 1, 1, 1], vec![1, 1], vec![1, 1], vec![1, 1], vec![0, 1]);
        assert_eq!(genome_distance(&a, &b), 10);
    }

    /// F2 — the discriminating reorder tooth: `b` is `a` with genes 0,1 SWAPPED (same translocation
    /// permutation V-3-d applies: swap rows+cols+parallel vectors+gene_ids). Same genome, different
    /// storage order. Index-by-gene_id must recover distance 0; index-by-array-position would see
    /// every entry as different and report a large false distance.
    #[test]
    fn v3e_known_fixture_reordered_pair_is_zero_distance() {
        let a = spec(2, vec![10, 20, 30, 40], vec![100, 200], vec![300, 400], vec![500, 600], vec![0, 1]);
        // b = translocation swap(0,1) applied to a: swap rows, then cols, then parallel vectors.
        let b = spec(2, vec![40, 30, 20, 10], vec![200, 100], vec![400, 300], vec![600, 500], vec![1, 0]);
        assert_eq!(
            genome_distance(&a, &b), 0,
            "reordered-but-identical genome must have distance 0 (id-based, not position-based, comparison)"
        );
    }

    // ── Tooth 4: monotonic in disjoint count ────────────────────────────────────────────────────

    #[test]
    fn v3e_monotonic_disjoint() {
        let a = zero2(vec![0, 1]);
        let b_same = zero2(vec![0, 1]);
        // b_extended = b_same plus one extra disjoint gene (zero-content, like a V-3-c insert).
        let b_extended = spec(3, vec![0; 9], vec![0, 0, 0], vec![0, 0, 0], vec![0, 0, 0], vec![0, 1, 55]);

        let d_before = genome_distance(&a, &b_same);
        let d_after = genome_distance(&a, &b_extended);
        assert!(
            d_after > d_before,
            "adding a disjoint gene must strictly increase distance: before={d_before} after={d_after}"
        );
    }
}
