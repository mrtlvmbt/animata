# Erosion

Three families. Use them together: **fluvial/landscape** for large-scale belt shape, **hydraulic**
for visual channel detail, **thermal** to keep slopes physical. All are iterative → must be
deterministic.

---

## A. Thermal erosion (mass wasting) — cheapest, run first/alongside

Any slope steeper than the **talus angle** sheds material downhill until stable. Smooths cliffs,
creates scree slopes, prevents unrealistically vertical walls.

```python
def thermal_step(h, talus, amount=0.5, cell=1.0):
    new = h.copy()
    for y,x in cells:
        dmax, total, lowers = 0, 0, []
        for nx,ny in neighbors8(x,y):
            d = (h[y,x]-h[ny,nx]) / cell           # slope, world units
            if d > talus:
                lowers.append((nx,ny,d)); total+=d; dmax=max(dmax,d)
        if not lowers: continue
        move = amount*(dmax-talus)*cell            # excess over talus
        for nx,ny,d in lowers:
            new[ny,nx] += move*(d/total)           # distribute by relative slope
        new[y,x] -= move
    return new
```
- `talus` in **slope units** (Δh/cell), not raw height — otherwise it terraces at higher resolutions.
- Use the *excess* over talus (`dmax-talus`), not the full slope, or it over-flattens.
- Different talus per material (sand low, bedrock high) → layered cliffs.

---

## B. Hydraulic erosion — two implementations

### B1. Droplet / particle (Beyer · Lague)
Simulate rain one droplet at a time; trace downhill, carry sediment by **capacity**, deposit when
slow/over-capacity. Great detail, easy to tune, naturally parallel over droplets.

```python
def simulate_droplet(h, rng, p):
    pos = rng.point_in_map(); vel=0; water=1.0; sediment=0
    dir = vec2(0,0)
    for _ in range(p.max_lifetime):
        cell = floor(pos); g = bilinear_gradient(h, pos)      # interpolated height + slope
        dir = dir*p.inertia - g*(1-p.inertia)                 # blend momentum + downhill
        if length(dir)==0: break
        dir = normalize(dir); new = pos + dir
        dh = height(h,new) - height(h,pos)                    # >0 means uphill (went into a pit)
        capacity = max(-dh, p.min_slope) * vel * water * p.capacity
        if sediment > capacity or dh > 0:                     # deposit
            drop = (sediment-capacity)*p.deposit if dh<=0 else min(dh, sediment)
            deposit_bilinear(h, pos, drop); sediment -= drop
        else:                                                 # erode (capped, spread over radius)
            grab = min((capacity-sediment)*p.erode, -dh)
            erode_brush(h, pos, grab, p.radius); sediment += grab
        vel = sqrt(max(0, vel*vel + dh*p.gravity))            # speed up going down
        water *= (1-p.evaporate)
        pos = new
        if out_of_bounds(pos): break
```
Key knobs: `inertia` (0=pure gravity, 1=straight; ~0.05–0.3), `capacity`, `erode`/`deposit` rates,
`evaporate`, erosion **brush radius** (erode over a disc → avoids 1-pixel canyons & noise).
Typical budget: **~hundreds of thousands of droplets** for a 512² map. Spawn positions seeded.

### B2. Grid / pipe-model + shallow water (Mei et al. 2007) — GPU-native
Every cell holds: terrain `b`, water `d`, suspended sediment `s`, outflow flux `f` to 4 neighbors,
velocity `v`. Water moves by **hydrostatic pressure difference** through virtual "pipes". Fully
parallel — one thread per cell, no per-droplet serialization.

```
Per step (all cells in parallel):
 1. Add water:   d += rain·Δt   (or source map)
 2. Flux update: for each of 4 neighbors,
        f_dir = max(0, f_dir + Δt·A·g·(Δh)/l )      # Δh = (b+d) height difference
    scale all outflux so it can't exceed water present (CFL/volume safety):
        K = min(1, d·l²/((Σf)·Δt));  f *= K
 3. Water+velocity: Δd = Δt·(Σf_in − Σf_out)/l²;  d += Δd
        v from net horizontal flux / average depth
 4. Erosion–deposition (capacity C = Kc·sin(tilt)·|v|):
        if C > s:  b -= Ks·(C−s);  s += Ks·(C−s)     # pick up
        else:      b += Kd·(s−C);  s -= Kd·(s−C)     # drop
 5. Sediment transport: advect s along v (semi-Lagrangian backtrace)
 6. Evaporate: d *= (1 − Ke·Δt)
```
Stability: step 2's scaling factor `K` is the **CFL** guard — without it large `Δt` causes negative
water / NaNs ("explodes"). Tilt angle `sin(tilt)` should have a floor so flat areas still transport
a little (else sediment locks up). This is the model behind most real-time erosion compute shaders.

**Droplet vs pipe:** droplet = simpler, lovely detail, CPU/GPU; pipe = true fluid, supports standing
water/lakes/rain maps and runs the whole map at once on GPU. Pick pipe if you also want water rendered.

---

## C. Fluvial erosion / Landscape Evolution Model (LEM) — the realism unlock

The geomorphology-correct model. Couples uplift and erosion over geologic time; produces concave
river profiles, knickpoints, dendritic valleys, and *correctly spaced mountain belts*.

**Stream-power law (detachment-limited):**
```
∂h/∂t = U(x) − K · A(x)^m · S(x)^n
        └uplift┘   └── fluvial incision ──┘
```
- `A` = upstream **drainage area** (from flow accumulation, see `rivers.md`) — the proxy for discharge.
- `S` = local slope toward the receiver (downstream) cell.
- `K` = erodibility (rock hardness, climate). `m≈0.5`, `n≈1` typical; `m/n≈0.5` sets profile concavity.
- Add hillslope diffusion `+ D·∇²h` for the smooth ridge-to-channel transition.

**Implicit O(n) solver (Braun & Willett 2013) — the standard, unconditionally stable:**
```
1. Priority-flood fill depressions; compute D8 receivers + drainage area A.   # rivers.md
2. Order nodes in a stack so every node comes after its receiver (topological / "Fastscape" ordering).
3. Apply uplift:  h += U·Δt.
4. Sweep nodes downstream→upstream; for each, solve the implicit incision update
   along its single receiver — a scalar Newton/closed-form step (n=1 is closed-form).
```
This solves the whole map in one linear pass per timestep and stays stable at huge `Δt` (10⁴–10⁶ yr
steps) where explicit schemes blow up. *Badlands* and *Fastscape* are the reference implementations.

### Detachment- vs transport-limited
- **Detachment-limited** (above): erosion set by bedrock resistance. Bedrock canyons, steep ranges.
- **Transport-limited**: erosion set by the flow's *capacity* to carry already-loosened sediment.
  Add a deposition term (`+ G/A · Σ upstream flux`) → alluvial fans, depositional plains, fills basins.
  Yuan/Davy 2019 and Fastscape's `SPL+deposition` handle this efficiently.

---

## Sequencing & tuning (practical recipe)
```
uplift field U  (tectonics.md)
  └─> N iterations of LEM (stream-power + diffusion)     # large-scale belts + valleys
        └─> droplet OR pipe hydraulic erosion pass        # fine channel detail / alluvium
              └─> thermal pass to settle over-steep cliffs # physical slopes
```
- Tune at **target resolution**; erosion constants are resolution-sensitive (slope = Δh/cell).
- Save intermediate `A`, sediment, and water maps — they're free **moisture/biome** and rock-strata inputs.
- Determinism: fixed iteration count, seeded droplet spawns, no early-exit on a global float threshold
  that could differ across platforms.

## Common artifacts
| Symptom | Cause | Fix |
|---------|-------|-----|
| Flat-bottomed lakes draining nowhere | depressions filled but never spilled | fill-then-route overflow (`rivers.md`) |
| Pipe model NaNs / negative water | Δt too big, no flux scaling | apply the CFL `K` scale in step 2; lower Δt |
| 1-pixel knife canyons (droplet) | erosion not spread over radius | use an erosion **brush** disc |
| Terraced/stair erosion | thermal talus too coarse for resolution | talus in slope units; smaller, more iters |
| Mountains erode to uniform mush | no uplift forcing, only removal | run as LEM with persistent `U` |
| Rivers don't deepen, sediment locks | transport with zero floor on flat tilt | floor `sin(tilt)`; add min transport |
