---
name: terrain-geomorph
description: >
  Deep geophysical terrain generation: heightmap synthesis from noise (fBm, ridged, domain
  warp, hypsometry, contour/topographic maps), plate-tectonics simulation (continental/oceanic
  crust, subduction, collision, orogeny, hotspots), erosion (droplet + pipe-model/shallow-water
  hydraulic, thermal, fluvial stream-power landscape evolution), river networks (D8/D∞ flow
  routing, priority-flood depression filling, drainage accumulation, Strahler order, meander
  migration, oxbow lakes), and 3D voxel terrain (density fields, marching cubes, dual contouring,
  surface nets, transvoxel LOD, caves/overhangs). Use for algorithm selection, the geomorphic
  "why", parameter tuning, debugging artifacts (flat lakes, terraced erosion, plate seams,
  non-manifold voxel meshes), and performance.
triggers:
  - "plate tectonics"
  - "tectonic uplift"
  - "orogeny"
  - "mountain range generation"
  - "hydraulic erosion"
  - "thermal erosion"
  - "droplet erosion"
  - "stream power law"
  - "landscape evolution model"
  - "river network generation"
  - "drainage basin"
  - "meander"
  - "oxbow lake"
  - "depression filling"
  - "flow accumulation"
  - "D8 flow routing"
  - "topographic map generation"
  - "contour lines"
  - "heightmap noise terrain"
  - "voxel terrain"
  - "marching cubes"
  - "dual contouring"
  - "caves overhangs terrain"
---

# Terrain Geomorphology Skill

Expert knowledge for **physically-grounded terrain generation**. Where `pcg-engineer` gives the
broad PCG stack, this skill goes deep on the geophysical chain that makes terrain read as *real*:

```
Tectonics (uplift field)  →  Heightmap synthesis  →  Erosion (carve)  →  Rivers/lakes  →  Voxelize (optional)
   plates, orogeny            noise, ridged, warp      hydraulic/fluvial    flow routing      density field
```

The single biggest realism lever is **the order**: noise alone looks like cotton wool. Real
landforms come from an **uplift source** (tectonics) shaped by a **removal process** (erosion)
whose **drainage** (rivers) is the visible signature. Skip erosion → no valleys, no ridgelines,
no dendritic rivers — just blobs.

## Quick Algorithm Selector

| Goal | Recommended approach | See |
|------|----------------------|-----|
| Continents, ocean basins, big mountain belts | Plate-tectonics sim (Voronoi plates + collision uplift) | `tectonics.md` |
| Realistic mountain *belts* (not isolated peaks) | Tectonic uplift map → fluvial erosion (stream-power) | `tectonics.md`, `erosion.md` |
| Sharp ridgelines / alpine relief | Ridged multifractal + thermal erosion (talus) | `heightmaps.md`, `erosion.md` |
| Eroded, water-carved hills with valleys | fBm base → droplet **or** pipe-model hydraulic erosion | `erosion.md` |
| Geologically "correct" large-scale terrain | Landscape-evolution model: `∂h/∂t = U − K·Aᵐ·Sⁿ` | `erosion.md` |
| Dendritic river network on a DEM | Priority-flood fill → D8/D∞ → flow accumulation → threshold | `rivers.md` |
| Meandering lowland river with oxbows | Howard–Knutson curvature migration + neck cutoff | `rivers.md` |
| Lakes that hold water (no flat artifacts) | Depression detection → fill-or-breach, link to outlet | `rivers.md` |
| Topographic / contour map output | Hypsometric remap → marching-squares isolines | `heightmaps.md` |
| Caves, overhangs, arches, vertical cliffs | 3D density field + marching cubes / dual contouring | `voxels.md` |
| Smooth voxel terrain, edited in real time | Surface nets / dual contouring + chunked octree LOD | `voxels.md` |

## Core Principles

### 1. Uplift before noise

Don't sculpt mountains by hand-stacking octaves. Generate an **uplift field `U(x,y)`** (tectonic
collision zones, hotspot tracks) and feed it as the *forcing* of an erosion/landscape model. The
mountains, foothills, and drainage then *emerge* and are mutually consistent. Pure-noise mountains
have no consistent watersheds, so rivers placed afterward look pasted on.

### 2. Everything downstream needs a depression-free flow surface

Rivers, lakes, erosion routing, and biome moisture all depend on knowing **where water goes**. The
foundational primitive is **Priority-Flood** depression filling (Barnes 2014), then a **flow
direction** (D8 or D∞) and **flow accumulation** (drainage area `A`). Get this pipeline right once;
half the other systems reuse it. See `rivers.md`.

### 3. Two erosion families — pick on purpose

- **Hydraulic, particle/grid (droplet, pipe-model/SWE)** — fast, GPU-friendly, *visual* realism for
  a fixed-size heightmap. Carves channels and deposits alluvium. No long-timescale tectonic context.
- **Fluvial / landscape-evolution (stream-power law)** — `E = K·Aᵐ·Sⁿ`. Couples to uplift, produces
  *geomorphically correct* concave river profiles, knickpoints, and mountain-belt spacing. Slower,
  needs the flow pipeline. This is what makes "real-looking continents."
- **Thermal** — mass wasting: any slope steeper than the *talus angle* sheds material to neighbors.
  Cheap; run it alongside hydraulic to keep cliffs from being unrealistically vertical.

### 4. Determinism is non-negotiable

Same as all PCG: seed everything (SplitMix64 / PCG-RNG), per-region seed derivation, never a shared
mutable global RNG. Erosion and tectonics are iterative — a single nondeterministic step poisons the
whole map and breaks save/load and reproducibility.

### 5. Scale-aware parameters

Erosion constants, talus angles, noise frequencies, and meander wavelengths are **resolution- and
world-scale-dependent**. A talus threshold tuned at 512² will terrace at 2048². Always express
thresholds in *world units per cell* (slope = Δh / cell_size), not in raw height deltas.

## When to load reference files

- **Noise→heightmap, fBm/ridged/billow, domain warp, hypsometry, contour/topo maps, masks** → `references/heightmaps.md`
- **Plate tectonics sim, crust types, boundaries, subduction, collision, orogeny, hotspots** → `references/tectonics.md`
- **Hydraulic (droplet + pipe/SWE), thermal, fluvial stream-power, landscape evolution, sediment** → `references/erosion.md`
- **Flow routing (D8/D∞), priority-flood, drainage accumulation, river extraction, meanders, lakes, Strahler** → `references/rivers.md`
- **Voxel terrain, density fields, marching cubes / dual contouring / surface nets, transvoxel LOD, caves** → `references/voxels.md`
- **Primary papers, repos, and further reading** → `references/sources.md`

## Debugging — common geomorphic artifacts

| Symptom | Likely cause | Fix |
|---------|-------------|-----|
| Mountains look like random lumps, no ridgelines | No erosion / no uplift source | Add fluvial erosion driven by an uplift field |
| Rivers don't reach the sea / pool randomly | Unfilled depressions, no flow routing | Priority-flood fill → D8 accumulation |
| Lakes render as perfectly flat squares draining nowhere | Pit filled but never linked to an outlet | Fill-then-spill: route overflow to lowest rim cell |
| Erosion produces stair-step terraces | Talus / step threshold too large for resolution | Express threshold per cell_size; smaller steps, more iters |
| Hydraulic erosion "explodes" / NaNs | Timestep too large in pipe model (CFL) | Reduce Δt or clamp flux to available water |
| Plate boundaries leave visible straight seams | Voronoi cell edges unsmoothed | Domain-warp boundary, blend uplift across a falloff band |
| Mountain belts uniformly tall, no foothills | Uplift applied directly as height (no erosion) | Treat uplift as rate `U`, integrate with erosion `K·Aᵐ·Sⁿ` |
| River network is a single thick line, no tributaries | Accumulation threshold constant | Threshold on `A` so channels branch where drainage grows |
| Meanders grow then self-intersect into spaghetti | No neck-cutoff step | Detect bend-neck proximity < cutoff_dist → splice + spawn oxbow |
| Voxel mesh has cracks between LOD levels | Mismatched edges across chunk resolutions | Transvoxel transition cells, or skirts |
| Dual-contouring surface self-intersects / non-manifold | Degenerate QEF in sharp/aliased cells | Clamp vertex to cell AABB, add QEF regularization |
| 3D-noise caves are uniform spaghetti everywhere | Single isolevel of raw 3D noise | Modulate density by depth gradient + 2D surface mask |
