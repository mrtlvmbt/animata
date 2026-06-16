# Dev bridge — autonomous verification for `life`

A runtime control/inspection channel so an agent (or a CI script) can **drive the
live app, read its state, and capture screenshots over `curl`** — verifying the
GUI and emergent behaviour without a human watching the display.

> **Status:** phases 1–4 implemented (`src/dev_bridge.rs`, `--features dev`).
> Verified live end-to-end: `cargo run --features dev` opens the window, binds
> `127.0.0.1:8127`, and every method answers — status/inspect/histogram, the
> controls, and `animata/screenshot` (PNG an agent can then view). Note: `inspect`
> and `select` take the world point as **top-level** `{"x":…,"y":…}` (not nested).
> Phase 5 (Python qa-agent + YAML scenarios) is still TODO.

Modeled on GRAV's `src/dev_bridge` + `tools/dev/qa-agent`, but adapted: GRAV gets
this nearly for free from Bevy's Remote Protocol (ECS reflection + an HTTP server
plugin). `life` is macroquad — a single-threaded `async` loop with no ECS, no
reflection, no built-in server — so the bridge is hand-built around that loop.

---

## 1. Why the shape differs from GRAV

| | GRAV (Bevy) | life (macroquad) |
|---|---|---|
| Server | `RemoteHttpPlugin` (built-in) | hand-spawned thread + `tiny_http` |
| State access | ECS queries / `Reflect` | mutate the `main()` locals directly |
| Input | synthetic events into `ButtonInput` | semantic commands (controls are few) |
| Screenshot | `Screenshot::primary_window()` | `get_screen_data().export_png()` |
| Threading | Bevy schedules the handler systems | bg HTTP thread ↔ main loop via channel |

Key consequence: in macroquad the **GL context and all state live on the main
thread only**. So the HTTP server runs on a background thread and *cannot* touch
the world or take a screenshot itself — it enqueues a request and blocks on a
reply that the main loop produces on its next frame.

---

## 2. Architecture

```
 ┌──────────────┐   POST /  (JSON-RPC)   ┌─────────────────────────┐
 │  curl / agent │ ─────────────────────▶ │ bg thread: tiny_http     │
 └──────────────┘ ◀───────────────────── │  parse → Cmd + oneshot   │
                       JSON reply         └───────────┬─────────────┘
                                                      │ push (Cmd, Sender)
                                              Arc<Mutex<VecDeque>>
                                                      │ drain each frame
                                          ┌───────────▼─────────────┐
                                          │ main loop (owns world,   │
                                          │ view, paused, params…)   │
                                          │  exec Cmd → Reply → send │
                                          └──────────────────────────┘
```

- **Transport**: HTTP/1.1 POST on `127.0.0.1:8127`, JSON-RPC 2.0 body (same
  envelope as GRAV: `{jsonrpc, id, method, params}` → `{result}|{error}`).
- **Channel**: `Arc<Mutex<VecDeque<(Cmd, mpsc::Sender<Reply>)>>>`. The HTTP
  thread parses the request into a `Cmd`, pushes it with a fresh oneshot
  `Sender`, then **blocks on the `Receiver`** (with a ~2 s timeout → JSON-RPC
  error) so the HTTP response is synchronous to the caller.
- **Drain point**: in `main()`, once per frame *after* keyboard handling and
  *before* `world.step()` for control/read cmds; screenshot is serviced *after*
  the frame is drawn (the framebuffer must exist). One `match cmd { … }` mutates
  the existing locals — no architectural change to the sim.
- **Determinism**: bridge cmds run at frame boundaries, never mid-step, so they
  can't corrupt a step. `step{n}` + `set_pause` give exact, reproducible advance.

Sketch:

```rust
// src/dev_bridge/mod.rs   (compiled only under feature = "dev")
pub enum Cmd {
    Status, Inspect { id: Option<u64>, at: Option<Vec2> }, Histogram,
    SetPause(bool), SetSpeed(u32), Step(u32), Reset { seed: Option<u64> },
    SetView { scale: Option<f32>, cx: Option<f32>, cy: Option<f32> },
    SetColor(ColorMode), Select { id: Option<u64>, at: Option<Vec2> },
    SetParam { name: String, value: f64 }, Save(String), Load(String),
    Screenshot(String),
}
pub struct Req { pub cmd: Cmd, pub reply: std::sync::mpsc::Sender<serde_json::Value> }
pub type Queue = std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<Req>>>;

pub fn spawn(port: u16) -> Queue { /* tiny_http server thread, parse → Req */ }
```

```rust
// in main(), each frame:
#[cfg(feature = "dev")]
dev_bridge::drain(&queue, |cmd| match cmd {        // returns serde_json::Value
    Cmd::SetPause(p)   => { paused = p; json!({"ok":true}) }
    Cmd::Step(n)       => { for _ in 0..n { world.step(); } json!({"tick": world.tick}) }
    Cmd::Reset{seed}   => { seed_counter = seed.unwrap_or(seed_counter);
                            world = World::new(seed_counter, behavior); json!({"ok":true}) }
    Cmd::Status        => bridge_status(&world, paused, speed, &view, color_mode),
    Cmd::Screenshot(_) => json!({"deferred": true}),   // serviced post-draw
    /* … */
});
```

Screenshot is the one command serviced after `draw_*`/`next_frame`:
`let img = get_screen_data(); img.export_png(&path);` then reply.

---

## 3. Method catalogue (maps to existing state)

**Read** (no mutation):
- `animata/status` → `tick, paused, speed, behavior, drought,` the whole latest
  `Snapshot` (pop, herbivores, predators, species, lineages, max_generation,
  avg_speed/sense/radius/metabolism/carnivory/ornament, diversity, niche_spread,
  avg_memory, **avg_segments, appendaged_frac, frac_underground, frac_air,
  avg_hidden, frac_finned**), plus `view{scale,center}`.
- `animata/inspect {id?|at?}` → one creature: `id, pos, layer, energy, age,
  generation, carnivory, primary_layer, n_hidden, synapse_count,
  segments:[{length,width,appendage,flexibility}], radius, max_speed, lineage,
  species_id`.
- `animata/histogram` → population distributions for richer asserts: counts per
  layer, per appendage kind, segment-count buckets, hidden-width buckets.

**Control** (mutate the `main()` locals):
- `animata/set_pause {paused}` · `animata/set_speed {steps}` · `animata/step {n}`
  (advance while paused) · `animata/reset {seed?}` · `animata/set_view
  {scale?,cx?,cy?}` · `animata/set_color {mode}` · `animata/select {id?|at?}` ·
  `animata/set_param {name, value}` (food_per_step|predator_gain|mutation_rate —
  the live sliders) · `animata/save {path}` · `animata/load {path}`.

**Capture**:
- `animata/screenshot {path}` → PNG of the current frame (serviced post-draw).

Controls are few and semantic, so — unlike GRAV — there's no need to synthesize
keystrokes; each hot-key has a direct method. (A generic `life/key {code}` could
be added later for completeness, but isn't needed for verification.)

---

## 4. Cargo / gating

```toml
[features]
dev = ["dep:tiny_http", "dep:serde_json"]

[dependencies]
tiny_http  = { version = "0.12", optional = true }
serde_json = { version = "1",    optional = true }
```

- All bridge code behind `#[cfg(feature = "dev")]`; the shipped build is byte-for-
  byte unchanged and pulls neither dep.
- `mod dev_bridge;` and the `drain(...)` call are `#[cfg(feature="dev")]`.
- Run: `cargo run --features dev`. The bridge binds `127.0.0.1:8127`.

---

## 5. Security

- Bind **loopback only** (`127.0.0.1`), never `0.0.0.0`.
- Compiled out unless `--features dev` — cannot exist in a release artifact.
- No auth (localhost dev tool); it can reset the sim and write files
  (`save`/`screenshot` paths) — restrict paths to the repo dir.

---

## 6. Agent / CI workflow

Direct (what unblocks *me* — I can view PNGs):

```sh
cargo run --features dev &                                   # launch with bridge
J(){ curl -s 127.0.0.1:8127 -d '{"jsonrpc":"2.0","id":1,"method":"'"$1"'","params":'"${2:-null}"'}'; }
J animata/reset '{"seed":6}'
J animata/set_speed '{"steps":8}'; sleep 20                     # let it evolve
J animata/status                                                # assert strata numerically
J animata/set_view '{"scale":12}'; J animata/screenshot '{"path":"shot.png"}'
# → then Read shot.png to eyeball strata tint / segmented bodies / brain inspector
```

Formalized (mirror GRAV's qa-agent), optional `tools/dev/qa-agent/`:
- `bridge.py` — thin JSON-RPC client, 1 method per `life/*` handler.
- `agent.py` — reduce loop: `reset → run N → numeric asserts → screenshot →
  optional visual judge → exit code`.
- `scenarios/*.yaml` — declarative, e.g.:

```yaml
name: strata_emerge
setup: { reset: { seed: 6 }, set_speed: { steps: 8 } }
run_steps: 6000
asserts:
  numeric:
    - { path: frac_underground, op: ">",  value: 0.15 }
    - { path: frac_air,         op: ">",  value: 0.05 }
    - { path: species,          op: ">",  value: 200 }
    - { path: population,       op: ">",  value: 3000 }
  visual:                                  # screenshot → Claude vision → {ok,why}
    - "creatures tinted by layer (dark underground, pale air); some bodies are segmented chains"
```

---

## 7. Implementation phases

1. **Skeleton** — `dev` feature, `dev_bridge` module, tiny_http thread, the
   queue + `drain()`, and `animata/status` + `animata/set_pause`. Prove curl round-trip.
2. **Control** — reset/step/speed/view/color/select/set_param/save/load.
3. **Capture** — `animata/screenshot` (post-draw `get_screen_data().export_png`).
4. **Inspect/histogram** — per-creature + distribution readouts.
5. **(optional) qa-agent** — Python client + 3–4 scenarios (strata, fins,
   brain-shrink, survival) for CI-runnable regression of the *emergent* claims.

Phases 1–4 are the Rust bridge (self-sufficient for curl-driven verification);
phase 5 packages it for repeatable CI and an LLM visual judge.
```
