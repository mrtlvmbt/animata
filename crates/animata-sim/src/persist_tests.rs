//! Persistence format lock. The round-trip (write→read→resume bit-identical) lives in `sim_tests`
//! (`snapshot_round_trips_bit_identical`); HERE we lock the ON-DISK BYTE LAYOUT so a future version
//! bump can't silently break decoding of real, old saves.

use super::*;
use crate::sim::Sim;
use crate::terrain::VoxelTerrain;

/// Replicates the PRE-refactor on-disk layout: `magic` as serialized field 0, then the body fields in
/// order. The legacy writer was `bincode::serialize_into(w, &Snapshot{ magic, terrain_seed, tick,
/// sim, terrain })`; this struct reproduces those exact bytes.
#[derive(serde::Serialize)]
struct SnapshotV2WithMagic {
    magic: u32,
    terrain_seed: u64,
    tick: u64,
    sim: SimState,
    terrain: TerrainState,
}

/// THE old-write↔new-read lock (F4/F5/F6): the new writer (hand-written 4-byte magic prefix +
/// magic-less body) must produce bytes IDENTICAL to the legacy `serialize_into(Snapshot{ magic, .. })`
/// layout. Byte-identity on a POPULATED snapshot (≥1 creature with a developed phenotype + a dirtied
/// overlay — never 0-creature, which would emit only a `Vec` length and skip the volatile
/// `Genome`/`Phenotype` bytes) proves the prefix split preserves the exact ANM2 format, so any real
/// old on-disk file (= legacy-writer output) still decodes through `Snapshot::read`. No committed blob
/// and no profile-dependent hash needed: it compares the two write paths on the same in-memory data.
#[test]
fn new_write_is_byte_identical_to_legacy_anm2_layout() {
    let mut t = VoxelTerrain::new(42);
    let mut s = Sim::new(42, &t);
    for tick in 0..300 {
        s.step(&mut t, tick);
    }
    assert!(s.population() > 1, "fixture must carry creatures to exercise Genome/Phenotype byte shapes");

    let snap = Snapshot::new(42, 300, s.to_state(), t.clone_state());
    let mut new_bytes = Vec::new();
    snap.write(&mut new_bytes).expect("new writer");

    // Move the (un-cloneable-cheaply) body into the legacy-layout struct AFTER new_bytes is captured.
    let legacy = SnapshotV2WithMagic {
        magic: MAGIC,
        terrain_seed: snap.terrain_seed,
        tick: snap.tick,
        sim: snap.sim,
        terrain: snap.terrain,
    };
    let legacy_bytes = bincode::serialize(&legacy).expect("legacy layout");

    assert_eq!(
        new_bytes, legacy_bytes,
        "new writer must reproduce the legacy ANM2 on-disk byte layout (else old saves mis-decode)"
    );
}

/// A foreign / corrupt file is rejected by the magic check, not silently mis-decoded.
#[test]
fn read_rejects_unknown_magic() {
    let junk = [0xDEu8, 0xAD, 0xBE, 0xEF, 0, 0, 0, 0];
    assert!(Snapshot::read(&junk[..]).is_err(), "unknown magic must be rejected");
}
