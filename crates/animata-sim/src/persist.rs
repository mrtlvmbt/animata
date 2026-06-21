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
//! struct is magic-less and `read()` peeks the tag, then dispatches to the matching version's
//! deserializer and MIGRATES forward to current. Current is `MAGIC` (ANM3); `MAGIC_V2` (ANM2) decodes
//! through the frozen [`v2`] shapes and `v2::migrate`. To add the NEXT version: bump `MAGIC`, FREEZE
//! the then-current body shape (the per-type frozen `*V2`-style structs live WITH the modules owning
//! their private fields — `genome`/`sim`/`terrain`/`sim_config` — each with a `migrate` that fills new
//! fields by CONTINUITY, i.e. the value that resumes the save AS IT RAN, unless product wants
//! convergence), and add a `MAGIC_VPREV => migrate(...)` arm. The
//! `new_write_is_byte_identical_to_field0_magic_layout` test locks the prefix split; the
//! `anm2_stream_migrates_to_current` test locks old→new decoding.

use crate::sim::SimState;
use crate::terrain::TerrainState;
use std::io::{Read, Write};

/// File magic: ASCII "ANM4" (LE on disk). The CURRENT snapshot version. Written as a 4-byte prefix
/// before the body, and matched on read to pick the (de)serializer. Bumped from ANM3 for gas-cycle
/// Phase 2 (new trailing fields: `Genome.aerobic_capacity`, `Params.aerobic_gain`, `Features.aerobic`).
const MAGIC: u32 = 0x414E_4D34;
/// Version "ANM2" (pre-gas-cycle). Decoded via the frozen [`v2::SnapshotBodyV2`] graph and upgraded by
/// [`v2::migrate`] all the way to current (continuity: oxygen_tolerance/aerobic_capacity 0, overlays
/// empty, oxygen/aerobic features off). Preserves the real ANM2 datum (the 486k save).
const MAGIC_V2: u32 = 0x414E_4D32;
// TODO(gas-cycle): ANM3 (Phase-1) saves are currently REJECTED — the ANM3→ANM4 migration (a frozen
// `v3` graph mirroring `v2`) is a focused follow-up. No real ANM3 saves exist yet (Phase 1 landed
// immediately before Phase 2), so the only datum that matters (the ANM2 486k save) stays loadable.
const MAGIC_V3: u32 = 0x414E_4D33;

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
            MAGIC => bincode::deserialize_from(r).map_err(|e| e.to_string()),
            // ANM2 (pre-gas-cycle): decode the frozen body, upgrade to current (continuity defaults).
            MAGIC_V2 => {
                let body: v2::SnapshotBodyV2 =
                    bincode::deserialize_from(r).map_err(|e| e.to_string())?;
                Ok(v2::migrate(body))
            }
            // ANM3 (gas-cycle Phase 1) migration is a pending follow-up (see MAGIC_V3 TODO).
            MAGIC_V3 => Err("ANM3 (Phase-1) saves: migration to ANM4 is a pending follow-up PR".into()),
            other => Err(format!("not a supported animata snapshot (magic 0x{other:08X})")),
        }
    }
}

#[path = "persist_v2.rs"]
mod v2;

#[cfg(test)]
#[path = "persist_tests.rs"]
mod tests;
