# Terrain Generation Reference

## Heightmap Generation

### fBm Variants for Different Terrain Types

```glsl
// --- Standard fBm (rolling hills, plains) ---
float fbm(vec2 p, int octaves) {
    float v = 0.0, a = 0.5;
    for (int i = 0; i < octaves; i++) {
        v += a * snoise(p);
        p  = p * 2.0 + vec2(1.7, 9.2);
        a *= 0.5;
    }
    return v;
}

// --- Ridged multifractal (mountains, ridges) ---
float ridged(vec2 p, int octaves) {
    float v = 0.0, a = 0.5, w = 1.0;
    for (int i = 0; i < octaves; i++) {
        float n = 1.0 - abs(snoise(p));
        n *= n * w;
        v += n * a;
        w  = clamp(n * 2.0, 0.0, 1.0);
        p  = p * lacunarity;
        a *= gain;
    }
    return v;
}

// --- Billow (puffy clouds, sand dunes) ---
float billow(vec2 p, int octaves) {
    float v = 0.0, a = 0.5;
    for (int i = 0; i < octaves; i++) {
        v += a * abs(snoise(p));
        p  = p * 2.0;
        a *= 0.5;
    }
    return v;
}
```

### Terrace Function
```python
def terrace(h, levels=8, sharpness=2.0):
    """Flatten terrain into stepped plateaus."""
    t = floor(h * levels) / levels
    remainder = frac(h * levels)
    # Smooth step between terraces
    blend = smoothstep(0.0, 1.0, remainder) ** (1.0 / sharpness)
    return t + blend / levels
```

### Domain Warping (Iq technique)
```glsl
// Two levels of warping for maximum organic feel
float terrain_warp(vec2 p) {
    vec2 q = vec2(fbm(p + vec2(0.0, 0.0)),
                  fbm(p + vec2(5.2, 1.3)));
    vec2 r = vec2(fbm(p + 4.0*q + vec2(1.7, 9.2)),
                  fbm(p + 4.0*q + vec2(8.3, 2.8)));
    return fbm(p + 4.0 * r);
}
```

---

## Erosion

### Hydraulic Erosion (Particle-Based)

```python
class ErosionParticle:
    """
    Simulate a water droplet sliding downhill, eroding and depositing sediment.
    Run 50,000–200,000 particles per 512×512 heightmap for good results.
    """
    def __init__(self, x, y):
        self.pos = (x, y)
        self.vel = (0, 0)
        self.water = 1.0
        self.sediment = 0.0

    INERTIA       = 0.05   # Higher = smoother paths
    CAPACITY      = 4.0    # Max sediment capacity
    DEPOSITION    = 0.3    # Deposition rate
    EROSION       = 0.3    # Erosion rate
    EVAPORATION   = 0.01
    MIN_SLOPE     = 0.01
    GRAVITY       = 4.0

    def step(self, heightmap):
        gx, gy = gradient(heightmap, self.pos)   # Bilinear gradient
        
        # Update velocity
        vx = self.vel[0] * INERTIA - gx * (1 - INERTIA)
        vy = self.vel[1] * INERTIA - gy * (1 - INERTIA)
        speed = sqrt(vx*vx + vy*vy)
        if speed < 1e-6: return False  # Particle settled
        
        vx, vy = vx/speed, vy/speed
        
        # Move
        nx, ny = self.pos[0] + vx, self.pos[1] + vy
        dh = sample(heightmap, nx, ny) - sample(heightmap, *self.pos)
        
        # Capacity
        capacity = max(-dh, MIN_SLOPE) * speed * self.water * CAPACITY
        
        if self.sediment > capacity or dh > 0:
            # Deposit
            deposit = (dh > 0) and (dh) or ((self.sediment - capacity) * DEPOSITION)
            self.sediment -= deposit
            heightmap_add(self.pos, deposit)
        else:
            # Erode
            erode = min((capacity - self.sediment) * EROSION, -dh)
            self.sediment += erode
            heightmap_add_brush(self.pos, -erode)  # Soften with 3×3 brush
        
        self.vel = (vx, vy)
        self.pos = (nx, ny)
        self.water *= (1 - EVAPORATION)
        return self.water > 0.01
```

### Thermal Erosion (Fast Slope Smoothing)
```python
def thermal_erosion(heightmap, iterations=50, talus_angle=0.5):
    """
    Flattens slopes steeper than talus_angle (material sliding threshold).
    Run AFTER hydraulic erosion. Fast neighbor comparison approach.
    """
    for _ in range(iterations):
        for y in range(1, H-1):
            for x in range(1, W-1):
                h = heightmap[y][x]
                for dx, dy in [(-1,0),(1,0),(0,-1),(0,1)]:
                    diff = h - heightmap[y+dy][x+dx]
                    if diff > talus_angle:
                        move = diff * 0.5
                        heightmap[y][x]      -= move
                        heightmap[y+dy][x+dx] += move
```

---

## Biome System

### Whittaker Biome Lookup
```python
BIOMES = {
    # (temp_band, moisture_band): biome
    (0, 0): "tundra",
    (0, 1): "tundra",
    (1, 0): "grassland",
    (1, 1): "shrubland",
    (1, 2): "woodland",
    (2, 0): "desert",
    (2, 1): "savanna",
    (2, 2): "tropical_seasonal_forest",
    (2, 3): "tropical_rainforest",
    (3, 0): "desert",
    (3, 1): "savanna",
    (3, 2): "tropical_seasonal_forest",
    (3, 3): "tropical_rainforest",
}

def assign_biome(height, temperature_noise, moisture_noise):
    # Altitude modifies temperature
    temp = temperature_noise - height * 0.5
    moist = moisture_noise
    
    temp_band  = clamp(int(temp * 4), 0, 3)
    moist_band = clamp(int(moist * 4), 0, 3)
    return BIOMES[(temp_band, moist_band)]

# For smooth transitions, store biome WEIGHTS per cell, not a hard enum.
# Blend 2–3 dominant biomes by weight when sampling surface rules.
```

---

## River Generation

### Gradient Descent Flowmap
```python
def trace_river(heightmap, start_x, start_y, min_length=50):
    """
    Follow steepest descent from a high-altitude point.
    Carve the path into the heightmap.
    """
    path = [(start_x, start_y)]
    x, y = start_x, start_y
    
    for _ in range(10000):
        # Find steepest downhill neighbor (8-connected)
        best_h = heightmap[y][x]
        bx, by = x, y
        for dx in [-1, 0, 1]:
            for dy in [-1, 0, 1]:
                nx, ny = x+dx, y+dy
                if 0 <= nx < W and 0 <= ny < H:
                    if heightmap[ny][nx] < best_h:
                        best_h = heightmap[ny][nx]
                        bx, by = nx, ny
        
        if bx == x and by == y:
            break  # Reached local minimum (lake or ocean)
        
        x, y = bx, by
        path.append((x, y))
    
    if len(path) < min_length:
        return None  # Discard short rivers
    
    # Carve river bed with increasing width downstream
    for i, (rx, ry) in enumerate(path):
        t = i / len(path)          # 0 at source, 1 at mouth
        width = int(1 + t * 4)     # 1px at source, 5px at mouth
        depth = 0.02 + t * 0.08
        carve_circle(heightmap, rx, ry, width, -depth)
    
    return path
```

---

## LOD Terrain Mesh

| Technique | Description | Use When |
|-----------|-------------|----------|
| Geomipmapping | Uniform grid, reduce poly count per chunk | Simple, low memory |
| CDLOD | Continuous quad-tree LOD, no cracks | Best general purpose |
| Geometry Clipmaps | Concentric rings around camera | Large open worlds, GPU-driven |
| Nanite-style | Virtual geometry, cluster LOD | Bleeding edge, high triangle budget |

```glsl
// Normal from heightmap (fast, in shader)
vec3 heightmap_normal(sampler2D hmap, vec2 uv, float texel, float scale) {
    float l = texture(hmap, uv + vec2(-texel, 0)).r;
    float r = texture(hmap, uv + vec2( texel, 0)).r;
    float b = texture(hmap, uv + vec2(0, -texel)).r;
    float t = texture(hmap, uv + vec2(0,  texel)).r;
    return normalize(vec3(l - r, 2.0 / scale, b - t));
}
```
