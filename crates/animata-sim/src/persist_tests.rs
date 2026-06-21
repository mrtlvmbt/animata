//! Persistence format lock. The round-trip (write→read→resume bit-identical) lives in `sim_tests`
//! (`snapshot_round_trips_bit_identical`); HERE we lock the ON-DISK BYTE LAYOUT so a future version
//! bump can't silently break decoding of real, old saves.

use super::*;
use crate::sim::Sim;
use crate::terrain::VoxelTerrain;

/// Replicates the field-0-magic on-disk layout: `magic` as serialized field 0, then the body fields in
/// order — i.e. what `bincode::serialize_into(w, &Snapshot{ magic, terrain_seed, tick, sim, terrain })`
/// would write. Used to prove the hand-written-prefix writer reproduces it byte-for-byte (the format
/// lock is version-agnostic: it holds for whatever the CURRENT `Snapshot` body shape is).
#[derive(serde::Serialize)]
struct SnapshotWithField0Magic {
    magic: u32,
    terrain_seed: u64,
    tick: u64,
    sim: SimState,
    terrain: TerrainState,
}

/// Format lock: the writer (hand-written 4-byte magic prefix + magic-less body) must produce bytes
/// IDENTICAL to the field-0-magic layout, on a POPULATED snapshot (≥1 creature with a developed
/// phenotype + a dirtied overlay — never 0-creature, which would emit only a `Vec` length and skip the
/// volatile `Genome`/`Phenotype` bytes). This proves the prefix split preserves the exact bincode
/// layout for the current version, so a future frozen `SnapshotBodyVN` can be diffed against it. No
/// committed blob and no profile-dependent hash: it compares the two write paths on the same data.
#[test]
fn new_write_is_byte_identical_to_field0_magic_layout() {
    let mut t = VoxelTerrain::new(42);
    let mut s = Sim::new(42, &t);
    for tick in 0..300 {
        s.step(&mut t, tick);
    }
    assert!(s.population() > 1, "fixture must carry creatures to exercise Genome/Phenotype byte shapes");

    let snap = Snapshot::new(42, 300, s.to_state(), t.clone_state());
    let mut new_bytes = Vec::new();
    snap.write(&mut new_bytes).expect("new writer");

    // Move the (un-cloneable-cheaply) body into the field-0-magic struct AFTER new_bytes is captured.
    let legacy = SnapshotWithField0Magic {
        magic: MAGIC,
        terrain_seed: snap.terrain_seed,
        tick: snap.tick,
        sim: snap.sim,
        terrain: snap.terrain,
    };
    let legacy_bytes = bincode::serialize(&legacy).expect("field-0-magic layout");

    assert_eq!(
        new_bytes, legacy_bytes,
        "writer must reproduce the field-0-magic on-disk byte layout (else old saves mis-decode)"
    );
}

/// A foreign / corrupt file is rejected by the magic check, not silently mis-decoded.
#[test]
fn read_rejects_unknown_magic() {
    let junk = [0xDEu8, 0xAD, 0xBE, 0xEF, 0, 0, 0, 0];
    assert!(Snapshot::read(&junk[..]).is_err(), "unknown magic must be rejected");
}
