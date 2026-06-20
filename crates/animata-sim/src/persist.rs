//! Full-state save/load — a binary snapshot of a running world that resumes BIT-IDENTICALLY.
//!
//! A snapshot carries the live mutable state only: the creatures (with their genomes) and the
//! terrain overlay (vegetation + nutrient pools). The terrain GEOMETRY is NOT stored — it is a pure
//! function of `seed`, regenerated on load — which keeps the file to the parts that actually evolve.
//! With the geometry regenerated and the overlay + creatures + tick restored, the run continues
//! exactly where it left off; the [`crate::sim::state_checksum`] round-trip test is the proof.
//!
//! Format is `bincode` (compact binary): the overlay is millions of columns, so a text format would
//! be many× larger and slow. A magic tag guards against feeding in an unrelated file.

use crate::sim::SimState;
use crate::terrain::TerrainState;
use std::io::{Read, Write};

/// File magic: ASCII "ANM1". Bumped if the snapshot layout changes incompatibly.
const MAGIC: u32 = 0x414E_4D31;

/// A complete world snapshot: the seed (to regenerate terrain geometry), the clock tick to resume
/// at, the sim state (creatures + counters + config) and the terrain overlay.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    magic: u32,
    pub terrain_seed: u64,
    pub tick: u64,
    pub sim: SimState,
    pub terrain: TerrainState,
}

impl Snapshot {
    /// Assemble a snapshot from the live pieces (the bin pairs `Sim::to_state` with
    /// `VoxelTerrain::clone_state`).
    pub fn new(terrain_seed: u64, tick: u64, sim: SimState, terrain: TerrainState) -> Self {
        Snapshot { magic: MAGIC, terrain_seed, tick, sim, terrain }
    }

    /// Serialise to a writer (the bin wraps a `BufWriter<File>`).
    pub fn write(&self, w: impl Write) -> Result<(), String> {
        bincode::serialize_into(w, self).map_err(|e| e.to_string())
    }

    /// Deserialise from a reader, rejecting anything that is not an animata snapshot.
    pub fn read(r: impl Read) -> Result<Self, String> {
        let snap: Snapshot = bincode::deserialize_from(r).map_err(|e| e.to_string())?;
        if snap.magic != MAGIC {
            return Err("not an animata snapshot (bad magic)".into());
        }
        Ok(snap)
    }
}
