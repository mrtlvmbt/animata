//! Terrain draw dispatch — CPU vs GPU rendering paths.
//! Extracted from main.rs to consolidate duplicate terrain rendering logic.

use crate::gpu_terrain::GpuChunk;
use crate::terrain::TerrainChunk;
use crate::camera::IsoCam;
use macroquad::prelude::*;

/// Draw terrain using either GPU-retained or CPU macroquad path, depending on `retained` flag.
/// Performs frustum culling on chunks and returns the count of chunks drawn.
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
/// Returns the number of chunks drawn.
pub fn draw_terrain(
    chunks_hex: &[TerrainChunk],
    chunks_cube: &[TerrainChunk],
    gpu_hex: &[GpuChunk],
    gpu_cube: &[GpuChunk],
    gpu_pipeline: Option<macroquad::miniquad::Pipeline>,
    camera: &IsoCam,
    use_cube: bool,
    retained: bool,
) -> usize {
    let frustum_planes = camera.frustum_planes();
    let mut chunks_drawn = 0;

    if retained && gpu_pipeline.is_some() {
        // GPU-retained path: use GPU chunks
        let gpu_chunks = if use_cube { gpu_cube } else { gpu_hex };
        crate::draw_gpu_terrain(gpu_chunks, gpu_pipeline.unwrap(), camera, &frustum_planes);
        for gpu_chunk in gpu_chunks {
            if frustum_planes.iter().all(|plane| plane.aabb_intersects(gpu_chunk.lo, gpu_chunk.hi)) {
                chunks_drawn += 1;
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
            }
        }
    }

    chunks_drawn
}
