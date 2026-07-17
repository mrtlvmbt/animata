//! U-2: World specification and deterministic landform configuration.
//!
//! Centralizes all inputs to world building (seed, flags, source), ensuring
//! that both the app worker thread and harnesses use the same `build_world(spec)`
//! path with deterministic flag derivation from seed.

use std::path::PathBuf;

/// Landform configuration flags (deterministically derived from seed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LandformFlags {
    pub tect: bool,
    pub aeolian: bool,
    pub volcanic: bool,
    pub glacial: bool,
    pub coastal: bool,
    pub ridges: bool,
    pub beaches: bool,
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
/// - landform flags are NOT stored; they're computed from `(seed, standalone)` via `landform_flags()`
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
}

/// Stage of world building (used for progress reporting via callback).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Stage {
    GenerateWorld = 0,
    BuildMeshes = 1,
    Done = 2,
}

impl Stage {
    /// Convert to ordinal for atomic storage (u8).
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// Convert from ordinal.
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Stage::GenerateWorld),
            1 => Some(Stage::BuildMeshes),
            2 => Some(Stage::Done),
            _ => None,
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

    // Guard: never all-off for the original five (avoid flat/boring maps)
    let tect = if !(tect || aeol || volc || glac || coast) {
        true  // force tectonic if all original five are off
    } else {
        tect
    };

    // Dependency clamps (per plan W-0)
    let ridg = ridg && tect;  // ridges need tectonic uplift
    let beach = beach && coast;  // beaches need coastal datum

    LandformFlags {
        tect,
        aeolian: aeol,
        volcanic: volc,
        glacial: glac,
        coastal: coast,
        ridges: ridg,
        beaches: beach,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_roundtrip_u8() {
        assert_eq!(Stage::GenerateWorld.as_u8(), 0);
        assert_eq!(Stage::BuildMeshes.as_u8(), 1);
        assert_eq!(Stage::Done.as_u8(), 2);

        assert_eq!(Stage::from_u8(0), Some(Stage::GenerateWorld));
        assert_eq!(Stage::from_u8(1), Some(Stage::BuildMeshes));
        assert_eq!(Stage::from_u8(2), Some(Stage::Done));
        assert_eq!(Stage::from_u8(3), None);
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
        };
        // Should NOT be reseedable: Dump + sim mode
        let can_reseed = matches!(spec.source, WorldSource::Procgen { .. }) && spec.standalone;
        assert!(!can_reseed, "Dump + sim_mode must NOT allow reseed (F15)");
    }
}
