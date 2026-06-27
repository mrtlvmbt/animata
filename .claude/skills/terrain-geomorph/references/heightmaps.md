# Heightmap Synthesis & Topographic Maps

Noise is the *raw material*, not the landscape. This file: building a base heightfield with
character, then turning a heightfield into a readable topographic/contour map.

## Noise choice (terrain context)

| Noise | Use | Watch out |
|-------|-----|-----------|
| Value | never for terrain (blocky) | — |
| Perlin | general, but axis-aligned grid bias | rotate each octave to hide grid |
| **Simplex / OpenSimplex2** | default; isotropic, cheap in 3D+ | patent-free variant = OpenSimplex2 |
| Worley / cellular (F1, F2−F1) | cracks, cells, crater rims, ridge skeletons | O(n) with grid hash; F2−F1 for edges |

Sample at **fractional** coordinates — integer lattice points of Perlin/Simplex are exactly 0.

## fBm and its terrain-shaping variants

```glsl
// Standard fBm — rolling hills, plains. Sum octaves at doubling freq, halving amplitude.
float fbm(vec2 p, int oct){
    float v=0., a=0.5;
    for(int i=0;i<oct;i++){ v += a*snoise(p); p = p*2.0 + vec2(1.7,9.2); a*=0.5; }
    return v;                         // ~[-1,1]
}

// Ridged multifractal — sharp ridgelines, alpine/mountain spines.
// Invert the V-shape of |noise| into ridges; weight higher octaves by lower-octave value.
float ridged(vec2 p, int oct){
    float v=0., a=0.5, w=1.0;
    for(int i=0;i<oct;i++){
        float n = 1.0 - abs(snoise(p));   // peaks where noise crosses 0
        n *= n;                           // sharpen
        n *= w;  w = clamp(n*2.0, 0.0, 1.0); // detail only near existing ridges
        v += a*n; p = p*2.0 + 3.1; a*=0.5;
    }
    return v;                         // [0,1]
}

// Billow — puffy, dune/cloud-like mounds. abs() makes rounded lobes.
float billow(vec2 p,int oct){ float v=0.,a=0.5; for(int i=0;i<oct;i++){ v+=a*abs(snoise(p)); p=p*2.0+1.3; a*=0.5;} return v; }
```

- **lacunarity** (freq mult, default 2.0): >2 = more "gaps"/detail spread; non-integer hides tiling.
- **gain/persistence** (amp mult, default 0.5): >0.5 = rougher, more high-freq energy.
- **octaves**: stop when `frequency * cell_size > 0.5` (Nyquist) — extra octaves are invisible aliasing cost.

### Hybrid / multifractal
Real terrain has **different roughness at different altitudes** (smooth valleys, jagged peaks).
Multiply per-octave amplitude by a function of the accumulated value so highlands get rougher:
`a_i *= clamp(value_so_far, 0, 1)`. This is the core idea behind Musgrave's hybrid-multifractal.

## Domain warping — the single best "organic" upgrade

Warp the *input coordinates* with noise before sampling. Cheap, removes the tell-tale noise look,
creates flow-like, swirled landforms (Iñigo Quílez's technique).

```glsl
float warped(vec2 p){
    vec2 q = vec2(fbm(p,6), fbm(p+vec2(5.2,1.3),6));
    vec2 r = vec2(fbm(p+4.0*q+vec2(1.7,9.2),6), fbm(p+4.0*q+vec2(8.3,2.8),6));
    return fbm(p+4.0*r,6);
}
```
Warp **strength** = how far coordinates move; too high = mushy, smeared. Save `length(q)`/`length(r)`
as extra channels — they make great moisture or rock-strata masks.

## Shaping a raw field into terrain

Order matters; apply as post-passes on the normalized field `h ∈ [0,1]`:

1. **Continental mask** — low-freq fBm thresholded → land/ocean; multiply to push edges to sea level.
2. **Redistribution / power curve** — `h = pow(h, k)`. `k>1` flattens lowlands + sharpens peaks (more plains, dramatic mountains); `k<1` raises basins.
3. **Terracing / plateaus** — quantize with a smooth step so mesas read as strata:
   ```python
   def terrace(h, levels=8, sharp=2.0):
       t = h*levels; f = t - floor(t)
       return (floor(t) + pow(f, sharp)) / levels
   ```
4. **Ridge/valley mixing** — `lerp(fbm, ridged, mountain_mask)` so ridges appear only in uplifted belts (mask from tectonics).
5. **Sea level** — clamp `max(h, sea)` and record a water mask for rivers/biomes.

> Don't terrace *before* erosion — erosion will fight the steps and you get noise. Terrace last, or
> bake it into rock-hardness for erosion to expose naturally.

## Hypsometry — making elevations *mean* something

A **hypsometric tint** maps elevation→color via a curve, not linearly. Real Earth's hypsometric
curve is bimodal (continental shelf + abyssal plain). For believable maps:

- Use a piecewise ramp: deep ocean → shelf → coast → lowland → upland → alpine → snow.
- Calibrate band widths to the **histogram** of your heightmap (equalize so each band has area), or
  the map looks all-one-color. `numpy.percentile(h, [10,25,...])` to place band edges.

## Topographic / contour map output

To render a classic topo map from a heightfield:

```python
# Marching squares: extract iso-lines at every `interval` meters.
def contour_segments(h, interval, cell_size):
    segs = []
    for y in range(H-1):
        for x in range(W-1):
            corners = [h[y,x], h[y,x+1], h[y+1,x+1], h[y+1,x]]
            lo, hi = min(corners), max(corners)
            for level in arange(ceil(lo/interval)*interval, hi, interval):
                # 16-case lookup; interpolate crossing along each edge linearly
                segs += march_square_case(corners, level, x, y, cell_size)
    return segs
```

Map-reading conventions to honor:
- **Index contours** every 5th line drawn thicker + labeled with elevation.
- **Contour spacing encodes slope**: lines close together = steep; far apart = flat. (Falls out for free.)
- **V's point upstream** where contours cross a valley — a good sanity check that your river network
  agrees with the heightfield. If V's point the wrong way, your flow routing is inconsistent.
- Hillshade underlay: `shade = max(0, dot(normal, light_dir))`, normal from central differences
  (`∂h/∂x`,`∂h/∂y`) scaled by `cell_size`. Multiply hypsometric tint × hillshade for relief maps.

## Common mistakes
- Normalizing per-tile instead of globally → seams between chunks (each tile remaps to its own min/max).
- Forgetting `cell_size` in slope/normal math → lighting and erosion scale wrong at different resolutions.
- Too many octaves at high frequency → expensive shimmer, no visible benefit (Nyquist).
- Building mountains purely additively → no consistent drainage; see `tectonics.md` + `erosion.md`.
