//! U-2: Raw terrain chunk representation (worker-thread safe).
//!
//! Terrain chunks are built on the worker thread as raw vertex/index buffers
//! (no GPU types). The main thread converts these to Mesh + uploads to GPU.
//!
//! RawChunk contains ONLY plain-data types (Vec<Vertex>, Vec<u16>, Vec3, bool):
//! all Send-safe, no Rc/Mutex/Arc, no Mesh/Texture2D (GPU types).

use macroquad::models::Vertex;
use macroquad::prelude::Vec3;
use sim_core::WorldView;

/// A terrain chunk: raw vertex/index buffers + AABB (no GPU types).
///
/// Built on worker thread; main thread converts this to TerrainChunk (which has a Mesh)
/// and uploads it to GPU. This separation ensures no GPU calls happen off-thread.
#[derive(Clone)]
pub struct RawChunk {
    /// Vertex positions, UVs, colors, normals (no Texture2D).
    pub vertices: Vec<Vertex>,
    /// Index buffer (u16 to stay under macroquad's u16-index limit).
    pub indices: Vec<u16>,
    /// AABB min corner (world space) for frustum culling.
    pub lo: Vec3,
    /// AABB max corner (world space) for frustum culling.
    pub hi: Vec3,
}

impl RawChunk {
    /// Create an empty chunk (zero vertices/indices).
    pub fn new() -> Self {
        RawChunk {
            vertices: Vec::new(),
            indices: Vec::new(),
            lo: Vec3::ZERO,
            hi: Vec3::ZERO,
        }
    }

    /// Compute AABB from vertices.
    pub fn compute_bounds(vertices: &[Vertex]) -> (Vec3, Vec3) {
        if vertices.is_empty() {
            return (Vec3::ZERO, Vec3::ZERO);
        }
        let mut min = vertices[0].position;
        let mut max = vertices[0].position;
        for v in vertices {
            min.x = min.x.min(v.position.x);
            min.y = min.y.min(v.position.y);
            min.z = min.z.min(v.position.z);
            max.x = max.x.max(v.position.x);
            max.y = max.y.max(v.position.y);
            max.z = max.z.max(v.position.z);
        }
        (min, max)
    }

    /// Create a chunk from vertices and indices, computing AABB.
    pub fn from_parts(vertices: Vec<Vertex>, indices: Vec<u16>) -> Self {
        let (lo, hi) = Self::compute_bounds(&vertices);
        RawChunk { vertices, indices, lo, hi }
    }
}

impl Default for RawChunk {
    fn default() -> Self {
        Self::new()
    }
}

/// Output of `build_world()`: a complete, ready-to-render world.
///
/// All fields are Send-safe (no GPU types). Main thread later:
/// 1. Converts hex/cube RawChunks → Meshes
/// 2. Uploads Meshes to GPU
/// 3. Begins rendering the world from `world` (the WorldView trait object)
pub struct BuiltWorld {
    /// Read-only world view (queries height, material, etc.).
    /// Boxed + Send so it can be built on worker thread.
    pub world: Box<dyn WorldView + Send>,

    /// Effective dimension of the world (output, not input).
    /// Honors standalone overrides; sim mode derives from config.
    pub dim: i64,

    /// Hex-mesh terrain chunks (raw buffers).
    pub hex: Vec<RawChunk>,

    /// Cube-mesh terrain chunks (raw buffers).
    pub cube: Vec<RawChunk>,

    /// Seed used for generation (for minimap cache keys, etc.).
    pub seed: u64,
}

/// Error type for world building.
#[derive(Debug, Clone)]
pub enum BuildError {
    /// Invalid dimension (must be > 0).
    #[allow(dead_code)]
    InvalidDim(i64),
    /// World generation failed (e.g., file load error).
    #[allow(dead_code)]
    WorldGenFailed(String),
    /// Mesh building failed (e.g., vertex overflow).
    MeshBuildFailed(String),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::InvalidDim(dim) => write!(f, "invalid world dim: {}", dim),
            BuildError::WorldGenFailed(msg) => write!(f, "world generation failed: {}", msg),
            BuildError::MeshBuildFailed(msg) => write!(f, "mesh building failed: {}", msg),
        }
    }
}

impl std::error::Error for BuildError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_chunk_empty() {
        let chunk = RawChunk::new();
        assert!(chunk.vertices.is_empty());
        assert!(chunk.indices.is_empty());
        assert_eq!(chunk.lo, Vec3::ZERO);
        assert_eq!(chunk.hi, Vec3::ZERO);
    }

    #[test]
    fn raw_chunk_bounds() {
        let vertices = vec![
            Vertex {
                position: Vec3::new(0.0, 0.0, 0.0),
                uv: Default::default(),
                color: Default::default(),
                normal: Default::default(),
            },
            Vertex {
                position: Vec3::new(10.0, 20.0, 30.0),
                uv: Default::default(),
                color: Default::default(),
                normal: Default::default(),
            },
        ];
        let chunk = RawChunk::from_parts(vertices, vec![0, 1]);
        assert_eq!(chunk.lo, Vec3::new(0.0, 0.0, 0.0));
        assert_eq!(chunk.hi, Vec3::new(10.0, 20.0, 30.0));
    }
}
