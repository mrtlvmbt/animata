# Dungeon Generation Reference

## Algorithm Comparison

| Algorithm | Output Feel | Connectivity | Speed | Best For |
|-----------|-------------|-------------|-------|----------|
| BSP | Rectangular, clean | Guaranteed | Fast | Classic roguelikes |
| Random Walk / Drunkard | Organic, cave-like | Guaranteed | Fast | Cave dungeons |
| WFC | Tileset-faithful | Depends on rules | Medium | Handcrafted feel |
| Room + Hallway | Flexible | Guaranteed | Fast | Most games |
| Cellular Automata | Smooth caves | Not guaranteed | Very fast | Cave biomes |
| Prefab Stitching | High quality | Guaranteed | Medium | AAA hand-feel |

---

## BSP Dungeon

```python
class BSPNode:
    def __init__(self, x, y, w, h):
        self.rect = (x, y, w, h)
        self.left = None
        self.right = None
        self.room = None

    def split(self, min_size=6):
        if self.left or self.right:
            return  # Already split
        
        w, h = self.rect[2], self.rect[3]
        horizontal = random.random() < 0.5
        if w > h * 1.25: horizontal = False
        if h > w * 1.25: horizontal = True
        
        max_split = (h if horizontal else w) - min_size
        if max_split <= min_size:
            return  # Too small to split
        
        split_pos = random.randint(min_size, max_split)
        x, y = self.rect[0], self.rect[1]
        
        if horizontal:
            self.left  = BSPNode(x, y,            w, split_pos)
            self.right = BSPNode(x, y+split_pos,  w, h-split_pos)
        else:
            self.left  = BSPNode(x,            y, split_pos, h)
            self.right = BSPNode(x+split_pos,  y, w-split_pos, h)
        
        self.left.split(min_size)
        self.right.split(min_size)

    def create_rooms(self, map_grid, padding=1):
        if self.left or self.right:
            if self.left:  self.left.create_rooms(map_grid, padding)
            if self.right: self.right.create_rooms(map_grid, padding)
            # Connect children
            connect_rooms(self.left.get_room(), self.right.get_room(), map_grid)
        else:
            # Leaf: place a room with random size inside this node
            x, y, w, h = self.rect
            rw = random.randint(w//2, w - padding*2)
            rh = random.randint(h//2, h - padding*2)
            rx = x + random.randint(padding, w - rw - padding)
            ry = y + random.randint(padding, h - rh - padding)
            self.room = (rx, ry, rw, rh)
            carve_rect(map_grid, *self.room)

def connect_rooms(room_a, room_b, grid):
    """L-shaped corridor between room centers."""
    ax, ay = room_center(room_a)
    bx, by = room_center(room_b)
    if random.random() < 0.5:
        carve_hline(grid, ax, bx, ay)
        carve_vline(grid, ay, by, bx)
    else:
        carve_vline(grid, ay, by, ax)
        carve_hline(grid, ax, bx, by)
```

---

## Room + Graph Dungeon (Recommended for Most Games)

```python
def generate_dungeon(width, height, room_count=15, seed=None):
    rng = SplitMix64(seed)
    rooms = []

    # 1. Place rooms (with retry on overlap)
    for _ in range(room_count * 10):
        if len(rooms) >= room_count:
            break
        rw = rng.randint(5, 14)
        rh = rng.randint(4, 10)
        rx = rng.randint(1, width  - rw - 1)
        ry = rng.randint(1, height - rh - 1)
        r  = Rect(rx, ry, rw, rh)
        if not any(r.intersects(o) for o in rooms):
            rooms.append(r)

    # 2. Connect via Delaunay triangulation
    points = [room.center() for room in rooms]
    triangles = delaunay(points)
    
    # 3. Build MST (minimum spanning tree = no dead ends guaranteed)
    edges = deduplicate_edges(triangles)
    mst   = kruskal_mst(edges)
    
    # 4. Add ~15% extra edges for loops (improves gameplay)
    extra = [e for e in edges if e not in mst]
    for e in extra:
        if rng.random() < 0.15:
            mst.append(e)

    # 5. Carve map
    grid = Grid(width, height, WALL)
    for room in rooms:
        carve_rect(grid, room)
    for (a, b) in mst:
        carve_corridor(grid, rooms[a].center(), rooms[b].center())

    return grid, rooms
```

---

## Cellular Automata Caves

```python
def cave_automata(width, height, fill_prob=0.45, iterations=5, seed=None):
    """
    Classic cave generation. Produces smooth, organic cave networks.
    Works best for dedicated cave levels, not mixed dungeon/cave.
    """
    rng = SplitMix64(seed)
    
    # Initialize: random wall/floor
    grid = [[1 if rng.random() < fill_prob else 0 
             for _ in range(width)] for _ in range(height)]
    
    # Border = always wall
    for x in range(width):  grid[0][x] = grid[height-1][x] = 1
    for y in range(height): grid[y][0] = grid[y][width-1]  = 1
    
    # Run cellular automata
    for _ in range(iterations):
        new_grid = [[0]*width for _ in range(height)]
        for y in range(1, height-1):
            for x in range(1, width-1):
                walls = sum(grid[y+dy][x+dx]
                            for dy in [-1,0,1] for dx in [-1,0,1])
                # Birth/survival rule: 4-5 / 5
                new_grid[y][x] = 1 if walls >= 5 else 0
        grid = new_grid
    
    # Flood fill to keep only largest connected region
    return keep_largest_region(grid)

# Tuning guide:
# fill_prob=0.40 → more open, sparse caves
# fill_prob=0.50 → denser, more walls
# iterations=3   → rougher walls
# iterations=8   → very smooth, round caves
```

---

## Special Rooms and Features

### Guaranteed Feature Placement
```python
def place_features(rooms, rng, dungeon_depth=1):
    """
    Place start, boss, treasure, shops with constraints.
    """
    # Sort rooms by distance from entrance (flood fill distance)
    distances = bfs_distances(rooms)
    sorted_rooms = sorted(rooms, key=lambda r: distances[r.id])
    
    n = len(sorted_rooms)
    features = {}
    features["start"]    = sorted_rooms[0]
    features["boss"]     = sorted_rooms[n-1]                  # Farthest room
    features["treasure"] = sorted_rooms[int(n * 0.75)]        # 3/4 way through
    features["shop"]     = sorted_rooms[rng.randint(n//3, n//2)]
    
    # Secret rooms branch off dead ends
    dead_ends = [r for r in rooms if connection_count(r) == 1 
                 and r not in features.values()]
    features["secret"] = rng.choice(dead_ends) if dead_ends else None
    
    return features
```

### Door Placement
```python
def place_doors(grid, corridors, rng, door_prob=0.3):
    """Place doors where corridors meet rooms."""
    for corridor in corridors:
        # Check both ends of corridor
        for end_pos in [corridor.start, corridor.end]:
            if is_room_entrance(grid, end_pos) and rng.random() < door_prob:
                grid[end_pos] = DOOR
```

---

## Multi-Floor / Tower Generation

```python
def generate_floor_plan(depth, world_seed):
    """
    Increase difficulty, complexity with depth.
    Keep consistent seed per floor for save/reload.
    """
    floor_seed = hash_combine(world_seed, depth)
    
    params = FloorParams(
        room_count = 8 + depth * 2,
        room_min_size = max(4, 6 - depth // 5),
        enemy_density = 0.1 + depth * 0.02,
        trap_chance   = min(0.3, depth * 0.03),
        algorithm = "bsp"     if depth < 5  else
                    "organic" if depth < 12 else
                    "wfc"     # Hand-crafted feel for late game
    )
    return generate_dungeon(**params, seed=floor_seed)
```

---

## Connectivity Validation

**Always validate before handing off to gameplay:**
```python
def validate_dungeon(grid, rooms):
    assert len(rooms) >= 2, "Need at least 2 rooms"
    
    # All rooms reachable from start
    reachable = flood_fill(grid, rooms[0].center(), FLOOR)
    for room in rooms:
        assert room.center() in reachable, f"Room {room.id} not reachable"
    
    # No 1×1 isolated floor tiles
    isolated = [p for p in all_floor_tiles(grid) 
                if all_neighbors_are_walls(grid, p)]
    assert not isolated, f"{len(isolated)} isolated floor tiles"
    
    # Boss room connected
    assert rooms[-1].center() in reachable, "Boss room unreachable"
    
    return True
```
