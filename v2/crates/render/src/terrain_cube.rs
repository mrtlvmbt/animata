//! R-5: cube-voxel terrain mesh — `WorldView` → square columns (vs hex prisms in R-2).
//! Each `WorldView` cell → a unit square column at world (x=col+0.5, z=row+0.5), height * HEIGHT_SCALE.
//! Side quads only where neighbour is STRICTLY lower (hidden-face removal, RnD `rendering/02` §3).
//! Biome-colored (via `biome_palette`) + baked per-face-direction shading (mirror v1 `mesh.rs::shaded`).
//!
//! Same `ROWS_PER_CHUNK` chunking + u16-index assert as hex terrain (`terrain.rs`).
//! Built ONCE at startup — cold terrain immutable for the run.

use crate::biome_palette::{biome_color, cliff_shade};
use crate::hex::HEIGHT_SCALE;
use crate::terrain::TerrainChunk;
use macroquad::models::{Mesh, Vertex};
use macroquad::prelude::*;
use sim_core::{Vec2Fixed, WorldView};

const ROWS_PER_CHUNK: i64 = 8;

/// Build the whole `world_dim × world_dim` cube terrain as row-band chunks.
/// Each chunk carries its own AABB (reuses `terrain::TerrainChunk`).
pub fn build_cube_terrain(world_dim: i64, world: &dyn WorldView) -> Vec<TerrainChunk> {
    let mut chunks = Vec::new();
    let mut row0 = 0i64;
    while row0 < world_dim {
        let row1 = (row0 + ROWS_PER_CHUNK).min(world_dim);
        chunks.push(build_chunk(world_dim, world, row0, row1));
        row0 = row1;
    }
    chunks
}

/// Build one row-band chunk of cube terrain.
fn build_chunk(world_dim: i64, world: &dyn WorldView, row0: i64, row1: i64) -> TerrainChunk {
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u16> = Vec::new();

    for row in row0..row1 {
        for col in 0..world_dim {
            let h = world.height(col, row) as f32 * HEIGHT_SCALE;
            // Square cell center: (col + 0.5, row + 0.5) in world space
            // Each cell is a 1×1 square, so corners are at ±0.5 from center
            let cx = col as f32 + 0.5;
            let cz = row as f32 + 0.5;
            let size = 0.5; // Half-size: extends ±0.5 from center in x,z

            let top_color = biome_color(world.biome(Vec2Fixed(col, row)));
            let cliff_color = cliff_shade(top_color);

            // ────────────────────────────────────────────────────────────────────
            // Top face: 4 corners (square), fan-triangulated from corner 0 (2 triangles, 4 unique verts).
            // Order: TL (top-left), TR (top-right), BR (bottom-right), BL (bottom-left)
            // (in x-z plane, viewed from above)
            // ────────────────────────────────────────────────────────────────────
            let base = vertices.len() as u16;
            // TL: (-0.5, h, -0.5) relative to center
            vertices.push(vertex(Vec3::new(cx - size, h, cz - size), top_color));
            // TR: (+0.5, h, -0.5)
            vertices.push(vertex(Vec3::new(cx + size, h, cz - size), top_color));
            // BR: (+0.5, h, +0.5)
            vertices.push(vertex(Vec3::new(cx + size, h, cz + size), top_color));
            // BL: (-0.5, h, +0.5)
            vertices.push(vertex(Vec3::new(cx - size, h, cz + size), top_color));
            // Fan triangulation: (0,1,2), (0,2,3)
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);

            // ────────────────────────────────────────────────────────────────────
            // Side quads: hidden-face removal (RnD 02 §3).
            // Emit a side ONLY where the neighbour is STRICTLY lower.
            // If neighbour is equal or higher, it covers that face — skip it.
            // Off-grid neighbours (map edge) are treated as height 0 — full cliff at boundary.
            // ────────────────────────────────────────────────────────────────────

            // 4 cardinal directions: West, East, South, North
            // (naming: col-1=West, col+1=East; row-1=North/up, row+1=South/down — using standard grid coords)
            let neighbors = [
                (col - 1, row),         // West (left)
                (col + 1, row),         // East (right)
                (col, row - 1),         // North (up in grid)
                (col, row + 1),         // South (down in grid)
            ];

            // For each edge: check if we should draw a side quad
            // Edge 0 (West, x = cx - size): from (cx-size, h, cz-size) to (cx-size, h, cz+size)
            // Edge 1 (East, x = cx + size): from (cx+size, h, cz-size) to (cx+size, h, cz+size)
            // Edge 2 (North, z = cz - size): from (cx-size, h, cz-size) to (cx+size, h, cz-size)
            // Edge 3 (South, z = cz + size): from (cx-size, h, cz+size) to (cx+size, h, cz+size)

            let edge_configs = [
                // West edge (x = cx - size)
                (
                    Vec3::new(cx - size, h, cz - size), // top_a (TL)
                    Vec3::new(cx - size, h, cz + size), // top_b (BL)
                ),
                // East edge (x = cx + size)
                (
                    Vec3::new(cx + size, h, cz + size), // top_a (BR)
                    Vec3::new(cx + size, h, cz - size), // top_b (TR)
                ),
                // North edge (z = cz - size)
                (
                    Vec3::new(cx + size, h, cz - size), // top_a (TR)
                    Vec3::new(cx - size, h, cz - size), // top_b (TL)
                ),
                // South edge (z = cz + size)
                (
                    Vec3::new(cx - size, h, cz + size), // top_a (BL)
                    Vec3::new(cx + size, h, cz + size), // top_b (BR)
                ),
            ];

            for (edge_idx, &(top_a, top_b)) in edge_configs.iter().enumerate() {
                let (ncol, nrow) = neighbors[edge_idx];
                let nh = if (0..world_dim).contains(&ncol) && (0..world_dim).contains(&nrow) {
                    world.height(ncol, nrow) as f32 * HEIGHT_SCALE
                } else {
                    0.0
                };

                // Only draw this side if neighbour is STRICTLY lower
                if nh >= h {
                    continue;
                }

                let bot_a = Vec3::new(top_a.x, nh, top_a.z);
                let bot_b = Vec3::new(top_b.x, nh, top_b.z);

                let cbase = vertices.len() as u16;
                vertices.push(vertex(top_a, cliff_color));
                vertices.push(vertex(top_b, cliff_color));
                vertices.push(vertex(bot_b, cliff_color));
                vertices.push(vertex(bot_a, cliff_color));
                // Two triangles: (0,1,2), (0,2,3)
                indices.extend_from_slice(&[cbase, cbase + 1, cbase + 2, cbase, cbase + 2, cbase + 3]);
            }
        }
    }

    assert!(
        vertices.len() <= u16::MAX as usize,
        "terrain chunk exceeded the u16 index limit ({} vertices) — shrink ROWS_PER_CHUNK",
        vertices.len()
    );

    // Compute AABB from vertices.
    let bounds = if vertices.is_empty() {
        (Vec3::ZERO, Vec3::ZERO)
    } else {
        let mut min = vertices[0].position;
        let mut max = vertices[0].position;
        for v in &vertices {
            min.x = min.x.min(v.position.x);
            min.y = min.y.min(v.position.y);
            min.z = min.z.min(v.position.z);
            max.x = max.x.max(v.position.x);
            max.y = max.y.max(v.position.y);
            max.z = max.z.max(v.position.z);
        }
        (min, max)
    };

    TerrainChunk { mesh: Mesh { vertices, indices, texture: None }, bounds }
}

fn vertex(position: Vec3, color: Color) -> Vertex {
    Vertex { position, uv: Vec2::ZERO, color: color.into(), normal: Vec4::ZERO }
}
