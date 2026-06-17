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

Controls: `WASD`/arrows pan, mouse wheel zoom, `Q`/`E` rotate the iso view 90°,
`R` regenerate with a new seed, `I` toggle the fps/ms readout.

Dev bridge: `cargo run --features dev` adds a localhost JSON-RPC server (drive the
camera, reseed, capture screenshots) — see `DEV_BRIDGE.md`.

Scale: **1 voxel = 1 m³**; the base map is 138×95 m. `MAP_SCALE` (in `config.rs`)
scales it (×16 per side is the eventual target, needs chunk streaming).

## Status

- **Phase 0–2** (done): orthographic iso `Camera3D`; noise worldgen into a chunked,
  bit-packed, ghost-padded `VoxelTerrain` (7 biomes + heights); rendered as batched
  per-chunk meshes (exposed top + cliff side faces, strata bands, baked per-face
  shading) on the GPU depth buffer.
- Next: vegetation (voxel trees), translucent water pass, then data-driven biomes.

See the working plan for the full phase breakdown.
