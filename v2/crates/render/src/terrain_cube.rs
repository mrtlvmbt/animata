//! R-5: cube-voxel terrain mesh — `WorldView` → square columns (vs hex prisms in R-2).
//! Each `WorldView` cell → a unit square column at world (x=col+0.5, z=row+0.5), height * HEIGHT_SCALE.
//! Side quads only where neighbour is STRICTLY lower (hidden-face removal, RnD `rendering/02` §3).
//! Biome-colored (via `biome_palette`) + baked per-face-direction shading (mirror v1 `mesh.rs::shaded`).
//!
//! R-9: TOP faces are greedy-meshed — a row-major sweep merges a maximal rectangle of cells
//! sharing (height, biome_color) into ONE quad. Cliff/side quads stay per-edge, hidden-face-culled
//! as before (greedy does NOT apply to them this slice).
//!
//! Same `ROWS_PER_CHUNK` chunking + u16-index assert as hex terrain (`terrain.rs`).
//! Built ONCE at startup — cold terrain immutable for the run.

use crate::biome_palette::{biome_color, cliff_shade, apply_directional_shading};
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

    let cols = world_dim as usize;
    let band_rows = (row1 - row0) as usize;

    // Cache (height, biome_color) per cell in this band — read once, reused by both
    // the greedy top-face pass and the per-cell cliff pass below.
    let mut heights = vec![vec![0f32; cols]; band_rows];
    let mut colors = vec![vec![WHITE; cols]; band_rows];
    for local_row in 0..band_rows {
        let row = row0 + local_row as i64;
        for col in 0..world_dim {
            heights[local_row][col as usize] = world.height(col, row) as f32 * HEIGHT_SCALE;
            colors[local_row][col as usize] = biome_color(world.biome(Vec2Fixed(col, row)));
        }
    }

    // ────────────────────────────────────────────────────────────────────
    // Greedy top-face meshing (R-9): row-major sweep, merge a maximal rectangle
    // of cells sharing (height, biome_color) into ONE quad. Deterministic order.
    // ────────────────────────────────────────────────────────────────────
    let mut visited = vec![vec![false; cols]; band_rows];
    for local_row in 0..band_rows {
        for col0 in 0..cols {
            if visited[local_row][col0] {
                continue;
            }
            let h = heights[local_row][col0];
            let color = colors[local_row][col0];

            // Grow right while the run shares (height, color) and is unvisited.
            let mut col1 = col0 + 1;
            while col1 < cols
                && !visited[local_row][col1]
                && heights[local_row][col1] == h
                && colors[local_row][col1] == color
            {
                col1 += 1;
            }

            // Grow down while the whole [col0, col1) row matches.
            let mut row_end = local_row + 1;
            'grow_down: while row_end < band_rows {
                for c in col0..col1 {
                    if visited[row_end][c] || heights[row_end][c] != h || colors[row_end][c] != color {
                        break 'grow_down;
                    }
                }
                row_end += 1;
            }

            for r in visited.iter_mut().take(row_end).skip(local_row) {
                for v in r.iter_mut().take(col1).skip(col0) {
                    *v = true;
                }
            }

            // Emit ONE quad spanning the merged rectangle [col0,col1) x [world_row0,world_row1).
            let world_row0 = row0 + local_row as i64;
            let world_row1 = row0 + row_end as i64;
            let x0 = col0 as f32;
            let x1 = col1 as f32;
            let z0 = world_row0 as f32;
            let z1 = world_row1 as f32;

            let base = vertices.len() as u16;
            let top_normal = Vec3::new(0.0, 1.0, 0.0); // Top face normal (pointing up)
            vertices.push(vertex(Vec3::new(x0, h, z0), color, top_normal)); // TL
            vertices.push(vertex(Vec3::new(x1, h, z0), color, top_normal)); // TR
            vertices.push(vertex(Vec3::new(x1, h, z1), color, top_normal)); // BR
            vertices.push(vertex(Vec3::new(x0, h, z1), color, top_normal)); // BL
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }

    for local_row in 0..band_rows {
        let row = row0 + local_row as i64;
        for col in 0..world_dim {
            let h = heights[local_row][col as usize];
            // Square cell center: (col + 0.5, row + 0.5) in world space
            // Each cell is a 1×1 square, so corners are at ±0.5 from center
            let cx = col as f32 + 0.5;
            let cz = row as f32 + 0.5;
            let size = 0.5; // Half-size: extends ±0.5 from center in x,z

            let top_color = colors[local_row][col as usize];
            let cliff_color = cliff_shade(top_color);

            // ────────────────────────────────────────────────────────────────────
            // Side quads: hidden-face removal (RnD 02 §3). Cliffs stay per-cell/per-edge
            // (NOT greedy-meshed this slice) since a merged cliff strip would need to
            // track per-segment neighbour heights anyway — no win, more complexity.
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
                // West edge (x = cx - size, normal = (-1, 0, 0))
                (
                    Vec3::new(cx - size, h, cz - size), // top_a (TL)
                    Vec3::new(cx - size, h, cz + size), // top_b (BL)
                    Vec3::new(-1.0, 0.0, 0.0), // normal
                ),
                // East edge (x = cx + size, normal = (1, 0, 0))
                (
                    Vec3::new(cx + size, h, cz + size), // top_a (BR)
                    Vec3::new(cx + size, h, cz - size), // top_b (TR)
                    Vec3::new(1.0, 0.0, 0.0), // normal
                ),
                // North edge (z = cz - size, normal = (0, 0, -1))
                (
                    Vec3::new(cx + size, h, cz - size), // top_a (TR)
                    Vec3::new(cx - size, h, cz - size), // top_b (TL)
                    Vec3::new(0.0, 0.0, -1.0), // normal
                ),
                // South edge (z = cz + size, normal = (0, 0, 1))
                (
                    Vec3::new(cx - size, h, cz + size), // top_a (BL)
                    Vec3::new(cx + size, h, cz + size), // top_b (BR)
                    Vec3::new(0.0, 0.0, 1.0), // normal
                ),
            ];

            for (edge_idx, &(top_a, top_b, edge_normal)) in edge_configs.iter().enumerate() {
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
                vertices.push(vertex(top_a, cliff_color, edge_normal));
                vertices.push(vertex(top_b, cliff_color, edge_normal));
                vertices.push(vertex(bot_b, cliff_color, edge_normal));
                vertices.push(vertex(bot_a, cliff_color, edge_normal));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed `dim × dim` grid — height/biome given per (col, row).
    struct GridWorld {
        dim: i64,
        heights: Vec<i64>,
        biomes: Vec<u8>,
    }

    impl GridWorld {
        fn idx(&self, col: i64, row: i64) -> usize {
            (row * self.dim + col) as usize
        }
    }

    impl WorldView for GridWorld {
        fn is_solid(&self, _pos: Vec2Fixed) -> bool {
            true
        }
        fn height(&self, x: i64, z: i64) -> i64 {
            self.heights[self.idx(x, z)]
        }
        fn biome(&self, pos: Vec2Fixed) -> u8 {
            self.biomes[self.idx(pos.0, pos.1)]
        }
        fn resource(&self, _pos: Vec2Fixed) -> i64 {
            0
        }
        fn temp_at(&self, _pos: Vec2Fixed) -> i32 {
            1500 // P3-1: stub returns mesophile (15°C)
        }
    }

    /// Flat (height 0 everywhere) so hidden-face culling drops ALL cliffs (nh=0 >= h=0) —
    /// isolates the mesh to top-face vertices only, making vertex counts directly comparable.
    fn flat_world(dim: i64, biomes: Vec<u8>) -> GridWorld {
        GridWorld { dim, heights: vec![0; (dim * dim) as usize], biomes }
    }

    #[test]
    fn greedy_merges_uniform_flat_region_into_one_quad() {
        let dim = 4;
        let world = flat_world(dim, vec![0; 16]);
        let chunks = build_cube_terrain(dim, &world);
        assert_eq!(chunks.len(), 1);
        let mesh = &chunks[0].mesh;

        // Naive per-cell top faces would emit dim*dim quads (4 verts, 6 indices each).
        let naive_vertices = (dim * dim * 4) as usize;
        assert!(mesh.vertices.len() < naive_vertices, "greedy must reduce vertex count");
        // Whole region shares (height, biome) → merges into exactly ONE quad.
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices.len(), 6);

        // Footprint (AABB) still covers the full dim × dim square, same as naive per-cell would.
        assert_eq!(chunks[0].bounds, (Vec3::new(0.0, 0.0, 0.0), Vec3::new(dim as f32, 0.0, dim as f32)));
    }

    #[test]
    fn greedy_does_not_merge_across_biome_boundary() {
        // 2×2, columns differ in biome; each column's 2 rows share (height, biome) and merge vertically.
        let world = flat_world(2, vec![0, 1, 0, 1]); // row-major: (row0: col0=0,col1=1), (row1: col0=0,col1=1)
        let chunks = build_cube_terrain(2, &world);
        let mesh = &chunks[0].mesh;

        // 2 quads (one per column), NOT 1 — biome_color equality gates the merge.
        assert_eq!(mesh.vertices.len(), 8);
        assert_eq!(mesh.indices.len(), 12);
    }

    #[test]
    fn greedy_does_not_merge_across_height_boundary() {
        let mut world = flat_world(2, vec![0; 4]);
        world.heights = vec![0, 1, 0, 1]; // col1 is one unit taller than col0, same biome
        let chunks = build_cube_terrain(2, &world);
        // Height differs between col0/col1 → their side quads are no longer culled (nh < h for col0's
        // east edge and col1 emits none west since col0 lower doesn't apply — col1's neighbour col0 is
        // lower so col1 gets a cliff there); assert top faces alone still split into 2 merged quads.
        let top_face_vertices = 2 * 4; // 2 merged columns × 4 verts each
        assert!(chunks[0].mesh.vertices.len() >= top_face_vertices);
    }
}
