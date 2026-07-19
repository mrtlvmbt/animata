//! W-17 G1 byte-equivalence harness — verifies parallel output is identical cross-invocation.
//! Builds the world twice in-process with fixed parameters, computes field checksums,
//! and asserts byte-for-byte equality (height/material/caps).
//!
//! Usage: `cargo run --release --bin w17_byte_equiv`
//! For CI: invoked 3 times as separate processes with RAYON_NUM_THREADS=1/2/unset

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use world::ProcgenWorld;
use sim_core::{WorldView, Vec2Fixed};

const SEED: u64 = 0xA11A_2A11;
const DIM: i64 = 256;
const HMAX: i64 = 200;
const RESOURCE_BASE: i64 = 120;

fn compute_checksum(w: &ProcgenWorld) -> u64 {
    let mut hasher = DefaultHasher::new();
    // Hash all fields that constitute the world output
    for x in 0..DIM {
        for z in 0..DIM {
            let pos = Vec2Fixed(x, z);
            // Get each field via WorldView trait methods (public interface)
            let h = w.height(x, z);
            let is_solid = w.is_solid(pos);
            let biome = w.biome(pos);
            let resource = w.resource(pos);

            h.hash(&mut hasher);
            is_solid.hash(&mut hasher);
            biome.hash(&mut hasher);
            resource.hash(&mut hasher);
        }
    }
    hasher.finish()
}

fn build_world(all_on: bool) -> (ProcgenWorld, u64) {
    let w = ProcgenWorld::new(
        DIM,
        HMAX,
        RESOURCE_BASE,
        SEED,
        None,
        true,  // enable_base
        all_on,  // enable_tectonics
        all_on,  // enable_aeolian
        all_on,  // enable_volcanic
        all_on,  // enable_glacial
        all_on,  // enable_coastal
        true,  // enable_erosion
        all_on,  // enable_ridges
        all_on,  // enable_beaches
        100,   // erosion_strength
        100,   // glacial_strength
    );
    let checksum = compute_checksum(&w);
    (w, checksum)
}

fn main() {
    println!("W-17 G1 byte-equivalence test");

    // Build ALL-ON twice in-process
    println!("\nBuilding ALL-ON config:");
    let (_w1, cs1) = build_world(true);
    println!("  Run 1 checksum: 0x{:016x}", cs1);

    let (_w2, cs2) = build_world(true);
    println!("  Run 2 checksum: 0x{:016x}", cs2);

    if cs1 == cs2 {
        println!("  ✓ ALL-ON checksums match");
    } else {
        eprintln!("  ✗ ALL-ON MISMATCH: 0x{:016x} != 0x{:016x}", cs1, cs2);
        std::process::exit(1);
    }

    // Build DEFAULT twice in-process
    println!("\nBuilding DEFAULT config:");
    let (_w3, cs3) = build_world(false);
    println!("  Run 1 checksum: 0x{:016x}", cs3);

    let (_w4, cs4) = build_world(false);
    println!("  Run 2 checksum: 0x{:016x}", cs4);

    if cs3 == cs4 {
        println!("  ✓ DEFAULT checksums match");
    } else {
        eprintln!("  ✗ DEFAULT MISMATCH: 0x{:016x} != 0x{:016x}", cs3, cs4);
        std::process::exit(1);
    }

    println!("\n✓ All in-process builds byte-identical");
    println!("ALL-ON:  0x{:016x}", cs1);
    println!("DEFAULT: 0x{:016x}", cs3);
}
