//! R-2: hex-voxel terrain mesh — `WorldView` → flat-top hex columns + cliff quads (RnD `rendering/01`
//! §1.3 height-as-Y + cliff quads, `rendering/02` §3 hidden-face removal). Built ONCE at startup —
//! the cold terrain is immutable for the run (R19); never rebuilt per frame.
//!
//! Split into row-band chunks so no single `Mesh` exceeds macroquad's `u16` index limit (65536):
//! worst case ~30 vertices/cell (6 top + up to 6×4 cliff), so [`ROWS_PER_CHUNK`] rows of
//! `world_dim=64` stays an order of magnitude under the limit.
//!
//! R-3: Each chunk carries a world-space AABB for frustum culling.

use crate::biome_palette::{surface_color_v2, cliff_shade, apply_directional_shading, ColorMode};
use crate::hex::{edge_for_direction, hex_center, hex_corner, neighbors, HEIGHT_SCALE};
use macroquad::models::{Mesh, Vertex};
use macroquad::prelude::*;
use sim_core::{Vec2Fixed, WorldView};
use std::f32::consts::PI;

/// Bevel fraction: shrink the top hexagon by this fraction of HEX_SIZE for the chamfer ring.
/// ~0.12 keeps the bevel subtle but visibly "toy-ish".
const BEVEL_FRAC: f32 = 0.12;

/// Per-mesh-kind vertex count maximum. Hex with bevel: ~54 verts/cell worst case.
/// Cube: ~30 verts/cell. Used for adaptive rows_per_chunk and capacity assertions.
const HEX_WITH_BEVEL_VERTS_PER_CELL: usize = 54;
const CUBE_VERTS_PER_CELL: usize = 30;

/// Hard capacity limits (macroquad buffer)
const HARD_CAPACITY_VERTS: usize = 60_000;
const HARD_CAPACITY_INDICES: usize = 120_000;

/// Rows per chunk, adaptive to `world_dim` so no single chunk mesh stays under the hard capacity.
/// For hex with bevel, this is ~54 verts/cell; keep chunks under ~60k verts and ~120k indices.
pub(crate) fn rows_per_chunk_hex(world_dim: i64) -> i64 {
    let verts_per_row = (world_dim as usize) * HEX_WITH_BEVEL_VERTS_PER_CELL;
    (HARD_CAPACITY_VERTS / verts_per_row.max(1)) as i64
}

/// Legacy function for backward compatibility
pub(crate) fn rows_per_chunk(world_dim: i64) -> i64 {
    rows_per_chunk_hex(world_dim).clamp(1, 8)
}

/// A terrain chunk: mesh + world-space AABB for frustum culling.
pub struct TerrainChunk {
    pub mesh: Mesh,
    pub bounds: (Vec3, Vec3), // (min, max)
}

/// The map's observed relief band `[p2, p98]` of cell heights — the per-map hypsometric normalization
/// window for [`crate::biome_palette::height_color`]. Real terrain is bottom-heavy (half the cells
/// near sea level), so a fixed `[0, hmax]` datum crams all relief into the ramp's low green third;
/// stretching against the 2nd/98th percentiles spreads the full green→brown→snow band over the relief
/// that actually exists and clamps the sparse outlier peaks (base-erosion needles) to the snow top.
/// Scanned once at build. Returns `(lo, hi)` with `hi > lo` guaranteed (falls back to a unit span on
/// a degenerate all-flat map).
pub(crate) fn hypsometric_range(world_dim: i64, world: &dyn WorldView) -> (i64, i64) {
    let mut hs: Vec<i64> = Vec::with_capacity((world_dim * world_dim) as usize);
    for row in 0..world_dim {
        for col in 0..world_dim {
            hs.push(world.height(col, row));
        }
    }
    if hs.is_empty() {
        return (0, 1);
    }
    hs.sort_unstable();
    let n = hs.len();
    let at = |p: f64| hs[(((p * (n as f64 - 1.0)).round()) as usize).min(n - 1)];
    let lo = at(0.02);
    let hi = at(0.98);
    if hi > lo {
        (lo, hi)
    } else {
        (lo, lo + 1)
    }
}

/// Build the whole `world_dim × world_dim` hex terrain as a handful of row-band chunks.
/// Each chunk carries its own AABB (computed once at build).
/// `seed`: used for per-column palette v2 jitter and determinism.
/// `bare_mode`: if true, water renders as dry-bed sand tint.
pub fn build_hex_terrain(
    world_dim: i64,
    world: &dyn WorldView,
    mode: ColorMode,
    seed: u64,
    bare_mode: bool,
) -> Vec<TerrainChunk> {
    let mut chunks = Vec::new();
    let (h_lo, h_hi) = hypsometric_range(world_dim, world);
    let rpc = rows_per_chunk_hex(world_dim).clamp(1, 8);
    let mut row0 = 0i64;
    while row0 < world_dim {
        let row1 = (row0 + rpc).min(world_dim);
        chunks.push(build_chunk(world_dim, world, row0, row1, mode, h_lo, h_hi, seed, bare_mode));
        row0 = row1;
    }
    chunks
}

/// Helper: calculate per-vertex AO by counting strictly-higher neighbors adjacent to each top corner.
fn calculate_vertex_ao(col: i64, row: i64, corner_idx: usize, world_dim: i64, world: &dyn WorldView) -> f32 {
    let h = world.height(col, row);
    let mut ao_count = 0.0;

    // Each corner touches 3 neighbors; for corner k, those are directions (k-1), k, (k+1) mod 6
    // (each direction shares an edge with its two adjacent corners)
    for dir_i in 0..6 {
        let edge = edge_for_direction(dir_i);
        if edge == corner_idx || edge == (corner_idx + 5) % 6 || edge == (corner_idx + 1) % 6 {
            let (ncol, nrow) = neighbors(col, row)[dir_i];
            if (0..world_dim).contains(&ncol) && (0..world_dim).contains(&nrow) {
                let nh = world.height(ncol, nrow);
                if nh > h {
                    // Scale by the height difference (max ~20 cells typically) and clamp
                    let diff = ((nh - h) as f32).min(20.0) / 20.0;
                    ao_count += diff;
                }
            }
        }
    }

    // Clamp AO to [0.0, 1.0] and return a darkness factor (1.0 = no shadow, 0.5 = dark)
    let ao_factor = 1.0 - (ao_count / 3.0).clamp(0.0, 1.0) * 0.5;
    ao_factor
}

fn build_chunk(
    world_dim: i64,
    world: &dyn WorldView,
    row0: i64,
    row1: i64,
    _mode: ColorMode,
    h_lo: i64,
    h_hi: i64,
    seed: u64,
    bare_mode: bool,
) -> TerrainChunk {
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u16> = Vec::new();

    for row in row0..row1 {
        for col in 0..world_dim {
            let h = world.height(col, row) as f32 * HEIGHT_SCALE;
            let (cx, cz) = hex_center(col, row);
            let material = world.surface_material(Vec2Fixed(col, row));
            let height_val = world.height(col, row);

            // Compute top color (palette v2 for visual quality)
            let top_color = surface_color_v2(material, height_val, h_lo, h_hi, col, row, seed);
            let top_color_bare = if bare_mode && material == 8 {
                // Water in bare mode: desaturated sand
                surface_color_v2(1, height_val, h_lo, h_hi, col, row, seed)
            } else {
                top_color
            };

            // Top face: 6 corners, fan-triangulated from corner 0 (4 triangles, 6 unique verts).
            // Add AO baking per vertex.
            let base = vertices.len() as u16;
            let top_normal = Vec3::new(0.0, 1.0, 0.0); // Top face normal (pointing up)
            for k in 0..6 {
                let ao_factor = calculate_vertex_ao(col, row, k, world_dim, world);
                let pos = hex_corner(cx, cz, h, k);
                let ao_color = Color::new(
                    top_color_bare.r * ao_factor,
                    top_color_bare.g * ao_factor,
                    top_color_bare.b * ao_factor,
                    top_color_bare.a,
                );
                vertices.push(vertex(pos, ao_color, top_normal));
            }
            for k in 1..5u16 {
                indices.extend_from_slice(&[base, base + k, base + k + 1]);
            }

            // Bevel: shrink the top hexagon by BEVEL_FRAC and add 6 chamfer quads
            let bevel_corners: Vec<Vec3> = (0..6)
                .map(|k| {
                    let pos = hex_corner(cx, cz, h, k);
                    // Shrink toward center: pos + (center - pos) * BEVEL_FRAC
                    let center = Vec3::new(cx, h, cz);
                    let toward_center = (center - pos) * BEVEL_FRAC;
                    pos + toward_center
                })
                .collect();

            // Emit 6 bevel quads (one per edge)
            for k in 0..6 {
                let outer_a = hex_corner(cx, cz, h, k);
                let outer_b = hex_corner(cx, cz, h, (k + 1) % 6);
                let inner_a = bevel_corners[k];
                let inner_b = bevel_corners[(k + 1) % 6];

                // Bevel quad normal: slightly tilted (not flat top) for lighting
                let edge_vec = (outer_b - outer_a).normalize();
                let inward_vec = (Vec3::new(cx, h, cz) - outer_a).normalize();
                let mut bevel_normal = (edge_vec.cross(Vec3::new(0.0, 1.0, 0.0)) + inward_vec * 0.3).normalize();
                if bevel_normal.y < 0.0 {
                    bevel_normal = -bevel_normal;
                }

                let bevel_color = cliff_shade(top_color_bare);
                let bbase = vertices.len() as u16;
                vertices.push(vertex(outer_a, bevel_color, bevel_normal));
                vertices.push(vertex(outer_b, bevel_color, bevel_normal));
                vertices.push(vertex(inner_b, bevel_color, bevel_normal));
                vertices.push(vertex(inner_a, bevel_color, bevel_normal));
                indices.extend_from_slice(&[bbase, bbase + 1, bbase + 2, bbase, bbase + 2, bbase + 3]);
            }

            // Cliff quads: hidden-face removal (RnD 02 §3) — emit a side ONLY where the neighbour is
            // STRICTLY lower; an equal-or-higher neighbour covers that face, so skip it. Off-grid
            // neighbours (map edge) are treated as height 0 — draws a full cliff at the boundary.
            let cliff_color = cliff_shade(top_color_bare);
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

    // Capacity asserts: hard limits per kind
    assert!(
        vertices.len() < HARD_CAPACITY_VERTS,
        "HEX chunk exceeded hard verts capacity: {} >= {}",
        vertices.len(),
        HARD_CAPACITY_VERTS
    );
    assert!(
        indices.len() < HARD_CAPACITY_INDICES,
        "HEX chunk exceeded hard indices capacity: {} >= {}",
        indices.len(),
        HARD_CAPACITY_INDICES
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
