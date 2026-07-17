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
pub mod tectonics;
pub mod volcanic;

/// W-0 landform flags struct — refactored from 5-bool tuple to named fields.
/// Splitmix64 bit layout: tect/aeolian/volcanic/glacial/coastal at shifts 3/13/23/33/43 (unchanged);
/// ridges at shift 53, beaches at shift 59. Dependency clamps: `ridges &= tect`, `beaches &= coastal`.
/// Empty-set guard ORs only the original five (new bits are dependent riders).
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

impl LandformFlags {
    /// Convenience constructor: all flags from the original five booleans, ridges and beaches default false.
    /// **Dependency clamps applied:** ridges only valid if tect is true; beaches only if coastal is true.
    pub fn from_five(tect: bool, aeolian: bool, volcanic: bool, glacial: bool, coastal: bool) -> Self {
        LandformFlags { tect, aeolian, volcanic, glacial, coastal, ridges: false, beaches: false }
    }

    /// Construct with explicit ridges/beaches, applying dependency clamps per W-0 contract.
    pub fn new(tect: bool, aeolian: bool, volcanic: bool, glacial: bool, coastal: bool, mut ridges: bool, mut beaches: bool) -> Self {
        // W-0 dependency clamps: ridges requires tect, beaches requires coastal.
        ridges = ridges && tect;
        beaches = beaches && coastal;
        LandformFlags { tect, aeolian, volcanic, glacial, coastal, ridges, beaches }
    }
}
