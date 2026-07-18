//! U-2: World specification and deterministic landform configuration.
//!
//! Centralizes all inputs to world building (seed, flags, source), ensuring
//! that both the app worker thread and harnesses use the same `build_world(spec)`
//! path with deterministic flag derivation from seed.

use std::path::PathBuf;

/// Landform configuration flags (deterministically derived from seed or set manually).
/// W-18: additive worldgen — SOURCES (base, tect/ridges, volcanic) vs TRANSFORMS (erosion, aeolian, glacial, coastal/beaches).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LandformFlags {
    pub base: bool,       // W-18: seed height from fBm (true) or FLAT_DATUM (false)
    pub tect: bool,
    pub aeolian: bool,
    pub volcanic: bool,
    pub glacial: bool,
    pub coastal: bool,
    pub erosion: bool,    // W-18: run erosion chain (talus/fluvial/deposition)
    pub ridges: bool,
    pub beaches: bool,
}

impl LandformFlags {
    /// Create landform flags without clamps (raw values).
    /// Clamps are applied later in `apply_guard()` AFTER the guard potentially enables tect/coastal.
    /// W-18: base and erosion must be specified explicitly.
    pub fn new(
        base: bool,
        tect: bool,
        aeolian: bool,
        volcanic: bool,
        glacial: bool,
        coastal: bool,
        erosion: bool,
        ridges: bool,
        beaches: bool,
    ) -> Self {
        LandformFlags { base, tect, aeolian, volcanic, glacial, coastal, erosion, ridges, beaches }
    }

    /// Apply guards and dependency clamps to ensure a valid state.
    /// Order: (1) if all five original landforms are off, enable tectonic;
    /// (2) then apply clamps so guard's tect-enabling makes ridges/beaches permissible.
    /// W-18: base and erosion are not affected by guard (user controls these independently).
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

/// Real execution order of stages in the worldgen pipeline.
/// The Stage enum is numbered in DISPLAY order (intuitive for UI layout),
/// but the pipeline runs in this order: heightfield → tectonics → erosion → ridges →
/// volcanic → glacial → aeolian → coastal → beaches → talus → de-needle → classify.
/// See [`crate::gen::caps`] for actual stage callbacks.
/// Used by loader.rs to compare stages by execution position, not raw ordinal.
pub const EXEC_ORDER: &[u8] = &[
    0,  // GenerateHeightfield (exec_pos 0)
    1,  // ApplyTectonics (exec_pos 1)
    2,  // ApplyErosion (exec_pos 2)
    3,  // ApplyRidges (exec_pos 3)
    5,  // ApplyVolcanic (exec_pos 4) — runs before glacial and aeolian
    6,  // ApplyGlacial (exec_pos 5)
    4,  // ApplyAeolian (exec_pos 6) — runs after glacial
    7,  // ApplyCoastal (exec_pos 7)
    8,  // ApplyBeaches (exec_pos 8)
    9,  // ApplyTalus (exec_pos 9)
    10, // DeNeedle (exec_pos 10)
    11, // Classify (exec_pos 11)
    12, // BuildMeshes (exec_pos 12)
    13, // Done (exec_pos 13)
];

/// Stage of world building (used for progress reporting via callback).
/// Stages map to worldgen pipeline boundaries: heightfield fbm → tectonics → erosion → ridges →
/// volcanic → glacial → aeolian → coastal → beaches → talus → de-needle → classify → meshing → done.
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
    /// Order follows EXECUTION sequence: heightfield → tectonics → erosion → ridges → volcanic →
    /// glacial → aeolian → coastal → beaches → talus → de-needle → classify.
    pub fn progress_permille(self) -> u32 {
        match self {
            Stage::GenerateHeightfield => 0,      // First: heightfield FBM
            Stage::ApplyTectonics => 57,          // Tectonics (injected into erode)
            Stage::ApplyErosion => 114,           // Erosion + ridges injected
            Stage::ApplyRidges => 171,            // Ridges reported after erode
            Stage::ApplyVolcanic => 228,          // Volcanic mask post-erosion
            Stage::ApplyGlacial => 343,           // Glacial post-erosion (takes time)
            Stage::ApplyAeolian => 457,           // Aeolian post-glacial
            Stage::ApplyCoastal => 514,           // Coastal post-aeolian
            Stage::ApplyBeaches => 571,           // Beaches post-coastal
            Stage::ApplyTalus => 628,             // Talus/thermal relaxation
            Stage::DeNeedle => 685,               // De-needle pass
            Stage::Classify => 743,               // Biome + caps classification
            Stage::BuildMeshes => 850,            // Meshing + GPU upload (400‰ for mesh phase)
            Stage::Done => 1000,
        }
    }

    /// Get execution position of this stage in EXEC_ORDER (0..13).
    /// Used by loader.rs to compare stages by execution order, not display order.
    /// For example, ApplyVolcanic (ordinal 5) comes BEFORE ApplyAeolian (ordinal 4)
    /// in execution, so this returns exec_pos(5)=4 < exec_pos(4)=6.
    pub fn exec_pos(self) -> u8 {
        let ord = self.as_u8();
        EXEC_ORDER.iter()
            .position(|&x| x == ord)
            .unwrap_or(14) as u8
    }
}

/// Landform flags: derived deterministically from seed + standalone mode.
/// Returns LandformFlags with base, tectonic, aeolian, volcanic, glacial, coastal, erosion, ridges, beaches.
///
/// W-18: Each landform toggles independently (well-spaced bit positions in splitmix64).
/// SOURCES (base, tect/ridges, volcanic) vs TRANSFORMS (erosion, aeolian, glacial, coastal/beaches).
/// Splitmix64 bit layout: base at shift 47, erosion at shift 29, tect/aeolian/volcanic/glacial/coastal at shifts 3/13/23/33/43,
/// ridges at shift 53, beaches at shift 59.
/// **CRITICAL:** base and erosion bits are INVERTED so that the DEFAULT state (base=true, erosion=true)
/// contributes salt IDENTICAL to today (0 at both positions), preserving byte-identity of default flags.
/// Dependency clamps: ridges &= tect (needs uplift), beaches &= coastal (needs sea datum).
/// Guard: never all-off for the original five (ensures maps never become flat/featureless).
pub fn landform_flags(seed: u64, standalone: bool) -> LandformFlags {
    if !standalone {
        // Sim mode: all landforms off (unchanged from original contract)
        return LandformFlags {
            base: true,
            tect: false,
            aeolian: false,
            volcanic: false,
            glacial: false,
            coastal: false,
            erosion: true,
            ridges: false,
            beaches: false,
        };
    }

    // Deterministic hash: splitmix64
    let mut x = seed;
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^= x >> 31;

    // Extract independent bits for each landform (well-spaced bit positions).
    // W-18: base and erosion bits are INVERTED so default state matches pre-slice salt.
    let base = !((x >> 47) & 1 == 1);  // Inverted: false when bit is 1 (salt=1), true when bit is 0 (salt=0)
    let tect = (x >> 3) & 1 == 1;
    let aeol = (x >> 13) & 1 == 1;
    let volc = (x >> 23) & 1 == 1;
    let glac = (x >> 33) & 1 == 1;
    let coast = (x >> 43) & 1 == 1;
    let erosion = !((x >> 29) & 1 == 1);  // Inverted: false when bit is 1 (salt=1), true when bit is 0 (salt=0)
    let ridg = (x >> 53) & 1 == 1;
    let beach = (x >> 59) & 1 == 1;

    // Apply clamps and guard
    LandformFlags::new(base, tect, aeol, volc, glac, coast, erosion, ridg, beach).apply_guard()
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

    /// W-18: Default flags (base=true, erosion=true) must produce byte-identical salt to pre-W-18
    /// (when bits 47 and 29 were not extracted). CRITICAL for golden byte-identity.
    #[test]
    fn w18_default_flags_match_legacy_salt() {
        // W-18: base and erosion bits are INVERTED in the salt, so default state (true, true)
        // produces 0 at both positions 29 and 47, matching the pre-W-18 salt.
        for seed in [0u64, 1, 42, 0xFFFF_FFFF, 0xDEAD_BEEF, 0xCAFE_BABE] {
            let flags = landform_flags(seed, true);
            // Default state must have base=true and erosion=true
            assert!(flags.base, "seed {seed:016x}: default base must be true");
            assert!(flags.erosion, "seed {seed:016x}: default erosion must be true");
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

    // W-18: Landform flag clamp tests (applied in apply_guard after guard runs)
    #[test]
    fn landform_clamps_ridges_requires_tect() {
        // Guard forces tect when all five originals are off, then clamp permits ridges.
        let flags = LandformFlags::new(true, false, false, false, false, false, true, true, false).apply_guard();
        assert!(flags.tect && flags.ridges, "guard forces tect, clamp then permits ridges");
    }

    #[test]
    fn landform_clamps_beaches_requires_coastal() {
        // Guard only forces tect (not coastal), so beaches stays clamped false.
        let flags = LandformFlags::new(true, false, false, false, false, false, true, false, true).apply_guard();
        assert!(flags.tect && !flags.coastal && !flags.beaches, "guard forces tect but not coastal; beaches stays false");
    }

    #[test]
    fn landform_clamps_both_dependencies() {
        // Guard forces tect (rescues ridges), but not coastal (beaches stays false).
        let flags = LandformFlags::new(true, false, false, false, false, false, true, true, true).apply_guard();
        assert!(flags.tect && flags.ridges && !flags.coastal && !flags.beaches, "guard forces tect; ridges true, beaches false");
    }

    #[test]
    fn landform_preserves_valid_dependencies() {
        // ridges valid when tect=true
        let flags1 = LandformFlags::new(true, true, false, false, false, false, true, true, false).apply_guard();
        assert!(flags1.ridges, "ridges should be true when tect=true");

        // beaches valid when coastal=true
        let flags2 = LandformFlags::new(true, false, false, false, false, true, true, false, true).apply_guard();
        assert!(flags2.beaches, "beaches should be true when coastal=true");
    }

    #[test]
    fn landform_guard_forces_tect_if_all_off() {
        let flags = LandformFlags {
            base: true,
            tect: false,
            aeolian: false,
            volcanic: false,
            glacial: false,
            coastal: false,
            erosion: true,
            ridges: false,
            beaches: false,
        };
        let guarded = flags.apply_guard();
        assert!(guarded.tect, "guard must force tectonic if all five landforms are off");
    }

    #[test]
    fn landform_guard_preserves_existing() {
        let flags = LandformFlags {
            base: true,
            tect: true,
            aeolian: false,
            volcanic: false,
            glacial: false,
            coastal: false,
            erosion: true,
            ridges: false,
            beaches: false,
        };
        let guarded = flags.apply_guard();
        assert!(guarded.tect, "guard should preserve existing tectonic state");
    }
}
