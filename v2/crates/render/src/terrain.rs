//! R-2: hex-voxel terrain mesh — `WorldView` → flat-top hex columns + cliff quads (RnD `rendering/01`
//! §1.3 height-as-Y + cliff quads, `rendering/02` §3 hidden-face removal). Built ONCE at startup —
//! the cold terrain is immutable for the run (R19); never rebuilt per frame.
//!
//! Split into row-band chunks so no single `Mesh` exceeds macroquad's `u16` index limit (65536):
//! worst case ~30 vertices/cell (6 top + up to 6×4 cliff), so [`ROWS_PER_CHUNK`] rows of
//! `world_dim=64` stays an order of magnitude under the limit.
//!
//! R-3: Each chunk carries a world-space AABB for frustum culling.

use crate::biome_palette::{material_color, cliff_shade, apply_directional_shading};
use crate::hex::{edge_for_direction, hex_center, hex_corner, neighbors, HEIGHT_SCALE};
use macroquad::models::{Mesh, Vertex};
use macroquad::prelude::*;
use sim_core::{Vec2Fixed, WorldView};
use std::f32::consts::PI;

/// Rows per chunk, adaptive to `world_dim` so no single chunk mesh exceeds the u16 index space
/// (macroquad batches with `u16` indices — see `main.rs`'s `gl_set_drawcall_buffer_capacity` note).
/// Worst case ≤30 verts/cell (6 top + up to 6×4 cliff); keep a chunk under ~50k verts (safely below
/// the 60k drawcall cap): `rows = 50000 / (world_dim * 30)`, clamped to `[1, 8]`. At `world_dim=64`
/// this is the historical 8; at 512 it drops to ~3.
pub(crate) fn rows_per_chunk(world_dim: i64) -> i64 {
    (50_000 / (world_dim.max(1) * 30)).clamp(1, 8)
}

/// A terrain chunk: mesh + world-space AABB for frustum culling.
pub struct TerrainChunk {
    pub mesh: Mesh,
    pub bounds: (Vec3, Vec3), // (min, max)
}

/// Build the whole `world_dim × world_dim` hex terrain as a handful of row-band chunks.
/// Each chunk carries its own AABB (computed once at build).
pub fn build_hex_terrain(world_dim: i64, world: &dyn WorldView) -> Vec<TerrainChunk> {
    let mut chunks = Vec::new();
    let rpc = rows_per_chunk(world_dim);
    let mut row0 = 0i64;
    while row0 < world_dim {
        let row1 = (row0 + rpc).min(world_dim);
        chunks.push(build_chunk(world_dim, world, row0, row1));
        row0 = row1;
    }
    chunks
}

fn build_chunk(world_dim: i64, world: &dyn WorldView, row0: i64, row1: i64) -> TerrainChunk {
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u16> = Vec::new();

    for row in row0..row1 {
        for col in 0..world_dim {
            let h = world.height(col, row) as f32 * HEIGHT_SCALE;
            let (cx, cz) = hex_center(col, row);
            let top_color = material_color(world.surface_material(Vec2Fixed(col, row)));

            // Top face: 6 corners, fan-triangulated from corner 0 (4 triangles, 6 unique verts).
            let base = vertices.len() as u16;
            let top_normal = Vec3::new(0.0, 1.0, 0.0); // Top face normal (pointing up)
            for k in 0..6 {
                vertices.push(vertex(hex_corner(cx, cz, h, k), top_color, top_normal));
            }
            for k in 1..5u16 {
                indices.extend_from_slice(&[base, base + k, base + k + 1]);
            }

            // Cliff quads: hidden-face removal (RnD 02 §3) — emit a side ONLY where the neighbour is
            // STRICTLY lower; an equal-or-higher neighbour covers that face, so skip it. Off-grid
            // neighbours (map edge) are treated as height 0 — draws a full cliff at the boundary.
            let cliff_color = cliff_shade(top_color);
            for (dir_i, &(ncol, nrow)) in neighbors(col, row).iter().enumerate() {
                let nh = if (0..world_dim).contains(&ncol) && (0..world_dim).contains(&nrow) {
                    world.height(ncol, nrow) as f32 * HEIGHT_SCALE
                } else {
                    0.0
                };
                if nh >= h {
                    continue;
                }
                let edge = edge_for_direction(dir_i);
                let top_a = hex_corner(cx, cz, h, edge);
                let top_b = hex_corner(cx, cz, h, (edge + 1) % 6);
                let bot_a = Vec3::new(top_a.x, nh, top_a.z);
                let bot_b = Vec3::new(top_b.x, nh, top_b.z);
                let cliff_normal = hex_cliff_normal(dir_i);
                let cbase = vertices.len() as u16;
                vertices.push(vertex(top_a, cliff_color, cliff_normal));
                vertices.push(vertex(top_b, cliff_color, cliff_normal));
                vertices.push(vertex(bot_b, cliff_color, cliff_normal));
                vertices.push(vertex(bot_a, cliff_color, cliff_normal));
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

/// Compute the outward cliff normal for a hex edge given the direction index.
/// The normal points outward in the XZ plane (perpendicular to the cliff edge).
/// For a flat-top hex, direction i's edge midpoint is at angle `60*i° + 30°`.
fn hex_cliff_normal(dir_index: usize) -> Vec3 {
    // Edge midpoint angle for direction i is `60*i° + 30°`
    let angle = PI / 3.0 * dir_index as f32 + PI / 6.0;
    Vec3::new(angle.cos(), 0.0, angle.sin())
}

/// Create a vertex with directional shading applied based on the face normal.
/// The normal MUST be normalized. Stores the normal in the vertex for potential shader use.
fn vertex(position: Vec3, color: Color, normal: Vec3) -> Vertex {
    let shaded_color = apply_directional_shading(color, normal);
    // Pack the normalized normal (Vec3) into Vec4, with w=1.0 to indicate validity
    Vertex {
        position,
        uv: Vec2::ZERO,
        color: shaded_color.into(),
        normal: Vec4::new(normal.x, normal.y, normal.z, 1.0),
    }
}
