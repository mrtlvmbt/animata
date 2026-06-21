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

/// Diagnostic (ignored): the on-disk I/O cost of a save — `write` + `read`/parse of the snapshot
/// (the overlay arrays dominate at MAP_SCALE=16) + the `set_state` restore. Quantifies the
/// non-generation part of a world LOAD. Run with `--release`.
#[test]
#[ignore]
fn report_load_io_cost() {
    let mut t = VoxelTerrain::new(42);
    let mut s = Sim::new(42, &t);
    for tick in 0..300 {
        s.step(&mut t, tick);
    }
    let snap = Snapshot::new(42, 300, s.to_state(), t.clone_state());
    let mut bytes = Vec::new();
    let tw = std::time::Instant::now();
    snap.write(&mut bytes).expect("write");
    let write_ms = tw.elapsed().as_secs_f64() * 1000.0;
    let tr = std::time::Instant::now();
    let restored = Snapshot::read(&bytes[..]).expect("read");
    let read_ms = tr.elapsed().as_secs_f64() * 1000.0;
    let mut t2 = VoxelTerrain::new(restored.terrain_seed);
    let ts = std::time::Instant::now();
    t2.set_state(restored.terrain).expect("set_state");
    let set_ms = ts.elapsed().as_secs_f64() * 1000.0;
    let line = format!(
        "[persist] {} KB · write {write_ms:.0}ms · read/parse {read_ms:.0}ms · set_state {set_ms:.2}ms (MAP_SCALE={})",
        bytes.len() / 1024,
        crate::config::MAP_SCALE
    );
    let _ = std::fs::write("/tmp/animata_persist.txt", &line);
    eprintln!("{line}");
}

/// Migration lock (migrate-not-reject): a real ANM2 stream (`MAGIC_V2` + the frozen pre-gas-cycle
/// body) decodes through the CURRENT `Snapshot::read` and upgrades to ANM3 — preserving every existing
/// field and filling the new ones by CONTINUITY (oxygen feature off, `oxygen_tolerance`/overlay at 0).
/// Built from a POPULATED world (≥1 creature with a developed genome + a dirtied overlay) so the
/// volatile `Genome`/overlay bytes are actually exercised, then down-converted via `to_v2`.
#[test]
fn anm2_stream_migrates_to_current() {
    let mut t = VoxelTerrain::new(7);
    let mut s = Sim::new(7, &t);
    for tick in 0..120 {
        s.step(&mut t, tick);
    }
    let st = s.to_state();
    let (n, photo, seed) = (st.creatures.len(), st.cfg.params.photo_rate, st.world_seed);
    assert!(n > 1, "fixture must carry creatures to exercise the frozen Genome bytes");

    // Write a genuine ANM2 stream: MAGIC_V2 prefix + the frozen ANM2 body (down-converted).
    let body = super::v2::SnapshotBodyV2 { terrain_seed: 7, tick: 120, sim: st.to_v2(), terrain: t.clone_state().to_v2() };
    let mut anm2 = MAGIC_V2.to_le_bytes().to_vec();
    bincode::serialize_into(&mut anm2, &body).expect("write ANM2 body");

    let snap = Snapshot::read(&anm2[..]).expect("ANM2 stream must MIGRATE, not be rejected");
    assert_eq!(snap.tick, 120);
    assert_eq!(snap.terrain_seed, 7);
    assert_eq!(snap.sim.world_seed, seed);
    assert_eq!(snap.sim.creatures.len(), n, "creatures preserved through migration");
    assert_eq!(snap.sim.cfg.params.photo_rate, photo, "existing params preserved");
    assert!(!snap.sim.cfg.features.oxygen, "CONTINUITY: a migrated ANM2 save resumes oxygen-off (anoxic)");
}

/// A foreign / corrupt file is rejected by the magic check, not silently mis-decoded.
#[test]
fn read_rejects_unknown_magic() {
    let junk = [0xDEu8, 0xAD, 0xBE, 0xEF, 0, 0, 0, 0];
    assert!(Snapshot::read(&junk[..]).is_err(), "unknown magic must be rejected");
}
