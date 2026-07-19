//! Load a **v2**-generated world dump (see `v2/crates/world/src/bin/terrain_dump.rs`) and build a
//! v1 [`VoxelTerrain`] from it, so the mature v1 hex-voxel renderer can draw the richer v2 relief.
//!
//! The dump is any square `dim`; it is nearest-neighbour resampled onto the fixed v1 grid
//! (`COLS*ROWS`, 1920²). The v2 primary-material id at each surface cell is mapped to the nearest v1
//! `BiomeKind` for colouring, and `Water` cells become an ocean column (water plane at the surface).
//!
//! Activated by the env var `ANIMATA_TERRAIN_DUMP=<path>` at startup (see `render::streamer::spawn_gen`).

use animata_sim::config::{COLS, ROWS};
use animata_sim::terrain::{BiomeKind, VoxelTerrain};

/// v2 `MaterialId` discriminants (mirror of `v2/crates/world/src/gen/material.rs` — the v1 crate
/// cannot import the v2 enum). `Water = 8` is treated specially (ocean column).
const MAT_WATER: u8 = 8;

/// v2 primary-material id → nearest v1 [`BiomeKind`] id (for surface colour). See the enum:
/// Air=0, Sand=1, Permafrost=2, Soil=3, Bedrock=4, Basalt=5, Tuff=6, Till=7, Water=8.
fn material_to_biome_id(m: u8) -> u8 {
    let b = match m {
        1 => BiomeKind::Desert,   // aeolian sand
        2 => BiomeKind::Snow,     // permafrost
        3 => BiomeKind::Plains,   // soil
        4 => BiomeKind::Mountain, // bedrock
        5 => BiomeKind::Mountain, // volcanic basalt
        6 => BiomeKind::Mountain, // volcanic tuff
        7 => BiomeKind::Tundra,   // glacial till
        8 => BiomeKind::Ocean,    // coastal water
        _ => BiomeKind::Plains,   // Air / unknown — above-surface fallback
    };
    b.id()
}

/// Read + parse the dump at `path`, returning `(dim, height[dim*dim] as i16, material[dim*dim])`.
fn read_dump(path: &str) -> Result<(usize, Vec<i16>, Vec<u8>), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
    if bytes.len() < 12 || &bytes[0..8] != b"ATDMP1\0\0" {
        return Err(format!("{path}: bad magic (not an ATDMP1 dump)"));
    }
    let dim = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    let n = dim.checked_mul(dim).ok_or("dim overflow")?;
    let want = 12 + n * 3;
    if bytes.len() != want {
        return Err(format!("{path}: size {} != expected {want} for dim {dim}", bytes.len()));
    }
    let mut height = Vec::with_capacity(n);
    let mut material = Vec::with_capacity(n);
    let mut off = 12;
    for _ in 0..n {
        height.push(i16::from_le_bytes([bytes[off], bytes[off + 1]]));
        material.push(bytes[off + 2]);
        off += 3;
    }
    Ok((dim, height, material))
}

/// Load `path` into a v1 [`VoxelTerrain`] on the fixed grid, nearest-neighbour resampling from the
/// dump's `dim`. `seed` is carried only as the terrain's label (the geometry comes from the dump).
pub fn load(path: &str, seed: u64) -> Result<VoxelTerrain, String> {
    let (dim, src_h, src_m) = read_dump(path)?;
    let n = COLS * ROWS;
    let mut surf = vec![0.0f32; n];
    let mut biome_ids = vec![0u8; n];
    let mut water = vec![0u8; n];
    for z in 0..ROWS {
        // Nearest-neighbour source row/col (map the fixed grid back onto the dump's dim).
        let sz = z * dim / ROWS;
        for x in 0..COLS {
            let sx = x * dim / COLS;
            let si = sz * dim + sx;
            let h = src_h[si].max(0) as f32;
            let m = src_m[si];
            let di = z * COLS + x;
            surf[di] = h;
            biome_ids[di] = material_to_biome_id(m);
            if m == MAT_WATER {
                // Float the translucent water plane at the surface of the flooded column.
                water[di] = src_h[si].clamp(0, 255) as u8;
            }
        }
    }
    eprintln!("[terrain-dump] loaded {path}: dim {dim} -> v1 grid {COLS}x{ROWS} (nearest-neighbour)");
    Ok(VoxelTerrain::from_external(seed, surf, biome_ids, water))
}
