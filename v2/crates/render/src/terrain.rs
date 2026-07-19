//! R-2: hex-voxel terrain mesh — `WorldView` → flat-top hex columns + cliff quads (RnD `rendering/01`
//! §1.3 height-as-Y + cliff quads, `rendering/02` §3 hidden-face removal). Built ONCE at startup —
//! the cold terrain is immutable for the run (R19); never rebuilt per frame.
//!
//! Split into row-band chunks so no single `Mesh` exceeds macroquad's `u16` index limit (65536):
//! worst case ~30 vertices/cell (6 top + up to 6×4 cliff), so [`ROWS_PER_CHUNK`] rows of
//! `world_dim=64` stays an order of magnitude under the limit.
//!
//! R-3: Each chunk carries a world-space AABB for frustum culling.

use crate::biome_palette::{cell_color, cliff_shade, apply_directional_shading};
use crate::hex::{edge_for_direction, hex_center, hex_corner, neighbors, HEIGHT_SCALE, HEX_SIZE};
use crate::raw_chunk::{RawChunk, BuildError};
use macroquad::models::{Mesh, Vertex};
use macroquad::prelude::*;
use sim_core::{Vec2Fixed, WorldView};
use std::f32::consts::PI;

/// Bevel fraction: shrink the top hexagon by this fraction of HEX_SIZE for the chamfer ring.
/// ~0.08 keeps the bevel as a thin 45° rim at the very top edge (not a tall shoulder).
/// The vertical drop is set equal to the horizontal inset (HEX_SIZE * BEVEL_FRAC) for a 45° slope.
const BEVEL_FRAC: f32 = 0.08;

/// Per-mesh-kind vertex count maximum. Hex with bevel: ~54 verts/cell worst case.
/// Cube: ~30 verts/cell. Used for adaptive rows_per_chunk and capacity assertions.
const HEX_WITH_BEVEL_VERTS_PER_CELL: usize = 54;
#[allow(dead_code)]
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
#[allow(dead_code)]
pub fn build_hex_terrain(
    world_dim: i64,
    world: &dyn WorldView,
    seed: u64,
    bare_mode: bool,
) -> Vec<TerrainChunk> {
    let mut chunks = Vec::new();
    let (h_lo, h_hi) = hypsometric_range(world_dim, world);
    let rpc = rows_per_chunk_hex(world_dim).clamp(1, 8);
    let mut row0 = 0i64;
    while row0 < world_dim {
        let row1 = (row0 + rpc).min(world_dim);
        chunks.push(build_chunk(world_dim, world, row0, row1, h_lo, h_hi, seed, bare_mode));
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

#[allow(dead_code)]
fn build_chunk(
    world_dim: i64,
    world: &dyn WorldView,
    row0: i64,
    row1: i64,
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

            // Compute top color (palette v2 for visual quality, with bare_mode water substitution)
            let top_color_bare = cell_color(material, height_val, h_lo, h_hi, col, row, seed, bare_mode);

            // Bevel: shrink the top hexagon by BEVEL_FRAC toward center. These inner corners form the
            // TOP FACE of the shrunk hexagon; the OUTER corners drop by BEVEL_DROP to form the chamfer ring.
            let center = Vec3::new(cx, h, cz);
            let bevel_corners: Vec<Vec3> = (0..6)
                .map(|k| {
                    let pos = hex_corner(cx, cz, h, k);
                    // Shrink toward center (horizontal only, keep full height h for top face)
                    let toward_center = (center - pos) * BEVEL_FRAC;
                    pos + toward_center
                })
                .collect();

            // Top face: fan-triangulated from the shrunk (inner) hexagon at full height h.
            // Vertices are the inner shrunk corners; AO baking darkens based on strictly-higher neighbors.
            let base = vertices.len() as u16;
            let top_normal = Vec3::new(0.0, 1.0, 0.0); // Top face normal (pointing up)
            for k in 0..6 {
                let ao_factor = calculate_vertex_ao(col, row, k, world_dim, world);
                let pos = bevel_corners[k];
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

            // Bevel quads: 6 chamfer quads connecting inner corners (at height h) to outer corners
            // (at height h - bevel_drop). The drop equals the inset (HEX_SIZE * BEVEL_FRAC) for a 45° slope.
            let bevel_drop = HEX_SIZE * BEVEL_FRAC;
            for k in 0..6 {
                let inner_a = bevel_corners[k];
                let inner_b = bevel_corners[(k + 1) % 6];
                let outer_a = Vec3::new(hex_corner(cx, cz, h, k).x, h - bevel_drop, hex_corner(cx, cz, h, k).z);
                let outer_b = Vec3::new(hex_corner(cx, cz, h, (k + 1) % 6).x, h - bevel_drop, hex_corner(cx, cz, h, (k + 1) % 6).z);

                // Bevel quad normal: computed from actual quad geometry (includes vertical slope).
                // For a quad (inner_a, inner_b, outer_b, outer_a), the normal is:
                // (inner_b - inner_a) × (outer_b - inner_b), which naturally includes the Y component
                // from the 45° slope (outer corners drop by bevel_drop).
                let edge1 = inner_b - inner_a;  // Top edge (horizontal)
                let edge2 = outer_b - inner_b;  // Diagonal edge (includes vertical drop)
                let mut bevel_normal = edge1.cross(edge2).normalize();
                // Ensure normal points outward-up (positive Y component)
                if bevel_normal.y < 0.0 {
                    bevel_normal = -bevel_normal;
                }

                let bevel_color = cliff_shade(top_color_bare);
                let bbase = vertices.len() as u16;
                // Quad: inner_a, inner_b, outer_b, outer_a (forms a slanted rectangle)
                vertices.push(vertex(inner_a, bevel_color, bevel_normal));
                vertices.push(vertex(inner_b, bevel_color, bevel_normal));
                vertices.push(vertex(outer_b, bevel_color, bevel_normal));
                vertices.push(vertex(outer_a, bevel_color, bevel_normal));
                indices.extend_from_slice(&[bbase, bbase + 1, bbase + 2, bbase, bbase + 2, bbase + 3]);
            }

            // Cliff quads: hidden-face removal (RnD 02 §3) — emit a side ONLY where the neighbour is
            // STRICTLY lower; an equal-or-higher neighbour covers that face, so skip it. Off-grid
            // neighbours (map edge) are treated as height 0 — draws a full cliff at the boundary.
            // Cliff top edge now starts at h - bevel_drop (the outer rim of the bevel).
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
                let outer_a = hex_corner(cx, cz, h, edge);
                let outer_b = hex_corner(cx, cz, h, (edge + 1) % 6);
                let top_a = Vec3::new(outer_a.x, h - bevel_drop, outer_a.z);
                let top_b = Vec3::new(outer_b.x, h - bevel_drop, outer_b.z);
                let bot_a = Vec3::new(outer_a.x, nh, outer_a.z);
                let bot_b = Vec3::new(outer_b.x, nh, outer_b.z);
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

// ── U-2: Raw chunk building (worker-thread safe, no Mesh/GPU types) ──────────────

/// Build the whole `world_dim × world_dim` hex terrain as raw chunks (vertices/indices only).
/// This is the primary entry point for `build_world()` on the worker thread.
/// Each chunk carries raw buffers + AABB; GPU conversion happens on main thread.
pub fn build_raw_hex_terrain(
    world_dim: i64,
    world: &dyn WorldView,
    seed: u64,
    bare_mode: bool,
    height_scale_override: Option<f32>,
) -> Result<Vec<RawChunk>, BuildError> {
    let mut chunks = Vec::new();
    let (h_lo, h_hi) = hypsometric_range(world_dim, world);
    let rpc = rows_per_chunk_hex(world_dim).clamp(1, 8);
    let effective_height_scale = height_scale_override.unwrap_or(HEIGHT_SCALE);
    let mut row0 = 0i64;
    while row0 < world_dim {
        let row1 = (row0 + rpc).min(world_dim);
        chunks.push(build_raw_chunk(world_dim, world, row0, row1, h_lo, h_hi, seed, bare_mode, effective_height_scale)?);
        row0 = row1;
    }
    Ok(chunks)
}

/// Build a single raw chunk (row band). Returns RawChunk (no Mesh, just raw buffers).
fn build_raw_chunk(
    world_dim: i64,
    world: &dyn WorldView,
    row0: i64,
    row1: i64,
    h_lo: i64,
    h_hi: i64,
    seed: u64,
    bare_mode: bool,
    height_scale: f32,
) -> Result<RawChunk, BuildError> {
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u16> = Vec::new();

    for row in row0..row1 {
        for col in 0..world_dim {
            let h = world.height(col, row) as f32 * height_scale;
            let (cx, cz) = hex_center(col, row);
            let material = world.surface_material(Vec2Fixed(col, row));
            let height_val = world.height(col, row);

            // Compute top color (palette v2 for visual quality, with bare_mode water substitution)
            let top_color_bare = cell_color(material, height_val, h_lo, h_hi, col, row, seed, bare_mode);

            // Bevel: shrink the top hexagon by BEVEL_FRAC toward center. These inner corners form the
            // TOP FACE of the shrunk hexagon; the OUTER corners drop by BEVEL_DROP to form the chamfer ring.
            let center = Vec3::new(cx, h, cz);
            let bevel_corners: Vec<Vec3> = (0..6)
                .map(|k| {
                    let pos = hex_corner(cx, cz, h, k);
                    // Shrink toward center (horizontal only, keep full height h for top face)
                    let toward_center = (center - pos) * BEVEL_FRAC;
                    pos + toward_center
                })
                .collect();

            // Top face: fan-triangulated from the shrunk (inner) hexagon at full height h.
            // Vertices are the inner shrunk corners; AO baking darkens based on strictly-higher neighbors.
            let base = vertices.len() as u16;
            let top_normal = Vec3::new(0.0, 1.0, 0.0); // Top face normal (pointing up)
            for k in 0..6 {
                let ao_factor = calculate_vertex_ao(col, row, k, world_dim, world);
                let pos = bevel_corners[k];
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

            // Bevel quads: 6 chamfer quads connecting inner corners (at height h) to outer corners
            // (at height h - bevel_drop). The drop equals the inset (HEX_SIZE * BEVEL_FRAC) for a 45° slope.
            let bevel_drop = HEX_SIZE * BEVEL_FRAC;
            for k in 0..6 {
                let inner_a = bevel_corners[k];
                let inner_b = bevel_corners[(k + 1) % 6];
                let outer_a = Vec3::new(hex_corner(cx, cz, h, k).x, h - bevel_drop, hex_corner(cx, cz, h, k).z);
                let outer_b = Vec3::new(hex_corner(cx, cz, h, (k + 1) % 6).x, h - bevel_drop, hex_corner(cx, cz, h, (k + 1) % 6).z);

                // Bevel quad normal: computed from actual quad geometry (includes vertical slope).
                // For a quad (inner_a, inner_b, outer_b, outer_a), the normal is:
                // (inner_b - inner_a) × (outer_b - inner_b), which naturally includes the Y component
                // from the 45° slope (outer corners drop by bevel_drop).
                let edge1 = inner_b - inner_a;  // Top edge (horizontal)
                let edge2 = outer_b - inner_b;  // Diagonal edge (includes vertical drop)
                let mut bevel_normal = edge1.cross(edge2).normalize();
                // Ensure normal points outward-up (positive Y component)
                if bevel_normal.y < 0.0 {
                    bevel_normal = -bevel_normal;
                }

                let bevel_color = cliff_shade(top_color_bare);
                let bbase = vertices.len() as u16;
                // Quad: inner_a, inner_b, outer_b, outer_a (forms a slanted rectangle)
                vertices.push(vertex(inner_a, bevel_color, bevel_normal));
                vertices.push(vertex(inner_b, bevel_color, bevel_normal));
                vertices.push(vertex(outer_b, bevel_color, bevel_normal));
                vertices.push(vertex(outer_a, bevel_color, bevel_normal));
                indices.extend_from_slice(&[bbase, bbase + 1, bbase + 2, bbase, bbase + 2, bbase + 3]);
            }

            // Cliff quads: hidden-face removal (RnD 02 §3) — emit a side ONLY where the neighbour is
            // STRICTLY lower; an equal-or-higher neighbour covers that face, so skip it. Off-grid
            // neighbours (map edge) are treated as height 0 — draws a full cliff at the boundary.
            // Cliff top edge now starts at h - bevel_drop (the outer rim of the bevel).
            let cliff_color = cliff_shade(top_color_bare);
            for (dir_i, &(ncol, nrow)) in neighbors(col, row).iter().enumerate() {
                let nh = if (0..world_dim).contains(&ncol) && (0..world_dim).contains(&nrow) {
                    world.height(ncol, nrow) as f32 * height_scale
                } else {
                    0.0
                };
                if nh >= h {
                    continue;
                }
                let edge = edge_for_direction(dir_i);
                let outer_a = hex_corner(cx, cz, h, edge);
                let outer_b = hex_corner(cx, cz, h, (edge + 1) % 6);
                let top_a = Vec3::new(outer_a.x, h - bevel_drop, outer_a.z);
                let top_b = Vec3::new(outer_b.x, h - bevel_drop, outer_b.z);
                let bot_a = Vec3::new(outer_a.x, nh, outer_a.z);
                let bot_b = Vec3::new(outer_b.x, nh, outer_b.z);
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
    if vertices.len() >= HARD_CAPACITY_VERTS {
        return Err(BuildError::MeshBuildFailed(format!(
            "HEX chunk exceeded hard verts capacity: {} >= {}",
            vertices.len(),
            HARD_CAPACITY_VERTS
        )));
    }
    if indices.len() >= HARD_CAPACITY_INDICES {
        return Err(BuildError::MeshBuildFailed(format!(
            "HEX chunk exceeded hard indices capacity: {} >= {}",
            indices.len(),
            HARD_CAPACITY_INDICES
        )));
    }

    // Wrap in RawChunk (compute AABB)
    Ok(RawChunk::from_parts(vertices, indices))
}
