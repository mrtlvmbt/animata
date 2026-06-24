//! Direct-encoded Ф0 genome (D3) — **6 integer traits**, no GRN/morphogenesis (Phase 2). Integer
//! everywhere: mutation is an integer perturbation, the metabolic cost is an integer function of
//! size, and the genome folds into the deterministic state hash. No float in the genetics layer.

use crate::{fnv_mix, seed_fold};
use bevy_ecs::prelude::Component;

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

/// The six Ф0 traits (research/13 §2). Ranges are clamped on mutation; all integer.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
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
}

impl Genome {
    /// The founder phenotype — viable (feeds more than it burns at abundance).
    pub fn founder() -> Self {
        Genome {
            metabolism_eff: 200,
            move_speed: 1,
            sense_range: 1,
            size: 4,
            repro_threshold: 1500,
            mutation_rate: 32,
        }
    }

    /// Integer metabolic cost units `size^(3/4)`.
    pub fn metab_units(&self) -> i64 {
        size_pow_three_quarters(self.size)
    }

    /// Deterministic mutated clone. `stream` is a per-birth seeded value; each trait draws a disjoint
    /// integer perturbation in `{-1,0,+1}` gated by `mutation_rate`, then is clamped to range.
    pub fn mutate(&self, stream: u64) -> Genome {
        let mut g = *self;
        let traits: [(&mut i32, i32, i32); 6] = [
            (&mut g.metabolism_eff, 0, 256),
            (&mut g.move_speed, 0, 8),
            (&mut g.sense_range, 0, 8),
            (&mut g.size, 1, 32),
            (&mut g.repro_threshold, 200, 5000),
            (&mut g.mutation_rate, 0, 256),
        ];
        for (i, (slot, lo, hi)) in traits.into_iter().enumerate() {
            let r = seed_fold(stream, &[0x6D75_7400 + i as u64]); // "mut" + trait index
            // Gate the mutation by mutation_rate/256, then a signed unit step.
            if (r & 0xFF) < self.mutation_rate as u64 {
                let delta = ((r >> 8) % 3) as i32 - 1; // -1,0,+1
                *slot = (*slot + delta).clamp(lo, hi);
            }
        }
        g
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
        ] {
            h = fnv_mix(h, v as u64);
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
        let g = Genome::founder();
        assert_eq!(g.mutate(123), g.mutate(123));
        for s in 0..200u64 {
            let m = g.mutate(s);
            assert!((0..=256).contains(&m.metabolism_eff));
            assert!((1..=32).contains(&m.size));
        }
    }
}
