//! Sim-neighbor spatial index (R8) — a uniform grid keyed in **Morton (Z-order)** so the canonical
//! agent visit order is locality-preserving and deterministic. The second of the two grids (the
//! first being the world voxel grid, queried via `WorldView`). `M_sim` is integer, immutable for the
//! run, and validated at construction (the "checked on load" invariant — no save/load until M5, so
//! the check is the constructor guard here).

use crate::{DetMap, Vec2Fixed};
use bevy_ecs::prelude::{Entity, Resource};

/// Interleave the low 16 bits of `x` and `z` into a 32-bit Morton code.
pub fn morton2(x: u32, z: u32) -> u32 {
    fn part1by1(mut n: u32) -> u32 {
        n &= 0x0000_ffff;
        n = (n | (n << 8)) & 0x00ff_00ff;
        n = (n | (n << 4)) & 0x0f0f_0f0f;
        n = (n | (n << 2)) & 0x3333_3333;
        n = (n | (n << 1)) & 0x5555_5555;
        n
    }
    part1by1(x) | (part1by1(z) << 1)
}

/// Uniform neighbor grid, rebuilt each tick (stage 0). Buckets keyed by Morton code → deterministic
/// iteration via `DetMap` (BTreeMap) key order.
#[derive(Resource, Default)]
pub struct NeighborGrid {
    pub m_sim: i64,
    pub buckets: DetMap<u32, Vec<Entity>>,
}

impl NeighborGrid {
    pub fn new(m_sim: i64) -> Self {
        NeighborGrid { m_sim, buckets: DetMap::new() }
    }

    pub fn clear(&mut self) {
        self.buckets.clear();
    }

    /// Insert an agent at `pos` into its Morton bucket.
    pub fn insert(&mut self, pos: Vec2Fixed, e: Entity) {
        let cx = (pos.0.rem_euclid(1 << 16) / self.m_sim) as u32;
        let cz = (pos.1.rem_euclid(1 << 16) / self.m_sim) as u32;
        self.buckets.entry(morton2(cx, cz)).or_default().push(e);
    }
}
