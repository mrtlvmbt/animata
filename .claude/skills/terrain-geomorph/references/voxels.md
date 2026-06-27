# Voxel Terrain — Density Fields, Meshing, LOD

Heightmaps can't do **overhangs, caves, arches, or vertical cliffs** — each (x,y) has exactly one
height. Voxels store a **3D scalar field** (density / signed distance), so the surface can fold over
itself. Cost: memory and meshing complexity. Use voxels only where the extra freedom pays off
(caves, cliffs, destructible/editable terrain); keep open ground as a heightmap.

## The density field

Surface = isosurface where `density(x,y,z) = 0`. Convention: **negative inside solid, positive in
air** (signed distance-like). Build it by *modulating a heightmap with 3D noise*, not raw 3D noise:

```python
def density(x, y, z):
    surface = terrain_height(x, z)               # 2D heightmap (xz plane, y up)
    d = surface - y                              # >0 below ground, <0 above  → base solid/air split
    # caves: carve where a 3D noise ridge is strong, gated by depth so caves don't open the surface
    cave = ridged3(x*fc, y*fc, z*fc)             # worm-like tunnels
    depth_gate = smoothstep(0, 8, surface - y)   # only carve a few meters below surface
    d -= cave_strength * cave * depth_gate
    # overhangs: add 3D fbm to the surface term so it can fold
    d += overhang_amp * fbm3(x*fo, y*fo, z*fo)
    return d
```
- **Raw 3D-noise isolevel everywhere = uniform spaghetti.** Always anchor to a 2D surface + depth
  gradient so the world has a ground plane and caves stay underground.
- **3D Worley worms / ridged tunnels** give connected cave systems; threshold `|F2−F1|` for tubes.
- Store density at cell **corners**; meshers interpolate the zero-crossing along edges.

## Meshing — pick by feature needs

| Algorithm | Vertices placed | Sharp features | Manifold | Notes |
|-----------|-----------------|----------------|----------|-------|
| **Marching Cubes** | on cube *edges* (per-edge interp) | no (rounds them) | yes | 256-case table (15 base); the default workhorse |
| **Surface Nets** | one vertex per cell, at avg of edge crossings | softened | yes | simplest dual method; smooth blobby terrain, cheap |
| **Dual Contouring** | one vertex per cell, placed by **QEF** using edge normals (Hermite data) | **yes** (cliffs, ledges) | not guaranteed | needs gradients; clamp vertex to cell to stay sane |
| **Transvoxel** | MC variant with transition cells | no | yes | solves LOD seams between chunk resolutions |

```
Marching Cubes per cell:
  1. case = bitmask of which 8 corners are inside (density<0)
  2. look up edge list for that case (precomputed table)
  3. for each active edge, vertex = lerp(corner_a, corner_b, by zero-crossing of density)
  4. emit triangles per the table
```
**Dual Contouring** wins for terrain because it reproduces **sharp edges** (cliff tops, plateau
rims) from the surface normals, where MC rounds everything. Its risk: the QEF can place a vertex
outside the cell on aliased/degenerate data → self-intersections / non-manifold mesh. Mitigate:
clamp the QEF solution to the cell AABB and add a small regularization term toward the cell center.

Normals: from the **gradient of the density field** (central differences), not from triangle
faces — gives smooth shading and is consistent across LODs.

## Chunking & LOD

```
World = grid of chunks (e.g. 32³ voxels). Generate/mesh per chunk on worker threads.
LOD: sample density at coarser stride farther from camera (octree). Each level = ½ resolution.
```
- **Seams between LOD levels** are the central problem: a fine chunk's edge vertices don't line up
  with a coarse neighbor's → cracks. Fixes:
  - **Transvoxel** transition cells (Lengyel) — purpose-built MC extension stitching 2:1 LOD borders.
  - **Skirts** — drop a vertical apron at chunk edges to hide gaps (cheap, slightly fake).
  - Snap/weld boundary vertices to the coarser grid.
- Octree (sparse voxel octree / SVO) for storage when most space is empty air or uniform solid —
  don't store 32³ dense floats per chunk if 90% is air.
- Generate density on GPU (compute shader), mesh on GPU or worker pool; upload only changed chunks.

## Editing / destructible
Voxels shine when terrain changes at runtime: add/subtract density in a brush radius, re-mesh only
affected chunks. Dual contouring + Hermite data preserves sharp edges through edits where MC would
progressively round dug holes.

## When NOT to voxelize
- Pure rolling terrain with no caves/overhangs → heightmap + erosion is far cheaper and supports
  trivial LOD (geometry clipmaps / CDLOD). Reserve voxels for the sub-regions that need 3D freedom,
  or use a hybrid: heightmap surface + voxel "patches" for caves and cliffs.

## Pitfalls
| Symptom | Cause | Fix |
|---------|-------|-----|
| Caves everywhere, no solid ground | raw 3D-noise isolevel, no surface anchor | modulate heightmap + depth gate |
| Cracks between chunks at distance | LOD resolution mismatch | transvoxel transition cells / skirts |
| Cliffs/ledges look melted | marching cubes rounds sharp features | dual contouring with Hermite normals |
| DC mesh self-intersects / non-manifold | unconstrained QEF on degenerate cells | clamp vertex to cell AABB + regularize QEF |
| Faceted lighting | normals from triangles | normals from density gradient |
| Caves break the surface into holes | no depth gate on carve term | gate carving below surface by `smoothstep(depth)` |
