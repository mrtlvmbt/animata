//! Shared deterministic PRNG (splitmix64) and the seed-folding discipline the simulation
//! relies on for replay. There is intentionally NO `rand`/`getrandom` dependency — all
//! randomness is a pure function of an explicit `u64` seed, so a run is reproducible from
//! its world seed alone (see the sim's determinism invariants).

/// splitmix64 finalizer: a bijective avalanche mix of one `u64`. Pure (no state), so it is
/// safe to fold seeds with. Same constants as the original reference / the erosion droplet RNG.
pub fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Fold several `u64` fields into one seed **non-commutatively** — `a^b` collides
/// (`id^tick == tick^id`), which would give two different creatures the SAME mutation; mixing
/// after each `wrapping_add` breaks that symmetry. Order of `fields` is significant.
pub fn seed_fold(base: u64, fields: &[u64]) -> u64 {
    let mut s = splitmix64(base);
    for &f in fields {
        s = splitmix64(s.wrapping_add(f));
    }
    s
}

/// A tiny deterministic PRNG: a splitmix64 stream from a `u64` seed. Drawing advances the
/// state, so a fixed draw order from a fixed seed always yields the same sequence.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform `f32` in `[0, 1)`.
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform `f32` in `[-1, 1)`.
    pub fn signed(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix_is_pure_and_avalanches() {
        // Pure: same input → same output; adjacent inputs → very different outputs.
        assert_eq!(splitmix64(42), splitmix64(42));
        let (a, b) = (splitmix64(1), splitmix64(2));
        assert_ne!(a, b);
        assert!((a ^ b).count_ones() > 10, "adjacent seeds barely differ — weak mix");
    }

    #[test]
    fn seed_fold_is_non_commutative() {
        // The whole point: swapping two fields must change the seed (else id^tick collisions).
        assert_ne!(seed_fold(7, &[3, 5]), seed_fold(7, &[5, 3]));
        assert_eq!(seed_fold(7, &[3, 5]), seed_fold(7, &[3, 5])); // but deterministic
    }

    #[test]
    fn rng_stream_is_deterministic() {
        let mut a = Rng::new(123);
        let mut b = Rng::new(123);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        let mut r = Rng::new(9);
        for _ in 0..1000 {
            let u = r.unit();
            assert!((0.0..1.0).contains(&u));
        }
    }
}
