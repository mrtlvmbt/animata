//! `world` — CPU `WorldView` backend (R29). A heightmap world queried by the sim (never owned by
//! it). **W-6 WIRE**: `ProcgenWorld` — built from the full `gen/` integer pipeline (W-1 heightmap →
//! W-4 erosion → W-5 final biome + resource caps) — is now the ONLY `WorldView` impl. The legacy
//! `NoiseWorld` (`f64 sin` value-noise, the deliberate float-arch boundary M1…W-5 built around) is
//! DELETED: `ProcgenWorld` is pure integer end-to-end (the `gen/` glob no-float guard enforces
//! this transitively), so the world's own contribution to the golden hash is now arch-IDENTICAL —
//! the golden stays per-arch (arm64) only because of the sim's UNRELATED f32 signal field.

use sim_core::{Vec2Fixed, WorldView};
use gen::caps::{classify_and_caps, CAP_MAX, FinalBiome};

/// W-1..W-6 world-gen pipeline stage home (see the module doc).
pub mod gen;

/// `ProcgenWorld` — built ONCE at `::new` from the full integer pipeline, then answers
/// `WorldView` queries by indexing cached arrays (never re-running erosion per query, which is
/// `O(iters·n log n)` — see the module doc's cold-init note).
pub struct ProcgenWorld {
    dim: i64,
    solid_level: i64,
    /// Post-erosion height, row-major `z*dim+x` (W-4's `ErosionState.height`, passed through W-5's
    /// `WorldFields.height`).
    height: Vec<i64>,
    /// The final post-override biome per cell (W-5's `FinalBiome`, cast to `u8` for the trait).
    final_biome: Vec<FinalBiome>,
    /// Resource, ALREADY rescaled into the `resource_base`-comparable magnitude at build time (see
    /// `rescale_cap`'s doc) — `resource()` just indexes + applies the solid-zeroing rule.
    resource: Vec<i64>,
}

/// Rescale a W-5 cap (`[0, CAP_MAX]`) into the SAME magnitude range the legacy `NoiseWorld` fed the
/// sim (`resource_base*(hmax-h)/hmax + 1`, i.e. `[1, resource_base+1]`) — **the scale-reconciliation
/// posture (critic F1): PRESERVE carrying-capacity magnitude, let the RICHNESS come from the
/// spatial pattern (real relief + varied biomes + edaphic overrides), not from a magnitude
/// blow-up.** `caps_from` was written against `CAP_MAX=300`; naively feeding that straight to the
/// sim would be a ~2.5× carrying-capacity shock vs the tuned `resource_base=120` the acceptance
/// corridors were calibrated against — this is the fix.
fn rescale_cap(cap: i64, resource_base: i64) -> i64 {
    cap * resource_base / CAP_MAX + 1
}

impl ProcgenWorld {
    /// Precompute-once (RnD 10 §1 cold init): runs `height_at → erode → classify_and_caps` a
    /// SINGLE time and caches the full-grid fields. Amortized over the whole run — the 8000-tick
    /// acceptance corridors pay this ONCE at build, never per tick.
    ///
    /// **Scale-reconciliation assert (critic F1/F3) — active in ALL builds, not `debug_assert!`:**
    /// checks the rescaled resource field's max/median land in the `resource_base`-comparable
    /// range. A dropped/wrong rescale (e.g. feeding the raw `[0,300]` cap straight through) would
    /// push `max` far past `resource_base+1` — caught HERE, at build time, before it ever reaches a
    /// tick or burns a CI/pin cycle on a guaranteed corridor breach.
    pub fn new(dim: i64, hmax: i64, resource_base: i64, seed: u64) -> Self {
        let fields = classify_and_caps(seed, hmax, dim as usize);
        let solid_level = hmax * 3 / 4; // mirrors NoiseWorld's solid-terrain semantics
        let n = (dim * dim) as usize;

        let mut resource = Vec::with_capacity(n);
        for i in 0..n {
            let r = if fields.height[i] >= solid_level {
                0 // solid terrain grows nothing (mirrors NoiseWorld's is_solid short-circuit)
            } else {
                rescale_cap(fields.caps[i], resource_base)
            };
            resource.push(r);
        }

        let max_resource = *resource.iter().max().unwrap_or(&0);
        let mut sorted = resource.clone();
        sorted.sort_unstable();
        let median_resource = sorted[sorted.len() / 2];
        assert!(
            max_resource <= resource_base + 1,
            "PROCGEN SCALE CHECK: max resource {max_resource} exceeds resource_base+1={} — \
             did the rescale get dropped/wrong (feeding the raw [0,{CAP_MAX}] cap straight \
             through)? (critic F1/F3 scale-reconciliation tooth)",
            resource_base + 1
        );
        assert!(
            median_resource >= 1,
            "PROCGEN SCALE CHECK: median resource {median_resource} is degenerate (<=0) — \
             the wired world would starve nearly everything"
        );

        ProcgenWorld { dim, solid_level, height: fields.height, final_biome: fields.final_biome, resource }
    }

    fn wrap(&self, v: i64) -> i64 {
        v.rem_euclid(self.dim)
    }

    fn idx(&self, x: i64, z: i64) -> usize {
        let (x, z) = (self.wrap(x), self.wrap(z));
        (z * self.dim + x) as usize
    }
}

impl WorldView for ProcgenWorld {
    fn height(&self, x: i64, z: i64) -> i64 {
        self.height[self.idx(x, z)]
    }

    fn is_solid(&self, pos: Vec2Fixed) -> bool {
        self.height(pos.0, pos.1) >= self.solid_level
    }

    fn biome(&self, pos: Vec2Fixed) -> u8 {
        self.final_biome[self.idx(pos.0, pos.1)] as u8
    }

    fn resource(&self, pos: Vec2Fixed) -> i64 {
        self.resource[self.idx(pos.0, pos.1)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: u64 = 0xA11A_2A11;
    /// Prod `HMAX` (critic F2 HMAX-degeneracy guard): matches the value the WHOLE `gen/` pipeline
    /// (climate/biome/drainage/erosion/caps) was calibrated and golden-tested against. A much
    /// smaller `HMAX` (the legacy `NoiseWorld`'s `16`) would put the relief spread BELOW
    /// `erosion::INCISION_EXPOSURE_THRESHOLD=20`, so Bedrock could never be exposed — the exact
    /// degeneracy this test guards against.
    const HMAX: i64 = 200;
    const DIM: i64 = 64;

    #[test]
    fn resource_nonneg_and_bounded() {
        let w = ProcgenWorld::new(DIM, HMAX, 120, SEED);
        for x in 0..DIM {
            for z in 0..DIM {
                let r = w.resource(Vec2Fixed(x, z));
                assert!((0..=121).contains(&r), "resource {r} out of [0,121] at ({x},{z})");
            }
        }
    }

    #[test]
    fn height_wraps_toroidally_like_noise_world_did() {
        let w = ProcgenWorld::new(DIM, HMAX, 120, SEED);
        assert_eq!(w.height(0, 0), w.height(DIM, 0), "x must wrap at dim");
        assert_eq!(w.height(0, 0), w.height(0, DIM), "z must wrap at dim");
        assert_eq!(w.height(-1, 0), w.height(DIM - 1, 0), "negative x must wrap");
    }

    #[test]
    fn procgen_world_is_deterministic_across_repeated_builds() {
        let a = ProcgenWorld::new(DIM, HMAX, 120, SEED);
        let b = ProcgenWorld::new(DIM, HMAX, 120, SEED);
        for x in 0..DIM {
            for z in 0..DIM {
                let pos = Vec2Fixed(x, z);
                assert_eq!(a.height(x, z), b.height(x, z));
                assert_eq!(a.biome(pos), b.biome(pos));
                assert_eq!(a.resource(pos), b.resource(pos));
            }
        }
    }

    /// Prod-scale RICHNESS + no-degeneracy check (critic F2 — the deliverable's point, non-golden).
    /// Guards the HMAX-degeneracy explicitly: relief spread must exceed
    /// `erosion::INCISION_EXPOSURE_THRESHOLD` (else Bedrock/`Rock` could never appear — a zonal-
    /// climate-only "≥2 biomes" check would silently pass even if erosion fully no-oped).
    #[test]
    fn procgen_world_is_rich_and_not_degenerate_at_prod_scale() {
        let w = ProcgenWorld::new(DIM, HMAX, 120, SEED);

        let mut min_h = i64::MAX;
        let mut max_h = i64::MIN;
        let mut biomes = std::collections::BTreeSet::new();
        let mut resources = std::collections::BTreeSet::new();
        let mut saw_rock = false;

        for x in 0..DIM {
            for z in 0..DIM {
                let h = w.height(x, z);
                min_h = min_h.min(h);
                max_h = max_h.max(h);
                let b = w.biome(Vec2Fixed(x, z));
                biomes.insert(b);
                if b == FinalBiome::Rock as u8 {
                    saw_rock = true;
                }
                resources.insert(w.resource(Vec2Fixed(x, z)));
            }
        }

        assert!(
            max_h - min_h > gen::erosion::INCISION_EXPOSURE_THRESHOLD,
            "relief spread ({}) must exceed INCISION_EXPOSURE_THRESHOLD ({}) — else erosion \
             cannot have exposed Bedrock (the HMAX-degeneracy this test guards against)",
            max_h - min_h, gen::erosion::INCISION_EXPOSURE_THRESHOLD
        );
        assert!(biomes.len() >= 2, "must have multiple distinct biomes, got {}", biomes.len());
        assert!(
            saw_rock,
            "erosion-driven Rock/Bedrock variety must appear at prod HMAX — else erosion silently \
             no-oped (zonal climate alone can satisfy '≥2 biomes' without this)"
        );
        assert!(resources.len() > 1, "resource must vary across cells, not be constant");
    }
}
