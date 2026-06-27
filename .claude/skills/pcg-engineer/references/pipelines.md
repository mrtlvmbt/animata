# PCG Pipelines, Streaming & Performance Reference

## Chunk-Based Infinite World Architecture

```
World
├── ChunkManager
│   ├── load_radius:   int    (chunks around player to keep loaded)
│   ├── gen_radius:    int    (chunks to pre-generate ahead of player)
│   └── cache:         LRU    (evict distant chunks from memory)
├── ChunkGenerator  (seeded, deterministic)
│   ├── HeightmapStage
│   ├── BiomeStage
│   ├── DecoratorStage
│   └── NavmeshStage
└── ChunkStreamer   (async, background threads)
```

### Chunk Coordinate System

```python
# Always use chunk coordinates (cx, cy) internally.
# World position: wx = cx * CHUNK_SIZE + local_x

CHUNK_SIZE = 64    # tiles or meters; power of 2

def world_to_chunk(wx, wy):
    return floor(wx / CHUNK_SIZE), floor(wy / CHUNK_SIZE)

def chunk_to_world(cx, cy):
    return cx * CHUNK_SIZE, cy * CHUNK_SIZE

# Chunk seed: derive from world seed + chunk coords
# MUST be position-independent order to stay deterministic
def chunk_seed(world_seed, cx, cy):
    return splitmix64(world_seed ^ (cx * 0x9e3779b9) ^ (cy * 0x6c62272e))
```

### Async Generation Pipeline

```python
import asyncio
from concurrent.futures import ThreadPoolExecutor

class ChunkManager:
    def __init__(self, world_seed, load_radius=3, gen_radius=5):
        self.world_seed   = world_seed
        self.load_radius  = load_radius
        self.gen_radius   = gen_radius
        self.chunks       = {}       # (cx, cy) → Chunk
        self.generating   = set()    # chunks in-flight
        self.executor     = ThreadPoolExecutor(max_workers=4)
    
    async def update(self, player_cx, player_cy):
        # Determine which chunks we want
        desired = {(cx, cy)
                   for cx in range(player_cx - self.gen_radius,
                                   player_cx + self.gen_radius + 1)
                   for cy in range(player_cy - self.gen_radius,
                                   player_cy + self.gen_radius + 1)}
        
        # Evict far chunks
        to_evict = [k for k in self.chunks
                    if chebyshev(k, (player_cx, player_cy)) > self.load_radius + 2]
        for k in to_evict:
            self.chunks[k].save()
            del self.chunks[k]
        
        # Kick off generation for missing chunks
        for coord in desired:
            if coord not in self.chunks and coord not in self.generating:
                self.generating.add(coord)
                asyncio.create_task(self._generate_chunk(*coord))
    
    async def _generate_chunk(self, cx, cy):
        loop = asyncio.get_event_loop()
        chunk = await loop.run_in_executor(
            self.executor,
            generate_chunk_sync,    # Pure function, no shared state
            cx, cy, self.world_seed
        )
        self.chunks[(cx, cy)] = chunk
        self.generating.discard((cx, cy))
```

---

## Deterministic RNG — SplitMix64

```python
class SplitMix64:
    """
    Fast, high-quality 64-bit RNG. Splittable = safe for parallel chunk gen.
    Passes BigCrush. Use this instead of system random for ALL PCG.
    """
    def __init__(self, seed: int):
        self.state = seed & 0xFFFFFFFFFFFFFFFF
    
    def next_u64(self) -> int:
        self.state = (self.state + 0x9e3779b97f4a7c15) & 0xFFFFFFFFFFFFFFFF
        z = self.state
        z = ((z ^ (z >> 30)) * 0xbf58476d1ce4e5b9) & 0xFFFFFFFFFFFFFFFF
        z = ((z ^ (z >> 27)) * 0x94d049bb133111eb) & 0xFFFFFFFFFFFFFFFF
        return z ^ (z >> 31)
    
    def random(self) -> float:
        return self.next_u64() / (2**64)
    
    def randint(self, lo, hi) -> int:
        return lo + self.next_u64() % (hi - lo + 1)
    
    def uniform(self, lo, hi) -> float:
        return lo + self.random() * (hi - lo)
    
    def choice(self, seq):
        return seq[self.randint(0, len(seq)-1)]
    
    def split(self) -> "SplitMix64":
        """Create independent child RNG from current state. Safe for parallelism."""
        child = SplitMix64(self.next_u64())
        return child
    
    def shuffle(self, lst):
        for i in range(len(lst)-1, 0, -1):
            j = self.randint(0, i)
            lst[i], lst[j] = lst[j], lst[i]
```

---

## Generation Stage Pipeline

```python
class ChunkGenerator:
    """
    Pipeline-based generator. Each stage reads previous stages' output.
    Order matters: terrain → biome → features → decoration → nav.
    """
    
    STAGES = [
        HeightmapStage,    # fBm terrain, erosion
        BiomeStage,        # Temperature/moisture → biome weights
        WaterStage,        # Lakes, rivers, coastlines
        FeatureStage,      # Dungeons, caves, ruins, points of interest
        VegetationStage,   # Tree/grass scatter with biome weights
        PropStage,         # Rocks, stumps, details
        NavmeshStage,      # Walkable area extraction
    ]
    
    def generate(self, cx, cy, world_seed) -> Chunk:
        rng   = SplitMix64(chunk_seed(world_seed, cx, cy))
        chunk = Chunk(cx, cy)
        
        for StageClass in self.STAGES:
            stage = StageClass()
            # Each stage gets its own split RNG → independent from others
            stage_rng = rng.split()
            stage.run(chunk, stage_rng, self.world_config)
        
        return chunk
```

---

## Performance Optimization

### Noise Performance
```
Function           | Relative Cost | Notes
Simplex 2D         | 1×            | Baseline
Simplex 3D         | 1.5×          | Use for animated/volumetric
Perlin 2D          | 1.2×          | Slightly slower than simplex
Worley (cellular)  | 3–5×          | Expensive; cache if possible
fBm (8 octaves)    | 8×            | Each octave = full noise sample
Domain warp        | 3×            | Warp + sample

Optimization tips:
- Reduce octave count at distance (LOD noise)
- Pre-generate heightmap texture in compute shader → sample in vertex shader
- Cache noise in 2D texture for large static terrain
- SIMD noise (FastNoiseSIMD / FastNoise2): 4–8× speedup on CPU
```

### Profiling Checklist
```
[ ] Profile chunk generation time — target <16ms per chunk (one frame budget)
[ ] Identify bottleneck stage (noise? erosion? placement? serialization?)
[ ] Check allocation patterns — GC pauses kill async pipelines
[ ] Measure cache hit rate for chunk LRU
[ ] Count RNG calls — excessive calls indicate poor algorithm choice
[ ] GPU memory for instance buffers — target <256MB total foliage
```

### Spatial Query Optimization
```python
# For point-in-region, nearest-neighbor queries at runtime:
# - KD-tree for static points (build once, query many)
# - Grid hash for dynamic objects (update O(1), query O(1) amortized)
# - BVH for polygon intersection

class SpatialGrid:
    def __init__(self, cell_size):
        self.cell_size = cell_size
        self.cells = defaultdict(list)
    
    def insert(self, obj, x, y):
        self.cells[self._cell(x, y)].append(obj)
    
    def query_radius(self, x, y, radius):
        r = ceil(radius / self.cell_size)
        cx, cy = self._cell(x, y)
        results = []
        for dx in range(-r, r+1):
            for dy in range(-r, r+1):
                results.extend(self.cells[(cx+dx, cy+dy)])
        return [o for o in results if dist(o.pos, (x,y)) <= radius]
    
    def _cell(self, x, y):
        return (int(x // self.cell_size), int(y // self.cell_size))
```

---

## Save / Load Serialization

```python
# IMPORTANT: Never serialize the generated content — only serialize the SEED.
# Regenerating from seed is faster than loading from disk for terrain.
# Only serialize PLAYER MODIFICATIONS (dug tunnels, placed buildings).

class WorldSave:
    world_seed: int          # The master seed — everything else derives from this
    player_state: dict       # Position, inventory, etc.
    chunk_modifications: dict  # (cx, cy) → diff vs. generated state
    
    # DO NOT store:
    # - Generated heightmaps (regen from seed)
    # - Vegetation positions (regen from seed)
    # - Enemy spawns (regen from seed + game state)
```

---

## Engine Integration Notes

### Unity
- Use `Unity.Mathematics` noise (burst-compiled) for runtime gen
- `ComputeShader` for heightmap generation (>256×256)
- `Graphics.DrawMeshInstancedIndirect` for GPU foliage
- `JobSystem` + `NativeArray` for parallel chunk jobs — no managed allocations

### Unreal Engine
- `UProceduralMeshComponent` for dynamic mesh
- `PCGGraph` (built-in PCG framework, UE5.2+) for scatter pipelines
- `FNoiseGenerator` from plugin or custom `UWorldPartitionStreamingPolicy` subclass
- Nanite-compatible procedural meshes: output static LODs, not truly dynamic

### Godot
- `Image` + `ImageTexture` for heightmaps
- `MultiMeshInstance3D` for instanced vegetation
- GDExtension (C++) for noise-heavy generation; GDScript too slow for >128×128
- `WorkerThreadPool` for async chunk generation
