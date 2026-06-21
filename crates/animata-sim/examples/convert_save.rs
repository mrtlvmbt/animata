//! Convert a world snapshot to the CURRENT save format, migrating older versions forward.
//! `Snapshot::read` dispatches on the magic prefix and migrates; `write` emits current. For a
//! same-version file this is a faithful re-write; for an older version it upgrades it in place-to-out.
//!
//! Usage: `cargo run -p animata-sim --release --example convert_save -- <in.anm> <out.anm>`

use animata_sim::persist::Snapshot;
use std::fs::File;
use std::io::{BufReader, BufWriter};

fn main() {
    let mut args = std::env::args().skip(1);
    let (inp, outp) = match (args.next(), args.next()) {
        (Some(i), Some(o)) => (i, o),
        _ => {
            eprintln!("usage: convert_save <in.anm> <out.anm>");
            std::process::exit(2);
        }
    };
    let snap = Snapshot::read(BufReader::new(File::open(&inp).expect("open input")))
        .expect("read + migrate snapshot");
    snap.write(BufWriter::new(File::create(&outp).expect("create output")))
        .expect("write current-format snapshot");
    println!("converted {inp} → {outp}  (tick {}, {} creatures)", snap.tick, snap.sim.creatures.len());
}
