# Procedural Textures Reference

## Noise → Texture Patterns Cheatsheet

| Pattern | Recipe |
|---------|--------|
| Marble | `sin(x * freq + fbm(p) * distort)` |
| Wood rings | `frac(sqrt(x²+y²) * freq + fbm(p) * 0.3)` |
| Cracked mud | `1 - Worley(p)`, threshold at 0.15 |
| Clouds | `fbm(p, octaves=8)` remapped to 0..1 |
| Lava | `fbm(p)` + color ramp (black→orange→white) |
| Sand / dunes | `billow(p)` + Sobel edge enhancement |
| Scales / cells | `Worley(p)` — distance to nearest point |
| Rust / wear | `fbm + pow(worley, 2) + edge_mask` |

---

## Tileable Noise

### Making Any Noise Tileable

```glsl
// Sample noise 4 times at corners of a torus, blend with weight.
// Works for any noise function. O(4x) cost.
float tileable_noise(vec2 uv, float size) {
    vec2 uv0 = uv;
    vec2 uv1 = uv - vec2(1.0, 0.0);
    vec2 uv2 = uv - vec2(0.0, 1.0);
    vec2 uv3 = uv - vec2(1.0, 1.0);
    
    float w0 = (1.0-uv.x) * (1.0-uv.y);
    float w1 = uv.x       * (1.0-uv.y);
    float w2 = (1.0-uv.x) * uv.y;
    float w3 = uv.x       * uv.y;
    
    return (w0 * noise(uv0 * size) +
            w1 * noise(uv1 * size) +
            w2 * noise(uv2 * size) +
            w3 * noise(uv3 * size));
}
```

### Histogram-Preserving Blending (Heitz & Neyret 2018)
The best technique for hiding tiling on albedo/normal textures without visible seams.

**Concept:** Instead of blending colors (which washes out contrast), blend in histogram-equalized space.

```glsl
// Simplified implementation:
// 1. Transform texture to Gaussian distribution (precomputed LUT per texture)
// 2. Blend in Gaussian space (blending is now perceptually correct)
// 3. Transform back with inverse LUT

vec3 sample_no_tiling(sampler2D tex, sampler2D Tinv, vec2 uv) {
    // Random offset per cell (blue noise ideal)
    vec2 cell = floor(uv);
    vec2 offset = hash22(cell) * 2.0 - 1.0;
    
    vec2 uv1 = uv + offset;
    vec2 uv2 = uv + offset + vec2(1, 0);  // neighboring cell
    
    // Sample texture in Gaussian space
    vec3 G1 = texture(Tinv, frac(uv1)).rgb;  // Tinv maps [0,1]→Gaussian
    vec3 G2 = texture(Tinv, frac(uv2)).rgb;
    
    // Blend with smooth weight
    float t = smoothstep(0.0, 1.0, frac(uv.x));
    vec3 G  = mix(G1, G2, t);
    
    // Back to [0,1] with forward LUT
    return texture(tex, G).rgb;
}
```

---

## Wang Tiles

Wang tiles eliminate visible repetition by ensuring neighboring tiles always share matching edges.

```python
class WangTileSet:
    """
    Each tile has edge colors on N/E/S/W.
    Select tiles so matching edges match colors.
    """
    def __init__(self, tile_images, edge_colors):
        self.tiles = tile_images
        self.edges = edge_colors   # dict: tile_id → {N,E,S,W: color}
    
    def sample(self, grid_x, grid_y, rng):
        # Get required edge colors from already-placed neighbors
        required = {}
        if grid_x > 0:
            left = self.placed[grid_y][grid_x - 1]
            required["W"] = self.edges[left]["E"]
        if grid_y > 0:
            top = self.placed[grid_y - 1][grid_x]
            required["N"] = self.edges[top]["S"]
        
        # Find tiles matching constraints
        candidates = [t for t in self.tiles
                      if all(self.edges[t][side] == color
                             for side, color in required.items())]
        
        return rng.choice(candidates) if candidates else self.fallback_tile

# Minimum viable Wang tile set: 4 colors = 16 tiles (corner variant).
# Each tile is ~512×512px at 2K resolution workflow.
```

---

## Reaction-Diffusion (Gray-Scott)

Produces organic patterns: spots, stripes, coral, bacteria, zebra.

```python
def gray_scott(width, height, steps=10000, seed=None,
               F=0.055, k=0.062):
    """
    F, k parameters control pattern type:
    F=0.035, k=0.065 → coral/sponge
    F=0.055, k=0.062 → spots
    F=0.025, k=0.060 → labyrinthine
    F=0.014, k=0.054 → worms
    """
    rng = SplitMix64(seed)
    A = np.ones((height, width))
    B = np.zeros((height, width))
    
    # Seed with small B patches
    for _ in range(10):
        cx, cy = rng.randint(0, width), rng.randint(0, height)
        B[cy-3:cy+3, cx-3:cx+3] = 1.0
    
    Du, Dv = 1.0, 0.5   # Diffusion rates
    dt = 1.0
    
    for _ in range(steps):
        # Laplacian (5-point stencil, toroidal boundary)
        lapA = (np.roll(A, 1, 0) + np.roll(A,-1, 0) +
                np.roll(A, 1, 1) + np.roll(A,-1, 1) - 4*A)
        lapB = (np.roll(B, 1, 0) + np.roll(B,-1, 0) +
                np.roll(B, 1, 1) + np.roll(B,-1, 1) - 4*B)
        
        reaction = A * B * B
        A += dt * (Du * lapA - reaction + F * (1 - A))
        B += dt * (Dv * lapB + reaction - (F + k) * B)
    
    return B   # Use B channel as texture (0..1, normalize)
```

---

## Substance-Style Node Graph in Code

Build textures by compositing noise layers with blend modes and masks.

```python
class TextureGraph:
    def fbm_node(self, scale=4, octaves=6, seed=0):
        return fbm(self.uv * scale, octaves, seed=seed)
    
    def voronoi_node(self, scale=4):
        return voronoi_f1(self.uv * scale)
    
    def blend(self, a, b, mode="multiply", opacity=1.0):
        if mode == "multiply": out = a * b
        elif mode == "overlay":
            out = np.where(b < 0.5, 2*a*b, 1 - 2*(1-a)*(1-b))
        elif mode == "screen":  out = 1 - (1-a)*(1-b)
        elif mode == "add":     out = np.clip(a + b, 0, 1)
        return lerp(a, out, opacity)
    
    def levels(self, t, in_lo=0, in_hi=1, gamma=1, out_lo=0, out_hi=1):
        t = (np.clip(t, in_lo, in_hi) - in_lo) / (in_hi - in_lo)
        return out_lo + (t ** (1/gamma)) * (out_hi - out_lo)
    
    def make_rock_albedo(self):
        base  = self.fbm_node(scale=2, octaves=8)
        cracks= 1 - self.voronoi_node(scale=6)
        worn  = self.fbm_node(scale=8, octaves=4, seed=42)
        
        base   = self.levels(base,  in_lo=0.3, in_hi=0.8)
        cracks = self.levels(cracks, in_lo=0.0, in_hi=0.2)
        
        result = self.blend(base, cracks, "multiply", 0.6)
        result = self.blend(result, worn, "overlay", 0.3)
        return color_ramp(result, [(0, (0.25, 0.22, 0.18)),
                                    (0.5, (0.45, 0.41, 0.37)),
                                    (1.0, (0.65, 0.62, 0.58))])
```

---

## Normal Map Generation

```python
def height_to_normal(heightmap, strength=1.0):
    """Convert grayscale heightmap to RGB normal map."""
    h, w = heightmap.shape
    normal = np.zeros((h, w, 3), dtype=np.float32)
    
    # Sobel filter
    dh_dx = np.gradient(heightmap, axis=1) * strength
    dh_dy = np.gradient(heightmap, axis=0) * strength
    
    # Tangent-space normal
    normal[:,:,0] = -dh_dx
    normal[:,:,1] = -dh_dy
    normal[:,:,2] = 1.0
    
    # Normalize
    length = np.sqrt(np.sum(normal**2, axis=2, keepdims=True))
    normal /= (length + 1e-8)
    
    # Pack to [0,1] range for texture storage
    return normal * 0.5 + 0.5
```

---

## Performance Tips

- **GPU-side generation**: Generate noise textures in compute shaders when >512×512.
- **Precompute static textures** (rock, ground, bark) offline; noise only for real-time variation.
- **Mip generation**: Always call GenerateMipMaps() after procedural texture bake.
- **Compression**: BC3/BC7 for albedo+alpha, BC5 for normals, BC4 for grayscale masks.
- **Tile size**: 512×512 minimum for detail; 1024×1024 standard; 2048 for hero assets.
