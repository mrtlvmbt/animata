//! Deterministic INTEGER rng. No floating point → cross-arch bit-identical (the mechanism behind
//! the M0 x86-only golden, F7). Mirrors v1's `seed_fold(world_seed, &[SALT, id, tick])` shape.

/// splitmix64 — a fast, well-distributed integer mixer. Pure integer ops, identical on every arch.
#[inline]
pub fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Fold a world seed with a salt + identifying parts into one deterministic stream value.
/// The single RNG entry point for the core (R10): same `(seed, parts)` ⇒ same value, always.
#[inline]
pub fn seed_fold(world_seed: u64, parts: &[u64]) -> u64 {
    let mut h = world_seed;
    for &p in parts {
        h = splitmix64(h ^ p);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_fold_is_pure() {
        assert_eq!(seed_fold(7, &[1, 2, 3]), seed_fold(7, &[1, 2, 3]));
        assert_ne!(seed_fold(7, &[1, 2, 3]), seed_fold(7, &[1, 2, 4]));
        assert_ne!(seed_fold(8, &[1, 2, 3]), seed_fold(7, &[1, 2, 3]));
    }
}
