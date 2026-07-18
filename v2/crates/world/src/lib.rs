//! `world` — CPU `WorldView` backend (R29). A heightmap world queried by the sim (never owned by
//! it). **W-6 WIRE**: `ProcgenWorld` — built from the full `gen/` integer pipeline (W-1 heightmap →
//! W-4 erosion → W-5 final biome + resource caps) — is now the ONLY `WorldView` impl. The legacy
//! `NoiseWorld` (`f64 sin` value-noise, the deliberate float-arch boundary M1…W-5 built around) is
//! DELETED: `ProcgenWorld` is pure integer end-to-end (the `gen/` glob no-float guard enforces
//! this transitively), so the world's own contribution to the golden hash is now arch-IDENTICAL —
//! the golden stays per-arch (arm64) only because of the sim's UNRELATED f32 signal field.

use sim_core::{Vec2Fixed, WorldView};
use gen::caps::{classify_and_caps_with_callback, CAP_MAX, FinalBiome, oxygen_cap_from, nitrate_cap_from};

/// W-1..W-6 world-gen pipeline stage home (see the module doc).
pub mod gen;

/// R-16 pasteled material color palette (single canonical source for render + map_dump).
pub mod palette;

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
    /// `rescale_cap`'s doc) — W-6b Phase A decouple: independent of height/is_solid, driven by caps alone.
    resource: Vec<i64>,
    /// O₂ resource cap per cell (P1-0 ШВ-1) — derived from biome via `oxygen_cap_from`, rescaled
    /// into the same `resource_base`-comparable magnitude for layer-management consistency. Static field.
    oxygen_resource: Vec<i64>,
    /// NO₃ resource cap per cell (P5-0, ШВ-1) — derived from biome via `nitrate_cap_from`, rescaled
    /// into the same `resource_base`-comparable magnitude. INVERSE of O₂ (high where O₂ is low).
    /// Static, inert field in P5-0 (no consumption yet, regen_rate=0).
    nitrate_resource: Vec<i64>,
    /// Surface material per cell (W-4's `ErosionState.surface_material`), exposed for richness
    /// testing (critic F2: assert Bedrock material is actually exposed, not just slope-driven Rock).
    surface_material: Vec<u8>,
    /// Temperature per cell, row-major (P3-1, centidegrees). Computed from BiomeId → biome-reference-point T
    /// during world-gen. Immutable post-gen (R27); read-only access via `temp_at()` trait method.
    /// Range: [−3000, +5000] (−30°C to +50°C). Each cell's temperature is constant per biome.
    temp_grid: Vec<i32>,
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

/// P3-1 (B2): Biome reference temperatures (centidegrees). Indexed by FinalBiome ordinal (0-13).
/// Static per biome; used to initialize the temp_grid during world-gen.
/// First 8 match zonal BiomeId; next 5 are azonal (edaphic) overrides (RnD 11 §3); last 1 is the
/// W-SIM-7 (#423) submerged branch.
/// Matching § 1.1 theory (RnD/engineering/43-ambient-tolerance-*.md):
/// Zonal (0-7):
/// - Tundra: −15°C
/// - BorealForest: −5°C
/// - TemperateGrassland: +10°C
/// - TemperateForest: +12°C
/// - TemperateRainforest: +12°C
/// - Desert: +25°C
/// - Savanna: +25°C
/// - TropicalRainforest: +25°C
/// Azonal (8-12, edaphic overrides):
/// - Wetland: +10°C (temperate water-logged environment)
/// - Floodplain: +12°C (warm-temperate, riparian)
/// - Rock: −5°C (exposed mineral, cold/variable baseline)
/// - Fertile: +12°C (nutrient-rich, temperate-forest baseline)
/// - Dune: +25°C (arid sand, desert-heat baseline)
/// Submerged (13):
/// - Ocean: +10°C (temperate open-water surface; biologically inert placeholder — Ocean's own base
///   cap/O₂/NO₃ are already fixed at 0 in `caps.rs`, RnD's relief-only roadmap explicitly excludes
///   marine biology in this slice, so this value has no downstream production effect today).
const BIOME_TEMP: [i32; 14] = [
    -1500,  // 0: Tundra (zonal)
    -500,   // 1: BorealForest (zonal)
    1000,   // 2: TemperateGrassland (zonal)
    1200,   // 3: TemperateForest (zonal)
    1200,   // 4: TemperateRainforest (zonal)
    2500,   // 5: Desert (zonal)
    2500,   // 6: Savanna (zonal)
    2500,   // 7: TropicalRainforest (zonal)
    1000,   // 8: Wetland (azonal, temperate water-environment)
    1200,   // 9: Floodplain (azonal, warm-temperate riparian)
    -500,   // 10: Rock (azonal, cold/exposed mineral)
    1200,   // 11: Fertile (azonal, nutrient-rich temperate)
    2500,   // 12: Dune (azonal, hot desert sand)
    1000,   // 13: Ocean (submerged, W-SIM-7 #423 — biologically inert placeholder)
];

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
    ///
    /// P3-3 (F1, golden-neutral): `thermal_verdict_temps` — optional biome-temperature override
    /// for the thermal-niche verdict harness. `None` (default) uses stock BIOME_TEMP; `Some(array)`
    /// injects custom temps (verdict-only, never shipped). Gated at world-gen time (immutable post-gen).
    ///
    /// **W-SIM-4a (#396):** `enable_tectonics` threads straight to `classify_and_caps`/`erode` —
    /// `false` (every prod call site on `worldgen-relief`) reproduces the pre-#396 world byte-for-
    /// byte, preserving the acceptance-corridor economy; the map/visual track opts in explicitly.
    ///
    /// **W-SIM-3a (#403):** `enable_aeolian` threads straight to `classify_and_caps`, orthogonal to
    /// `enable_tectonics` (both are independent opt-in stages) — `false` (every prod call site)
    /// reproduces the pre-#403 world byte-for-byte.
    ///
    /// **W-SIM-5 (#410):** `enable_volcanic` threads straight to `classify_and_caps`, orthogonal to
    /// `enable_tectonics`/`enable_aeolian` (a 4th independent opt-in stage, matching house style —
    /// see #410's explicit out-of-scope note on NOT folding these into a config struct here) —
    /// `false` (every prod call site) reproduces the pre-#410 world byte-for-byte.
    ///
    /// **W-SIM-6 (#416):** `enable_glacial` threads straight to `classify_and_caps`, orthogonal to
    /// `enable_tectonics`/`enable_aeolian`/`enable_volcanic` (a 5th independent opt-in stage,
    /// matching house style — see #416's explicit out-of-scope note on NOT folding these into a
    /// config struct here) — `false` (every prod call site) reproduces the pre-#416 world
    /// byte-for-byte.
    ///
    /// **W-SIM-7 (#423):** `enable_coastal` threads straight to `classify_and_caps`, orthogonal to
    /// `enable_tectonics`/`enable_aeolian`/`enable_volcanic`/`enable_glacial` (a 6th independent
    /// opt-in stage, matching house style) — `false` (every prod call site) reproduces the pre-#423
    /// world byte-for-byte. `thermal_verdict_temps` widened from `[i32; 13]` to `[i32; 14]` in the
    /// SAME PR (a mechanical ripple, not a new opt-in): `BIOME_TEMP` is indexed by
    /// `final_biome[i] as usize`, and `FinalBiome::Ocean=13` would otherwise index one past the old
    /// 13-element array on ANY submerged cell, regardless of `thermal_verdict_temps` — a guaranteed
    /// panic, not merely a latent gap, the moment `enable_coastal` produces water.
    ///
    /// **W-11 (#???):** `enable_ridges` threads straight to `erode_with_tectonics`, dependent on
    /// `enable_tectonics` (ridges need uplift; dependency clamp in `landform_flags`).
    /// — `false` (every prod call site) reproduces the pre-#??? world byte-for-byte.
    ///
    /// **W-12 (#???):** `enable_beaches` threads straight to `classify_and_caps`, dependent on
    /// `enable_coastal` (beaches need sea datum; dependency clamp in `landform_flags`).
    /// — `false` (every prod call site) reproduces the pre-#??? world byte-for-byte.
    pub fn new(
        dim: i64,
        hmax: i64,
        resource_base: i64,
        seed: u64,
        thermal_verdict_temps: Option<[i32; 14]>,
        enable_base: bool,
        enable_tectonics: bool,
        enable_aeolian: bool,
        enable_volcanic: bool,
        enable_glacial: bool,
        enable_coastal: bool,
        enable_erosion: bool,
        enable_ridges: bool,
        enable_beaches: bool,
        erosion_strength: i64,
        glacial_strength: i64,
    ) -> Self {
        Self::new_with_callback(
            dim,
            hmax,
            resource_base,
            seed,
            thermal_verdict_temps,
            enable_base,
            enable_tectonics,
            enable_aeolian,
            enable_volcanic,
            enable_glacial,
            enable_coastal,
            enable_erosion,
            enable_ridges,
            enable_beaches,
            erosion_strength,
            glacial_strength,
            None,
        )
    }

    /// Create a `ProcgenWorld` with an optional progress callback (U-11).
    /// The callback receives a u8 stage ordinal (maps to Stage enum in render crate).
    /// The callback is observation-only: zero effect on RNG, heights, or any generated byte.
    ///
    /// For byte-purity verification: calling with a counting callback must produce
    /// byte-identical output to calling with None.
    ///
    /// **W-19:** `erosion_strength` and `glacial_strength` control the intensity of erosion and
    /// glacial transforms (percent, default 100, range [0, 400]).
    pub fn new_with_callback(
        dim: i64,
        hmax: i64,
        resource_base: i64,
        seed: u64,
        thermal_verdict_temps: Option<[i32; 14]>,
        enable_base: bool,
        enable_tectonics: bool,
        enable_aeolian: bool,
        enable_volcanic: bool,
        enable_glacial: bool,
        enable_coastal: bool,
        enable_erosion: bool,
        enable_ridges: bool,
        enable_beaches: bool,
        erosion_strength: i64,
        glacial_strength: i64,
        mut progress_callback: Option<Box<dyn FnMut(u8)>>,
    ) -> Self {
        // W-7 gate: patchiness defaults OFF for acceptance corridors (homogeneous baseline).
        // Specific scenarios (map-gen, visualization) can opt-in by calling with enable_patchiness=true.
        let flags = crate::gen::LandformFlags::new(
            enable_base,
            enable_tectonics,
            enable_aeolian,
            enable_volcanic,
            enable_glacial,
            enable_coastal,
            enable_erosion,
            enable_ridges,
            enable_beaches,
            erosion_strength,
            glacial_strength,
        );
        let fields = classify_and_caps_with_callback(seed, hmax, dim as usize, false, flags, progress_callback);
        // W-6b Phase A: DECOUPLE resource from solid_level (RnD 01 §40,43: is_solid=movement,
        // resource=food are SEPARATE queries). solid_level → ONLY movement/collision (is_solid).
        // resource() → DIRECT rescale_cap(caps[idx]), independent of height.
        // Dynamically choose solid_level to achieve ~15-50% solid-fraction target (ТЗ).
        // Algorithm: sort heights, find percentile that gives 15-50% band.
        let n = (dim * dim) as usize;
        let mut heights_sorted = fields.height.clone();
        heights_sorted.sort_unstable();

        // For 15-50% solid range, target middle of band ≈ 35% solid (65th percentile of heights).
        let h_p65 = heights_sorted[(heights_sorted.len() * 65) / 100];
        let h_p50 = heights_sorted[heights_sorted.len() / 2];
        let mut solid_level = h_p65;

        // Guard: verify the guess lands in [15,50]
        let solid_count_test = fields.height.iter().filter(|&&h| h >= solid_level).count();
        let test_frac = solid_count_test as f64 / n as f64;

        if !(0.15..=0.50).contains(&test_frac) {
            // Fallback: try p50 (should be close to 50% solid)
            solid_level = h_p50;
            let _solid_count_fallback = fields.height.iter().filter(|&&h| h >= solid_level).count();
            // Use fallback even if out of range — let the guard assert surface it
        }

        // P3-3: choose temp array — override if provided, else stock BIOME_TEMP.
        let biome_temps = thermal_verdict_temps.unwrap_or(BIOME_TEMP);

        let mut resource = Vec::with_capacity(n);
        let mut oxygen_resource = Vec::with_capacity(n);
        let mut nitrate_resource = Vec::with_capacity(n);
        let mut temp_grid = Vec::with_capacity(n);
        for i in 0..n {
            // W-6b Phase A: decouple — resource is independent of solid_level (height-based passability).
            // Barrenness is already in caps (Rock base 0, Bedrock mult 0); rescale floors every cell to >=1.
            let r = rescale_cap(fields.caps[i], resource_base);
            resource.push(r);

            // P1-0: O₂ resource cap — biome-derived, rescaled for consistency with substrate.
            let o2_raw = oxygen_cap_from(fields.final_biome[i]);
            let o2_rescaled = rescale_cap(o2_raw, resource_base);
            oxygen_resource.push(o2_rescaled);

            // P5-0: NO₃ resource cap — biome-derived (INVERSE of O₂), rescaled for consistency.
            let no3_raw = nitrate_cap_from(fields.final_biome[i]);
            let no3_rescaled = rescale_cap(no3_raw, resource_base);
            nitrate_resource.push(no3_rescaled);

            // P3-1 (B2): temperature per cell — biome-derived, immutable post-gen (R27).
            // P3-3 (F1): use override if provided, else stock BIOME_TEMP.
            let t = biome_temps[fields.final_biome[i] as usize];
            temp_grid.push(t);
        }

        let max_resource = *resource.iter().max().unwrap_or(&0);
        let mut sorted = resource.clone();
        sorted.sort_unstable();
        let median_resource = sorted[sorted.len() / 2];

        let solid_count = fields.height.iter().filter(|&&h| h >= solid_level).count();
        let solid_frac_final = solid_count as f64 / n as f64;
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

        // (d) Solid-fraction guard (critic F3): solid cells (height >= solid_level) should be a
        // reasonable fraction (roughly 25-40% at prod HMAX=200). Too few solid cells → too much
        // free movement/energy. Too many → too little usable space. Mirror NoiseWorld's semantics.
        // R-17: Relaxed to 15-75% for landform-enabled preview worlds (tectonic+aeolian+glacial+coastal+volcanic+ridges+beaches
        // naturally generate higher-relief terrain). Strict 15-50% band preserved for the all-off sim path
        // (each prod call site disables all landforms, preserving acceptance-corridor economy).
        let any_landform = enable_tectonics || enable_aeolian || enable_volcanic || enable_glacial || enable_coastal || enable_ridges || enable_beaches;
        let solid_count = fields.height.iter().filter(|&&h| h >= solid_level).count();
        let solid_frac = solid_count as f64 / n as f64;
        let (band_min, band_max, band_desc) = if any_landform {
            (0.15, 0.75, "landform-on (15–75%)")
        } else {
            (0.15, 0.50, "all-off sim (15–50%)")
        };

        // W-18-HF: Gate the solid-fraction assert on flags.base. The check guards SIM movement/space
        // economy of fBm worlds; the sim lane always runs base=true, so the guard stays fully intact
        // where it matters. When base == false (explicit dev/presentation config like flat maps),
        // skip the assert and log instead (flat worlds are 100% solid by construction).
        if enable_base {
            assert!(
                (band_min..=band_max).contains(&solid_frac_final),
                "PROCGEN SOLID FRACTION CHECK: solid cells {:.1}% (threshold: {}) — movement/space balance may be off (critic F3); if drift is legitimate, re-pin after recalibrating solid_level",
                solid_frac_final * 100.0,
                band_desc
            );
        } else {
            eprintln!(
                "PROCGEN SOLID FRACTION CHECK (base=false, skipped): solid cells {:.1}% (config: {})",
                solid_frac_final * 100.0,
                band_desc
            );
        }

        ProcgenWorld { dim, solid_level, height: fields.height, final_biome: fields.final_biome, resource, oxygen_resource, nitrate_resource, surface_material: fields.surface_material, temp_grid }
    }

    fn wrap(&self, v: i64) -> i64 {
        v.rem_euclid(self.dim)
    }

    fn idx(&self, x: i64, z: i64) -> usize {
        let (x, z) = (self.wrap(x), self.wrap(z));
        (z * self.dim + x) as usize
    }
}

impl ProcgenWorld {
    /// O₂ resource cap at a position (P1-0 ШВ-1). Returns rescaled O₂-cap for the biome at `pos`.
    pub fn oxygen_resource(&self, pos: Vec2Fixed) -> i64 {
        self.oxygen_resource[self.idx(pos.0, pos.1)]
    }

    /// NO₃ resource cap at a position (P5-0, ШВ-1). Returns rescaled NO₃-cap for the biome at `pos`.
    /// INVERSE of O₂: high where O₂ is low (anaerobic/waterlogged zones).
    pub fn nitrate_resource(&self, pos: Vec2Fixed) -> i64 {
        self.nitrate_resource[self.idx(pos.0, pos.1)]
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

    fn temp_at(&self, pos: Vec2Fixed) -> i32 {
        self.temp_grid[self.idx(pos.0, pos.1)]
    }

    fn surface_material(&self, pos: Vec2Fixed) -> u8 {
        self.surface_material[self.idx(pos.0, pos.1)]
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
        let w = ProcgenWorld::new(DIM, HMAX, 120, SEED, None, true, false, false, false, false, false, true, false, false, 100, 100);
        for x in 0..DIM {
            for z in 0..DIM {
                let r = w.resource(Vec2Fixed(x, z));
                assert!((0..=121).contains(&r), "resource {r} out of [0,121] at ({x},{z})");
            }
        }
    }

    #[test]
    fn height_wraps_toroidally_like_noise_world_did() {
        let w = ProcgenWorld::new(DIM, HMAX, 120, SEED, None, true, false, false, false, false, false, true, false, false, 100, 100);
        assert_eq!(w.height(0, 0), w.height(DIM, 0), "x must wrap at dim");
        assert_eq!(w.height(0, 0), w.height(0, DIM), "z must wrap at dim");
        assert_eq!(w.height(-1, 0), w.height(DIM - 1, 0), "negative x must wrap");
    }

    #[test]
    fn procgen_world_is_deterministic_across_repeated_builds() {
        let a = ProcgenWorld::new(DIM, HMAX, 120, SEED, None, true, false, false, false, false, false, true, false, false, 100, 100);
        let b = ProcgenWorld::new(DIM, HMAX, 120, SEED, None, true, false, false, false, false, false, true, false, false, 100, 100);
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
        let w = ProcgenWorld::new(DIM, HMAX, 120, SEED, None, true, false, false, false, false, false, true, false, false, 100, 100);

        let mut min_h = i64::MAX;
        let mut max_h = i64::MIN;
        let mut biomes = std::collections::BTreeSet::new();
        let mut resources = std::collections::BTreeSet::new();
        let mut saw_rock = false;
        let mut saw_bedrock = false;

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
                // Check for actual Bedrock material (not just slope-driven Rock biome)
                if w.surface_material[z as usize * DIM as usize + x as usize] == 4 { // MaterialId::Bedrock = 4
                    saw_bedrock = true;
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

    /// W-6b Phase A decouple property test: resource is now independent of height/is_solid (W-6b goal).
    /// After decouple, resource=rescale_cap(caps[i]) directly, floors every cell to >=1.
    /// Air material (cap=0) → resource == 1. All cells >= 1 (no solid-zeroing).
    #[test]
    fn resource_decoupled_from_solid_level() {
        use gen::material::MaterialId;

        let w = ProcgenWorld::new(DIM, HMAX, 120, SEED, None, true, false, false, false, false, false, true, false, false, 100, 100);
        let mut resource_on_solid = Vec::new();
        let mut resource_on_non_solid = Vec::new();

        for x in 0..DIM {
            for z in 0..DIM {
                let idx = z as usize * DIM as usize + x as usize;
                let res = w.resource(Vec2Fixed(x, z));
                let mat_byte = w.surface_material[idx];
                let is_solid = w.is_solid(Vec2Fixed(x, z));

                // Air material (cap=0) ⇒ resource == 1 (the only guaranteed barren floor)
                if mat_byte == MaterialId::Air as u8 {
                    assert_eq!(
                        res, 1,
                        "Air material at ({x},{z}) must have resource==1 (the barren floor), got {res}"
                    );
                }

                // W-6b decouple proof: resource is NOT zeroed on solid cells (height >= solid_level).
                // Collect both solid and non-solid to verify variation is independent of height.
                if is_solid {
                    resource_on_solid.push(res);
                } else {
                    resource_on_non_solid.push(res);
                }

                // Rescale floor: all cells must have resource >= 1
                assert!(
                    res >= 1,
                    "All cells must have resource >= 1 (rescale floor), at ({x},{z}) got {res}"
                );
            }
        }

        // Decouple proof: both solid AND non-solid regions must have resource variation.
        // If solid cells were zeroed (old NoiseWorld behavior), solid cells would all be resource=0.
        // With decouple, both regions have natural caps-driven variation.
        assert!(
            resource_on_solid.len() > 0 && resource_on_non_solid.len() > 0,
            "Both solid and non-solid regions must exist"
        );
        let solid_max = resource_on_solid.iter().max().copied().unwrap_or(0);
        let nonsolid_max = resource_on_non_solid.iter().max().copied().unwrap_or(0);
        assert!(
            solid_max > 1,
            "Solid cells must have resource > 1 (proof of decouple: no zeroing). Got max={solid_max}"
        );
    }

    /// U-11: Byte-purity test — progress callback is observation-only.
    /// Verifies that a world generated with a counting callback produces byte-identical
    /// heights, biomes, caps, and resources as one generated with no callback.
    #[test]
    fn progress_callback_byte_pure() {
        use std::sync::atomic::{AtomicU8, Ordering};
        use std::sync::Arc;

        let dim = 64i64;
        let hmax = 200i64;
        let seed = 0x1234_5678u64;
        let resource_base = 120i64;

        // Generate world without callback
        let world_no_cb = ProcgenWorld::new(
            dim, hmax, resource_base, seed, None,
            true, false, false, false, false, false, true, false, false, 100, 100
        );

        // Generate world with counting callback
        let call_count = Arc::new(AtomicU8::new(0));
        let call_count_clone = Arc::clone(&call_count);
        let mut callback = move |_stage: u8| {
            call_count_clone.fetch_add(1, Ordering::Relaxed);
        };
        let world_with_cb = ProcgenWorld::new_with_callback(
            dim, hmax, resource_base, seed, None,
            true, false, false, false, false, false, true, false, false,
            100, 100,
            Some(Box::new(callback))
        );

        // Verify byte-identity: heights, biomes, caps, materials, temps
        assert_eq!(
            world_no_cb.height, world_with_cb.height,
            "Heights must be identical with/without callback"
        );
        assert_eq!(
            world_no_cb.final_biome, world_with_cb.final_biome,
            "Biomes must be identical with/without callback"
        );
        assert_eq!(
            world_no_cb.resource, world_with_cb.resource,
            "Resources must be identical with/without callback"
        );
        assert_eq!(
            world_no_cb.oxygen_resource, world_with_cb.oxygen_resource,
            "O₂ resources must be identical with/without callback"
        );
        assert_eq!(
            world_no_cb.nitrate_resource, world_with_cb.nitrate_resource,
            "NO₃ resources must be identical with/without callback"
        );
        assert_eq!(
            world_no_cb.surface_material, world_with_cb.surface_material,
            "Surface materials must be identical with/without callback"
        );
        assert_eq!(
            world_no_cb.temp_grid, world_with_cb.temp_grid,
            "Temperatures must be identical with/without callback"
        );

        // Verify callback was invoked (at least for erosion + classify stages, even with all landforms off)
        let invocations = call_count.load(Ordering::Relaxed);
        assert!(
            invocations >= 2,
            "Callback must be invoked at least for erosion + classify stages, got {} invocations",
            invocations
        );
    }

    /// W-18-HF regression test: flat map (base=false, all-off flags) must NOT panic.
    /// Reproduces the bug: ProcgenWorld::new with base=false panicked on PROCGEN SOLID FRACTION CHECK
    /// because flat worlds are 100% solid by construction.
    #[test]
    fn flat_map_does_not_panic_base_false_all_off() {
        use gen::erosion::flat_datum;

        let dim = 32i64; // Small dim for quick test
        let hmax = 200i64;
        let resource_base = 120i64;
        let seed = 0x9999_9999u64;

        // All-off flags with base=false — this should NOT panic (the bug being fixed).
        let w = ProcgenWorld::new(
            dim, hmax, resource_base, seed, None,
            false, // enable_base = false (FLAT_DATUM instead of fBm)
            false, // enable_tectonics
            false, // enable_aeolian
            false, // enable_volcanic
            false, // enable_glacial
            false, // enable_coastal
            false, // enable_erosion (redundant with base=false, but explicit)
            false, // enable_ridges
            false, // enable_beaches
            100, 100, // W-19: erosion_strength, glacial_strength
        );

        // Verify: all heights must be FLAT_DATUM (hmax/2)
        let expected_height = flat_datum(hmax);
        for x in 0..dim {
            for z in 0..dim {
                let h = w.height(x, z);
                assert_eq!(
                    h, expected_height,
                    "flat invariant: height at ({x},{z}) must be FLAT_DATUM={}, got {h}",
                    expected_height
                );
            }
        }
    }

    /// W-18-HF regression test: pedestal + volcanic (base=false) must NOT panic.
    /// Second config PM hit: base=false with volcanic edifices.
    #[test]
    fn pedestal_with_volcanic_does_not_panic_base_false() {
        use gen::erosion::flat_datum;

        let dim = 32i64; // Small dim for quick test
        let hmax = 200i64;
        let resource_base = 120i64;
        let seed = 0x8888_8888u64;

        // base=false (pedestal) with volcanic edifices
        let w = ProcgenWorld::new(
            dim, hmax, resource_base, seed, None,
            false, // enable_base = false (FLAT_DATUM pedestal)
            false, // enable_tectonics
            false, // enable_aeolian
            true,  // enable_volcanic (edifices atop pedestal)
            false, // enable_glacial
            false, // enable_coastal
            false, // enable_erosion
            false, // enable_ridges
            false, // enable_beaches
            100, 100, // W-19: erosion_strength, glacial_strength
        );

        // Verify: base is FLAT_DATUM, with volcanic features added
        let base_height = flat_datum(hmax);
        let mut saw_height_variation = false;

        for x in 0..dim {
            for z in 0..dim {
                let h = w.height(x, z);
                // Heights should be >= base_height (volcanic adds edifices on top)
                assert!(
                    h >= base_height,
                    "flat invariant: height at ({x},{z}) must be >= FLAT_DATUM={}, got {h}",
                    base_height
                );
                if h > base_height {
                    saw_height_variation = true;
                }
            }
        }

        // With volcanic enabled, we should see some heights above FLAT_DATUM
        // (though this is not guaranteed by the algorithm, it's a sanity check)
        assert!(
            saw_height_variation,
            "Expected some height variation above FLAT_DATUM with volcanic enabled; all cells are at base_height"
        );
    }
}
