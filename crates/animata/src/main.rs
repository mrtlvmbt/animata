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

// The simulation + world model live in the graphics-free `animata-sim` crate. The renderer only
// needs these modules by name; the rest (genome/grid/rng/tectonics/erosion/hydrology) are internal
// to the sim. `Vec2` comes from the same glam major macroquad re-exports, so types line up.
use animata_sim::{clock, config, sim, terrain};

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

/// Max zoom-out (visible world height): frame the whole map with margin — the coarse tier
/// covers all of it, so there are no empty edges however far out you go.
fn max_zoom() -> f32 {
    COLS.max(ROWS) as f32 * VOX * 1.2
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
#[derive(Clone, Copy, PartialEq, Eq)]
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
    /// The views drawn as a 2D field minimap (vs the 3D scene reshade / no overlay).
    fn is_field_map(self) -> bool {
        matches!(
            self,
            DebugView::Temp | DebugView::Moist | DebugView::WaterDist | DebugView::Slope | DebugView::Biomass
        )
    }
    /// Views whose field changes over time (biomass regrows / is grazed) → the minimap must be
    /// rebuilt every frame, not cached by seed.
    fn is_dynamic(self) -> bool {
        matches!(self, DebugView::Biomass)
    }
}

/// Build a small colourmap texture of a per-column environment field for the debug minimap.
/// Samples the whole map down to a fixed pixel size, so the cost is bounded. Static fields are
/// cached (paid on a view/seed change); the dynamic biomass field is rebuilt each frame at the
/// current `tick`. Ramps read at a glance: temp blue→red, moisture tan→teal, water-distance
/// bright(near)→dark(far), slope dark→yellow, biomass barren brown→lush green.
fn build_field_minimap(t: &VoxelTerrain, view: DebugView, tick: u64) -> Texture2D {
    const MW: usize = 220;
    let mh = (MW * ROWS / COLS).max(1);
    let mut img = Image::gen_image_color(MW as u16, mh as u16, BLANK);
    for py in 0..mh {
        for px in 0..MW {
            let x = (px * COLS / MW).min(COLS - 1);
            let y = (py * ROWS / mh).min(ROWS - 1);
            let c = match view {
                DebugView::Temp => {
                    let v = t.temperature_at(x, y);
                    Color::new(v, 0.15, 1.0 - v, 1.0) // cold blue → hot red
                }
                DebugView::Moist => {
                    let v = t.moisture_at(x, y);
                    Color::new(0.65 * (1.0 - v) + 0.1, 0.35 + 0.45 * v, 0.25 + 0.5 * v, 1.0) // dry tan → wet teal
                }
                DebugView::WaterDist => {
                    let f = t.water_dist_at(x, y) as f32 / 255.0;
                    if f == 0.0 {
                        Color::new(0.2, 0.5, 1.0, 1.0) // water itself
                    } else {
                        let b = 1.0 - 0.85 * f; // near bright → far dark
                        Color::new(b, b, b, 1.0)
                    }
                }
                DebugView::Slope => {
                    let v = t.slope_at(x, y); // flat dark → steep yellow-white
                    Color::new(v, v, 0.25 * v, 1.0)
                }
                DebugView::Biomass => {
                    if t.is_water(x, y) {
                        Color::new(0.18, 0.32, 0.5, 1.0) // water: no vegetation
                    } else {
                        let v = t.biomass_at(x, y, tick); // barren brown → lush green
                        Color::new(0.45 * (1.0 - v) + 0.1, 0.25 + 0.6 * v, 0.12, 1.0)
                    }
                }
                _ => BLANK,
            };
            img.set_pixel(px as u32, py as u32, c);
        }
    }
    Texture2D::from_image(&img)
}

#[macroquad::main(window_conf)]
async fn main() {
    let mut cam = IsoCam::new();
    let mut seed: u64 = 1;
    // The world is generated on a background thread so the first frame (and every regen)
    // never blocks the render loop. `terrain` is `None` until the initial job finishes.
    let mut terrain: Option<VoxelTerrain> = None;
    let mut gen: Option<GenJob> = Some(spawn_gen(seed));

    // Chunk meshes are STREAMED around the camera (see `Streamer`) rather than all built
    // up front — the world model is fully resident but the meshes are not, so a ×16 map
    // stays within memory. The streamer fills in each frame from `terrain`.
    let pipeline;
    let water_pipe;
    let mut streamer = Streamer::new();
    {
        let InternalGlContext {
            quad_context: ctx, ..
        } = unsafe { get_internal_gl() };
        pipeline = chunk_pipeline(ctx);
        water_pipe = water_pipeline(ctx);
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
    let mut show_info = true;
    // Sim time base (S2). The main loop schedules fixed sub-steps from the real frame `dt`
    // (`clock.substeps`) and drives one `sim.step` per sub-step; `P` pauses. `advance` stays a
    // pure counter (HUD/day-frac). The creature sim (C0) is created once the world is ready.
    let mut clock = WorldClock::new();
    let mut sim: Option<Sim> = None;
    // `G` cycles the debug view: off → Topo (GPU height/depth, water hidden) → Temp → Moist
    // → WaterDist → off. Topo reshades the 3D scene; the climate/water-dist modes overlay a
    // colourmap MINIMAP of the per-column field (the live consumer of the S1 env getters, so
    // they verify visually — poles cold / equator hot — and aren't dead code in any build).
    let mut debug_view = DebugView::None;
    // Cached minimap texture for the field views, rebuilt only when the view or seed changes
    // (sampling the field every frame would be wasteful). `None` for the Off/Topo views.
    let mut field_map: Option<(DebugView, u64, Texture2D)> = None;
    // `H` hides the translucent water surface, baring the seabed/terrain underneath.
    let mut water_on = true;
    // `J` toggles the WATER/LAND mask: land flat grey, generation-flagged water flat blue —
    // dry cells that should be flooded show as grey holes inside the blue (a gen bug probe).
    let mut mask = false;
    // `O` toggles the dark step-edge outline (the contour strips baked along every terrace
    // rim). On by default; off bares the plain shaded faces.
    let mut outline = true;
    // Left-drag pans the map: the ground point grabbed on press stays under the cursor.
    let mut grab: Option<Vec2> = None;

    // Dev bridge: localhost JSON-RPC for driving/inspecting the viewer (see
    // DEV_BRIDGE.md). Off unless built with `--features dev`.
    #[cfg(feature = "dev")]
    let bridge = dev_bridge::spawn(8127);
    #[cfg(feature = "dev")]
    let mut pending_shots: Vec<(String, std::sync::mpsc::Sender<serde_json::Value>)> = Vec::new();

    loop {
        let dt = get_frame_time();
        // Pick up a finished background world (non-blocking). On readiness, swap it in and
        // reset the streamer so meshes rebuild around the camera from the new terrain.
        if let Some(job) = &gen {
            if let Ok(t) = job.rx.try_recv() {
                // Seed the creature population from the new world (deterministic from its seed).
                sim = Some(Sim::new(seed, &t));
                terrain = Some(t);
                gen = None;
                let InternalGlContext { quad_context: ctx, .. } = unsafe { get_internal_gl() };
                streamer.clear(ctx);
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
                sim.step(terrain, clock.tick());
            }
        }

        // ---- Input (no GUI) ----
        if is_key_pressed(KeyCode::I) {
            show_info = !show_info;
        }
        if is_key_pressed(KeyCode::G) {
            debug_view = debug_view.next();
        }
        if is_key_pressed(KeyCode::H) {
            water_on = !water_on;
        }
        if is_key_pressed(KeyCode::P) {
            clock.paused = !clock.paused;
        }
        if is_key_pressed(KeyCode::J) {
            mask = !mask;
        }
        if is_key_pressed(KeyCode::O) {
            outline = !outline;
        }
        let wheel = mouse_wheel().1;
        if wheel != 0.0 {
            // Zoom toward the cursor: keep the ground point under the mouse fixed by
            // shifting the target by how much that point would otherwise move.
            let before = ground_under_cursor(&cam);
            cam.zoom = (cam.zoom * (1.0 - wheel.signum() * 0.1)).clamp(8.0, max_zoom());
            let after = ground_under_cursor(&cam);
            cam.target.x += before.x - after.x;
            cam.target.z += before.y - after.y;
        }
        // Left-drag pan: lock the grabbed ground point under the moving cursor.
        if is_mouse_button_pressed(MouseButton::Left) {
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
        if is_mouse_button_down(MouseButton::Right) {
            if let Some(t) = &mut terrain {
                let g = ground_under_cursor(&cam);
                let (gx, gy) = ((g.x / VOX).floor() as i32, (g.y / VOX).floor() as i32);
                let r = 24i32;
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
            let speed = cam.zoom * dt * 0.5; // pan faster when zoomed out
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
                        cam.zoom = v.clamp(8.0, max_zoom());
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
                    sim = Some(Sim::new(seed, &t)); // re-seed the population from the new world
                    terrain = Some(t);
                    let InternalGlContext { quad_context: ctx, .. } = unsafe { get_internal_gl() };
                    streamer.clear(ctx);
                    let _ = reply.send(serde_json::json!({"seed": seed}));
                }
                dev_bridge::Cmd::Render { water: w, topo: tp } => {
                    if let Some(w) = w {
                        water_on = w;
                    }
                    if let Some(tp) = tp {
                        // `topo` stays a bool over the wire: true selects the Topo view, false
                        // clears to Off (the climate minimaps are driven by `G` interactively).
                        debug_view = if tp { DebugView::Topo } else { DebugView::None };
                    }
                    let _ = reply.send(serde_json::json!({"water": water_on, "topo": debug_view == DebugView::Topo}));
                }
                dev_bridge::Cmd::Screenshot(path) => {
                    pending_shots.push((path, reply)); // serviced post-draw below
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
        let mut drawn = 0usize;
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

        // Creatures: LOD dots over the blitted scene (C0). Project each creature's column-top
        // world point through the same camera matrix; draw a small dot, tinted by lineage so
        // clusters are visible. Off-screen ones are culled by the projection.
        let mut on_screen = 0usize;
        if let (Some(sim), Some(terrain)) = (sim.as_ref(), terrain.as_ref()) {
            let (sw, sh) = (screen_width(), screen_height());
            for c in &sim.creatures {
                let (cx, cy) = sim::column_index(c.pos);
                let wy = terrain.height(cx as i32, cy as i32) as f32 * VOX + 0.5;
                let clip = vp * vec4(c.pos.x, wy, c.pos.y, 1.0);
                if clip.w <= 0.0 {
                    continue;
                }
                let (nx, ny) = (clip.x / clip.w, clip.y / clip.w);
                if !(-1.0..=1.0).contains(&nx) || !(-1.0..=1.0).contains(&ny) {
                    continue;
                }
                let (px, py) = ((nx * 0.5 + 0.5) * sw, (1.0 - (ny * 0.5 + 0.5)) * sh);
                // Fill = the creature's evolved coloration (greyscale, dark..light) so camouflage
                // is visible: cryptic creatures blend into their biome, conspicuous ones stand out.
                // A thin dark ring keeps even a light dot legible over any terrain.
                let g = c.coloration();
                // Dot size grows with body size (√biomass) so multicellular creatures read bigger.
                let r = 2.0 + 1.2 * (c.biomass() as f32).sqrt();
                draw_circle(px, py, r + 0.8, Color::new(0.0, 0.0, 0.0, 0.6));
                draw_circle(px, py, r, Color::new(g, g, g, 1.0));
                on_screen += 1;
            }
        }

        // Minimal debug readout (toggle `I`): fps + frame time. Drawn with a 1px
        // shadow so it stays legible over any terrain colour.
        // Build the readout unconditionally (reads `drawn` in every build config),
        // draw it only when toggled on.
        let (det, crs) = (streamer.detail.len(), streamer.coarse.len());
        let mode = if mask {
            "   [WATER/LAND mask, J]"
        } else {
            match debug_view {
                DebugView::Topo => "   [TOPO: height/depth, G]",
                DebugView::Temp => "   [TEMP map, G]",
                DebugView::Moist => "   [MOIST map, G]",
                DebugView::WaterDist => "   [WATER-DIST map, G]",
                DebugView::Slope => "   [SLOPE map, G]",
                DebugView::Biomass => "   [BIOMASS map, G — right-drag to graze]",
                DebugView::None if !water_on => "   [water off, H]",
                DebugView::None => "",
            }
        };
        let outl = if outline { "" } else { "   [outline off, O]" };
        let line = format!(
            "{fps:.0} fps   {frame_ms:.2} ms   seed {seed}   {COLS}x{ROWS} m   draws {drawn}   detail {det} coarse {crs}{mode}{outl}"
        );
        // Sim-clock + population readout. The creature count is the always-built consumer of
        // the sim getters; absent until the world is ready.
        let pause = if clock.paused { "  [PAUSED, P]" } else { "" };
        let life = match (sim.as_ref(), terrain.as_ref()) {
            (Some(s), Some(t)) => {
                let (multi, _) = s.complexity_mix();
                let m = s.stratum_mix(t);
                format!(
                    "   pop {} E {:.0}   bm {:.2}   multi {:.0}% carn {:.0}% auto {:.0}%   species {} niches {}   allop {:.2} crypsis {:.2}   nutri {:.2}   strata u{:.0}/s{:.0}/a{:.0}/w{:.0}   on-scr {on_screen}",
                    s.population(), s.avg_energy(), s.avg_biomass(), multi * 100.0,
                    s.frac_carnivore() * 100.0, s.frac_autotroph() * 100.0, s.species_count(), s.niche_coverage(t),
                    s.thermal_correlation(t), s.crypsis_correlation(t), s.avg_nutrient(t, clock.tick()),
                    m[0] * 100.0, m[1] * 100.0, m[2] * 100.0, m[3] * 100.0
                )
            }
            _ => String::new(),
        };
        let clock_line = format!(
            "tick {}   sim {:.1}s   day {:.2}   x{:.1}{life}{pause}",
            clock.tick(), clock.sim_time(), clock.day_frac(), clock.time_scale
        );
        if show_info {
            draw_text(&line, 9.0, 23.0, 24.0, Color::new(0.0, 0.0, 0.0, 0.6));
            draw_text(&line, 8.0, 22.0, 24.0, Color::new(0.95, 0.97, 1.0, 1.0));
            draw_text(&clock_line, 9.0, 45.0, 22.0, Color::new(0.0, 0.0, 0.0, 0.6));
            draw_text(&clock_line, 8.0, 44.0, 22.0, Color::new(0.85, 0.92, 1.0, 1.0));
        }

        // Field colourmap minimap (the env-getter consumer): rebuild the texture on a view/seed
        // change (static fields) or every frame (the dynamic biomass field), then blit it
        // top-right with a label. Off for the None/Topo views (Topo reshades the 3D scene).
        if debug_view.is_field_map() {
            if let Some(t) = &terrain {
                let stale = debug_view.is_dynamic()
                    || field_map
                        .as_ref()
                        .map(|(v, s, _)| *v != debug_view || *s != seed)
                        .unwrap_or(true);
                if stale {
                    field_map = Some((debug_view, seed, build_field_minimap(t, debug_view, clock.tick())));
                }
            }
            if let Some((_, _, tex)) = &field_map {
                let (mw, mh) = (tex.width() * 1.4, tex.height() * 1.4);
                let (mx, my) = (screen_width() - mw - 12.0, 40.0);
                draw_rectangle(mx - 3.0, my - 3.0, mw + 6.0, mh + 6.0, Color::new(0.0, 0.0, 0.0, 0.6));
                draw_texture_ex(tex, mx, my, WHITE,
                    DrawTextureParams { dest_size: Some(vec2(mw, mh)), ..Default::default() });
            }
        } else if field_map.is_some() {
            field_map = None; // drop the cached texture when leaving the field views
        }

        // Background generation progress bar (only while a world is being built). Centred
        // near the bottom; same shadow-text convention as the HUD above.
        if let Some(job) = &gen {
            let p = job.progress.load(std::sync::atomic::Ordering::Relaxed) as f32 / 1000.0;
            let w = screen_width();
            let (bw, bh, margin) = (w * 0.5, 14.0, 24.0);
            let x = (w - bw) * 0.5;
            let y = screen_height() - margin - bh;
            draw_rectangle(x - 2.0, y - 2.0, bw + 4.0, bh + 4.0, Color::new(0.0, 0.0, 0.0, 0.5));
            draw_rectangle(x, y, bw, bh, Color::new(0.12, 0.14, 0.18, 0.9));
            draw_rectangle(x, y, bw * p, bh, Color::new(0.45, 0.75, 1.0, 1.0));
            let label = format!("generating world   seed {}   {:.0}%", job.seed, p * 100.0);
            draw_text(&label, x + 1.0, y - 6.0, 22.0, Color::new(0.0, 0.0, 0.0, 0.6));
            draw_text(&label, x, y - 7.0, 22.0, Color::new(0.95, 0.97, 1.0, 1.0));
        }

        // Dev bridge: service deferred screenshots now the frame is fully drawn.
        // Read the offscreen target (fresh, pre-present) rather than the window
        // back-buffer, so capture doesn't need the window foregrounded.
        #[cfg(feature = "dev")]
        for (path, reply) in pending_shots.drain(..) {
            let img = capture_target(&scene_rt);
            img.export_png(&path);
            let _ = reply.send(serde_json::json!({"saved": path}));
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

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
