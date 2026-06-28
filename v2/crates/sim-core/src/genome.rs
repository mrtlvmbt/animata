//! Direct-encoded Ф0 genome — **8 integer traits + 2 evolvable regulatory fields** (D-slice/GRN seed,
//! issue #169). No GRN W-matrix/morphogenesis (Phase 2). Integer everywhere: mutation is an integer
//! perturbation, the metabolic cost is a pure-integer function, and the genome folds into the
//! deterministic state hash. No float in the genetics layer (enforced by the lint below).
// Guard: no float arithmetic in the conserved layer (M0/F2). Complements the token-grep in
// no_float_guard.rs: `float_arithmetic` catches operations on inferred-float types that the grep
// misses (e.g. `let x = 1.5; x + 1.0` where no `f32`/`f64` keyword appears).
#![deny(clippy::float_arithmetic)]

use crate::{brain_w_ho, brain_w_ih, fnv_mix, seed_fold, BRAIN_WEIGHTS};
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

/// The eight Ф0 traits + two B-2 layer-targeting traits + two D-slice regulatory fields (GRN seed).
/// Ranges are clamped on mutation; all integer.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Genome {
    /// Resource→energy conversion efficiency, as a fraction of 256 (0..=256).
    pub metabolism_eff: i32,
    /// Cells moved per tick (movement is metabolically priced).
    pub move_speed: i32,
    /// Gradient-sensing radius in cells (sensing is priced). Raw trait; BOTH consumers use the
    /// regulated expression `sense_range_eff` (cached in `Sensors.effort`) instead of this directly.
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
    /// Evolved brain weights for the FIXED topology (D-Brain-1) — `int8` Q1.7, packed `W_ih·W_hh·W_ho`
    /// (layout = the shared [`crate::brain_w_ih`]/`brain_w_hh`/`brain_w_ho` indices). Inherited and
    /// mutated exactly like the eight Ф0 traits; the `brain` crate reads this vector during inference.
    /// Resident here (genome-SoA in the ECS) so no genome→weights repack happens on a Brain tick.
    pub weights: [i8; BRAIN_WEIGHTS],
    // ── D-slice: evolvable regulatory gene (GRN seed, issue #169) ───────────────────────────────
    /// Substrate setpoint S₀ for `sense_range` regulation (D-slice). Founder ≈ R̄=79 (Slice-C median);
    /// range `0..=256` brackets the real layer-0 field distribution so both regulation directions
    /// are viable (reg is born dead if `sign(local − setpoint)` is constant over the real field).
    pub reg_setpoint: i32,
    /// Signed regulatory slope for `sense_range` (D-slice). Founder = 0 (regulation OFF).
    /// Sign is EVOLVABLE — the population discovers the beneficial direction (§8 pitfall guard).
    /// Range `[−reg_gain_max, +reg_gain_max]` (default ±4): gentle steps on `sense_range` (0..=8).
    pub reg_gain: i32,
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
            // D-slice: regulation OFF at founding — evolution discovers the beneficial direction.
            // reg_setpoint ≈ R̄=79 (Slice-C plateau median) so the setpoint sits inside the field
            // distribution, keeping both sign(local − setpoint) directions viable.
            reg_setpoint: 80,
            reg_gain: 0,
        }
    }

    /// Integer metabolic cost units `size^(3/4)`.
    pub fn metab_units(&self) -> i64 {
        size_pow_three_quarters(self.size)
    }

    /// Regulated effective sensing expression (D-slice / GRN seed, issue #169).
    ///
    /// `eff = (sense_range + reg_gain · sign(local_resource − reg_setpoint)).clamp(0, 8)`
    ///
    /// Pure integer: `signum` of an `i64` difference yields −1, 0, or +1 — no float, no division
    /// (R13). Clamped to the same `0..=8` range as `sense_range`. At `reg_gain = 0` (the founder):
    /// `eff == sense_range` for ALL `local_resource` — behaviourally inert (§8 pitfall guard).
    ///
    /// Called ONCE per tick in `stage_sense` and cached in `Sensors.effort`. Both consumers
    /// (gradient radius in `stage_sense` and sense cost in `stage_metabolism`) read the cached
    /// value — single computation, two reads, cost and benefit cannot diverge.
    pub fn sense_range_eff(&self, local_resource: i64) -> i32 {
        let s = local_resource.saturating_sub(self.reg_setpoint as i64).signum() as i32;
        (self.sense_range + self.reg_gain * s).clamp(0, 8)
    }

    /// Deterministic mutated clone. `stream` is a per-birth seeded value; each trait draws a disjoint
    /// integer perturbation in `{-1,0,+1}` gated by `mutation_rate`, then is clamped to range.
    /// `n_layers` clamps layer traits to `0..=n_layers-1` — must equal the field's actual layer
    /// count (guaranteed by `build_sim` setting `econ.n_layers = config.n_layers`).
    /// `reg_gain_max` clamps `reg_gain` to `[−reg_gain_max, +reg_gain_max]`; set to 0 to lock
    /// regulation OFF (the A/B control line in the selective-value experiment).
    pub fn mutate(&self, stream: u64, n_layers: usize, reg_gain_max: i32) -> Genome {
        let mut g = *self;
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
        // Brain weights mutate the same way — their RNG draws come AFTER the 8 Ф0 traits
        // (disjoint salt stream), keeping the existing 8-trait salt sequence byte-identical (§5.2).
        for (wi, w) in g.weights.iter_mut().enumerate() {
            let r = seed_fold(stream, &[0x7700_0000 + wi as u64]); // "w" + weight index
            if (r & 0xFF) < self.mutation_rate as u64 {
                let delta = ((r >> 8) % 3) as i64 - 1; // -1,0,+1
                *w = (*w as i64 + delta).clamp(-127, 127) as i8;
            }
        }
        // D-slice: reg fields mutate LAST — after brain weights — with disjoint "reg\0" salts
        // (0x7265_6700 + i). Keeps the 8-trait and brain-weight salt sequences byte-identical;
        // the only trajectory change from D is through these two new fields (§5.2 stream hygiene).
        let reg_lo = -reg_gain_max;
        let reg_traits: [(&mut i32, i32, i32); 2] = [
            (&mut g.reg_setpoint, 0, 256),
            (&mut g.reg_gain, reg_lo, reg_gain_max),
        ];
        for (i, (slot, lo, hi)) in reg_traits.into_iter().enumerate() {
            let r = seed_fold(stream, &[0x7265_6700 + i as u64]); // "reg\0" + index
            if (r & 0xFF) < self.mutation_rate as u64 {
                let delta = ((r >> 8) % 3) as i32 - 1;
                *slot = (*slot + delta).clamp(lo, hi);
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
        // D-slice: reg fields must be in the hash — a field outside the lock silently decouples
        // mutation from state, making the trajectory irreproducible across saves (F9).
        h = fnv_mix(h, self.reg_setpoint as u64);
        h = fnv_mix(h, self.reg_gain as u64);
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
        assert_eq!(g.mutate(123, 2, 4), g.mutate(123, 2, 4));
        for s in 0..200u64 {
            let m = g.mutate(s, 2, 4);
            assert!((0..=256).contains(&m.metabolism_eff));
            assert!((1..=32).contains(&m.size));
            assert!((0..=1).contains(&m.uptake_layer));
            assert!((0..=1).contains(&m.excrete_layer));
            assert!((-4..=4).contains(&m.reg_gain), "reg_gain {} OOB", m.reg_gain);
            assert!((0..=256).contains(&m.reg_setpoint), "reg_setpoint {} OOB", m.reg_setpoint);
        }
        // L=1 bench path: layers clamped to 0.
        let g1 = Genome::founder(1);
        assert_eq!(g1.excrete_layer, 0);
        let m1 = g1.mutate(0, 1, 4);
        assert_eq!(m1.uptake_layer, 0);
        assert_eq!(m1.excrete_layer, 0);
    }
}
