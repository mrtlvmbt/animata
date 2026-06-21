//! Frozen ANM2 snapshot shape + the ANM2→current (ANM3) migration. The per-type frozen shapes live
//! WITH the modules that own their private fields (`genome::GenomeV2`, `sim::{CreatureV2,SimStateV2}`,
//! `terrain::TerrainStateV2`, `sim_config::{FeaturesV2,ParamsV2,SimConfigV2}`), each with a `migrate`
//! that fills the new fields by CONTINUITY (a pre-feature save was anoxic: `oxygen_tolerance = 0`,
//! oxygen overlay empty, the `oxygen` feature off). NEVER edit the frozen shapes — they must reproduce
//! the EXACT ANM2 bincode layout. This module just composes the top-level body.

use super::Snapshot;
use crate::sim::SimStateV2;
use crate::terrain::TerrainStateV2;

/// ANM2 body (magic-less): `Snapshot` with the frozen ANM2 `sim`/`terrain` shapes.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct SnapshotBodyV2 {
    pub terrain_seed: u64,
    pub tick: u64,
    pub sim: SimStateV2,
    pub terrain: TerrainStateV2,
}

/// ANM2 → current `Snapshot`. Delegates to each frozen shape's `migrate` (gas-cycle Phase 1 added
/// `Genome.oxygen_tolerance`, `Params.oxygen_lethality`, `Features.oxygen`, the terrain oxygen overlay).
pub(crate) fn migrate(b: SnapshotBodyV2) -> Snapshot {
    Snapshot {
        terrain_seed: b.terrain_seed,
        tick: b.tick,
        sim: b.sim.migrate(),
        terrain: b.terrain.migrate(),
    }
}
