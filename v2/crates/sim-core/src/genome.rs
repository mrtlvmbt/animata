//! Direct-encoded Ф0 genome — **8 integer traits + photo-regulation gene (D′-2b)**. Integer
//! everywhere: mutation is an integer perturbation, the metabolic cost is an integer function of
//! size, and the genome folds into the deterministic state hash. No float in the genetics layer.
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

/// The six Ф0 traits + two B-2 layer-targeting traits (research/13 §2). Ranges are clamped on
/// mutation; all integer.
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
}

/// Phase-2 E-1: cold, lean cache of the decoded genome traits consumed by hot-path stages.
///
/// Attached at every spawn site (founders + children) so a `&Phenotype` query is REQUIRED
/// (not optional) — a missed spawn site makes that entity invisible to the consumer stage,
/// which is detectable via a shifted golden (the correct detection signal, not a runtime panic).
///
/// **Ф0 content**: only `uptake_layer` — the single raw integer field consumed by
/// `stage_interactions`. Future slices (E-2/E-3) add morphogen-derived fields here.
///
/// NOT folded into `hash_contribution`: phenotype is a deterministic cold derivative of the
/// genome that is already in the hash; double-hashing is redundant (plan §2/§6, R19).
#[derive(bevy_ecs::prelude::Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Phenotype {
    /// Layer index the entity will eat from (direct copy of `Genome::uptake_layer` for Ф0).
    pub uptake_layer: i32,
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
            // Test-only E-1/E-4 injection flag — always false in production.
            #[cfg(test)]
            force_decode_none: false,
        }
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
    pub fn mutate(&self, stream: u64, n_layers: usize, has_light: bool, reg_gain_max: i32) -> Genome {
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
        g
    }

    /// Brain-weight L1 genetic distance — the speciation metric (M5/criterion 2).
    /// Protected by the `deny(float_arithmetic)` guard on this file. Integer, arch-independent.
    pub fn brain_weight_l1(&self, other: &Genome) -> i64 {
        self.weights.iter().zip(other.weights.iter())
            .map(|(a, b)| (*a as i64 - *b as i64).abs())
            .sum()
    }

    /// Decode this genome to a `Phenotype` (Phase-2 E-1 seam entry point).
    ///
    /// For Ф0 (direct-encoded genomes) the decode is a trivial 1:1 projection — the phenotype
    /// carries the raw integer traits the hot-path stages actually consume. Returns `Some` for
    /// every valid Ф0 genome. The `None` arm is wired so the BirthDeath gate can skip stillborns
    /// without code-change when E-4 introduces real viability logic.
    ///
    /// Pure and deterministic: no RNG, no clock, no thread-dependent work.
    /// Phenotype is NOT folded into `hash_contribution` (it is a cold derivative of Genome;
    /// genome IS in the hash, decode is deterministic ⟹ phenotype is fully determined — plan §2/R19).
    pub fn decode(&self) -> Option<Phenotype> {
        // E-1/E-4 test injection: when force_decode_none=true, the gate fires the skip path.
        // In Ф0 production this branch is compiled OUT entirely (#[cfg(test)]).
        #[cfg(test)]
        if self.force_decode_none {
            return None;
        }
        Some(Phenotype {
            uptake_layer: self.uptake_layer,
        })
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
        assert_eq!(g.mutate(123, 2, false, 4), g.mutate(123, 2, false, 4));
        for s in 0..200u64 {
            let m = g.mutate(s, 2, false, 4);
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
            let m = g.mutate(s, 2, true, 4);
            assert!((0..=256).contains(&m.photo_gain), "photo_gain must be in [0,256]");
            assert!((-4..=4).contains(&m.reg_gain), "reg_gain must be in [-reg_gain_max, +reg_gain_max]");
        }
        // reg_gain_max=0 locks regulation OFF even when has_light=true.
        for s in 0..200u64 {
            let m = g.mutate(s, 2, true, 0);
            assert_eq!(m.reg_gain, 0, "reg_gain must stay 0 when reg_gain_max=0 (D′-2c lock)");
        }
        // L=1 bench path: layers clamped to 0.
        let g1 = Genome::founder(1);
        assert_eq!(g1.excrete_layer, 0);
        let m1 = g1.mutate(0, 1, false, 0);
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
            let a = g.decode();
            let b = g.decode();
            assert_eq!(a, b, "decode must be deterministic: same genome → same Phenotype");
        }
        // Also holds for a mutated genome.
        let g = Genome::founder(2);
        let mutated = g.mutate(0xDEAD_BEEF, 2, true, 4);
        assert_eq!(mutated.decode(), mutated.decode(), "decode deterministic on mutated genome");
    }

    /// Every Ф0 genome decodes to Some — Ф0 viability is unconditional.
    #[test]
    fn decode_some_for_all_phi0_founders() {
        for n_layers in [1usize, 2, 3] {
            let g = Genome::founder(n_layers);
            assert!(g.decode().is_some(), "founder genome must decode to Some (Ф0 trivial case)");
        }
    }

    /// Ф0 decode is a 1:1 projection: phenotype.uptake_layer == genome.uptake_layer.
    /// Proves the consumer's field is bit-exact — no computed quantity or truncation.
    #[test]
    fn phenotype_uptake_layer_matches_genome() {
        let g = Genome::founder(2);
        let ph = g.decode().expect("Ф0 must decode to Some");
        assert_eq!(ph.uptake_layer, g.uptake_layer,
            "Phenotype::uptake_layer must equal Genome::uptake_layer for Ф0");
        // Also for mutated genome — projection stays 1:1 regardless of trait value.
        for s in 0..50u64 {
            let m = g.mutate(s, 2, false, 0);
            let mph = m.decode().expect("mutated Ф0 must decode to Some");
            assert_eq!(mph.uptake_layer, m.uptake_layer,
                "1:1 projection must hold after mutation (seed={s})");
        }
    }

    /// None-gate wiring test: proves the REAL `Genome::decode()` — the function called by
    /// `stage_birth_death` at `let Some(child_phenotype) = child_genome.decode() else { continue; }`
    /// — returns `None` when the E-4 injection flag is set, and `Some` otherwise.
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
        assert!(g.decode().is_some(), "Ф0 genome must decode to Some (gate passes)");

        // E-4 injection: set force_decode_none=true → THE SAME decode() returns None.
        // This is the identical function stage_birth_death calls on child_genome.
        let mut stillborn = Genome::founder(2);
        stillborn.force_decode_none = true;
        assert!(stillborn.decode().is_none(),
            "force_decode_none=true must make decode() return None (gate fires → spawn skipped)");

        // Mutated children inherit the flag (mutate copies *self) → entire lineage stays stillborn.
        let mutated_child = stillborn.mutate(0xDEAD_CAFE, 2, false, 0);
        assert!(mutated_child.force_decode_none,
            "force_decode_none must be inherited by mutate() so the entire lineage stays stillborn");
        assert!(mutated_child.decode().is_none(),
            "inherited flag: child decode() also returns None (lineage-level stillbirth)");

        // Normal mutated child (force_decode_none=false) returns Some — mutation alone never triggers None.
        let normal_child = g.mutate(0xDEAD_CAFE, 2, false, 0);
        assert!(!normal_child.force_decode_none, "normal child must NOT inherit false as true");
        assert!(normal_child.decode().is_some(), "normal child decode() must return Some");
    }
}
