//! Preview bin: dump the v2-generated world as a binary (height + primary-material) grid for the
//! **v1** hex-voxel renderer to load (`crates/animata` `ANIMATA_TERRAIN_DUMP=<file>`). GOLDEN-NEUTRAL:
//! only READS the world via `gen::caps::classify_and_caps`, never touches the sim path, so it moves
//! no determinism golden. Pure CPU — no macroquad / GPU.
//!
//! Format (all little-endian): magic `b"ATDMP1\0\0"` (8 bytes) | `dim: u32` | then `dim*dim` records
//! row-major (`idx = z*dim + x`): `height: i16`, `material: u8` (a `MaterialId` discriminant).
//!
//! Usage:  terrain_dump <dim> [seed] [out.bin]
//!   dim      map edge in cells (required), e.g. 512 or 1920 (v1 grid = 1920)
//!   seed     u64, decimal or 0x-hex (default 1)
//!   out.bin  output path (default `terrain_<dim>_<seed>.bin`)
//!
//! All five landform stages (tectonics / aeolian / volcanic / glacial / coastal) are ON so the dump
//! carries the full diverse-relief material palette; patchiness is OFF.

use std::io::Write;
use world::gen::caps::classify_and_caps;

/// Matches the production world height ceiling (`cli::HMAX`) so every height-relative threshold
/// fires exactly as the real generator sees it.
const HMAX: i64 = 200;

fn parse_seed(s: &str) -> u64 {
    s.strip_prefix("0x").map_or_else(|| s.parse().unwrap_or(1), |h| u64::from_str_radix(h, 16).unwrap_or(1))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dim: usize = match args.get(1).and_then(|s| s.parse().ok()) {
        Some(d) if d > 0 => d,
        _ => {
            eprintln!("usage: terrain_dump <dim> [seed] [out.bin]   (dim = map edge in cells, e.g. 512; v1 grid = 1920)");
            std::process::exit(2);
        }
    };
    let seed: u64 = args.get(2).map_or(1, |s| parse_seed(s));
    let out = args.get(3).cloned().unwrap_or_else(|| format!("terrain_{dim}_{seed:#x}.bin"));

    // patchiness=false, then all five landforms ON.
    let f = classify_and_caps(seed, HMAX, dim, false, LandformFlags::from_five(true, true, true, true, true));
    assert_eq!(f.height.len(), dim * dim, "height must be dim*dim");
    assert_eq!(f.surface_material.len(), dim * dim, "surface_material must be dim*dim");

    let mut buf = Vec::with_capacity(8 + 4 + dim * dim * 3);
    buf.extend_from_slice(b"ATDMP1\0\0");
    buf.extend_from_slice(&(dim as u32).to_le_bytes());
    for i in 0..dim * dim {
        let h = f.height[i].clamp(i16::MIN as i64, i16::MAX as i64) as i16;
        buf.extend_from_slice(&h.to_le_bytes());
        buf.push(f.surface_material[i]);
    }
    std::fs::File::create(&out).and_then(|mut fp| fp.write_all(&buf)).expect("write dump");

    let (mut hmin, mut hmax) = (i64::MAX, i64::MIN);
    for &h in &f.height {
        hmin = hmin.min(h);
        hmax = hmax.max(h);
    }
    eprintln!("wrote {out}  ({dim}x{dim}, seed={seed:#x}, all landforms ON, height [{hmin}..{hmax}])");
}
