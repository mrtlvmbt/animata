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
//!
//! ## Versioning & migration (forward, never reject)
//! The 4-byte magic is written/read as a **hand-managed LE prefix** — NOT a serde field — so the body
//! struct is magic-less and `read()` can peek the tag, then dispatch to the matching version's
//! deserializer and migrate forward to current. Today only `MAGIC` (ANM2) is current, so the single
//! arm is an identity load. When a future change bumps the layout (e.g. a new `Params` field): bump
//! `MAGIC` to ANM3, FREEZE the then-current body shape into a `v2` module (`SnapshotBodyV2` +
//! whatever nested types diverge), and add a `MAGIC_V2 => migrate_v2(...)` arm that maps old→current
//! (a new field gets its CONTINUITY no-op value unless product wants convergence). The
//! `new_write_is_byte_identical_to_legacy_anm2_layout` test locks that this prefix split preserves the
//! exact on-disk ANM2 bytes, so old files keep decoding.

use crate::sim::SimState;
use crate::terrain::TerrainState;
use std::io::{Read, Write};

/// File magic: ASCII "ANM2" (LE on disk). The CURRENT snapshot version. Written as a 4-byte prefix
/// before the body, and matched on read to pick the (de)serializer. Bump on an incompatible layout
/// change AND add a migration arm (see the module-level "Versioning & migration" note) — the policy
/// is to MIGRATE old saves forward, not reject them.
const MAGIC: u32 = 0x414E_4D32;

/// A complete world snapshot body: the seed (to regenerate terrain geometry), the clock tick to
/// resume at, the sim state (creatures + counters + config) and the terrain overlay. The magic tag is
/// NOT a field here — it is the 4-byte prefix handled by [`Snapshot::write`]/[`Snapshot::read`], so
/// the serialized body stays magic-less and each future frozen `SnapshotBodyVN` stays symmetric.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    pub terrain_seed: u64,
    pub tick: u64,
    pub sim: SimState,
    pub terrain: TerrainState,
}

impl Snapshot {
    /// Assemble a snapshot from the live pieces (the bin pairs `Sim::to_state` with
    /// `VoxelTerrain::clone_state`).
    pub fn new(terrain_seed: u64, tick: u64, sim: SimState, terrain: TerrainState) -> Self {
        Snapshot { terrain_seed, tick, sim, terrain }
    }

    /// Serialise to a writer (the bin wraps a `BufWriter<File>`): the 4-byte magic prefix, then the
    /// bincode body. This is byte-identical to the legacy `serialize_into(Snapshot{ magic, .. })`
    /// layout (magic was field 0), proven by `new_write_is_byte_identical_to_legacy_anm2_layout`.
    pub fn write(&self, mut w: impl Write) -> Result<(), String> {
        w.write_all(&MAGIC.to_le_bytes()).map_err(|e| e.to_string())?;
        bincode::serialize_into(w, self).map_err(|e| e.to_string())
    }

    /// Deserialise from a reader: peek the 4-byte magic prefix, dispatch to the matching version, and
    /// migrate forward to current. Unknown/too-old magic is rejected.
    pub fn read(mut r: impl Read) -> Result<Self, String> {
        let mut tag = [0u8; 4];
        r.read_exact(&mut tag).map_err(|e| e.to_string())?;
        match u32::from_le_bytes(tag) {
            // Current == ANM2 ⇒ identity load. A future bump adds e.g.
            //   MAGIC_V2 => migrate_v2(bincode::deserialize_from::<_, v2::SnapshotBodyV2>(r)?),
            MAGIC => bincode::deserialize_from(r).map_err(|e| e.to_string()),
            other => Err(format!("not a supported animata snapshot (magic 0x{other:08X})")),
        }
    }
}

#[cfg(test)]
#[path = "persist_tests.rs"]
mod tests;
