//! `world` — CPU `WorldView` backend (R29). A heightmap world queried by the sim (never owned by it).
//!
//! With the `noise` feature (default) the heightmap is f64 value-noise (`sin`-based → libm →
//! ARCH-DIVERGENT). That float is deliberately confined here: it shapes the static terrain and the
//! resource POTENTIAL, so the M1 trajectory becomes arch-dependent (→ arm64-pinned golden) while the
//! conserved energy/field layer in `sim-core`/`fields` stays pure integer. With `noise` off the world
//! is flat and fully integer.

use sim_core::{Vec2Fixed, WorldView};

/// W-1..W-6 world-gen pipeline stage home (see the module doc). Prod-inert until W-6 wires the
/// assembled pipeline into a `WorldView` impl — `NoiseWorld` below is unaffected.
pub mod gen;

/// A square heightmap world.
pub struct NoiseWorld {
    dim: i64,
    hmax: i64,
    solid_level: i64,
    resource_base: i64,
    seed: u64,
}

impl NoiseWorld {
    pub fn new(dim: i64, hmax: i64, resource_base: i64, seed: u64) -> Self {
        NoiseWorld { dim, hmax, solid_level: hmax * 3 / 4, resource_base, seed }
    }

    fn wrap(&self, v: i64) -> i64 {
        v.rem_euclid(self.dim)
    }
}

/// f64 value-noise in `[0,1)` — uses `sin`, so it diverges between x86 and arm64 (the intended
/// float-arch boundary). Only compiled with the `noise` feature.
#[cfg(feature = "noise")]
fn noise01(x: i64, z: i64, seed: u64) -> f64 {
    let xf = x as f64;
    let zf = z as f64;
    let s = seed as f64 * 0.001;
    let v = (xf * 12.9898 + zf * 78.233 + s).sin() * 43758.5453;
    v - v.floor()
}

impl WorldView for NoiseWorld {
    fn height(&self, x: i64, z: i64) -> i64 {
        let (x, z) = (self.wrap(x), self.wrap(z));
        #[cfg(feature = "noise")]
        {
            (noise01(x, z, self.seed) * self.hmax as f64) as i64
        }
        #[cfg(not(feature = "noise"))]
        {
            let _ = (x, z);
            0
        }
    }

    fn is_solid(&self, pos: Vec2Fixed) -> bool {
        self.height(pos.0, pos.1) >= self.solid_level
    }

    fn biome(&self, pos: Vec2Fixed) -> u8 {
        let h = self.height(pos.0, pos.1);
        if h >= self.solid_level {
            2 // rock
        } else if h >= self.hmax / 2 {
            1 // upland
        } else {
            0 // lowland
        }
    }

    fn resource(&self, pos: Vec2Fixed) -> i64 {
        // Solid terrain grows nothing; lowlands are richest (valleys collect resource).
        if self.is_solid(pos) {
            return 0;
        }
        let h = self.height(pos.0, pos.1);
        self.resource_base * (self.hmax - h) / self.hmax + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_nonneg_and_bounded() {
        let w = NoiseWorld::new(64, 16, 300, 7);
        for x in 0..64 {
            for z in 0..64 {
                let r = w.resource(Vec2Fixed(x, z));
                assert!((0..=301).contains(&r));
            }
        }
    }

    #[cfg(not(feature = "noise"))]
    #[test]
    fn flat_world_is_integer_uniform() {
        let w = NoiseWorld::new(64, 16, 300, 7);
        assert_eq!(w.height(3, 9), 0);
        assert!(!w.is_solid(Vec2Fixed(3, 9)));
        assert_eq!(w.resource(Vec2Fixed(3, 9)), 301);
    }
}
