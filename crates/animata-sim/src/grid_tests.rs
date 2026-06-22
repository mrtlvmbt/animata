use super::*;
use crate::rng::Rng;
use glam::vec2;

fn brute_nearest(points: &[Vec2], from: Vec2, max: f32, ok: impl Fn(usize) -> bool) -> Option<usize> {
    points
        .iter()
        .enumerate()
        .filter(|&(i, &p)| ok(i) && (p - from).length() <= max)
        .min_by(|(_, a), (_, b)| {
            (**a - from).length_squared().partial_cmp(&(**b - from).length_squared()).unwrap()
        })
        .map(|(i, _)| i)
}

#[test]
fn nearest_within_matches_brute_force() {
    let mut rng = Rng::new(1);
    let pts: Vec<Vec2> = (0..500).map(|_| vec2(rng.unit() * 200.0, rng.unit() * 200.0)).collect();
    let mut g = SpatialGrid::default();
    g.rebuild(&pts, 200.0, 200.0, 12.0);
    for _ in 0..200 {
        let from = vec2(rng.unit() * 200.0, rng.unit() * 200.0);
        let r = rng.unit() * 60.0;
        let (got, _) = g.nearest2_within(&pts, from, r, |_| true, |_| false);
        let want = brute_nearest(&pts, from, r, |_| true);
        // Same point, or (ties aside) the same distance.
        match (got, want) {
            (Some(a), Some(b)) => {
                let (da, db) = ((pts[a] - from).length(), (pts[b] - from).length());
                assert!((da - db).abs() < 1e-3, "grid {da} vs brute {db}");
            }
            (None, None) => {}
            _ => panic!("grid {got:?} vs brute {want:?}"),
        }
    }
}

// The predator-skip gate's safety invariant: `sum_in_radius` over a per-cell count is a conservative
// SUPERSET of what `nearest2_within` can find, so it MUST never return 0 when the real threat scan
// would return `Some`. A false 0 would let a creature wrongly skip a real threat → broken determinism.
#[test]
fn sum_in_radius_never_false_skips_a_real_match() {
    let mut rng = Rng::new(9);
    let pts: Vec<Vec2> = (0..500).map(|_| vec2(rng.unit() * 200.0, rng.unit() * 200.0)).collect();
    let mut g = SpatialGrid::default();
    g.rebuild(&pts, 200.0, 200.0, 12.0);
    // A "predator" subset, and the per-cell count the sim would build from it (same `cell_index`).
    let is_pred: Vec<bool> = (0..pts.len()).map(|i| i % 7 == 0).collect();
    let mut counts = vec![0u32; g.num_cells()];
    for (i, &p) in pts.iter().enumerate() {
        if is_pred[i] {
            counts[g.cell_index(p)] += 1;
        }
    }
    for _ in 0..400 {
        let from = vec2(rng.unit() * 200.0, rng.unit() * 200.0);
        let r = rng.unit() * 60.0;
        // The real scan's threat result (ok_b = "is a predator").
        let (_, threat) = g.nearest2_within(&pts, from, r, |_| false, |i| is_pred[i]);
        let sum = g.sum_in_radius(&counts, from, r);
        // The gate would skip iff sum == 0; that is allowed ONLY when the real scan found nothing.
        if threat.is_some() {
            assert!(sum > 0, "false skip: scan found {threat:?} within {r} but sum_in_radius == 0");
        }
    }
}

#[test]
fn nearest2_respects_both_predicates() {
    let pts = vec![vec2(0.0, 0.0), vec2(5.0, 0.0), vec2(10.0, 0.0), vec2(50.0, 0.0)];
    let mut g = SpatialGrid::default();
    g.rebuild(&pts, 100.0, 100.0, 8.0);
    // From the origin point: nearest "even index" and nearest "odd index" within 20.
    let (a, b) = g.nearest2_within(&pts, vec2(0.0, 0.0), 20.0, |i| i % 2 == 0 && i != 0, |i| i % 2 == 1);
    assert_eq!(a, Some(2)); // even, index 2 at x=10
    assert_eq!(b, Some(1)); // odd, index 1 at x=5
}
