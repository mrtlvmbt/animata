//! Preview bin: dump the **v1**-generated world (`VoxelTerrain::new(seed)`) as a binary
//! (height + material) grid for the **v2** renderer to load (`v2/crates/render --v1-dump <file>`),
//! so the v1 worldgen can be compared against the v2 worldgen in the SAME (v2) renderer.
//!
//! The v1 grid is fixed at `COLS*ROWS` (1920²). The v2 renderer builds its whole mesh once with no
//! LOD, so we DOWNSAMPLE (nearest-neighbour) to a tractable square `dim` (default 512 — the scale the
//! v2 renderer natively shows). v1 `BiomeKind` is mapped to the nearest v2 `MaterialId` for colour.
//!
//! Format (all little-endian): magic `b"ATDMP1\0\0"` (8 bytes) | `dim: u32` | then `dim*dim` records
//! row-major (`idx = z*dim + x`): `height: i16`, `material: u8` — the same ATDMP1 layout the v1
//! `terrain_dump` (v2→v1 direction) uses, so one loader shape serves both.
//!
//! Usage:  v1_map_dump [dim] [seed] [out.bin]   (dim default 512, seed default 1)

use std::io::Write;

use animata_sim::config::{COLS, ROWS};
use animata_sim::terrain::{BiomeKind, VoxelTerrain};

/// v1 `BiomeKind` → nearest v2 `MaterialId` discriminant (Air=0, Sand=1, Permafrost=2, Soil=3,
/// Bedrock=4, Basalt=5, Tuff=6, Till=7, Water=8). Water is set separately from `is_water`.
fn biome_to_material(b: BiomeKind) -> u8 {
    match b {
        BiomeKind::Ocean => 8,    // Water
        BiomeKind::Beach => 1,    // Sand
        BiomeKind::Desert => 1,   // Sand
        BiomeKind::Mountain => 4, // Bedrock
        BiomeKind::Snow => 2,     // Permafrost
        BiomeKind::Tundra => 2,   // Permafrost
        _ => 3,                   // Plains/Forest/Taiga/Savanna/Swamp/Jungle → Soil
    }
}

fn parse_seed(s: &str) -> u64 {
    s.strip_prefix("0x").map_or_else(|| s.parse().unwrap_or(1), |h| u64::from_str_radix(h, 16).unwrap_or(1))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dim: usize = args.get(1).and_then(|s| s.parse().ok()).filter(|&d| d > 0).unwrap_or(512);
    let seed: u64 = args.get(2).map_or(1, |s| parse_seed(s));
    let out = args.get(3).cloned().unwrap_or_else(|| format!("v1map_{dim}_{seed:#x}.bin"));

    eprintln!("[v1_map_dump] generating v1 world seed={seed:#x} ({COLS}x{ROWS}) …");
    let t = VoxelTerrain::new(seed);

    let mut buf = Vec::with_capacity(8 + 4 + dim * dim * 3);
    buf.extend_from_slice(b"ATDMP1\0\0");
    buf.extend_from_slice(&(dim as u32).to_le_bytes());
    let (mut hmin, mut hmax) = (i16::MAX, i16::MIN);
    for tz in 0..dim {
        let sz = tz * ROWS / dim;
        for tx in 0..dim {
            let sx = tx * COLS / dim;
            let h = t.height_at(sx, sz) as i16;
            let m = if t.is_water(sx, sz) { 8 } else { biome_to_material(t.biome_at(sx, sz)) };
            hmin = hmin.min(h);
            hmax = hmax.max(h);
            buf.extend_from_slice(&h.to_le_bytes());
            buf.push(m);
        }
    }
    std::fs::File::create(&out).and_then(|mut f| f.write_all(&buf)).expect("write dump");
    eprintln!("[v1_map_dump] wrote {out}  ({dim}x{dim} downsampled from {COLS}², seed={seed:#x}, height [{hmin}..{hmax}])");
}
