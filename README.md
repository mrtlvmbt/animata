# animata — voxel isometric world

A Minecraft-like **voxel environment** rendered in orthographic isometric on
macroquad's 3D pipeline (real cubes + GPU depth buffer). Each block has a logical
voxel position `(gx, gy, gz)`, so the world is 3D-ready by construction.

> **Reset note.** This repo previously hosted a DNA-based a-life simulation. That
> full simulation is archived at git tag **`sim-v1`** (branch `archive/sim-v1`) and
> can be restored at any time. The current line of work rebuilds the **environment**
> first — simulation and GUI are intentionally off — and the simulation will be
> re-integrated on top of the voxel world later.

## Run

```
cargo run            # environment viewer
```

Controls: `WASD`/arrows pan, mouse wheel zoom, `Q`/`E` rotate the iso view 90°.

## Status

- **Phase 0** (current): orthographic iso `Camera3D` + camera controls + a test
  height-field of cubes (depth-buffer / camera spike).
- Next: noise worldgen + chunked `VoxelTerrain` (bit-packed, ghost-padded), exposed
  -face chunk meshing, biome palette, vegetation, water.

See the working plan for the full phase breakdown.
