# Plate Tectonics Simulation

Tectonics supplies the **uplift field** — the source term every realistic large-scale terrain needs.
Two practical schools:

1. **Lithosphere collision sim** (à la *pyplatec* / Lauri Viitanen's PlaTec) — rasterized plates with
   mass that physically collide, subduct, and aggregate. Good for whole continents from scratch.
2. **Procedural tectonic authoring** (Cortial et al. 2019, *Procedural Tectonic Planets*) — approximate
   the *phenomena* (subduction, collision, ridges) as deformation operators on crust, with user control
   over plate motion. Faster, art-directable, avoids full physical simulation.

Both produce the same deliverables: a **crust-type map** (continental vs oceanic), a **boundary-type
map** (convergent/divergent/transform), and an **uplift/age field** to drive erosion.

## The vocabulary you must model

| Concept | Why it matters for terrain |
|---------|---------------------------|
| **Continental crust** | thick, buoyant, *old*, low density → stays high, forms land |
| **Oceanic crust** | thin, dense, *young near ridges* → sits low, forms ocean floor |
| **Divergent boundary** | plates separate → mid-ocean **ridge**, new crust, rifts/volcanism |
| **Convergent: O–C** | ocean subducts under continent → **trench** + volcanic **arc mountains** (Andes) |
| **Convergent: C–C** | neither subducts → crust crumples → giant **collision belt** (Himalaya) |
| **Convergent: O–O** | one subducts → **island arc** (Japan, Aleutians) |
| **Transform boundary** | plates slide past → faults, offset ridges, little vertical relief |
| **Hotspot** | mantle plume, plate-independent → **volcanic chain** with age gradient (Hawaii) |

## Building the plate field

```python
# 1. Seed plates as Voronoi regions over the sphere/plane.
plate_seeds = poisson_disk(n=8..40, seed=world_seed)     # few big + many small is most Earth-like
plate_id    = voronoi_nearest(grid, plate_seeds)         # each cell → owning plate

# 2. Assign each plate a rigid motion: an Euler pole (axis) + angular velocity on a sphere,
#    or a 2D drift vector + small rotation on a plane.
for p in plates:
    p.euler_pole = random_unit_vec(rng)
    p.omega      = rng.uniform(0.2, 1.0)    # deg / step
    p.crust      = "continental" if rng()<0.4 else "oceanic"
    p.base_elev  = +0.3 if p.crust=="continental" else -0.4
```

**Smooth the seams.** Raw Voronoi gives straight plate edges that betray the algorithm. Domain-warp
the boundary (`plate_id` lookup on warped coords) and blend `base_elev` across a falloff band so
coasts and boundary mountains don't render as polygon edges.

## Classifying boundaries (the heart of it)

For each boundary cell between plate A and B, project their velocities onto the boundary normal `n`:

```python
rel = velocity(B, cell) - velocity(A, cell)
conv =  dot(rel, n)        # >0 converging, <0 diverging
shear = dot(rel, t)        # along-boundary → transform component

if conv >  eps:   boundary = "convergent"
elif conv < -eps: boundary = "divergent"
else:             boundary = "transform"
```

Then apply the right deformation per the crust pair:

| Boundary | A,B crust | Deformation written into uplift `U` |
|----------|-----------|-------------------------------------|
| convergent | C–C | broad, high uplift belt; widen with `|conv|`; double-sided |
| convergent | O–C | trench (sharp negative) on ocean side + arc ridge inland on continent |
| convergent | O–O | trench + narrow island-arc ridge on overriding plate |
| divergent | O–O | ridge crest (mild +), flanks subside with crust age |
| divergent | C–C | rift valley (negative) → proto-ocean |
| transform | any | near-zero vertical; offset features, add fault noise |

### Uplift falloff
Mountains aren't a 1-cell line. Spread uplift inland with a profile — typically exponential or a
skewed bump peaking a short distance from the boundary (arc volcanoes sit *behind* the trench):

```python
U[cell] += peak * exp(-(dist_to_boundary - offset)**2 / (2*width**2))   # gaussian ridge
```

## Time stepping (collision sim school)

```
for step in range(N):
    move each plate by its Euler rotation (advect crust raster)
    detect overlaps (two plates claim a cell):
        if one is oceanic  → it SUBDUCTS: remove its crust there, add uplift to overriding plate
        if both continental→ COLLIDE: stack mass → strong uplift, freeze relative motion (suture)
    detect gaps (no plate claims a cell):
        spawn new OCEANIC crust at age 0 (divergent ridge)
    age all crust (+1); oceanic crust deepens with sqrt(age)  # half-space cooling: depth ∝ √age
    erode/diffuse the uplift slightly so belts don't grow unbounded
```

`depth_ocean ≈ ridge_depth + c·sqrt(age)` reproduces the real abyssal-hill profile and gives
ridges-high / old-ocean-deep automatically.

## Hotspots
Plate-independent. Fixed point in the mantle frame; as crust drifts over it, stamp a volcano and
record creation time → a **chain** whose age increases away from the current hotspot location.
Classic test of whether your plate-motion field is consistent.

## Handing off to erosion
Output `U(x,y)` (uplift **rate**, not final height) + crust/age maps. Then run the **stream-power
landscape model** (`erosion.md`): `∂h/∂t = U − K·Aᵐ·Sⁿ`. The mountains, foothills, and rivers
co-evolve into a consistent belt. Applying `U` directly as height (skipping erosion) is the #1
reason simulated tectonics still looks fake — you get uniform plateaus with no drainage.

## Pitfalls
- **Straight Voronoi seams** → warp + blend boundaries (above).
- **Runaway uplift** → uplift is a *rate* capped by erosion; add diffusion each step.
- **All-same-size plates** → use few large + many small (power-law sizes) for Earth-like coastlines.
- **Ignoring crust age on ocean floor** → flat oceans; add √age deepening for ridges/abyssal contrast.
- **Symmetric arcs** → real arcs are one-sided (trench vs back-arc); offset the uplift peak inland.
