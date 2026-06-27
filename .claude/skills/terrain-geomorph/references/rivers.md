# Rivers, Drainage Networks, Meanders & Lakes

Everything here builds on one pipeline: **make the surface drain → know where water flows → measure
how much → extract channels.** This same pipeline feeds fluvial erosion (`erosion.md`) and biome
moisture.

```
DEM → [1] fill/breach depressions → [2] flow directions → [3] flow accumulation → [4] channel extract → [5] meander/lake post
```

## 1. Depression filling — Priority-Flood (Barnes 2014)

Pits with no outlet break flow routing. **Priority-Flood** floods inward from the map edge using a
**priority queue keyed by elevation**, guaranteeing every cell drains. Optimal: `O(n)` integer,
`O(n log n)` float — the fastest known and the one to use.

```python
import heapq
def priority_flood_fill(dem):
    H,W = dem.shape; filled = dem.copy()
    closed = zeros(bool); pq = []
    for c in border_cells(dem):                       # seed with all edges
        heapq.heappush(pq,(dem[c],c)); closed[c]=True
    while pq:
        e,(y,x) = heapq.heappop(pq)
        for ny,nx in neighbors8(y,x):
            if closed[ny,nx]: continue
            filled[ny,nx] = max(filled[ny,nx], e)     # raise pits up to spill level
            closed[ny,nx] = True
            heapq.heappush(pq,(filled[ny,nx],(ny,nx)))
    return filled
```
- **Fill vs breach:** filling *raises* pits to their spill level (makes lakes). Breaching *cuts* a
  channel out (keeps terrain low). Hybrid (breach small, fill large) avoids both huge lakes and deep gashes.
- The `+ε` variant adds a tiny gradient toward the outlet so filled flats still route (no zero-slope ties).
- **Priority-Flood+FlowDirs** assigns D8 directions *while* flooding — one pass, no separate fill needed.

## 2. Flow direction

| Method | Idea | Trade-off |
|--------|------|-----------|
| **D8** | each cell → single steepest of 8 neighbors | simple, but flow snaps to 8 directions (parallel-river artifact on planar slopes) |
| **D∞ (Tarboton)** | flow to the steepest *downslope triangular facet*, split between the 2 bounding cells by angle | smooth, realistic dispersion; preferred for accumulation/erosion |
| MFD (multiple) | distribute to all lower neighbors by slope^p | diffuse; good for hillslopes, bad for crisp channels |

```python
def d8_receiver(dem, y, x, cell):
    best, bdir = 0, None
    for ny,nx in neighbors8(y,x):
        d = (dem[y,x]-dem[ny,nx]) / (cell*dist(y,x,ny,nx))   # slope; diag dist = √2
        if d > best: best, bdir = d, (ny,nx)
    return bdir   # None ⇒ outlet/pit (shouldn't happen post-fill)
```

## 3. Flow accumulation (drainage area `A`)

How many upstream cells drain through each cell — proxy for discharge. Process cells in **decreasing
elevation** (or topological receiver order) so every cell is handled after everything draining into it:

```python
def flow_accumulation(receivers):                    # receivers: cell → downstream cell
    A = ones(N)                                       # each cell counts itself (×cell_area)
    for c in sorted_by_elevation_desc(cells):
        r = receivers[c]
        if r is not None: A[r] += A[c]
    return A                                          # multiply by cell_area for real area
```
D∞ version splits `A[c]` between the two receivers by the facet angle.

## 4. Channel / network extraction

A cell is **channelized** where drainage exceeds a threshold — but a *constant* threshold makes one
fat line. Use the geomorphic **area–slope** criterion so channels begin where flow concentrates:

```python
is_channel = A * (slope**theta) > tau     # theta≈1.0..2.0; channels start lower on steep ground
```
- Trace river polylines by walking receivers from each channel **source** (channel cell with no
  channelized upstream neighbor) down to the sea/outlet, merging at confluences.
- **River width** ∝ `sqrt(A)` (hydraulic geometry: width ~ discharge^0.5). Carve the heightfield a
  bit at channels (`h -= depth·smoothstep`) so they read as incised, not painted on.

### Strahler stream order (network hierarchy)
```
source segment            → order 1
two equal orders meet (k) → order k+1
unequal orders meet       → max of the two
```
Use order to scale width, render only main stems at distance (LOD), and validate the network
(Horton's laws: order count should fall ~geometrically — bifurcation ratio ~3–5).

## 5a. Lakes (do it right — avoids flat-square artifact)

A real lake = a depression filled **to its spill point**, with overflow continuing downstream.
```
For each depression found during priority-flood:
  spill_elev = lowest rim cell (the saddle to a lower neighbor outside the pit)
  lake cells = pit cells with filled_elev <= spill_elev   → flat water surface at spill_elev
  route the lake's accumulated inflow OUT through the spill cell into the receiver network
```
Skipping the spill-routing is the classic bug: the pit fills, water vanishes, and you get a flat
plate that drains nowhere. Always **fill *and* spill**. Endorheic (no outlet to sea, e.g. salt lakes)
are valid — mark them and let evaporation balance inflow instead of routing out.

## 5b. Meandering rivers (lowland sinuosity, oxbows)

Channel networks above give *where* rivers are; meandering models *how the planform wiggles* over
time. Standard: **Howard–Knutson (1984)** curvature-driven migration.

```python
# River as an ordered polyline of nodes (centerline). Iterate:
def migrate(nodes, dt, E, friction):
    C  = local_curvature(nodes)                       # 1/radius at each node
    # nominal bank-erosion rate peaks at radius/width ≈ 3, lower for sharp & gentle bends
    R  = nominal_rate(C, width)
    # migration = weighted sum of UPSTREAM curvature (lag: shear stress integrates from upstream)
    M  = convolve_upstream(R, weight=lambda s: exp(-friction*s))
    for i,n in enumerate(nodes):
        n.pos += normal(nodes,i) * E * M[i] * dt      # move perpendicular to centerline
    resample(nodes)                                   # keep node spacing uniform
    cutoffs(nodes)
```
Behaviors to implement:
- **Migration ∝ upstream-weighted curvature** — bends grow downstream and amplify (the wiggle propagates).
- **Neck cutoff**: when two non-adjacent centerline points come within `cutoff_dist`, splice the
  channel across the neck → the abandoned loop becomes an **oxbow lake**. Without cutoffs the river
  self-intersects into spaghetti.
- **Sinuosity** = channel length / valley length; natural meandering rivers ~1.3–3. Tune `E`,
  friction, and resample spacing to hit a target sinuosity; validate against the real metric.
- **Point bars / cut banks**: deposit on the inside (convex) bank, erode the outside — drives the
  asymmetric migration and, if you carry it to the heightfield, the floodplain texture.
- Meander **wavelength ≈ 7–11 × channel width** (empirical) — a good sanity check on output.

## Determinism & performance
- Priority-Flood and accumulation are deterministic by construction (no RNG); keep tie-breaking by a
  fixed cell index so float-equal elevations resolve identically across platforms.
- For large DEMs: priority-flood is the bottleneck → use the **two-pass / hash-heap** variants, or
  tile with overlapping borders and stitch spill levels.
- Cache `receivers` + `A` — erosion (`erosion.md`) reuses them every timestep; don't recompute filling
  unless the heightfield changed.

## Pitfalls recap
- Flat-square lake → forgot to spill (route overflow out).
- Parallel straight rivers on a slope → D8 artifact; switch to D∞.
- One fat river, no tributaries → constant accumulation threshold; use area·slopeᶿ.
- Meander spaghetti → no neck-cutoff step.
- Rivers flow uphill / pool mid-slope → routing computed before depression filling.
