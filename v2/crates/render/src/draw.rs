//! Terrain draw dispatch — CPU vs GPU rendering paths.
//! Extracted from main.rs to consolidate duplicate terrain rendering logic.

use crate::gpu_terrain::GpuChunk;
use crate::terrain::TerrainChunk;
use crate::camera::IsoCam;
use macroquad::prelude::*;

/// Statistics from a terrain draw pass.
#[derive(Debug, Clone, Copy)]
pub struct DrawStats {
    /// Number of chunks that passed frustum culling and were drawn.
    pub chunks_drawn: usize,
    /// Total number of vertices drawn across all chunks.
    pub verts_drawn: usize,
}

/// Draw terrain using either GPU-retained or CPU macroquad path, depending on `retained` flag.
/// Performs frustum culling on chunks and returns draw statistics.
///
/// Arguments:
/// - `chunks_hex`: hex terrain chunks (used if `use_cube` is false)
/// - `chunks_cube`: cube terrain chunks (used if `use_cube` is true)
/// - `gpu_hex`: GPU-uploaded hex chunks (used if `retained` is true and `use_cube` is false)
/// - `gpu_cube`: GPU-uploaded cube chunks (used if `retained` is true and `use_cube` is true)
/// - `gpu_pipeline`: GPU pipeline (used if `retained` is true)
/// - `camera`: isometric camera for frustum culling and unprojection
/// - `use_cube`: if true, use cube terrain; if false, use hex terrain
/// - `retained`: if true, use GPU-retained rendering; if false, use CPU macroquad
///
/// Returns draw statistics (chunks and vertex counts).
pub fn draw_terrain(
    chunks_hex: &[TerrainChunk],
    chunks_cube: &[TerrainChunk],
    gpu_hex: &[GpuChunk],
    gpu_cube: &[GpuChunk],
    gpu_pipeline: Option<macroquad::miniquad::Pipeline>,
    camera: &IsoCam,
    use_cube: bool,
    retained: bool,
) -> DrawStats {
    let frustum_planes = camera.frustum_planes();
    let mut chunks_drawn = 0;
    let mut verts_drawn = 0;

    if retained && gpu_pipeline.is_some() {
        // GPU-retained path: use GPU chunks
        let gpu_chunks = if use_cube { gpu_cube } else { gpu_hex };
        crate::draw_gpu_terrain(gpu_chunks, gpu_pipeline.unwrap(), camera, &frustum_planes);
        for gpu_chunk in gpu_chunks {
            if frustum_planes.iter().all(|plane| plane.aabb_intersects(gpu_chunk.lo, gpu_chunk.hi)) {
                chunks_drawn += 1;
                verts_drawn += gpu_chunk.n_idx as usize;
            }
        }
    } else {
        // CPU macroquad path: use CPU chunks
        let terrain_chunks = if use_cube { chunks_cube } else { chunks_hex };
        for chunk in terrain_chunks {
            let (min, max) = chunk.bounds;
            if frustum_planes.iter().all(|plane| plane.aabb_intersects(min, max)) {
                draw_mesh(&chunk.mesh);
                chunks_drawn += 1;
                verts_drawn += chunk.mesh.vertices.len();
            }
        }
    }

    DrawStats { chunks_drawn, verts_drawn }
}
