# Environment Decoration & Placement Reference

## Foliage and Object Scatter

### Poisson Disk Sampling (Blue Noise Scatter)

```python
def poisson_disk_sampling(width, height, min_dist, seed=None, max_attempts=30):
    """
    Generates uniformly distributed points with guaranteed min_dist separation.
    Ideal for trees, rocks, props. O(n) performance.
    """
    rng = SplitMix64(seed)
    cell_size = min_dist / sqrt(2)
    grid_w = ceil(width  / cell_size)
    grid_h = ceil(height / cell_size)
    grid = {}
    
    def to_cell(p):
        return (int(p[0]/cell_size), int(p[1]/cell_size))
    
    def valid(p):
        cx, cy = to_cell(p)
        for dx in range(-2, 3):
            for dy in range(-2, 3):
                neighbor = grid.get((cx+dx, cy+dy))
                if neighbor and dist(p, neighbor) < min_dist:
                    return False
        return 0 <= p[0] < width and 0 <= p[1] < height
    
    start = (width/2, height/2)
    points = [start]
    active = [start]
    grid[to_cell(start)] = start
    
    while active:
        idx = rng.randint(0, len(active)-1)
        found = False
        for _ in range(max_attempts):
            angle = rng.random() * 2 * pi
            r     = rng.uniform(min_dist, 2 * min_dist)
            p = (active[idx][0] + r*cos(angle),
                 active[idx][1] + r*sin(angle))
            if valid(p):
                points.append(p)
                active.append(p)
                grid[to_cell(p)] = p
                found = True
                break
        if not found:
            active.pop(idx)
    
    return points
```

### Density Map Masking

```python
def masked_scatter(terrain, density_layers, rng):
    """
    Combine multiple masks to control where objects spawn.
    All masks are 0..1 grayscale maps at terrain resolution.
    """
    # Generate candidate points (dense grid)
    candidates = poisson_disk_sampling(W, H, min_dist=3.0, seed=rng.next())
    
    result = []
    for px, py in candidates:
        h     = terrain.height_at(px, py)
        slope = terrain.slope_at(px, py)    # 0=flat, 1=vertical
        normal_y = terrain.normal_at(px, py).y
        
        # Compute combined weight from all density layers
        weight = 1.0
        weight *= density_layers["biome"].sample(px, py)
        weight *= density_layers["altitude"].sample(h)
        weight *= (1.0 - slope)  # No trees on steep slopes
        weight *= (1.0 - density_layers["road"].sample(px, py))  # Clear roads
        weight *= density_layers["noise"].sample(px, py)  # Natural clumping
        
        if rng.random() < weight:
            result.append((px, py, h))
    
    return result

# Typical density layer setup for forest:
# - biome weight: 0 (desert), 0.8 (temperate), 1.0 (rainforest)
# - altitude mask: sigmoid curve, 0 above treeline
# - slope mask: 1 - clamp(slope / MAX_TREE_SLOPE, 0, 1)
# - noise mask: fbm at scale 0.02 → remapped 0..1 for natural clumping
```

---

## GPU Instancing for Vegetation

```hlsl
// Vertex shader for GPU-instanced grass/foliage
// Instance data packed as: position (xyz) + scale + rotation

StructuredBuffer<float4> _InstanceData;  // xy=pos, z=scale, w=rotation

void vert(uint instanceID : SV_InstanceID, ...) {
    float4 data   = _InstanceData[instanceID];
    float3 worldPos = float3(data.x, SampleHeightmap(data.xy), data.y);
    float  scale   = data.z;
    float  rot     = data.w;
    
    // Apply rotation + scale
    float c = cos(rot), s = sin(rot);
    float3x3 rotMat = float3x3(c, 0, s,  0, 1, 0,  -s, 0, c);
    
    // Wind displacement (sine wave on y)
    float windPhase = _Time.y + data.x * 0.5 + data.y * 0.3;
    float windStrength = vertex.y * _WindStrength;
    worldPos.x += sin(windPhase) * windStrength;
    
    o.pos = mul(UNITY_MATRIX_VP, float4(worldPos + mul(rotMat, vertex.xyz * scale), 1));
}
```

**Performance targets:**
- Grass: up to 500k instances @ 60fps (GPU indirect draw)
- Trees: up to 50k instances with LOD
- Rocks/props: up to 100k instances
- Always use frustum + occlusion culling in compute shader before draw

---

## L-Systems for Roads and Plants

### L-System Engine

```python
class LSystem:
    def __init__(self, axiom, rules, angle=25, step=1.0):
        self.axiom = axiom
        self.rules = rules    # dict: symbol → replacement string
        self.angle = angle
        self.step  = step
    
    def iterate(self, n):
        s = self.axiom
        for _ in range(n):
            s = "".join(self.rules.get(c, c) for c in s)
        return s
    
    def draw(self, string):
        """Turtle graphics interpretation."""
        x, y, heading = 0, 0, 90
        stack = []
        lines = []
        
        for c in string:
            if c in "Ff":
                nx = x + self.step * cos(radians(heading))
                ny = y + self.step * sin(radians(heading))
                if c == "F": lines.append(((x,y),(nx,ny)))
                x, y = nx, ny
            elif c == "+": heading += self.angle
            elif c == "-": heading -= self.angle
            elif c == "[": stack.append((x, y, heading))
            elif c == "]": x, y, heading = stack.pop()
        
        return lines

# --- Example systems ---

# Fractal tree
TREE = LSystem(
    axiom="F",
    rules={"F": "F[+F]F[-F][F]"},
    angle=25, step=5
)

# Koch snowflake (island boundaries)
ISLAND = LSystem(
    axiom="F--F--F",
    rules={"F": "F+F--F+F"},
    angle=60, step=2
)

# Road network (stochastic)
ROADS = LSystem(
    axiom="F",
    rules={"F": "F[+F]F", "G": "G[-G]G"},   # Stochastic: pick rule randomly
    angle=90, step=20
)
```

### Road Generation for Cities

```python
def generate_road_network(bounds, seed, road_count=40):
    rng = SplitMix64(seed)
    roads = []
    
    # 1. Major roads: grid-aligned or radial from city center
    center = bounds.center()
    for i in range(4):
        angle = i * 90 + rng.uniform(-15, 15)
        roads.append(Ray(center, angle, length=bounds.size * 0.8))
    
    # 2. Secondary roads: parallel to major roads with offset
    for major_road in list(roads):
        for _ in range(rng.randint(2, 5)):
            offset = rng.uniform(20, 60)
            roads.append(major_road.parallel(offset))
            roads.append(major_road.parallel(-offset))
    
    # 3. Intersect roads, snap endpoints to grid
    intersections = find_all_intersections(roads)
    
    # 4. Build graph
    graph = RoadGraph(roads, intersections)
    
    # 5. Extract city blocks (enclosed polygons)
    blocks = graph.extract_faces()
    
    return graph, blocks
```

---

## City Block Subdivision

```python
def subdivide_block(polygon, min_lot_width=8, min_lot_depth=10, seed=None):
    """
    Recursively subdivide a city block into building lots.
    Uses longest-edge splitting for natural street grid feel.
    """
    rng = SplitMix64(seed)
    lots = []
    queue = [polygon]
    
    while queue:
        poly = queue.pop()
        
        if poly.area() < min_lot_width * min_lot_depth:
            lots.append(poly)
            continue
        
        # Find longest edge → split perpendicular to it
        longest = max(poly.edges(), key=lambda e: e.length())
        split_t = rng.uniform(0.35, 0.65)   # Split near midpoint
        split_line = longest.perpendicular_at(split_t)
        
        a, b = poly.split(split_line)
        if a and b:
            queue.extend([a, b])
        else:
            lots.append(poly)
    
    return lots

def place_building(lot, style, rng):
    """Place building footprint in lot with setback."""
    setback = {
        "commercial": 2,
        "residential": 4,
        "industrial": 6,
    }[style]
    
    footprint = lot.inset(setback)
    height = {
        "commercial": rng.uniform(3, 20),
        "residential": rng.uniform(1, 4) * 3,
        "industrial": rng.uniform(1, 2) * 6,
    }[style]
    
    return Building(footprint, height, style)
```

---

## Resource and Landmark Placement

```python
def place_landmarks(world_map, categories, rng):
    """
    Place unique landmarks with exclusion zones.
    Uses stratified sampling over world regions.
    """
    placed = {cat: [] for cat in categories}
    
    for category, rules in categories.items():
        for region in world_map.regions:
            if not rules.biome_filter(region.biome):
                continue
            
            candidates = poisson_disk_sampling(
                region.bounds,
                min_dist=rules.min_spacing,
                seed=rng.next()
            )
            
            for pos in candidates:
                h = world_map.height_at(pos)
                slope = world_map.slope_at(pos)
                
                # Check placement rules
                if not rules.altitude_range[0] <= h <= rules.altitude_range[1]:
                    continue
                if slope > rules.max_slope:
                    continue
                if any(dist(pos, p) < rules.global_exclusion
                       for cat_list in placed.values()
                       for p in cat_list):
                    continue
                
                placed[category].append(pos)
                if len(placed[category]) >= rules.max_count:
                    break
    
    return placed
```

---

## Vegetation LOD Strategy

```
Distance 0–20m:   Full mesh, full texture, wind simulation, collision
Distance 20–50m:  Reduced mesh (50% tris), compressed texture
Distance 50–100m: Billboard (2D sprite facing camera), 1 drawcall per type
Distance 100–200m: Impostor (pre-rendered from 8 angles), batched
Distance 200m+:   Grass density map only (terrain color tint)
```

**Crossfade at LOD boundaries**: Dither between LOD levels for 2–3 meters to avoid popping.
Use `clip(dither_pattern - lod_fade)` in shader — free on GPU, imperceptible to player.
