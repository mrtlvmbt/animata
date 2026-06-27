---
name: bevy-ecs
description: >
  Bevy 0.15 ECS reference. Use for ANY Bevy API question: plugins, systems, components, resources,
  queries, commands, events, states, schedules, system ordering, run conditions, exclusive systems,
  Local<T>.
triggers:
  - "add a system"
  - "empty query"
  - "read a resource"
  - "send/receive events"
  - "gate system on AppState"
  - "system ordering"
  - "Commands vs World"
  - "init_resource vs insert_resource"
  - "Startup vs OnEnter"
  - "Bevy API"
---

# Bevy 0.15 ECS Reference

Bevy 0.15 with Rust. Engine: `bevy = "0.15"`, UI (optional): `bevy_egui = "0.31"`.

---

## Plugin System

```rust
pub struct MyPlugin;

impl Plugin for MyPlugin {
    fn build(&self, app: &mut App) {
        app
            .insert_resource(MyResource::default())
            .init_resource::<MyOtherResource>()  // requires Default
            .add_event::<MyEvent>()
            .init_state::<MyState>()
            .add_systems(Startup, setup_system)
            .add_systems(Update, (system_a, system_b))
            .add_systems(FixedUpdate, physics_step)
            .add_systems(PostUpdate, sync_transforms.after(physics_step))
            .add_systems(OnEnter(MyState::Running), spawn_scene);
    }
}
```

**Rule:** `init_state::<S>()` must come **after** `DefaultPlugins` — StateTransition schedule
is registered by DefaultPlugins. In `main.rs`: `.add_plugins(DefaultPlugins...).init_state::<AppState>()`.

---

## Schedules

| Schedule | When | Use for |
|---|---|---|
| `Startup` | Once at launch | spawn initial entities, init resources |
| `OnEnter(S)` | Once when entering state S | spawn scene after loading |
| `OnExit(S)` | Once when leaving state S | cleanup |
| `FixedUpdate` | Fixed timestep (physics) | gravity, attitude, integration |
| `Update` | Every frame | input, animation, UI |
| `PostUpdate` | After Update | sync transforms, render prep |
| `Last` | End of frame | cleanup, debug |

**Example schedule overview:**
```
Startup:       start_heightmap_load, spawn_space_grid
Loading state: poll_heightmap_task (Update)
OnEnter(Running): spawn_minimal_scene, initial_camera_orient
FixedUpdate:   gravity_step, attitude_step, landing_system
Update:        spin_bodies, warp_input, camera systems, LOD
PostUpdate:    sync_transforms, sync_ring_transforms, scroll_grid
```

---

## Components

```rust
#[derive(Component, Clone, Copy, Debug)]
pub struct Position(pub DVec3);   // custom newtype

#[derive(Component, Default)]
pub struct Velocity(pub DVec3);

// Marker component (zero-size):
#[derive(Component)]
pub struct Body;

// Bundle — spawn multiple at once:
#[derive(Bundle)]
pub struct PhysicsBundle {
    pub pos: Position,
    pub vel: Velocity,
    pub mass: Mass,
}
```

---

## Queries

```rust
// Basic: iterate all entities with these components
fn system(query: Query<(&Position, &Velocity)>) {
    for (pos, vel) in &query { ... }
}

// Mutable + filter
fn physics(mut query: Query<(&mut Position, &Velocity), With<Body>>) {
    for (mut pos, vel) in &mut query { ... }
}

// Exclude component
fn sync(query: Query<&Position, Without<InBodyFrame>>) { ... }

// Multiple filters
fn update(query: Query<&Transform, (Changed<Position>, With<Body>)>) { ... }

// Fetch by entity (fallback to .get())
fn find(query: Query<&Velocity>, entity: Entity) {
    if let Ok(vel) = query.get(entity) { ... }
}

// Entity + components
fn with_entity(query: Query<(Entity, &Position)>) {
    for (entity, pos) in &query { ... }
}

// Optional component
fn mixed(query: Query<(&Position, Option<&Atmosphere>)>) {
    for (pos, atmo) in &query {
        if let Some(a) = atmo { ... }
    }
}
```

**Multiple queries in one system** — allowed as long as they don't alias mutably:
```rust
fn two_queries(
    bodies: Query<&Position, With<Body>>,
    craft: Query<&mut Position, With<Spacecraft>>,   // OK: different archetypes
) { ... }
```

---

## Resources

```rust
#[derive(Resource, Default)]
pub struct SimConfig {
    pub sub_steps: u32,
    pub sim_elapsed: f64,
}

// Insert at startup (takes ownership, overwrites if exists):
app.insert_resource(SimConfig { sub_steps: 8, sim_elapsed: 0.0 });

// Insert with Default:
app.init_resource::<SimConfig>();

// In system:
fn tick(mut config: ResMut<SimConfig>, time: Res<Time<Fixed>>) {
    config.sim_elapsed += time.delta_secs_f64() * config.time_scale;
}

// Optional resource (won't panic if not inserted):
fn maybe(res: Option<Res<HeightmapRes>>) {
    let Some(hm) = res else { return };
}
```

---

## Commands

```rust
fn spawn_body(mut commands: Commands) {
    let id = commands.spawn((
        Position(DVec3::ZERO),
        Velocity(DVec3::ZERO),
        Mass(1e24),
        Body,
    )).id();

    // Add/remove components:
    commands.entity(id).insert(Atmosphere::default());
    commands.entity(id).remove::<Atmosphere>();

    // Despawn (immediate + children):
    commands.entity(id).despawn_recursive();

    // Resources:
    commands.insert_resource(MyResource::new());
    commands.remove_resource::<MyResource>();
}
```

**Commands are deferred** — applied at end of schedule stage, not immediately.
For immediate mutation use `World` (exclusive system).

---

## Events

```rust
#[derive(Event)]
pub struct LandingEvent {
    pub craft: Entity,
    pub body: Entity,
    pub surface_pos: DVec3,
}

// Register:
app.add_event::<LandingEvent>();

// Send:
fn detect_landing(mut writer: EventWriter<LandingEvent>) {
    writer.send(LandingEvent { craft, body, surface_pos });
}

// Read (clears after each frame):
fn handle_landing(mut reader: EventReader<LandingEvent>) {
    for ev in reader.read() {
        // handle ev
    }
}
```

**Events are double-buffered** — survive for 2 frames. Reading in same frame as send = OK.

---

## States

```rust
#[derive(States, Clone, PartialEq, Eq, Hash, Debug, Default)]
pub enum AppState {
    #[default]
    Loading,
    Running,
}

// Register (must be after DefaultPlugins):
app.init_state::<AppState>();

// Gate system on state:
app.add_systems(Update, poll_heightmap.run_if(in_state(AppState::Loading)));
app.add_systems(OnEnter(AppState::Running), spawn_minimal_scene);
app.add_systems(Update, game_systems.run_if(in_state(AppState::Running)));

// Transition:
fn poll(mut next: ResMut<NextState<AppState>>) {
    next.set(AppState::Running);
}
```

---

## System Ordering

```rust
// After/before:
app.add_systems(PostUpdate, sync_transforms);
app.add_systems(PostUpdate, sync_ring_transforms.after(sync_transforms));

// Chain (ordered tuple, left-to-right):
app.add_systems(Update, (update_orbit_cache, draw_orbit_gizmos).chain());

// SystemSet (label multiple systems together):
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum PhysicsSet { Integrate, Collide, PostPhysics }

app.configure_sets(FixedUpdate, (
    PhysicsSet::Integrate,
    PhysicsSet::Collide.after(PhysicsSet::Integrate),
    PhysicsSet::PostPhysics.after(PhysicsSet::Collide),
));
```

---

## Advanced Query Patterns

```rust
// ParamSet: two queries that overlap in component access
fn system(mut params: ParamSet<(
    Query<&mut Transform, With<Player>>,
    Query<&Transform, Without<Player>>,
)>) {
    let player_pos = params.p0().single().translation;
    for t in params.p1().iter() { ... }
}

// Or filter: match entities with either component
Query<Entity, Or<(With<ComponentA>, With<ComponentB>)>>

// Optional components in query
Query<(&Transform, Option<&Velocity>)>
// opt_vel is Option<&Velocity>
for (transform, opt_vel) in query.iter() { ... }

// Change detection: only process entities where component changed this frame
Query<&Position, Changed<Position>>
// Added: only entities that got this component this frame
Query<&Health, Added<Health>>
```

---

## Performance Patterns

```rust
// Parallel query iteration (read-only or single-component write)
query.par_iter().for_each(|transform| {
    // runs on Bevy's thread pool, no &mut World access
    process(transform);
});
// par_iter_mut for mutable:
query.par_iter_mut().for_each_mut(|mut vel| {
    vel.apply_drag(dt);
});

// SparseSet storage: faster add/remove, slower iteration
// Use for components that are frequently added/removed (e.g. temporary states)
#[derive(Component)]
#[component(storage = "SparseSet")]
struct Highlighted;

// spawn_batch: batch-spawn entities efficiently
commands.spawn_batch((0..1000).map(|i| {
    (Position { x: i as f32, y: 0.0 }, Velocity::default())
}));
// Note: spawn_batch requires all entities to have the same component bundle type
```

---

## Run Conditions

```rust
// Built-in:
.run_if(in_state(AppState::Running))
.run_if(any_with_component::<QuadtreeLod>())
.run_if(resource_exists::<HeightmapRes>())
.run_if(resource_changed::<SimConfig>())

// Custom:
fn high_warp(config: Res<SimConfig>) -> bool { config.warp_level > 4 }
app.add_systems(FixedUpdate, kepler_step.run_if(high_warp));
```

---

## Local<T> — Per-System State

```rust
fn physics_step(
    mut scratch: Local<Vec<DVec3>>,   // allocated once, persists across frames
    query: Query<&Position>,
) {
    scratch.clear();  // reuse allocation
    for pos in &query { scratch.push(pos.0); }
}
```

Use `Local<T>` for scratch buffers to avoid heap allocation per frame.
This project: `gravity_step` uses `Local<Vec<(DVec3, f64)>>` for scratch.

---

## Exclusive Systems (World access)

```rust
fn startup(world: &mut World) {
    // Direct world access — no query needed
    let config = world.resource::<LodGlobalConfig>();
    let levels = config.heightmap_levels.clone();
    // Spawn async task:
    let task = AsyncComputeTaskPool::get().spawn(async move { ... });
    world.insert_resource(HeightmapLoadTask(task));
}

// Register as exclusive:
app.add_systems(Startup, startup);  // &mut World systems auto-detected
```

---

## Common Bevy 0.15 Pitfalls

| Problem | Cause | Fix |
|---|---|---|
| Panic: `StateTransition not found` | `init_state` before DefaultPlugins | Move `init_state` after `add_plugins(DefaultPlugins...)` |
| Startup system never runs | AppState gating wrong | Use `OnEnter(State)` not `.run_if` for one-shots |
| Query returns empty | Wrong component filter | Check With/Without, verify component is actually on entity |
| `try_ctx_mut()` panic | EguiContexts used before UI init | Use `try_ctx_mut()` instead of `ctx_mut()` |
| Camera not rendering | Missing Camera3d component | Spawn with `Camera3dBundle` or `(Camera3d, Transform, ...)` |
| Resource missing panic | `Res<T>` when not inserted | Use `Option<Res<T>>` or ensure resource inserted in Plugin |
| `cannot borrow world mutably` | Two conflicting queries | Use `ParamSet` or split into separate systems |

---

## Asset Loading Pattern (Async)

This project's heightmap loading is the canonical async pattern:

```rust
#[derive(Resource)]
struct HeightmapLoadTask(Task<Option<HeightmapPyramid>>);

fn start_load(world: &mut World) {
    let task = AsyncComputeTaskPool::get().spawn(async move {
        // heavy work on background thread
        Some(HeightmapPyramid { levels })
    });
    world.insert_resource(HeightmapLoadTask(task));
}

fn poll_load(
    mut commands: Commands,
    task_res: Option<ResMut<HeightmapLoadTask>>,
    mut next: ResMut<NextState<AppState>>,
) {
    let Some(mut holder) = task_res else { return };
    let Some(result) = future::block_on(future::poll_once(&mut holder.0)) else { return };
    // task done — commit result, transition state
    if let Some(pyramid) = result {
        commands.insert_resource(HeightmapRes(Arc::new(pyramid)));
    }
    commands.remove_resource::<HeightmapLoadTask>();
    next.set(AppState::Running);
}
```

---

## Required Components (Bevy 0.15)

Declare component dependencies directly on structs. Auto-inject on spawn unless overridden.

```rust
#[derive(Component, Default)]
#[require(Transform, Visibility)]   // auto-added when spawning TerrainNode
struct TerrainNode { pub face: u8, pub depth: u8, pub nx: u32, pub ny: u32 }

// Spawn with auto-required components:
commands.spawn(TerrainNode { face: 0, depth: 4, nx: 3, ny: 7 });
// → TerrainNode + Transform + Visibility auto-inserted

// Custom initializer:
#[derive(Component)]
#[require(Health = || Health(100))]
struct Player;
```

**Gotcha:** All required components must implement `Default`. Required cycles produce
confusing errors at startup.

---

## Observers & Component Hooks (Bevy 0.15)

React to component lifecycle events without polling.

```rust
// Component hook — fires when component added/removed:
app.observe(|trigger: Trigger<OnAdd, HasTerrain>, mut commands: Commands| {
    let entity = trigger.entity();
    commands.entity(entity).insert(BodyVisualLod::Sphere);
});

// Runtime observer for cleanup:
app.observe(|trigger: Trigger<OnRemove, TerrainLoadTask>| {
    // fires when TerrainLoadTask removed (task done)
});
```

**Execution order:** Observers execute before hooks on add, after hooks on remove.
Commands from observers batch-apply after all observers run — not mid-observer.

**When to use:** Auto-attach auxiliary components, auto-cleanup on remove.

---

## Single<&T> Query Parameter (Bevy 0.15)

Typesafe sugar for "exactly one entity matches this query". Panics if zero or 2+ match.

```rust
// Old way:
fn update(camera: Query<&Position, With<MainCamera>>) {
    let Ok(cam_pos) = camera.get_single() else { return };
}

// New way — panics if not exactly 1 match:
fn update(camera: Single<&Position, With<MainCamera>>) {
    let cam_pos = *camera;  // directly derefs to &Position
}

// Optional (0 or 1 match):
fn update(camera: Option<Single<&Position, With<MainCamera>>>) {
    let Some(cam_pos) = camera else { return };
}
```

**Gotcha:** `Single<>` causes "could not access system parameter" panic if not exactly 1 entity
matches. Always use `Option<Single<>>` for entities that might not exist at system startup.

Project uses `Single<>` in: `body_visual_lod_select`, `terrain_lod_select`,
`update_atmosphere_uniforms`, `update_sol_billboard`.

---

## Useful Bevy Built-in Resources

```rust
Res<Time>              // wall-clock time; .delta_secs(), .elapsed_secs()
Res<Time<Fixed>>       // fixed timestep; .delta_secs_f64() in FixedUpdate
Res<Time<Virtual>>     // virtual (warpable) time
Res<Assets<Mesh>>      // mesh asset store
ResMut<Assets<Mesh>>   // add/modify meshes
Res<AssetServer>       // load assets from disk
```
