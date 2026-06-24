//! Deterministic state hashing (R19). FNV-1a over a stably-ordered fold — no external dep, pure
//! integer, cross-arch identical.

use bevy_ecs::entity::Entity;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// One FNV-1a step over the little-endian bytes of `v`.
#[inline]
pub fn fnv_mix(mut h: u64, v: u64) -> u64 {
    for b in v.to_le_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// THE single per-entity reduction point of the core (R10/R19).
///
/// `collect → sort by Entity (stable key) → fold` — NEVER natural query order: bevy moves archetype
/// rows on spawn/despawn, so `query.iter()` order varies run-to-run even with the same seed. Sorting
/// by `Entity::to_bits()` (generation+index, stable across runs given the same spawn sequence) makes
/// the fold order canonical, hence the hash reproducible.
pub fn deterministic_fold(mut items: Vec<(Entity, u64)>) -> u64 {
    items.sort_unstable_by_key(|(e, _)| e.to_bits());
    let mut h = FNV_OFFSET;
    for (e, v) in items {
        h = fnv_mix(h, e.to_bits());
        h = fnv_mix(h, v);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::world::World;

    #[test]
    fn fold_is_order_independent() {
        let mut w = World::new();
        let a = w.spawn_empty().id();
        let b = w.spawn_empty().id();
        let c = w.spawn_empty().id();
        let forward = deterministic_fold(vec![(a, 10), (b, 20), (c, 30)]);
        let shuffled = deterministic_fold(vec![(c, 30), (a, 10), (b, 20)]);
        assert_eq!(forward, shuffled, "fold must not depend on input order");
        // A different value for one entity must change the hash.
        assert_ne!(forward, deterministic_fold(vec![(a, 11), (b, 20), (c, 30)]));
    }
}
