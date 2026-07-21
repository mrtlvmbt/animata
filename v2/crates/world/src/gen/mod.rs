//! World-gen pipeline stage home (PM plan-consensus, W-ladder). Every stage (W-1 height, W-2
//! climate/biome, W-3 hydrology, W-4 erosion, W-5 resource-caps, W-6 assembly) lives under this
//! module, one file/sub-module per stage. **Every `.rs` under `world/src/gen/` is covered by the
//! recursive glob no-float guard** (`world/tests/no_float_guard_gen.rs`, pattern
//! `world/src/gen/**/*.rs`) — a stage file needs NO per-file registration to be scanned, unlike
//! `sim-core`'s hardcoded module allow-list (`no_float_guard.rs`). This is deliberate: the whole
//! pipeline must be pure integer/fixed-point (the point of replacing the legacy `NoiseWorld`'s
//! `f64 sin` path), and the glob makes that enforced-by-construction rather than resting on
//! remembering to add each new file to a list.
//!
//! **W-6 status**: `gen::caps::classify_and_caps` now backs the production `ProcgenWorld`
//! (`world/src/lib.rs`) — the pipeline is WIRED. `NoiseWorld`, the last `f64 sin` in the world
//! path, is deleted.
//!
//! **W-SIM-4a (#396, `worldgen-relief` branch):** `tectonics` is the first landform slice — a
//! deterministic fault network consumed by `erosion::erode_with_tectonics` (fault-scarp height step
//! + fault-aligned resistance-lineament override), opt-in and default-off (`enable_tectonics`
//! threads `erode` → `classify_and_caps` → `ProcgenWorld::new`).
//!
//! **W-SIM-3a (#403, `worldgen-relief` branch):** `aeolian` is the second landform slice — a
//! deterministic Werner slab-CA (wind-driven dunes) run POST-erosion by `caps::classify_and_caps`,
//! opt-in and default-off (`enable_aeolian` threads the same way `enable_tectonics` does, orthogonal
//! to it — both are independent opt-in stages).
//!
//! **W-SIM-5 (#410, `worldgen-relief` branch):** `volcanic` is the third landform slice — CONSTRUCTIVE
//! (additive) viscosity-selected edifices, folded into the initial height field by
//! `erosion::erode_with_tectonics` PRE-erosion (the same seam tectonics already injects at), opt-in
//! and default-off (`enable_volcanic`, orthogonal to `enable_tectonics`/`enable_aeolian`).
//!
//! **W-SIM-6 (#416, `worldgen-relief` branch):** `glacial` is the fourth landform slice — an
//! ELA-gated ice-incision (subtractive) + till-deposition (additive) pass run POST-erosion,
//! PRE-aeolian by `caps::classify_and_caps`, opt-in and default-off (`enable_glacial`, orthogonal to
//! `enable_tectonics`/`enable_aeolian`/`enable_volcanic`).
//!
//! **W-SIM-7 (#423, `worldgen-relief` branch):** `coastal` is the fifth and LAST landform slice — a
//! sea-level datum (the world's first water) + cliff/wave-cut-platform pass run POST-aeolian,
//! PRE-final-classify by `caps::classify_and_caps`, opt-in and default-off (`enable_coastal`,
//! orthogonal to `enable_tectonics`/`enable_aeolian`/`enable_volcanic`/`enable_glacial`).

pub mod aeolian;
pub mod biome;
pub mod caps;
pub mod climate;
pub mod coastal;
pub mod drainage;
pub mod erosion;
pub mod glacial;
pub mod height;
pub mod material;
pub mod moisture;
pub mod orogeny;
pub mod plate;
pub mod tectonics;
pub mod volcanic;

/// W-0 landform flags struct — refactored from 5-bool tuple to named fields.
/// W-18: additive worldgen — SOURCES (base, tect/ridges, volcanic) vs TRANSFORMS (erosion, aeolian, glacial, coastal/beaches).
/// Splitmix64 bit layout: base at shift 47, erosion at shift 29, tect/aeolian/volcanic/glacial/coastal at shifts 3/13/23/33/43 (unchanged);
/// ridges at shift 53, beaches at shift 59. Dependency clamps: `ridges &= tect`, `beaches &= coastal`.
/// Empty-set guard ORs only the original five (new bits are dependent riders).
/// W-19: strength parameters for erosion and glacial, default 100 (percent, byte-identical to no-strength baseline).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    pub erosion_strength: i64,  // W-19: erosion intensity percent, default 100, range [0, 400]
    pub glacial_strength: i64,  // W-19: glacial intensity percent, default 100, range [0, 400]
}

impl Default for LandformFlags {
    fn default() -> Self {
        LandformFlags {
            base: true,
            tect: false,
            aeolian: false,
            volcanic: false,
            glacial: false,
            coastal: false,
            erosion: true,
            ridges: false,
            beaches: false,
            erosion_strength: 100,
            glacial_strength: 100,
        }
    }
}

impl LandformFlags {
    /// Convenience constructor: all flags from the original five booleans, ridges and beaches default false.
    /// **Dependency clamps applied:** ridges only valid if tect is true; beaches only if coastal is true.
    /// W-18: base and erosion default to true (preserves pre-slice behavior).
    /// W-19: strength defaults to 100 (byte-identical baseline).
    pub fn from_five(tect: bool, aeolian: bool, volcanic: bool, glacial: bool, coastal: bool) -> Self {
        LandformFlags {
            base: true,
            tect,
            aeolian,
            volcanic,
            glacial,
            coastal,
            erosion: true,
            ridges: false,
            beaches: false,
            erosion_strength: 100,
            glacial_strength: 100,
        }
    }

    /// Construct with explicit ridges/beaches, applying dependency clamps per W-0 contract.
    /// W-18: base and erosion must be set explicitly; no defaults applied here.
    /// W-19: strength parameters (default 100 for byte-identity; range [0, 400]).
    pub fn new(base: bool, tect: bool, aeolian: bool, volcanic: bool, glacial: bool, coastal: bool, erosion: bool, mut ridges: bool, mut beaches: bool, erosion_strength: i64, glacial_strength: i64) -> Self {
        // W-0 dependency clamps: ridges requires tect, beaches requires coastal.
        ridges = ridges && tect;
        beaches = beaches && coastal;
        LandformFlags {
            base,
            tect,
            aeolian,
            volcanic,
            glacial,
            coastal,
            erosion,
            ridges,
            beaches,
            erosion_strength,
            glacial_strength,
        }
    }
}

/// Slice-0 (terragen-v3): Terrain process parameters for physics-driven relief generation.
/// Aggregates plate simulation, sea-level datum, and per-process strength modifiers.
/// All fields are `i64` or `bool` — the struct is `Copy` (required for threading through the gen pipeline).
/// Defaults reproduce current behavior exactly (byte-identical goldens).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerrainProcessParams {
    /// Plate tectonics simulation enable flag (default `false`; plate sim not yet implemented in Slice-0).
    pub enable_plate_sim: bool,
    /// Number of plates for simulation (default `15`; inert this slice).
    pub plate_count: i64,
    /// Plate orogeny strength, percent (default `100`; inert this slice).
    pub plate_strength: i64,
    /// ELA (Equilibrium Line Altitude) threshold, percent of hmax (default `60`; inert this slice).
    pub ela_threshold_percent: i64,
    /// Sea level datum (default `-1` = unset / derive from context; **sentinel value** — height 0 is
    /// valid, so `-1` marks "no explicit sea level"). When `< 0`, sea level is derived as today;
    /// when `>= 0`, the given height becomes the world's water datum (e.g. `sea_level=100` makes all
    /// cells below 100 submerged). Inert this slice (coastal stage already uses internal logic; this
    /// field is plumbing for future integration).
    pub sea_level: i64,
    /// Volcanic edifice construction strength, percent (default `100`; inert this slice).
    pub volcanic_strength: i64,
    /// Aeolian (wind-driven) dune formation strength, percent (default `100`; inert this slice).
    pub aeolian_strength: i64,
    /// Coastal (wave-cut platform + cliff) carving strength, percent (default `100`; inert this slice).
    pub coastal_strength: i64,
}

impl Default for TerrainProcessParams {
    fn default() -> Self {
        TerrainProcessParams {
            enable_plate_sim: false,
            plate_count: 15,
            plate_strength: 100,
            ela_threshold_percent: 60,
            sea_level: -1,
            volcanic_strength: 100,
            aeolian_strength: 100,
            coastal_strength: 100,
        }
    }
}

/// Marker: base-fBm is SIM-LANE-ONLY per terragen-v3 decision #2.
/// **Sunset condition:** Goldens re-pinned on physics world OR sim map contract renegotiated.
/// **Sunset action:** Removed in the fBm-removal slice (Slice-4): replace every `if SIM_ONLY_BASE_FBM`
/// guard with unconditional logic (unwrap the negation), then delete the const. No behavior change
/// this slice; the marker is introduced for future slicing.
pub const SIM_ONLY_BASE_FBM: bool = true;
