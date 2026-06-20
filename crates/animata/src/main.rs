//! animata — voxel isometric world (environment viewer).
//!
//! Reset from the former a-life simulation (archived at git tag `sim-v1` / branch
//! `archive/sim-v1`). The simulation and all GUI are intentionally OFF: this is a
//! bare environment viewer that grows a Minecraft-like voxel world on macroquad's
//! 3D pipeline (real geometry + GPU depth buffer).
//!
//! Phase 2: the terrain is rendered as **batched chunk meshes** — one cached `Mesh`
//! per chunk, built once from exposed faces only (each column's top + the cliff side
//! faces toward lower neighbours), with shading baked into vertex colours per face
//! normal. The GPU depth buffer handles all occlusion. Replaces the phase-1 pillar
//! preview. (Macro-culling / streaming come with the ×16 map; ~54 chunks draw fine.)

#[cfg(feature = "dev")]
mod dev_bridge;

mod render;
mod ui;

// The simulation + world model live in the graphics-free `animata-sim` crate. The renderer only
// needs these modules by name; the rest (genome/grid/rng/tectonics/erosion/hydrology) are internal
// to the sim. `Vec2` comes from the same glam major macroquad re-exports, so types line up.
use animata_sim::{clock, config, sim, terrain};
use animata_sim::persist::Snapshot;
use animata_sim::sim_config::SimConfig;
#[cfg(feature = "dev")]
use animata_sim::metrics::{MetricRegistry, MetricValue, SimView};

use clock::WorldClock;
use config::*;
use sim::Sim;
use macroquad::prelude::*;
use macroquad::miniquad::{PassAction, RenderingBackend, UniformsSource};
use terrain::VoxelTerrain;

use render::camera::{aabb_in_view, new_scene_target, IsoCam};
use render::gpu::{chunk_pipeline, water_pipeline, ChunkUniforms, GpuChunk, WaterUniforms};
use render::streamer::{center_chunk, spawn_gen, GenJob, Streamer, SUPER};

fn window_conf() -> Conf {
    Conf {
        window_title: "animata — voxel world".to_owned(),
        window_width: WIN_W,
        window_height: WIN_H,
        high_dpi: true,
        ..Default::default()
    }
}

// ---- Camera / input tuning (no more magic numbers smeared across the loop) ----
/// Tightest zoom-in (smallest visible world height); the camera never zooms closer than this.
const MIN_ZOOM: f32 = 8.0;
/// Mouse-wheel zoom step — fraction of the current zoom per wheel notch.
const ZOOM_STEP: f32 = 0.1;
/// Keyboard pan speed factor (× zoom × dt) — pans faster when zoomed out.
const PAN_SPEED: f32 = 0.5;
/// Right-drag graze patch radius in columns (the debug clear-cut tool).
const GRAZE_PATCH_R: i32 = 24;
/// Time-scale keys (`[` slower / `]` faster): multiplicative step and the clamp range. The real
/// ceiling is `MAX_SUBSTEPS` (the per-frame sim-step cap) — past ~24–48× (30–60 fps) extra scale just
/// drops backlog instead of simulating faster — so this cap sits above that, not as the limiter.
const TIME_SCALE_STEP: f32 = 1.5;
const MIN_TIME_SCALE: f32 = 0.1;
const MAX_TIME_SCALE: f32 = 64.0;
/// Whole-map zoom-out margin: the coarse tier covers all of it, so no empty edges however far out.
const MAX_ZOOM_MARGIN: f32 = 1.2;
// ---- Creature LOD (zoom-aware: individuals up close, bacterial mats when zoomed out) ----
/// Body radius in METRES per √(biomass cells). A single cell (biomass 1) ≈ this radius — the
/// documented creature scale is ~0.12 m (mouse-sized; see `config.rs` density contract), so a lone
/// microbe is a sub-decimetre speck that shrinks with zoom instead of a fixed fat pixel.
const CREATURE_RADIUS_M: f32 = 0.06;
/// Legibility floor (px) for an individual dot when zoomed in, so a lone microbe stays clickable.
const CREATURE_MIN_PX: f32 = 1.0;
/// LOD switch: below this many pixels-per-metre (zoomed out) individuals fall sub-pixel, so we draw
/// per-column bacterial-mat coverage instead of dots.
const LOD_MAT_PX_PER_M: f32 = 4.0;
/// Colony density (creatures in a column) at which a mat tile reaches its peak opacity.
const MAT_FULL_COUNT: f32 = 6.0;
/// Opacity ceiling of a mat tile, so terrain stays faintly readable under a dense colony.
const MAT_MAX_ALPHA: f32 = 0.85;
/// Min on-screen size (px) of one body cell before we draw the MORPHOLOGY (cluster of typed cells)
/// instead of a single dot — i.e. only when zoomed in close (few creatures on screen, cost bounded).
const BODY_CELL_MIN_PX: f32 = 2.5;

/// Colour for a body cell by type (`genome::body_layout` ids): structural = the creature's evolved
/// greyscale coloration (camouflage still reads), function cells get a distinct hue so organs show.
fn cell_color(cell_type: u8, coloration: f32) -> Color {
    match cell_type {
        1 => Color::new(0.85, 0.25, 0.20, 1.0), // effector — muscle red
        2 => Color::new(0.90, 0.80, 0.20, 1.0), // storage — yellow
        3 => Color::new(0.20, 0.80, 0.90, 1.0), // sensor — cyan
        4 => Color::new(0.85, 0.20, 0.70, 1.0), // predator — magenta
        5 => Color::new(0.60, 0.70, 1.00, 1.0), // flight — sky blue
        6 => Color::new(0.55, 0.40, 0.25, 1.0), // burrow — brown
        7 => Color::new(0.30, 0.80, 0.30, 1.0), // photo — green
        _ => Color::new(coloration, coloration, coloration, 1.0), // structural — evolved coloration
    }
}
/// Default save file (cwd) for the `F5`/`F9` quick-save keys and the path-less dev-bridge save/load.
const SAVE_PATH: &str = "animata-save.bin";

/// Write a full-state snapshot of the running world to `path` (geometry is regenerated from the seed
/// on load, so only the creatures + terrain overlay + tick are stored). Human-readable error on fail.
fn save_world(
    path: &str,
    seed: u64,
    tick: u64,
    sim: &Sim,
    terrain: &VoxelTerrain,
) -> Result<(), String> {
    let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
    Snapshot::new(seed, tick, sim.to_state(), terrain.clone_state())
        .write(std::io::BufWriter::new(f))
}

/// Read a snapshot from `path` (does not yet apply it — the caller regenerates terrain from the
/// snapshot's seed and restores the state).
fn load_snapshot(path: &str) -> Result<Snapshot, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    Snapshot::read(std::io::BufReader::new(f))
}

/// A snapshot load running on a background thread, so `F9` never blocks the render loop — the slow
/// part is regenerating terrain geometry from the saved seed, exactly like a reseed. The worker
/// reads + parses the file, regenerates terrain, applies the overlay, and ships the ready pieces
/// back; the main thread polls `rx` each frame and reads `progress` (permille) for the same bar the
/// generator uses. The current world stays live and interactive until the load is ready.
struct LoadJob {
    rx: std::sync::mpsc::Receiver<Result<LoadedWorld, String>>,
    progress: std::sync::Arc<std::sync::atomic::AtomicU32>,
}

/// The fully-prepared world a [`LoadJob`] ships back. Applied on the main thread (the only
/// GL-touching step — the streamer reset — stays there); `set_state` already ran on the worker, so a
/// size mismatch surfaces as the channel's `Err` before anything here is swapped in.
struct LoadedWorld {
    terrain: VoxelTerrain,
    sim: sim::SimState,
    tick: u64,
    seed: u64,
}

/// Kick off a background load of `path` (mirrors [`spawn_gen`](render::streamer::spawn_gen)). File
/// read + parse + terrain regen + overlay restore all run off the main thread.
fn spawn_load(path: String) -> LoadJob {
    use std::sync::atomic::Ordering;
    let progress = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let (tx, rx) = std::sync::mpsc::channel();
    let p = progress.clone();
    std::thread::spawn(move || {
        let res = (|| -> Result<LoadedWorld, String> {
            let snap = load_snapshot(&path)?;
            let seed = snap.terrain_seed;
            let mut terrain = VoxelTerrain::generate(seed, &|f| {
                p.store((f.clamp(0.0, 1.0) * 1000.0) as u32, Ordering::Relaxed);
            });
            terrain.set_state(snap.terrain)?; // size-checked; Err aborts before the main swap
            Ok(LoadedWorld { terrain, sim: snap.sim, tick: snap.tick, seed })
        })();
        let _ = tx.send(res); // receiver may be gone if the app exited mid-load — ignore
    });
    LoadJob { rx, progress }
}

/// Max zoom-out (visible world height): frame the whole map with margin — the coarse tier
/// covers all of it, so there are no empty edges however far out you go.
fn max_zoom() -> f32 {
    COLS.max(ROWS) as f32 * VOX * MAX_ZOOM_MARGIN
}

/// The ground-plane point (returned as `(x, z)`) under the mouse cursor: unproject the
/// cursor through the camera and intersect the ray with `y = 0`. Used for zoom-to-cursor.
fn ground_under_cursor(cam: &IsoCam) -> Vec2 {
    let (mx, my) = mouse_position();
    let (sw, sh) = (screen_width().max(1.0), screen_height().max(1.0));
    let nx = mx / sw * 2.0 - 1.0;
    let ny = 1.0 - my / sh * 2.0; // screen Y is top-down; NDC Y is bottom-up
    let inv = cam.camera().matrix().inverse();
    let near = inv.project_point3(vec3(nx, ny, -1.0));
    let far = inv.project_point3(vec3(nx, ny, 1.0));
    let d = far - near;
    let t = if d.y.abs() > 1e-6 { -near.y / d.y } else { 0.0 };
    let hit = near + d * t;
    vec2(hit.x, hit.z)
}

/// Debug overlay selected by `G` (cycles in this order). `Topo` reshades the 3D scene on the
/// GPU; the climate / water-distance views overlay a per-column colourmap MINIMAP — the live
/// in-app consumer of the S1 environment getters.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DebugView {
    None,
    Topo,
    Temp,
    Moist,
    WaterDist,
    Slope,
    Biomass,
}

impl DebugView {
    fn next(self) -> Self {
        match self {
            DebugView::None => DebugView::Topo,
            DebugView::Topo => DebugView::Temp,
            DebugView::Temp => DebugView::Moist,
            DebugView::Moist => DebugView::WaterDist,
            DebugView::WaterDist => DebugView::Slope,
            DebugView::Slope => DebugView::Biomass,
            DebugView::Biomass => DebugView::None,
        }
    }
    /// The views drawn as a 2D field minimap (vs the 3D scene reshade / no overlay). Used by the
    /// egui minimap panel to recolour the preview and show the field legend.
    fn is_field_map(self) -> bool {
        matches!(
            self,
            DebugView::Temp | DebugView::Moist | DebugView::WaterDist | DebugView::Slope | DebugView::Biomass
        )
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    let mut cam = IsoCam::new();
    let mut seed: u64 = 1;
    // The world is generated on a background thread so the first frame (and every regen)
    // never blocks the render loop. `terrain` is `None` until the initial job finishes.
    let mut terrain: Option<VoxelTerrain> = None;
    let mut gen: Option<GenJob> = Some(spawn_gen(seed));
    // A background snapshot load (`F9`), polled like `gen`; the current world stays live until ready.
    let mut load: Option<LoadJob> = None;

    // The default sim config, loaded from assets/config/sim.ron (editable without a rebuild); falls
    // back to the built-in default if the file is missing or malformed. Live dev-bridge changes
    // apply on top until the next reseed.
    let mut sim_cfg = match macroquad::file::load_string("assets/config/sim.ron").await {
        Ok(s) => SimConfig::from_ron(&s).unwrap_or_else(|e| {
            eprintln!("[config] assets/config/sim.ron: {e}; using defaults");
            SimConfig::default()
        }),
        Err(_) => SimConfig::default(),
    };

    // Chunk meshes are STREAMED around the camera (see `Streamer`) rather than all built
    // up front — the world model is fully resident but the meshes are not, so a ×16 map
    // stays within memory. The streamer fills in each frame from `terrain`.
    // Shaders live in assets/shaders/ (editable without a rebuild); fall back to the copy baked in
    // with `include_str!` if the asset isn't reachable (e.g. running outside the repo).
    let chunk_vert = load_shader("assets/shaders/chunk.vert", include_str!("../../../assets/shaders/chunk.vert")).await;
    let chunk_frag = load_shader("assets/shaders/chunk.frag", include_str!("../../../assets/shaders/chunk.frag")).await;
    let water_vert = load_shader("assets/shaders/water.vert", include_str!("../../../assets/shaders/water.vert")).await;
    let water_frag = load_shader("assets/shaders/water.frag", include_str!("../../../assets/shaders/water.frag")).await;

    let pipeline;
    let water_pipe;
    let mut streamer = Streamer::new();
    {
        let InternalGlContext {
            quad_context: ctx, ..
        } = unsafe { get_internal_gl() };
        pipeline = chunk_pipeline(ctx, &chunk_vert, &chunk_frag);
        water_pipe = water_pipeline(ctx, &water_vert, &water_frag);
    }

    // The scene is rendered into this offscreen target every frame, then blitted to
    // the window. A screenshot reads the target's texture directly — i.e. the
    // finished pixels *before* the window present — so capture is decoupled from the
    // window back-buffer (GRAV-style framebuffer read) instead of `get_screen_data`,
    // which only sees the throttled front buffer of a foregrounded window.
    // NB: it MUST have its own depth attachment (`depth: true`) — the bare
    // `render_target()` has none, which silently disables depth testing in the pass
    // and lets far faces overdraw near ones.
    let mut scene_rt = new_scene_target(screen_width() as u32, screen_height() as u32);

    // Frame timing (EMA-smoothed) + an on-screen readout toggle (`I`).
    let mut fps = 0.0f32;
    let mut frame_ms = 0.0f32;
    // GUI toggle state (egui widgets + keyboard hotkeys flip the same fields); snapshotted into
    // Copy locals each frame so the render pass below reads plain `debug_view`/`mask`/… as before.
    let mut ui_state = ui::UiState {
        show_info: true,
        debug_view: DebugView::None,
        water_on: true,
        mask: false,
        outline: true,
        open_panel: None,
    };
    // Population sparkline buffer: last 48 samples, pushed on a tick cadence (freezes when paused).
    let mut pop_hist: std::collections::VecDeque<f32> = std::collections::VecDeque::with_capacity(48);
    let mut pop_last_tick: u64 = u64::MAX;
    // Perf counters are produced DURING render (after the UI pass); the next frame's panels read
    // them with a one-frame lag — invisible on an fps/draw readout, and it keeps `wants_pointer`
    // fresh for input gating (the UI pass runs before world mouse input).
    let mut drawn = 0usize;
    let mut on_screen = 0usize;
    // Fonts/style are installed on the first egui pass (egui keeps them for the context lifetime).
    let mut fonts_set = false;
    // Persistent HUD GPU resources (the minimap egui texture), held across frames.
    let mut hud_cache = ui::HudCache::default();
    // Transient top-centre system notice (save/load feedback): (message, start time in `get_time()`
    // seconds). The HUD derives the slide-in + fade from the elapsed time; cleared after its life.
    let mut toast: Option<(String, f64)> = None;
    const TOAST_LIFE_MS: f32 = 2600.0;
    // Sim time base (S2). The main loop schedules fixed sub-steps from the real frame `dt`
    // (`clock.substeps`) and drives one `sim.step` per sub-step; `P` pauses. `advance` stays a
    // pure counter (HUD/day-frac). The creature sim (C0) is created once the world is ready.
    let mut clock = WorldClock::new();
    let mut sim: Option<Sim> = None;
    // Live metric time-series for the dev bridge (`animata/metrics`); sampled after each sub-step.
    #[cfg(feature = "dev")]
    let mut metrics = MetricRegistry::default();
    // `G` cycles the debug view: off → Topo (GPU height/depth, water hidden) → Temp → Moist
    // → WaterDist → off. Topo reshades the 3D scene; the climate/water-dist modes overlay a
    // colourmap MINIMAP of the per-column field (the live consumer of the S1 env getters, so
    // they verify visually — poles cold / equator hot — and aren't dead code in any build).
    // `H` hides the translucent water surface; `J` toggles the WATER/LAND mask (land grey,
    // flagged water blue — dry holes inside blue flag a gen bug); `O` toggles the dark step-edge
    // outline. All three live in `ui_state` now (checkbox + hotkey share the field).
    // Left-drag pans the map: the ground point grabbed on press stays under the cursor.
    let mut grab: Option<Vec2> = None;

    // Dev bridge: localhost JSON-RPC for driving/inspecting the viewer (see
    // DEV_BRIDGE.md). Off unless built with `--features dev`.
    #[cfg(feature = "dev")]
    let bridge = dev_bridge::spawn(8127);
    #[cfg(feature = "dev")]
    let mut pending_shots: Vec<(String, bool, std::sync::mpsc::Sender<serde_json::Value>)> = Vec::new();

    loop {
        let dt = get_frame_time();
        // Pick up a finished background world (non-blocking). On readiness, swap it in and
        // reset the streamer so meshes rebuild around the camera from the new terrain.
        if let Some(job) = &gen {
            if let Ok(t) = job.rx.try_recv() {
                // Seed the creature population from the new world (deterministic from its seed).
                sim = Some(Sim::with_config(seed, &t, sim_cfg));
                terrain = Some(t);
                gen = None;
                let InternalGlContext { quad_context: ctx, .. } = unsafe { get_internal_gl() };
                streamer.clear(ctx);
            }
        }
        // Pick up a finished background load the same way (restore overlay + creatures + tick).
        if let Some(job) = &load {
            if let Ok(res) = job.rx.try_recv() {
                match res {
                    Ok(w) => {
                        seed = w.seed;
                        sim_cfg = w.sim.cfg;
                        sim = Some(Sim::from_state(w.sim));
                        clock.set_tick(w.tick);
                        terrain = Some(w.terrain);
                        let InternalGlContext { quad_context: ctx, .. } = unsafe { get_internal_gl() };
                        streamer.clear(ctx);
                        eprintln!("[load] restored {SAVE_PATH} at tick {}", w.tick);
                        toast = Some(("Loaded".into(), get_time()));
                    }
                    Err(e) => {
                        eprintln!("[load] failed: {e}");
                        toast = Some((format!("Load failed: {e}"), get_time()));
                    }
                }
                load = None;
            }
        }
        // Smooth the frame-time readout so it doesn't jitter.
        frame_ms = 0.9 * frame_ms + 0.1 * dt * 1000.0;
        if dt > 0.0 {
            fps = 0.9 * fps + 0.1 / dt;
        }
        // Drive the sim: schedule whole sub-steps from real `dt` (capped, so a lag spike can't
        // spiral), then run EXACTLY one fixed `sim.step` per sub-step, each at its own tick.
        // `advance` stays a pure counter (HUD/day-frac); the interactive cadence is best-effort
        // (not for seed replay — that path is the fixed-step headless harness).
        let substeps = clock.substeps(dt);
        for _ in 0..substeps {
            clock.advance(1);
            if let (Some(sim), Some(terrain)) = (sim.as_mut(), terrain.as_mut()) {
                let tick = clock.tick();
                sim.step(terrain, tick);
                // Sample the metric registry (dev only) so `animata/metrics` can serve live values.
                #[cfg(feature = "dev")]
                metrics.maybe_sample(&SimView { sim, terrain, tick });
            }
        }

        // ---- GUI pass (egui) — runs before world input so `wants_pointer` gates the mouse.
        // Perf counters (`drawn`/`on_screen`) come from LAST frame's render (produced after this
        // pass); `det`/`crs` read the current streamer state.
        let det = streamer.detail.len();
        let crs = streamer.coarse.len();
        let life = match (sim.as_ref(), terrain.as_ref()) {
            (Some(s), Some(t)) => {
                let (multi, _) = s.complexity_mix();
                let sm = s.stratum_mix(t);
                Some(ui::LifeStats {
                    population: s.population() as u64,
                    avg_energy: s.avg_energy(),
                    avg_biomass: s.avg_biomass(),
                    multi,
                    carn: s.frac_carnivore(),
                    auto: s.frac_autotroph(),
                    species: s.species_count() as u64,
                    niches: s.niche_coverage(t) as u64,
                    allop: s.thermal_correlation(t),
                    crypsis: s.crypsis_correlation(t),
                    nutri: s.avg_nutrient(t, clock.tick()),
                    strata: sm,
                })
            }
            _ => None,
        };
        // Expire the transient save notice; otherwise hand the HUD the elapsed ms (it owns the fade).
        let toast_view = match &toast {
            Some((msg, start)) => {
                let dt = ((get_time() - *start) as f32) * 1000.0;
                if dt >= TOAST_LIFE_MS {
                    toast = None;
                    None
                } else {
                    Some((msg.clone(), dt))
                }
            }
            None => None,
        };
        // Sparkline sample on a tick cadence (~every 10 ticks); deduped by tick so it freezes on
        // pause and is independent of fps/time_scale.
        if let Some(l) = &life {
            let tnow = clock.tick();
            if pop_last_tick == u64::MAX || tnow >= pop_last_tick.wrapping_add(10) {
                if pop_hist.len() == 48 {
                    pop_hist.pop_front();
                }
                pop_hist.push_back(l.population as f32);
                pop_last_tick = tnow;
            }
        }
        // Visible-world quad on the map for the minimap viewport frame: unproject the 4 screen
        // corners onto the ground plane (exact at any yaw — the azimuth-45° view is a rotated quad,
        // not an axis-aligned box), expressed as map-space fractions.
        let minimap_view = {
            let inv = cam.camera().matrix().inverse();
            let (mw, mh) = (COLS as f32 * VOX, ROWS as f32 * VOX);
            [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                .iter()
                .map(|&(nx, ny)| {
                    let near = inv.project_point3(vec3(nx, ny, -1.0));
                    let far = inv.project_point3(vec3(nx, ny, 1.0));
                    let d = far - near;
                    let t = if d.y.abs() > 1e-6 { -near.y / d.y } else { 0.0 };
                    let hit = near + d * t;
                    [hit.x / mw, hit.z / mh]
                })
                .collect::<Vec<_>>()
        };
        let hud_metrics = ui::SimMetrics {
            fps,
            frame_ms,
            drawn,
            detail: det,
            coarse: crs,
            on_screen,
            seed,
            cols: COLS,
            rows: ROWS,
            tick: clock.tick(),
            sim_time: clock.sim_time() as f32,
            day_frac: clock.day_frac(),
            time_scale: clock.time_scale,
            paused: clock.paused,
            life,
            pop_hist: pop_hist.iter().copied().collect(),
            minimap_view,
            toast: toast_view,
        };
        let mut actions = ui::UiActions::default();
        egui_macroquad::ui(|ctx| {
            // Register IBM Plex + Phosphor and the global style ONCE (egui keeps them for the
            // context's lifetime). Fonts are vendored in assets/fonts/ (OFL), baked via include_bytes.
            if !fonts_set {
                ui::theme::install_fonts(
                    ctx,
                    include_bytes!("../../../assets/fonts/IBMPlexSans-Regular.ttf"),
                    include_bytes!("../../../assets/fonts/IBMPlexMono-Regular.ttf"),
                );
                ui::theme::install_style(ctx);
                fonts_set = true;
            }
            // high_dpi=true ⇒ macroquad reports physical px; match egui's scale so panels aren't
            // tiny on Retina (F2).
            ctx.set_pixels_per_point(macroquad::miniquad::window::dpi_scale());
            actions = ui::draw_hud(ctx, &mut ui_state, &hud_metrics, &mut hud_cache, terrain.as_ref());
        });
        let wants_ptr = actions.wants_pointer;

        // ---- Input ---- (keyboard hotkeys always live; egui widgets flip the same `ui_state`.)
        if is_key_pressed(KeyCode::I) {
            ui_state.show_info = !ui_state.show_info;
        }
        if is_key_pressed(KeyCode::G) {
            ui_state.debug_view = ui_state.debug_view.next();
        }
        if is_key_pressed(KeyCode::H) {
            ui_state.water_on = !ui_state.water_on;
        }
        if is_key_pressed(KeyCode::P) || actions.toggle_pause {
            clock.paused = !clock.paused;
        }
        // Time speed: `[` slows, `]` speeds (multiplicative, clamped); the panel slider/buttons feed
        // `actions.set_time_scale`. Same `time_scale` the dev-bridge `set_timescale` drives.
        if is_key_pressed(KeyCode::LeftBracket) {
            clock.time_scale = (clock.time_scale / TIME_SCALE_STEP).max(MIN_TIME_SCALE);
        }
        if is_key_pressed(KeyCode::RightBracket) {
            clock.time_scale = (clock.time_scale * TIME_SCALE_STEP).min(MAX_TIME_SCALE);
        }
        if let Some(ts) = actions.set_time_scale {
            clock.time_scale = ts.clamp(MIN_TIME_SCALE, MAX_TIME_SCALE);
        }
        // Quick-save (`F5`) / quick-load (`F9`) the whole world to/from `SAVE_PATH`. Both the load's
        // terrain regen and save's serialise stay off the hot path: save is fast; load runs on a
        // background thread (`spawn_load`) and swaps in when ready, like a reseed.
        if is_key_pressed(KeyCode::F5) || actions.save {
            match (sim.as_ref(), terrain.as_ref()) {
                (Some(s), Some(t)) => match save_world(SAVE_PATH, seed, clock.tick(), s, t) {
                    Ok(()) => {
                        eprintln!("[save] wrote {SAVE_PATH} at tick {}", clock.tick());
                        toast = Some(("Saved".into(), get_time()));
                    }
                    Err(e) => {
                        eprintln!("[save] failed: {e}");
                        toast = Some((format!("Save failed: {e}"), get_time()));
                    }
                },
                _ => eprintln!("[save] world not ready"),
            }
        }
        // Start a background load if one isn't already running. Cancel any in-flight regen — the
        // load wins. The current world stays interactive; the poll above swaps it in when ready.
        if (is_key_pressed(KeyCode::F9) || actions.load) && load.is_none() {
            gen = None;
            load = Some(spawn_load(SAVE_PATH.to_string()));
        }
        if is_key_pressed(KeyCode::J) {
            ui_state.mask = !ui_state.mask;
        }
        if is_key_pressed(KeyCode::O) {
            ui_state.outline = !ui_state.outline;
        }
        // Snapshot toggle state into Copy locals (`debug_view`/`mask`/`outline`/`water_on`) so the
        // render pass below reads them as before. `show_info` is handled inside the GUI pass.
        let ui::UiState { show_info: _, debug_view, water_on, mask, outline, open_panel: _ } = ui_state;

        // World mouse interactions are gated on `!wants_ptr` so a click on a panel doesn't reach
        // the world (F8). Keyboard pan/rotate stay live (egui claims no keys without a text focus).
        let wheel = if wants_ptr { 0.0 } else { mouse_wheel().1 };
        if wheel != 0.0 {
            // Zoom toward the cursor: keep the ground point under the mouse fixed by
            // shifting the target by how much that point would otherwise move.
            let before = ground_under_cursor(&cam);
            cam.zoom = (cam.zoom * (1.0 - wheel.signum() * ZOOM_STEP)).clamp(MIN_ZOOM, max_zoom());
            let after = ground_under_cursor(&cam);
            cam.target.x += before.x - after.x;
            cam.target.z += before.y - after.y;
        }
        // Left-drag pan: lock the grabbed ground point under the moving cursor. Don't START a pan
        // when the press lands on a panel (F8); an in-flight pan finishes normally.
        if !wants_ptr && is_mouse_button_pressed(MouseButton::Left) {
            grab = Some(ground_under_cursor(&cam));
        }
        if !is_mouse_button_down(MouseButton::Left) {
            grab = None;
        } else if let Some(g) = grab {
            let cur = ground_under_cursor(&cam);
            cam.target.x += g.x - cur.x;
            cam.target.z += g.y - cur.y;
        }
        // Right-drag GRAZE (debug): clear-cut the vegetation in a patch under the cursor —
        // the default-build consumer of `graze`, and a manual way to verify regrowth (graze a
        // spot in the Biomass view, watch it grow back). Patch radius so it shows on the
        // down-sampled minimap.
        if !wants_ptr && is_mouse_button_down(MouseButton::Right) {
            if let Some(t) = &mut terrain {
                let g = ground_under_cursor(&cam);
                let (gx, gy) = ((g.x / VOX).floor() as i32, (g.y / VOX).floor() as i32);
                let r = GRAZE_PATCH_R;
                let tick = clock.tick();
                for yy in (gy - r).max(0)..(gy + r).min(ROWS as i32) {
                    for xx in (gx - r).max(0)..(gx + r).min(COLS as i32) {
                        t.graze(xx as usize, yy as usize, 1.0, tick); // clear-cut (take all)
                    }
                }
            }
        }
        // Pan in the ground plane (WASD / arrows), rotated by the current yaw.
        let mut pan = Vec2::ZERO;
        if is_key_down(KeyCode::A) || is_key_down(KeyCode::Left) {
            pan.x -= 1.0;
        }
        if is_key_down(KeyCode::D) || is_key_down(KeyCode::Right) {
            pan.x += 1.0;
        }
        if is_key_down(KeyCode::W) || is_key_down(KeyCode::Up) {
            pan.y -= 1.0;
        }
        if is_key_down(KeyCode::S) || is_key_down(KeyCode::Down) {
            pan.y += 1.0;
        }
        if pan != Vec2::ZERO {
            let speed = cam.zoom * dt * PAN_SPEED; // pan faster when zoomed out
            let (c, s) = (cam.yaw.cos(), cam.yaw.sin());
            cam.target.x += (pan.x * c - pan.y * s) * speed;
            cam.target.z += (pan.x * s + pan.y * c) * speed;
        }
        // Rotate the iso view in 90° steps.
        if is_key_pressed(KeyCode::Q) {
            cam.yaw -= std::f32::consts::FRAC_PI_2;
        }
        if is_key_pressed(KeyCode::E) {
            cam.yaw += std::f32::consts::FRAC_PI_2;
        }
        // Regenerate the world with a fresh seed — in the background. The current map stays
        // visible and interactive until the new one is ready (swapped in by the poll above).
        // A regen already in flight ignores further presses.
        if is_key_pressed(KeyCode::R) && gen.is_none() {
            seed = seed.wrapping_add(1);
            gen = Some(spawn_gen(seed));
        }

        // ---- Dev bridge: service queued commands on the main thread ----
        #[cfg(feature = "dev")]
        for req in dev_bridge::take(&bridge) {
            let dev_bridge::Req { cmd, reply } = req;
            match cmd {
                dev_bridge::Cmd::Status => {
                    let c = cam.camera();
                    // Environment fields under the camera-centre column (steerable numeric
                    // assert surface for the S1 substrate). `null` until the world is ready.
                    let env = terrain.as_ref().map(|t| {
                        let x = (cam.target.x / VOX).floor().clamp(0.0, (COLS - 1) as f32) as usize;
                        let y = (cam.target.z / VOX).floor().clamp(0.0, (ROWS - 1) as f32) as usize;
                        serde_json::json!({
                            "col": [x, y],
                            "temp": t.temperature_at(x, y),
                            "moist": t.moisture_at(x, y),
                            "slope": t.slope_at(x, y),
                            "water_dist": t.water_dist_at(x, y),
                            "biome": format!("{:?}", t.biome_at(x, y)),
                            "biomass": t.biomass_at(x, y, clock.tick()),
                        })
                    });
                    let _ = reply.send(serde_json::json!({
                        "fps": fps,
                        "frame_ms": frame_ms,
                        "seed": seed,
                        "depth": { "z_near": c.z_near, "z_far": c.z_far, "range": c.z_far - c.z_near },
                        "view": { "cx": cam.target.x, "cz": cam.target.z, "zoom": cam.zoom, "yaw": cam.yaw },
                        "map": { "cols": COLS, "rows": ROWS, "vox_m": VOX, "map_scale": MAP_SCALE,
                                 "detail_chunks": streamer.detail.len(), "coarse_tiles": streamer.coarse.len() },
                        "env": env,
                        "clock": { "tick": clock.tick(), "sim_time": clock.sim_time(),
                                   "day_frac": clock.day_frac(), "time_scale": clock.time_scale,
                                   "paused": clock.paused },
                        "sim": sim.as_ref().map(|s| {
                            let (multi, complex) = s.complexity_mix();
                            let allopatry = terrain.as_ref().map(|t| s.thermal_correlation(t));
                            let strata = terrain.as_ref().map(|t| s.stratum_mix(t));
                            serde_json::json!({
                                "population": s.population(),
                                "avg_energy": s.avg_energy(),
                                "avg_biomass": s.avg_biomass(),
                                "frac_multicellular": multi,
                                "frac_complex": complex,
                                "frac_carnivore": s.frac_carnivore(),
                                "frac_autotroph": s.frac_autotroph(),
                                "avg_nutrient": terrain.as_ref().map(|t| s.avg_nutrient(t, clock.tick())),
                                "allopatry": allopatry,
                                "crypsis": terrain.as_ref().map(|t| s.crypsis_correlation(t)),
                                "species": s.species_count(),
                                "niche_coverage": terrain.as_ref().map(|t| s.niche_coverage(t)),
                                "strata_und_surf_air_water": strata,
                                "births": s.births,
                                "deaths": s.deaths,
                                "kills": s.kills,
                            })
                        }),
                        "config": config_json(sim.as_ref()),
                    }));
                }
                dev_bridge::Cmd::SetClock { scale, paused } => {
                    if let Some(s) = scale {
                        clock.time_scale = s.max(0.0);
                    }
                    if let Some(p) = paused {
                        clock.paused = p;
                    }
                    let _ = reply.send(serde_json::json!({
                        "time_scale": clock.time_scale, "paused": clock.paused,
                    }));
                }
                dev_bridge::Cmd::Graze { x, y, amount } => {
                    let taken = terrain.as_mut().and_then(|t| {
                        (x < COLS && y < ROWS).then(|| t.graze(x, y, amount, clock.tick()))
                    });
                    let _ = reply.send(serde_json::json!({
                        "taken": taken, "tick": clock.tick(),
                    }));
                }
                dev_bridge::Cmd::Biomass { x, y } => {
                    let biomass = terrain.as_ref().and_then(|t| {
                        (x < COLS && y < ROWS).then(|| t.biomass_at(x, y, clock.tick()))
                    });
                    let _ = reply.send(serde_json::json!({
                        "biomass": biomass, "tick": clock.tick(),
                    }));
                }
                dev_bridge::Cmd::SetView { cx, cz, zoom, yaw } => {
                    if let Some(v) = cx {
                        cam.target.x = v;
                    }
                    if let Some(v) = cz {
                        cam.target.z = v;
                    }
                    if let Some(v) = zoom {
                        cam.zoom = v.clamp(MIN_ZOOM, max_zoom());
                    }
                    if let Some(v) = yaw {
                        cam.yaw = v;
                    }
                    let _ = reply.send(serde_json::json!({"ok": true}));
                }
                dev_bridge::Cmd::Reseed { seed: s } => {
                    // Synchronous on the dev path: scripted inspection expects the new world
                    // (e.g. an immediate screenshot) deterministically, so we block here.
                    seed = s.unwrap_or(seed.wrapping_add(1));
                    gen = None; // cancel any in-flight background regen — this wins
                    let t = VoxelTerrain::new(seed);
                    sim = Some(Sim::with_config(seed, &t, sim_cfg)); // re-seed the population from the new world
                    terrain = Some(t);
                    let InternalGlContext { quad_context: ctx, .. } = unsafe { get_internal_gl() };
                    streamer.clear(ctx);
                    let _ = reply.send(serde_json::json!({"seed": seed}));
                }
                dev_bridge::Cmd::Render { water: w, topo: tp } => {
                    // Write to `ui_state` (the source of truth); the render snapshot picks it up next
                    // frame — same field the HUD checkbox / `H`/`G` hotkeys flip.
                    if let Some(w) = w {
                        ui_state.water_on = w;
                    }
                    if let Some(tp) = tp {
                        // `topo` stays a bool over the wire: true selects the Topo view, false
                        // clears to Off (the climate minimaps are driven by `G` interactively).
                        ui_state.debug_view = if tp { DebugView::Topo } else { DebugView::None };
                    }
                    let _ = reply.send(serde_json::json!({"water": ui_state.water_on, "topo": ui_state.debug_view == DebugView::Topo}));
                }
                dev_bridge::Cmd::Screenshot { path, window } => {
                    pending_shots.push((path, window, reply)); // serviced post-draw below
                }
                dev_bridge::Cmd::GetConfig => {
                    let _ = reply.send(config_json(sim.as_ref()));
                }
                dev_bridge::Cmd::SetFeature { name, enabled } => {
                    let resp = match sim.as_mut() {
                        Some(s) => {
                            let mut c = s.config();
                            if c.features.set(&name, enabled) {
                                s.set_config(c);
                                serde_json::json!({ "ok": true, "feature": name, "enabled": enabled })
                            } else {
                                serde_json::json!({ "ok": false, "error": format!("unknown feature: {name}") })
                            }
                        }
                        None => serde_json::json!({ "ok": false, "error": "no sim yet" }),
                    };
                    let _ = reply.send(resp);
                }
                dev_bridge::Cmd::SetParam { name, value } => {
                    let resp = match sim.as_mut() {
                        Some(s) => {
                            let mut c = s.config();
                            if c.params.set(&name, value) {
                                s.set_config(c);
                                serde_json::json!({ "ok": true, "param": name, "value": value })
                            } else {
                                serde_json::json!({ "ok": false, "error": format!("unknown param: {name}") })
                            }
                        }
                        None => serde_json::json!({ "ok": false, "error": "no sim yet" }),
                    };
                    let _ = reply.send(resp);
                }
                dev_bridge::Cmd::Metrics { id, last } => {
                    let _ = reply.send(metrics_json(&metrics, id.as_deref(), last));
                }
                dev_bridge::Cmd::Save { path } => {
                    let p = path.unwrap_or_else(|| SAVE_PATH.to_string());
                    let resp = match (sim.as_ref(), terrain.as_ref()) {
                        (Some(s), Some(t)) => match save_world(&p, seed, clock.tick(), s, t) {
                            Ok(()) => serde_json::json!({ "ok": true, "saved": p, "tick": clock.tick() }),
                            Err(e) => serde_json::json!({ "ok": false, "error": e }),
                        },
                        _ => serde_json::json!({ "ok": false, "error": "world not ready" }),
                    };
                    let _ = reply.send(resp);
                }
                dev_bridge::Cmd::Load { path } => {
                    let p = path.unwrap_or_else(|| SAVE_PATH.to_string());
                    // Synchronous (unlike the interactive F9 background load): scripted inspection
                    // expects the loaded world present on the reply, so regenerate + restore inline.
                    let resp = match load_snapshot(&p) {
                        Ok(snap) => {
                            let mut t = VoxelTerrain::new(snap.terrain_seed);
                            match t.set_state(snap.terrain) {
                                Ok(()) => {
                                    let tick = snap.tick;
                                    seed = snap.terrain_seed;
                                    sim_cfg = snap.sim.cfg;
                                    sim = Some(Sim::from_state(snap.sim));
                                    clock.set_tick(tick);
                                    terrain = Some(t);
                                    gen = None; // cancel any in-flight regen/load — the load wins
                                    load = None;
                                    let InternalGlContext { quad_context: ctx, .. } =
                                        unsafe { get_internal_gl() };
                                    streamer.clear(ctx);
                                    serde_json::json!({ "ok": true, "loaded": p, "tick": tick, "seed": seed })
                                }
                                Err(e) => serde_json::json!({ "ok": false, "error": e }),
                            }
                        }
                        Err(e) => serde_json::json!({ "ok": false, "error": e }),
                    };
                    let _ = reply.send(resp);
                }
            }
        }

        // ---- Render ----
        // Keep the offscreen target matched to the (possibly resized) window.
        if scene_rt.texture.width() != screen_width()
            || scene_rt.texture.height() != screen_height()
        {
            scene_rt = new_scene_target(screen_width() as u32, screen_height() as u32);
        }

        // Pass 1: render the visible chunks into the offscreen target via raw miniquad
        // — persistent buffers, one draw call per visible chunk, no per-frame upload.
        let vp = cam.camera().matrix();
        let center = center_chunk(&cam);
        drawn = 0; // reset the (loop-persistent) perf counter the GUI pass read this frame
        {
            let mut gl = unsafe { get_internal_gl() };
            gl.flush(); // flush any pending macroquad 2D before our own pass
            let ctx = gl.quad_context;
            // Stream: detail tier around the camera + coarse super-tiles over the rest.
            // No terrain yet (initial generation still running) ⇒ nothing to stream/draw;
            // the pass below just clears to sky and the progress bar shows over it.
            if let Some(terrain) = &terrain {
                streamer.update(ctx, terrain, center, cam.zoom);
            }
            ctx.begin_pass(
                Some(scene_rt.render_pass.raw_miniquad_id()),
                PassAction::Clear {
                    color: Some((0.53, 0.62, 0.78, 1.0)), // sky
                    depth: Some(1.0),
                    stencil: None,
                },
            );
            ctx.apply_pipeline(&pipeline);
            // dbg.x = topo height view, dbg.y = water/land mask, dbg.z = step-edge outline on.
            let dbg = vec4(
                if debug_view == DebugView::Topo { 1.0 } else { 0.0 },
                if mask { 1.0 } else { 0.0 },
                if outline { 1.0 } else { 0.0 },
                0.0,
            );
            ctx.apply_uniforms(UniformsSource::table(&ChunkUniforms { mvp: vp, dbg }));
            // Per super-tile draw EITHER its detail chunks (if ready) OR its coarse buffer
            // (otherwise) — never both. So the tiers never overlap (no z-fight) and a
            // not-yet-ready tile shows coarse instead of flashing empty (no flicker).
            // Frustum-culled by AABB.
            let ready = &streamer.ready;
            let draw = |chunks: &[GpuChunk], drawn: &mut usize, ctx: &mut dyn RenderingBackend| {
                for c in chunks {
                    if aabb_in_view(&vp, c.lo, c.hi) {
                        ctx.apply_bindings(&c.bindings);
                        ctx.draw(0, c.n_idx, 1);
                        *drawn += 1;
                    }
                }
            };
            for (key, lc) in &streamer.coarse {
                if !ready.contains(key) {
                    draw(&lc.opaque, &mut drawn, ctx);
                }
            }
            for (&(cx, cy), lc) in &streamer.detail {
                if ready.contains(&(cx.div_euclid(SUPER), cy.div_euclid(SUPER))) {
                    draw(&lc.opaque, &mut drawn, ctx);
                }
            }
            // Water: second, translucent, animated pass over the opaque scene. Skipped in
            // topo mode (bed laid bare) or when toggled off with `H`. Same draw rule as the
            // opaque tiers so the two never overlap; `depth_write:false` lets terrain in
            // front still occlude it without the water occluding itself.
            // Mask mode forces the water pass on (flat blue) even over the topo gate; normal
            // mode draws it unless topo or `H` hid it.
            if mask || (debug_view != DebugView::Topo && water_on) {
                ctx.apply_pipeline(&water_pipe);
                let params = vec4(get_time() as f32, if mask { 1.0 } else { 0.0 }, 0.0, 0.0);
                ctx.apply_uniforms(UniformsSource::table(&WaterUniforms { mvp: vp, params }));
                for (key, lc) in &streamer.coarse {
                    if !ready.contains(key) {
                        draw(&lc.water, &mut drawn, ctx);
                    }
                }
                for (&(cx, cy), lc) in &streamer.detail {
                    if ready.contains(&(cx.div_euclid(SUPER), cy.div_euclid(SUPER))) {
                        draw(&lc.water, &mut drawn, ctx);
                    }
                }
            }
            ctx.end_render_pass();
        }

        // Pass 2: blit the offscreen scene to the window (render targets are y-flipped).
        draw_texture_ex(
            &scene_rt.texture,
            0.0,
            0.0,
            WHITE,
            DrawTextureParams {
                dest_size: Some(vec2(screen_width(), screen_height())),
                flip_y: true,
                ..Default::default()
            },
        );

        // Creatures: zoom-aware LOD over the blitted scene. Close in, draw INDIVIDUALS sized in
        // world metres (a single-cell microbe is a sub-metre speck that shrinks as the camera pulls
        // back — not a fixed fat pixel). Past a zoom-out threshold an individual falls below a pixel,
        // so we switch to BACTERIAL MATS: per-column density tinted by the colony's mean coloration
        // (dense colony → solid mat, sparse → faint film). Off-screen points are culled by projection.
        on_screen = 0; // reset the (loop-persistent) perf counter the GUI pass read this frame
        if let (Some(sim), Some(terrain)) = (sim.as_ref(), terrain.as_ref()) {
            let (sw, sh) = (screen_width(), screen_height());
            let px_per_m = sh / cam.zoom; // ortho: visible world-height = zoom
            // Project a world point on a column top to screen px; None if behind/off-screen.
            let project = |wx: f32, wz: f32, wy: f32| -> Option<(f32, f32)> {
                let clip = vp * vec4(wx, wy, wz, 1.0);
                if clip.w <= 0.0 {
                    return None;
                }
                let (nx, ny) = (clip.x / clip.w, clip.y / clip.w);
                if !(-1.0..=1.0).contains(&nx) || !(-1.0..=1.0).contains(&ny) {
                    return None;
                }
                Some(((nx * 0.5 + 0.5) * sw, (1.0 - (ny * 0.5 + 0.5)) * sh))
            };
            if px_per_m >= LOD_MAT_PX_PER_M {
                // INDIVIDUALS. Zoomed in far enough that a single cell spans ≥ a couple pixels, draw
                // the MORPHOLOGY — the developed body as a cluster of lattice cells, tinted by type
                // (structural = the evolved greyscale coloration, so camouflage still reads; function
                // cells get a type colour, so organs are visible). Otherwise the body is sub-cell, so
                // draw the cheaper world-scaled dot (also bounds cost: morphology only at high zoom,
                // where few creatures are on screen).
                let cell_px = CREATURE_RADIUS_M * px_per_m * 2.0;
                for c in &sim.creatures {
                    let (cx, cy) = sim::column_index(c.pos);
                    let wy = terrain.height(cx as i32, cy as i32) as f32 * VOX + 0.5;
                    let Some((px, py)) = project(c.pos.x, c.pos.y, wy) else {
                        continue;
                    };
                    let g = c.coloration();
                    if cell_px >= BODY_CELL_MIN_PX {
                        for (dx, dy, ty) in c.body_layout_for_render() {
                            let bx = px + dx as f32 * cell_px;
                            let by = py + dy as f32 * cell_px;
                            let (h, s) = (cell_px * 0.5, cell_px);
                            draw_rectangle(bx - h - 0.5, by - h - 0.5, s + 1.0, s + 1.0, Color::new(0.0, 0.0, 0.0, 0.5));
                            draw_rectangle(bx - h, by - h, s, s, cell_color(ty, g));
                        }
                    } else {
                        // Body radius in metres (√biomass) → pixels; floor keeps a lone microbe clickable.
                        let r = (CREATURE_RADIUS_M * (c.biomass() as f32).sqrt() * px_per_m).max(CREATURE_MIN_PX);
                        draw_circle(px, py, r + 0.8, Color::new(0.0, 0.0, 0.0, 0.6));
                        draw_circle(px, py, r, Color::new(g, g, g, 1.0));
                    }
                    on_screen += 1;
                }
            } else {
                // BACTERIAL MATS — aggregate creatures per column (count + summed coloration), then
                // draw one coverage tile per occupied column: alpha ramps with colony density,
                // greyscale = the colony's mean coloration. Adjacent dense columns tile into a mat.
                let mut bucket: std::collections::HashMap<(usize, usize), (u32, f32)> =
                    std::collections::HashMap::new();
                for c in &sim.creatures {
                    let (cx, cy) = sim::column_index(c.pos);
                    let e = bucket.entry((cx, cy)).or_insert((0, 0.0));
                    e.0 += 1;
                    e.1 += c.coloration();
                }
                let tile = (VOX * px_per_m).max(1.0); // a column footprint in px
                for ((cx, cy), (count, colsum)) in bucket {
                    let wx = (cx as f32 + 0.5) * VOX;
                    let wz = (cy as f32 + 0.5) * VOX;
                    let wy = terrain.height(cx as i32, cy as i32) as f32 * VOX + 0.5;
                    let Some((px, py)) = project(wx, wz, wy) else {
                        continue;
                    };
                    let g = colsum / count as f32; // mean coloration of the colony
                    let a = (count as f32 / MAT_FULL_COUNT).min(MAT_MAX_ALPHA);
                    draw_rectangle(px - tile * 0.5, py - tile * 0.5, tile, tile, Color::new(g, g, g, a));
                    on_screen += count as usize;
                }
            }
        }

        // (The HUD/stats text is now the egui panels rendered at the start of the frame and
        // composited by `egui_macroquad::draw()` at the end; see the GUI pass above.)

        // (The env-field minimap is now the egui minimap panel — top-right — built in the GUI pass
        // at the start of the frame; see `ui::minimap`.)

        // Background progress bar — shown while a world is being generated (reseed) OR loaded.
        // Centred near the bottom; same shadow-text convention as the old HUD.
        let bar = gen
            .as_ref()
            .map(|j| (j.progress.clone(), format!("generating world   seed {}", j.seed)))
            .or_else(|| {
                load
                    .as_ref()
                    .map(|j| (j.progress.clone(), "loading world".to_string()))
            });
        if let Some((progress, label_base)) = bar {
            let p = progress.load(std::sync::atomic::Ordering::Relaxed) as f32 / 1000.0;
            let w = screen_width();
            let (bw, bh, margin) = (w * 0.5, 14.0, 24.0);
            let x = (w - bw) * 0.5;
            let y = screen_height() - margin - bh;
            draw_rectangle(x - 2.0, y - 2.0, bw + 4.0, bh + 4.0, Color::new(0.0, 0.0, 0.0, 0.5));
            draw_rectangle(x, y, bw, bh, Color::new(0.12, 0.14, 0.18, 0.9));
            draw_rectangle(x, y, bw * p, bh, Color::new(0.45, 0.75, 1.0, 1.0));
            let label = format!("{label_base}   {:.0}%", p * 100.0);
            draw_text(&label, x + 1.0, y - 6.0, 22.0, Color::new(0.0, 0.0, 0.0, 0.6));
            draw_text(&label, x, y - 7.0, 22.0, Color::new(0.95, 0.97, 1.0, 1.0));
        }

        // Composite the egui panels (built in the GUI pass at the frame's start) over the scene.
        // Its own render pass — drawn last so it sits on top, after creatures and the minimap.
        egui_macroquad::draw();

        // Dev bridge: service deferred screenshots now the frame is FULLY drawn (incl. the egui HUD
        // just composited above). `window` → the whole window back-buffer with the HUD
        // (`get_screen_data`); otherwise the offscreen 3D target only (no HUD, no foreground needed).
        #[cfg(feature = "dev")]
        for (path, window, reply) in pending_shots.drain(..) {
            let img = if window {
                get_screen_data()
            } else {
                capture_target(&scene_rt)
            };
            img.export_png(&path);
            let _ = reply.send(serde_json::json!({"saved": path, "window": window}));
        }

        next_frame().await;
    }
}

/// Read an offscreen render target's pixels into an `Image` ready for PNG export.
/// GPU render targets are stored bottom-up, so the rows are flipped back.
#[cfg(feature = "dev")]
fn capture_target(rt: &RenderTarget) -> Image {
    let mut img = rt.texture.get_texture_data();
    let (w, h) = (img.width as usize, img.height as usize);
    let row = w * 4;
    let bytes = &mut img.bytes;
    for y in 0..h / 2 {
        let (top, bot) = (y * row, (h - 1 - y) * row);
        for i in 0..row {
            bytes.swap(top + i, bot + i);
        }
    }
    img
}

/// Load a shader from `assets/` at runtime (editable without a rebuild), falling back to the copy
/// baked in with `include_str!` if the file isn't reachable.
async fn load_shader(path: &str, baked: &str) -> String {
    macroquad::file::load_string(path).await.unwrap_or_else(|_| baked.to_string())
}

/// The live `SimConfig` (features + params) as JSON, or `null` before the sim exists. Used by the
/// dev bridge's `get_config` and embedded in `status`.
#[cfg(feature = "dev")]
fn config_json(sim: Option<&Sim>) -> serde_json::Value {
    match sim {
        Some(s) => {
            let cfg = s.config();
            let features: serde_json::Map<String, serde_json::Value> =
                cfg.features.pairs().iter().map(|(k, v)| (k.to_string(), serde_json::json!(v))).collect();
            let params: serde_json::Map<String, serde_json::Value> =
                cfg.params.pairs().iter().map(|(k, v)| (k.to_string(), serde_json::json!(v))).collect();
            serde_json::json!({ "features": features, "params": params })
        }
        None => serde_json::Value::Null,
    }
}

/// A metric value as JSON. `u64` checksums are stringified (they exceed JSON's exact-integer range).
#[cfg(feature = "dev")]
fn mv_json(v: MetricValue) -> serde_json::Value {
    match v {
        MetricValue::Scalar(x) => serde_json::json!(x),
        MetricValue::Checksum(h) => serde_json::json!(h.to_string()),
    }
}

/// `{ latest: {id: value…}, series: [[tick, value]…] | null }` — the latest of every metric, plus
/// the time-series of `id` (capped to the last `last` samples) if requested.
#[cfg(feature = "dev")]
fn metrics_json(reg: &MetricRegistry, id: Option<&str>, last: Option<usize>) -> serde_json::Value {
    let latest: serde_json::Map<String, serde_json::Value> = reg
        .ids()
        .filter_map(|name| reg.latest(name).map(|v| (name.to_string(), mv_json(v))))
        .collect();
    let series = id.and_then(|name| reg.series(name)).map(|s| {
        let start = last.map(|k| s.len().saturating_sub(k)).unwrap_or(0);
        s[start..].iter().map(|(t, v)| serde_json::json!([t, mv_json(*v)])).collect::<Vec<_>>()
    });
    serde_json::json!({ "latest": latest, "series": series })
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
