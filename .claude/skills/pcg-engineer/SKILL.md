---
name: pcg-engineer
description: >
  PCG expert for games: terrain (fBm, erosion, biomes, rivers), dungeons (BSP, WFC, cellular automata),
  textures (noise synthesis, Wang tiles, reaction-diffusion, tiling), environment scatter (Poisson,
  L-systems, city gen), runtime pipelines (chunk streaming, seeding, engine integration). Use for
  algorithm choice, architecture, debugging generation artifacts, performance.
triggers:
  - "terrain generation"
  - "dungeon/map generation"
  - "noise functions"
  - "WFC"
  - "procedural textures"
  - "foliage scatter"
  - "city/road gen"
  - "biome systems"
  - "PCG seeding"
  - "engine PCG integration"
---

# PCG Engineer Skill

Expert knowledge in procedural content generation. This skill covers the full PCG stack:
noise functions → terrain → biomes → environment decoration → runtime pipelines.

## Quick Algorithm Selector

| Goal | Recommended Algorithm | See Reference |
|------|-----------------------|---------------|
| Smooth organic terrain | Fractal Brownian Motion (fBm) + domain warping | `terrain.md` |
| Sharp cliff / mesa terrain | Ridged multifractal, Worley noise | `terrain.md` |
| Caves / tunnels | 3D Perlin threshold, Marching Cubes, cellular automata | `terrain.md` |
| Dungeon rooms | BSP, random walk, WFC | `dungeons.md` |
| Dungeon corridors | A*, Delaunay + MST, L-shaped hallways | `dungeons.md` |
| Open world regions | Voronoi + Lloyd relaxation, biome assignment | `terrain.md` |
| Tileable textures | Wang tiles, tiling noise, histogram-preserving blend | `textures.md` |
| Organic textures | Reaction-diffusion (Gray-Scott), cellular noise | `textures.md` |
| Procedural materials | Noise graph (Substance-style), FBm stacking | `textures.md` |
| Scatter vegetation | Poisson disk sampling, GPU instancing, density maps | `environment.md` |
| Realistic rivers | Hydraulic erosion, gradient descent flowmaps | `terrain.md` |
| City / settlement | L-system roads, lot subdivision, grammar rules | `environment.md` |
| Infinite worlds | Chunk-based streaming, seeded RNG per chunk | `pipelines.md` |
| Reproducibility | Deterministic seeding, splittable RNG (SplitMix64) | `pipelines.md` |

---

## Core Principles

### 1. Noise Fundamentals — Always Get This Right First

```python
# Value noise: fast, but blocky. Avoid for terrain.
# Perlin noise: smooth gradients, good general purpose.
# Simplex noise: better isotropy, faster in 3D+. Prefer over Perlin.
# Worley (cellular) noise: organic cells, good for cracks, scales, stones.
# fBm (Fractal Brownian Motion): stack octaves of noise. The standard for terrain.

def fbm(x, y, octaves=6, lacunarity=2.0, gain=0.5):
    value = 0.0
    amplitude = 0.5
    frequency = 1.0
    for _ in range(octaves):
        value += amplitude * noise(x * frequency, y * frequency)
        frequency *= lacunarity
        amplitude *= gain
    return value

# Domain warping = warp input coordinates with noise before sampling.
# Creates organic, non-repetitive shapes. Inigo Quilez's technique.
def domain_warp(x, y):
    q = vec2(fbm(x, y), fbm(x + 5.2, y + 1.3))
    r = vec2(fbm(x + 4*q.x + 1.7, y + 4*q.y + 9.2),
             fbm(x + 4*q.x + 8.3, y + 4*q.y + 2.8))
    return fbm(x + 4*r.x, y + 4*r.y)
```

**Common mistakes:**
- Using `random.random()` inside generation — breaks reproducibility. Always seed.
- Sampling noise at integer coordinates — values are always 0. Add fractional offset.
- Forgetting to normalize output (−1..1 or 0..1) before passing to downstream systems.
- fBm with too many octaves at high frequency = expensive + invisible detail.

---

### 2. Seeding and Reproducibility

All PCG systems must be **deterministic given a seed**. This is non-negotiable for save/load, multiplayer, and iteration.

```python
# Use SplitMix64 or PCG (Permuted Congruential Generator) — not system random.
# Derive per-chunk seeds from world seed + chunk coordinates:
def chunk_seed(world_seed: int, cx: int, cy: int) -> int:
    h = world_seed
    h ^= cx * 0x9e3779b97f4a7c15
    h ^= cy * 0x6c62272e07bb0142
    h = (h ^ (h >> 30)) * 0xbf58476d1ce4e5b9
    h = (h ^ (h >> 27)) * 0x94d049bb133111eb
    return h ^ (h >> 31)

# Never pass a shared RNG instance across chunk boundaries.
# Each chunk/region gets its own RNG instance seeded from chunk_seed().
```

---

### 3. Space Partitioning — Choosing the Right Structure

| Structure | Best For | Pitfall |
|-----------|----------|---------|
| BSP Tree | Dungeon rooms, rectangular subdivision | Rooms look grid-like without jitter |
| Voronoi | Biome regions, territory, organic areas | Naive impl O(n²); use Fortune's algo |
| Delaunay | Road networks, connecting Voronoi points | Need robust library (avoid float precision bugs) |
| Quadtree / Octree | Spatial queries, LOD terrain chunks | Overhead for small scenes |
| Poisson Disk | Object scatter, foliage, resource placement | Rejection sampling slow at high density |

---

### 4. Wave Function Collapse (WFC) — Quick Reference

WFC generates tilemaps/voxel worlds by constraint propagation. Common failure modes:

**Contradiction** (unsolvable state):
- Cause: Tile adjacency rules too restrictive, or entropy heuristic picks bad start cell.
- Fix: Add "wildcard" tiles, backtracking, or relax constraints at borders.

**Repetitive output**:
- Cause: Too few example tiles, low-entropy cells collapsed too early.
- Fix: Larger sample, weighted tile frequencies, entropy noise jitter.

**Performance**:
- Use bitsets for superposition states (64-tile = single u64).
- Propagation via priority queue (min-entropy cell first).
- For large maps: chunk-by-chunk with overlapping border cells.

```
Workflow:
1. Define tiles + adjacency rules (from sample image or hand-authored).
2. Initialize grid: all tiles possible in every cell.
3. Pick lowest-entropy cell → collapse to one tile (weighted random).
4. Propagate constraints to neighbors (remove now-impossible tiles).
5. Repeat until done or contradiction → backtrack or restart.
```

---

### 5. Terrain Pipeline (Standard)

```
Heightmap Generation
  └─ fBm base layer
  └─ Domain warp for organic feel
  └─ Ridge noise for mountains
  └─ Terrace function for plateaus (optional)

Erosion (optional but hugely improves realism)
  └─ Hydraulic erosion: simulate water droplets, carve channels
  └─ Thermal erosion: flatten steep slopes
  └─ 50–100 iterations of particle erosion per 512x512 chunk

Biome Assignment
  └─ Use temperature + moisture maps (separate fBm layers)
  └─ Whittaker diagram lookup: (temp, moisture) → biome enum

Mesh Generation
  └─ Heightmap → vertex grid
  └─ LOD: Quadtree / CDLOD / geometry clipmaps
  └─ Normal calculation: central differences or Sobel filter

Detail / Decoration → see environment.md
```

---

### 6. Environment Decoration — Fast Rules

**Foliage scatter:**
- Never use uniform grid — use Poisson disk or blue noise.
- Layer density maps: base noise × slope mask × altitude mask × biome weight.
- Cull by slope: trees don't grow on 45°+ inclines (dot(normal, up) threshold).
- GPU instancing mandatory for >10k objects.

**Resource / landmark placement:**
- Minimum distance constraints (Poisson disk per category).
- Importance sampling: weight by gameplay value map.
- Use seeded per-region RNG, not global state.

---

## When to Load Reference Files

Read the appropriate reference when the user's question is domain-specific:

- **Terrain generation, erosion, heightmaps, biomes, rivers** → read `references/terrain.md`
- **Dungeons, rooms, corridors, roguelike maps** → read `references/dungeons.md`
- **Textures, noise-based materials, tiling, substance graphs** → read `references/textures.md`
- **Environment decoration, scatter, cities, L-systems** → read `references/environment.md`
- **Streaming, chunks, runtime pipelines, performance, seeding** → read `references/pipelines.md`

---

## Debugging PCG Artifacts — Common Diagnoses

| Symptom | Likely Cause | Fix |
|---------|-------------|-----|
| Terrain has visible grid pattern | Perlin gradient grid alignment | Switch to Simplex, add rotation per octave |
| Texture tiles visibly repeat | Frequency too low, no variation | Histogram-preserving blend, Wang tiles |
| Dungeon has isolated rooms | Graph not connected | Delaunay triangulation + MST corridors |
| WFC contradiction rate >5% | Rule over-constraint | Add wildcards, increase backtrack budget |
| Biomes have hard edges | No blending between regions | Interpolate biome weights, not hard cutoff |
| Generation slow at runtime | Single-threaded, no caching | Async chunk jobs, cache heightmap tiles |
| Different results same seed | Global mutable RNG | Per-system seeded RNG instances |
| Erosion creates flat lakes | No drainage out | Implement fill-then-drain or depression linking |
