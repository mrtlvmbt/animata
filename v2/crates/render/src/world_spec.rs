//! U-2: World specification and deterministic landform configuration.
//!
//! Centralizes all inputs to world building (seed, flags, source), ensuring
//! that both the app worker thread and harnesses use the same `build_world(spec)`
//! path with deterministic flag derivation from seed.

use std::path::PathBuf;

/// Landform configuration flags (deterministically derived from seed or set manually).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LandformFlags {
    pub tect: bool,
    pub aeolian: bool,
    pub volcanic: bool,
    pub glacial: bool,
    pub coastal: bool,
    pub ridges: bool,
    pub beaches: bool,
}

impl LandformFlags {
    /// Create landform flags without clamps (raw values).
    /// Clamps are applied later in `apply_guard()` AFTER the guard potentially enables tect/coastal.
    pub fn new(
        tect: bool,
        aeolian: bool,
        volcanic: bool,
        glacial: bool,
        coastal: bool,
        ridges: bool,
        beaches: bool,
    ) -> Self {
        LandformFlags { tect, aeolian, volcanic, glacial, coastal, ridges, beaches }
    }

    /// Apply guards and dependency clamps to ensure a valid state.
    /// Order: (1) if all five original landforms are off, enable tectonic;
    /// (2) then apply clamps so guard's tect-enabling makes ridges/beaches permissible.
    pub fn apply_guard(mut self) -> Self {
        // Guard: never all-off for the original five (avoid flat/boring maps)
        if !(self.tect || self.aeolian || self.volcanic || self.glacial || self.coastal) {
            self.tect = true;
        }
        // W-0 dependency clamps: ridges requires tect, beaches requires coastal
        // Applied AFTER guard so that guard's tect-enabling affects clamp results.
        self.ridges = self.ridges && self.tect;
        self.beaches = self.beaches && self.coastal;
        self
    }
}

/// World data source: procedural generation or loaded dump.
#[derive(Clone, Debug)]
pub enum WorldSource {
    /// Procedurally generate a world. `dim_request` is honored ONLY in standalone mode;
    /// sim mode ignores it to preserve the pinned-param contract (render dim must match
    /// the sim worker's world).
    Procgen { dim_request: Option<i64> },
    /// Load a v1-format world dump from a file (carries its own dim).
    Dump(PathBuf),
}

/// Specification for world building (single source of truth for app + harnesses).
///
/// All world-build inputs live here, re-derived fresh inside `build_world()`:
/// - config is NOT stored; it's re-created from `seed` via `cli::default_config(seed)`
/// - landform flags are NOT stored by default; they're computed from `(seed, standalone)` via `landform_flags()`
/// - **U-10**: explicit_landform_flags override (if Some) replaces seed-derived flags
/// - effective `dim` is an OUTPUT (BuiltWorld::dim), never stored as input
///
/// This design ensures regen is type-safe ("regen == cold launch") and sim mode
/// can never read a stale launch-seed config through the back door.
#[derive(Clone, Debug)]
pub struct WorldSpec {
    /// RNG seed for terrain generation and creature spawning.
    pub seed: u64,
    /// If true, apply standalone landform variety (seed-derived flags).
    /// If false (sim mode), all landforms stay off (preserves pinned-param contract).
    pub standalone: bool,
    /// If true, water renders as dry-bed sand tint instead of water color.
    pub bare_mode: bool,
    /// Where the world comes from (Procgen or Dump).
    pub source: WorldSource,
    /// U-10: Explicit landform flags override. If Some, use these instead of deriving from seed.
    /// If None, compute flags from seed via landform_flags(seed, standalone).
    pub explicit_landform_flags: Option<LandformFlags>,
}

/// Stage of world building (used for progress reporting via callback).
/// Stages map to worldgen pipeline boundaries: heightfield fbm → tectonics → erosion → ridges →
/// aeolian → volcanic → glacial → coastal → beaches → talus → de-needle → classify → meshing → done.
/// Skipped stages (flags off) report instantly — bar must not stall on disabled landforms.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Stage {
    GenerateHeightfield = 0,
    ApplyTectonics = 1,
    ApplyErosion = 2,
    ApplyRidges = 3,
    ApplyAeolian = 4,
    ApplyVolcanic = 5,
    ApplyGlacial = 6,
    ApplyCoastal = 7,
    ApplyBeaches = 8,
    ApplyTalus = 9,
    DeNeedle = 10,
    Classify = 11,
    BuildMeshes = 12,
    Done = 13,
}

impl Stage {
    /// Convert to ordinal for atomic storage (u8).
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// Convert from ordinal.
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Stage::GenerateHeightfield),
            1 => Some(Stage::ApplyTectonics),
            2 => Some(Stage::ApplyErosion),
            3 => Some(Stage::ApplyRidges),
            4 => Some(Stage::ApplyAeolian),
            5 => Some(Stage::ApplyVolcanic),
            6 => Some(Stage::ApplyGlacial),
            7 => Some(Stage::ApplyCoastal),
            8 => Some(Stage::ApplyBeaches),
            9 => Some(Stage::ApplyTalus),
            10 => Some(Stage::DeNeedle),
            11 => Some(Stage::Classify),
            12 => Some(Stage::BuildMeshes),
            13 => Some(Stage::Done),
            _ => None,
        }
    }

    /// Russian label for display in the loader UI.
    pub fn label_ru(self) -> &'static str {
        match self {
            Stage::GenerateHeightfield => "Высотная карта",
            Stage::ApplyTectonics => "Тектоника",
            Stage::ApplyErosion => "Эрозия",
            Stage::ApplyRidges => "Гребни",
            Stage::ApplyAeolian => "Ветер",
            Stage::ApplyVolcanic => "Вулканы",
            Stage::ApplyGlacial => "Ледники",
            Stage::ApplyCoastal => "Побережье",
            Stage::ApplyBeaches => "Пляжи",
            Stage::ApplyTalus => "Осыпи",
            Stage::DeNeedle => "Сглаживание",
            Stage::Classify => "Классификация",
            Stage::BuildMeshes => "Меширование",
            Stage::Done => "Готово",
        }
    }

    /// Progress permille (0–1000) for this stage.
    /// Gen pipeline occupies 0..800, meshing/upload 800..1000.
    pub fn progress_permille(self) -> u32 {
        match self {
            Stage::GenerateHeightfield => 0,
            Stage::ApplyTectonics => 67,
            Stage::ApplyErosion => 133,
            Stage::ApplyRidges => 200,
            Stage::ApplyAeolian => 267,
            Stage::ApplyVolcanic => 333,
            Stage::ApplyGlacial => 400,
            Stage::ApplyCoastal => 467,
            Stage::ApplyBeaches => 533,
            Stage::ApplyTalus => 600,
            Stage::DeNeedle => 667,
            Stage::Classify => 733,
            Stage::BuildMeshes => 800,
            Stage::Done => 1000,
        }
    }
}

/// Landform flags: derived deterministically from seed + standalone mode.
/// Returns LandformFlags with tectonic, aeolian, volcanic, glacial, coastal, ridges, beaches.
///
/// Each landform toggles independently (well-spaced bit positions in splitmix64).
/// Dependency clamps: ridges &= tect (needs uplift), beaches &= coastal (needs sea datum).
/// Guard: never all-off for the original five (ensures maps never become flat/featureless).
pub fn landform_flags(seed: u64, standalone: bool) -> LandformFlags {
    if !standalone {
        // Sim mode: all landforms off (unchanged from original contract)
        return LandformFlags {
            tect: false,
            aeolian: false,
            volcanic: false,
            glacial: false,
            coastal: false,
            ridges: false,
            beaches: false,
        };
    }

    // Deterministic hash: splitmix64
    let mut x = seed;
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^= x >> 31;

    // Extract independent bits for each landform (well-spaced bit positions)
    let tect = (x >> 3) & 1 == 1;
    let aeol = (x >> 13) & 1 == 1;
    let volc = (x >> 23) & 1 == 1;
    let glac = (x >> 33) & 1 == 1;
    let coast = (x >> 43) & 1 == 1;
    let ridg = (x >> 53) & 1 == 1;
    let beach = (x >> 59) & 1 == 1;

    // Apply clamps and guard
    LandformFlags::new(tect, aeol, volc, glac, coast, ridg, beach).apply_guard()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_roundtrip_u8() {
        assert_eq!(Stage::GenerateHeightfield.as_u8(), 0);
        assert_eq!(Stage::ApplyTectonics.as_u8(), 1);
        assert_eq!(Stage::ApplyErosion.as_u8(), 2);
        assert_eq!(Stage::BuildMeshes.as_u8(), 12);
        assert_eq!(Stage::Done.as_u8(), 13);

        assert_eq!(Stage::from_u8(0), Some(Stage::GenerateHeightfield));
        assert_eq!(Stage::from_u8(1), Some(Stage::ApplyTectonics));
        assert_eq!(Stage::from_u8(2), Some(Stage::ApplyErosion));
        assert_eq!(Stage::from_u8(12), Some(Stage::BuildMeshes));
        assert_eq!(Stage::from_u8(13), Some(Stage::Done));
        assert_eq!(Stage::from_u8(14), None);
    }

    #[test]
    fn landform_flags_deterministic() {
        let flags1 = landform_flags(0x1234_5678, true);
        let flags2 = landform_flags(0x1234_5678, true);
        assert_eq!(flags1, flags2, "same seed must produce same flags");

        let flags3 = landform_flags(0x1234_5679, true);
        // Different seed very likely produces different flags (not guaranteed, but highly probable)
        // Just verify it returns a struct
        let _ = flags3;
    }

    #[test]
    fn landform_flags_sim_mode_all_off() {
        let flags = landform_flags(0xDEAD_BEEF, false);
        assert!(!flags.tect && !flags.aeolian && !flags.volcanic && !flags.glacial
                && !flags.coastal && !flags.ridges && !flags.beaches,
                "sim mode must have all landforms off");
    }

    #[test]
    fn landform_flags_never_all_off() {
        // Test a range of seeds; at least one of the original five landforms should always be on
        for seed in [0u64, 1, 42, 0xFFFF_FFFF, 0xDEAD_BEEF, 0xCAFE_BABE] {
            let flags = landform_flags(seed, true);
            assert!(flags.tect || flags.aeolian || flags.volcanic || flags.glacial || flags.coastal,
                    "seed {seed:016x} produced all-off landforms");
        }
    }

    #[test]
    fn landform_flags_dependency_clamps() {
        // Ridges must be false if tect is false
        for seed in [0u64, 1, 42, 0xFFFF_FFFF, 0xDEAD_BEEF, 0xCAFE_BABE] {
            let flags = landform_flags(seed, true);
            if flags.ridges {
                assert!(flags.tect, "seed {seed:016x}: ridges true but tect false");
            }
        }
        // Beaches must be false if coastal is false
        for seed in [0u64, 1, 42, 0xFFFF_FFFF, 0xDEAD_BEEF, 0xCAFE_BABE] {
            let flags = landform_flags(seed, true);
            if flags.beaches {
                assert!(flags.coastal, "seed {seed:016x}: beaches true but coastal false");
            }
        }
    }

    /// U-3: Reseed guard conditions — verify which combinations allow reseed.
    /// Reseed is enabled ONLY when source == Procgen && standalone (F12/F15).
    #[test]
    fn reseed_guard_procgen_standalone() {
        let spec = WorldSpec {
            seed: 0x1234_5678,
            standalone: true,
            bare_mode: false,
            source: WorldSource::Procgen { dim_request: None },
            explicit_landform_flags: None,
        };
        // Should be reseedable: Procgen + standalone
        let can_reseed = matches!(spec.source, WorldSource::Procgen { .. }) && spec.standalone;
        assert!(can_reseed, "Procgen + standalone must allow reseed");
    }

    #[test]
    fn reseed_guard_procgen_sim_mode() {
        let spec = WorldSpec {
            seed: 0x1234_5678,
            standalone: false,  // Sim mode
            bare_mode: false,
            source: WorldSource::Procgen { dim_request: None },
            explicit_landform_flags: None,
        };
        // Should NOT be reseedable: Procgen but sim mode
        let can_reseed = matches!(spec.source, WorldSource::Procgen { .. }) && spec.standalone;
        assert!(!can_reseed, "Procgen + sim_mode must NOT allow reseed (F12)");
    }

    #[test]
    fn reseed_guard_dump_standalone() {
        let spec = WorldSpec {
            seed: 0x1234_5678,
            standalone: true,
            bare_mode: false,
            source: WorldSource::Dump(std::path::PathBuf::from("/tmp/dump.atdmp1")),
            explicit_landform_flags: None,
        };
        // Should NOT be reseedable: Dump source (even though standalone)
        let can_reseed = matches!(spec.source, WorldSource::Procgen { .. }) && spec.standalone;
        assert!(!can_reseed, "Dump + standalone must NOT allow reseed (F15)");
    }

    #[test]
    fn reseed_guard_dump_sim_mode() {
        let spec = WorldSpec {
            seed: 0x1234_5678,
            standalone: false,
            bare_mode: false,
            source: WorldSource::Dump(std::path::PathBuf::from("/tmp/dump.atdmp1")),
            explicit_landform_flags: None,
        };
        // Should NOT be reseedable: Dump + sim mode
        let can_reseed = matches!(spec.source, WorldSource::Procgen { .. }) && spec.standalone;
        assert!(!can_reseed, "Dump + sim_mode must NOT allow reseed (F15)");
    }

    // U-10: Landform flag clamp tests (applied in apply_guard after guard runs)
    #[test]
    fn landform_clamps_ridges_requires_tect() {
        // Guard forces tect when all five originals are off, then clamp permits ridges.
        let flags = LandformFlags::new(false, false, false, false, false, true, false).apply_guard();
        assert!(flags.tect && flags.ridges, "guard forces tect, clamp then permits ridges");
    }

    #[test]
    fn landform_clamps_beaches_requires_coastal() {
        // Guard only forces tect (not coastal), so beaches stays clamped false.
        let flags = LandformFlags::new(false, false, false, false, false, false, true).apply_guard();
        assert!(flags.tect && !flags.coastal && !flags.beaches, "guard forces tect but not coastal; beaches stays false");
    }

    #[test]
    fn landform_clamps_both_dependencies() {
        // Guard forces tect (rescues ridges), but not coastal (beaches stays false).
        let flags = LandformFlags::new(false, false, false, false, false, true, true).apply_guard();
        assert!(flags.tect && flags.ridges && !flags.coastal && !flags.beaches, "guard forces tect; ridges true, beaches false");
    }

    #[test]
    fn landform_preserves_valid_dependencies() {
        // ridges valid when tect=true
        let flags1 = LandformFlags::new(true, false, false, false, false, true, false).apply_guard();
        assert!(flags1.ridges, "ridges should be true when tect=true");

        // beaches valid when coastal=true
        let flags2 = LandformFlags::new(false, false, false, false, true, false, true).apply_guard();
        assert!(flags2.beaches, "beaches should be true when coastal=true");
    }

    #[test]
    fn landform_guard_forces_tect_if_all_off() {
        let flags = LandformFlags {
            tect: false,
            aeolian: false,
            volcanic: false,
            glacial: false,
            coastal: false,
            ridges: false,
            beaches: false,
        };
        let guarded = flags.apply_guard();
        assert!(guarded.tect, "guard must force tectonic if all five landforms are off");
    }

    #[test]
    fn landform_guard_preserves_existing() {
        let flags = LandformFlags {
            tect: true,
            aeolian: false,
            volcanic: false,
            glacial: false,
            coastal: false,
            ridges: false,
            beaches: false,
        };
        let guarded = flags.apply_guard();
        assert!(guarded.tect, "guard should preserve existing tectonic state");
    }
}
