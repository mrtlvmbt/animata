//! A [`WorldView`] backed by an ATDMP1 dump file (see `crates/animata-sim/src/bin/v1_map_dump.rs`),
//! so the **v1**-generated map can be drawn by THIS (v2) renderer — holding the renderer constant to
//! compare v1 vs v2 worldgen. Activated by `--v1-dump <path>` (standalone only).
//!
//! Format (little-endian): magic `b"ATDMP1\0\0"` (8 bytes) | `dim: u32` | then `dim*dim` records
//! row-major (`idx = z*dim + x`): `height: i16`, `material: u8` (a v2 `MaterialId` discriminant).

use sim_core::{Vec2Fixed, WorldView};

/// Immutable terrain loaded from a dump. Only the fields the terrain mesher reads
/// (`height` / `surface_material`) carry real data; the rest are inert stubs (standalone = no sim).
pub struct DumpWorld {
    pub dim: i64,
    heights: Vec<i64>,
    materials: Vec<u8>,
}

impl DumpWorld {
    /// Parse the ATDMP1 dump at `path`.
    pub fn load(path: &str) -> Result<Self, String> {
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
        let mut heights = Vec::with_capacity(n);
        let mut materials = Vec::with_capacity(n);
        let mut off = 12;
        for _ in 0..n {
            heights.push(i16::from_le_bytes([bytes[off], bytes[off + 1]]) as i64);
            materials.push(bytes[off + 2]);
            off += 3;
        }
        Ok(Self { dim: dim as i64, heights, materials })
    }

    fn idx(&self, x: i64, z: i64) -> usize {
        let x = x.clamp(0, self.dim - 1);
        let z = z.clamp(0, self.dim - 1);
        (z * self.dim + x) as usize
    }
}

impl WorldView for DumpWorld {
    fn is_solid(&self, _pos: Vec2Fixed) -> bool {
        true
    }
    fn height(&self, x: i64, z: i64) -> i64 {
        self.heights[self.idx(x, z)]
    }
    fn biome(&self, _pos: Vec2Fixed) -> u8 {
        0 // the mesh colours by surface_material, not biome
    }
    fn resource(&self, _pos: Vec2Fixed) -> i64 {
        0
    }
    fn temp_at(&self, _pos: Vec2Fixed) -> i32 {
        1500
    }
    fn surface_material(&self, pos: Vec2Fixed) -> u8 {
        self.materials[self.idx(pos.0, pos.1)]
    }
}
