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

// ---- FNV-1a 64-bit fold (the determinism-checksum primitive, PR1 lock) ----
// Integer-only mixing: floats are folded by their bit pattern (`f32::to_bits`), NEVER by
// floating-point addition — sum-of-floats is not associative, so a parallel/reordered reduce
// would give a different checksum and the lock would lie (F2). FNV is order-sensitive, which is
// what we want: the checksum is over a fixed, deterministic field order.

// (Consumed by the determinism-checksum tests now; by the metrics-registry checksum metric in PR5.)
/// FNV-1a 64-bit offset basis — the checksum's start value.
#[allow(dead_code)]
pub const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
#[allow(dead_code)]
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Fold one `u64` (8 bytes, little-endian) into an FNV-1a accumulator.
#[allow(dead_code)]
pub fn fnv_fold_u64(h: &mut u64, v: u64) {
    for b in v.to_le_bytes() {
        *h ^= b as u64;
        *h = h.wrapping_mul(FNV_PRIME);
    }
}

/// Fold one `u32` (e.g. an `f32::to_bits()` pattern) into an FNV-1a accumulator.
#[allow(dead_code)]
pub fn fnv_fold_u32(h: &mut u64, v: u32) {
    for b in v.to_le_bytes() {
        *h ^= b as u64;
        *h = h.wrapping_mul(FNV_PRIME);
    }
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
#[path = "rng_tests.rs"]
mod tests;
