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

mod config;
#[cfg(feature = "dev")]
mod dev_bridge;
mod erosion;
mod hydrology;
mod tectonics;
mod terrain;

use config::*;
use macroquad::miniquad::{
    Bindings, BlendFactor, BlendState, BlendValue, BufferSource, BufferType, BufferUsage,
    Comparison, Equation, PassAction, Pipeline, PipelineParams, RenderingBackend, ShaderMeta,
    ShaderSource, UniformBlockLayout, UniformDesc, UniformType, UniformsSource, VertexAttribute,
    VertexFormat,
};
use macroquad::prelude::*;
use terrain::{cell_biome, cell_height, feature_unit, BiomeKind, VoxelTerrain};

fn window_conf() -> Conf {
    Conf {
        window_title: "animata — voxel world".to_owned(),
        window_width: WIN_W,
        window_height: WIN_H,
        high_dpi: true,
        ..Default::default()
    }
}

/// Orthographic isometric camera: looks down a fixed iso angle at `target`, with
/// `zoom` = world-height visible (smaller = closer) and `yaw` rotating in 90° steps.
struct IsoCam {
    target: Vec3,
    zoom: f32,
    yaw: f32,
}

impl IsoCam {
    fn new() -> Self {
        IsoCam {
            target: vec3(COLS as f32 * VOX * 0.5, 0.0, ROWS as f32 * VOX * 0.5),
            zoom: 170.0, // frames the whole base map
            yaw: 0.0,
        }
    }

    /// Build the macroquad camera. True-isometric elevation (~35.264°); azimuth
    /// 45° + yaw. Orthographic, so distance doesn't change size — `zoom` (fovy) is
    /// the visible world height.
    fn camera(&self) -> Camera3D {
        let elev = 35.264_f32.to_radians();
        let azim = 45_f32.to_radians() + self.yaw;
        let dir = vec3(azim.cos() * elev.cos(), elev.sin(), azim.sin() * elev.cos());
        Camera3D {
            position: self.target + dir * 400.0,
            target: self.target,
            up: vec3(0.0, 1.0, 0.0),
            fovy: self.zoom,
            aspect: Some(screen_width() / screen_height()),
            projection: Projection::Orthographics,
            render_target: None,
            viewport: None,
            z_near: 0.1,
            z_far: 3000.0,
        }
    }
}

/// Offscreen colour+depth target the scene renders into. The depth attachment is the
/// point: `render_target()` makes a colour-only target with no depth buffer, so a
/// depth-testing 3D camera drawing into it loses occlusion (far faces overdraw near).
fn new_scene_target(w: u32, h: u32) -> RenderTarget {
    let rt = render_target_ex(
        w,
        h,
        RenderTargetParams {
            depth: true,
            ..Default::default()
        },
    );
    rt.texture.set_filter(FilterMode::Nearest);
    rt
}

/// Conservative frustum cull: project the AABB's 8 corners through the camera's
/// view-projection matrix and keep the mesh unless every corner falls off the same
/// screen edge. Cheap (8 mat·vec per mesh) and yaw-agnostic; only the x/y screen
/// bounds are tested (the ortho z range comfortably covers the world depth).
fn aabb_in_view(vp: &Mat4, lo: Vec3, hi: Vec3) -> bool {
    let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for x in [lo.x, hi.x] {
        for y in [lo.y, hi.y] {
            for z in [lo.z, hi.z] {
                let c = *vp * vec4(x, y, z, 1.0);
                let w = c.w.abs().max(1e-6);
                let (nx, ny) = (c.x / w, c.y / w);
                minx = minx.min(nx);
                maxx = maxx.max(nx);
                miny = miny.min(ny);
                maxy = maxy.max(ny);
            }
        }
    }
    const M: f32 = 0.02; // small margin so chunks aren't popped at the very edge
    !(maxx < -1.0 - M || minx > 1.0 + M || maxy < -1.0 - M || miny > 1.0 + M)
}

// ---- Retained-mode chunk rendering (persistent GPU buffers) ----
//
// macroquad's `draw_mesh` re-uploads a mesh's vertices into its batch buffer every
// frame, so drawing a big world costs O(visible vertices) per frame even when the
// geometry never changes. Instead we upload each chunk mesh to an immutable GPU
// buffer ONCE (here, via raw miniquad), and each frame issue a cheap draw call per
// *visible* chunk — per-frame cost becomes O(visible chunk count). Mirrors macroquad's
// own 3D vertex layout so we can feed it the `Mesh` vertices unchanged.

const CHUNK_VERT: &str = r#"#version 100
attribute vec3 position;
attribute vec4 color0;
uniform mat4 mvp;
varying lowp vec4 color;
varying highp float vy;
void main() {
    gl_Position = mvp * vec4(position, 1.0);
    color = color0 / 255.0;
    vy = position.y;
}"#;

// `dbg.x > 0.5` switches to a TOPO debug view: colour by absolute height with per-level
// contour banding, so the cube topology / land height / underwater DEPTH are readable
// (underwater = a cold gradient by depth, land = a terrain ramp). Water is simply not
// drawn in this mode (the render loop skips it), so the bed shape is laid bare.
const CHUNK_FRAG: &str = r#"#version 100
varying lowp vec4 color;
varying highp float vy;
uniform highp vec4 dbg;
void main() {
    if (dbg.x > 0.5) {
        // Quantise to integer levels and colour each by height, with STRONG per-level
        // brightness alternation + a dark line every 5 levels — so every cube step reads
        // as its own band (a topographic-map look). Waterline is at level 6.
        highp float lv = floor(vy);
        highp float t = clamp(lv / 40.0, 0.0, 1.0);
        highp vec3 c = mix(vec3(0.03, 0.08, 0.35), vec3(0.10, 0.65, 0.85), smoothstep(0.0, 0.15, t)); // depth -> shallow
        c = mix(c, vec3(0.92, 0.86, 0.55), smoothstep(0.15, 0.20, t)); // shore
        c = mix(c, vec3(0.28, 0.66, 0.28), smoothstep(0.20, 0.42, t)); // lowland
        c = mix(c, vec3(0.82, 0.74, 0.30), smoothstep(0.42, 0.60, t)); // hills
        c = mix(c, vec3(0.58, 0.40, 0.28), smoothstep(0.60, 0.80, t)); // mountain
        c = mix(c, vec3(1.0, 1.0, 1.0), smoothstep(0.80, 1.0, t)); // peak
        // MONOTONIC by height (no per-level brightness flip — that read as false
        // alternating ridges on a smooth slope). Only a thin dark contour line every 4
        // levels for scale, like a bathymetric map: a bowl reads as a smooth ramp.
        highp float contour = (mod(lv, 4.0) < 0.5) ? 0.62 : 1.0;
        gl_FragColor = vec4(c * contour, 1.0);
    } else {
        gl_FragColor = color;
    }
}"#;

#[repr(C)]
struct ChunkUniforms {
    mvp: Mat4,
    dbg: Vec4,
}

/// One chunk's geometry living in immutable GPU buffers, plus its world AABB for culling.
struct GpuChunk {
    bindings: Bindings,
    n_idx: i32,
    lo: Vec3,
    hi: Vec3,
}

/// Build the pipeline once. Vertex layout matches macroquad's `Vertex` (40 bytes:
/// position f32x3, uv f32x2, color u8x4, normal f32x4) so the `Mesh` buffers upload
/// as-is; the shader only consumes position + colour. Alpha blending for the water pass.
fn chunk_pipeline(ctx: &mut dyn RenderingBackend) -> Pipeline {
    let shader = ctx
        .new_shader(
            ShaderSource::Glsl { vertex: CHUNK_VERT, fragment: CHUNK_FRAG },
            ShaderMeta {
                images: vec![],
                uniforms: UniformBlockLayout {
                    uniforms: vec![
                        UniformDesc::new("mvp", UniformType::Mat4),
                        UniformDesc::new("dbg", UniformType::Float4),
                    ],
                },
            },
        )
        .expect("chunk shader");
    ctx.new_pipeline(
        &[macroquad::miniquad::BufferLayout::default()],
        &[
            VertexAttribute::new("position", VertexFormat::Float3),
            VertexAttribute::new("texcoord", VertexFormat::Float2),
            VertexAttribute::new("color0", VertexFormat::Byte4),
            VertexAttribute::new("normal", VertexFormat::Float4),
        ],
        shader,
        PipelineParams {
            depth_test: Comparison::LessOrEqual,
            depth_write: true,
            color_blend: Some(BlendState::new(
                Equation::Add,
                BlendFactor::Value(BlendValue::SourceAlpha),
                BlendFactor::OneMinusValue(BlendValue::SourceAlpha),
            )),
            ..Default::default()
        },
    )
}

/// Upload built chunk meshes to immutable GPU buffers.
fn upload_chunks(ctx: &mut dyn RenderingBackend, batches: &[Batch]) -> Vec<GpuChunk> {
    batches
        .iter()
        .map(|b| {
            let vb = ctx.new_buffer(
                BufferType::VertexBuffer,
                BufferUsage::Immutable,
                BufferSource::slice(&b.mesh.vertices),
            );
            let ib = ctx.new_buffer(
                BufferType::IndexBuffer,
                BufferUsage::Immutable,
                BufferSource::slice(&b.mesh.indices),
            );
            GpuChunk {
                bindings: Bindings {
                    vertex_buffers: vec![vb],
                    index_buffer: ib,
                    images: vec![],
                },
                n_idx: b.mesh.indices.len() as i32,
                lo: b.lo,
                hi: b.hi,
            }
        })
        .collect()
}

/// Release a chunk set's GPU buffers (before re-uploading on reseed).
fn free_chunks(ctx: &mut dyn RenderingBackend, chunks: &[GpuChunk]) {
    for c in chunks {
        ctx.delete_buffer(c.bindings.vertex_buffers[0]);
        ctx.delete_buffer(c.bindings.index_buffer);
    }
}

// ---- Chunk streaming -------------------------------------------------------------
//
// Holding every chunk's GPU mesh at once is fine at ×8 (~0.7 GB) but blows past memory
// at ×16 (~2.9 GB). The world MODEL (heights/biomes/water) stays fully resident — it's
// cheap — but the meshes are streamed: only chunks within a radius of the camera are
// built + uploaded, and chunks that fall outside are freed. A per-frame BUILD BUDGET
// amortises the meshing so entering new terrain doesn't spike a frame.

/// One loaded chunk's GPU geometry (a dense chunk can split into several batches), plus
/// the LOD it was built at (so the streamer rebuilds it when its ring changes).
struct LoadedChunk {
    opaque: Vec<GpuChunk>,
    water: Vec<GpuChunk>,
    lod: u32,
}

/// New chunk meshes built+uploaded per frame (amortises panning + LOD changes).
const BUILD_BUDGET: usize = 24;
/// LOD ring radii (chunks from the camera): full detail within `LOD0`, half-res out to
/// `LOD1`, quarter-res out to `MAX_LOAD_RADIUS`. Far chunks are ~16× cheaper, so a much
/// wider area is affordable — but the draw COUNT (one per chunk) still grows with the
/// window, which the zoom cap bounds.
const LOD0_RADIUS: i32 = 5;
const LOD1_RADIUS: i32 = 12;
const MAX_LOAD_RADIUS: i32 = 24;
const LOAD_MARGIN: i32 = 2;

/// LOD for a chunk at Chebyshev distance `d` (chunks) from the camera centre.
fn lod_for(d: i32) -> u32 {
    if d <= LOD0_RADIUS {
        0
    } else if d <= LOD1_RADIUS {
        1
    } else {
        2
    }
}

struct Streamer {
    loaded: std::collections::HashMap<(usize, usize), LoadedChunk>,
}

impl Streamer {
    fn new() -> Self {
        Streamer { loaded: std::collections::HashMap::new() }
    }

    fn loaded_count(&self) -> usize {
        self.loaded.len()
    }

    /// Free everything (on reseed / map change).
    fn clear(&mut self, ctx: &mut dyn RenderingBackend) {
        for lc in self.loaded.values() {
            free_chunks(ctx, &lc.opaque);
            free_chunks(ctx, &lc.water);
        }
        self.loaded.clear();
    }

    /// Reconcile the loaded set with the window around `center` (radius `radius`): free
    /// chunks that left, then build up to `BUILD_BUDGET` of the chunks that are missing OR
    /// loaded at the wrong LOD (nearest first), so the view fills inward, distant chunks
    /// drop to coarse LOD, and the per-frame cost stays bounded.
    fn update(&mut self, ctx: &mut dyn RenderingBackend, t: &VoxelTerrain, center: (i32, i32), radius: i32) {
        let (ccx, ccy) = center;
        let (cx0, cx1) = ((ccx - radius).max(0), (ccx + radius).min(t.chunks_x as i32 - 1));
        let (cy0, cy1) = ((ccy - radius).max(0), (ccy + radius).min(t.chunks_y as i32 - 1));
        self.loaded.retain(|&(cx, cy), lc| {
            let inside = (cx0..=cx1).contains(&(cx as i32)) && (cy0..=cy1).contains(&(cy as i32));
            if !inside {
                free_chunks(ctx, &lc.opaque);
                free_chunks(ctx, &lc.water);
            }
            inside
        });
        // Chunks needing a (re)build: missing, or loaded at a now-wrong LOD.
        let mut todo: Vec<(i64, usize, usize, u32)> = Vec::new();
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                let d = (cx - ccx).abs().max((cy - ccy).abs());
                let want = lod_for(d);
                let key = (cx as usize, cy as usize);
                let needs = match self.loaded.get(&key) {
                    None => true,
                    Some(lc) => lc.lod != want,
                };
                if needs {
                    let (dx, dy) = ((cx - ccx) as i64, (cy - ccy) as i64);
                    todo.push((dx * dx + dy * dy, key.0, key.1, want));
                }
            }
        }
        if !todo.is_empty() {
            todo.sort_unstable_by_key(|m| m.0);
            for &(_, cx, cy, lod) in todo.iter().take(BUILD_BUDGET) {
                if let Some(old) = self.loaded.remove(&(cx, cy)) {
                    free_chunks(ctx, &old.opaque);
                    free_chunks(ctx, &old.water);
                }
                let (o, w) = build_chunk_mesh(t, cx, cy, lod);
                let opaque = upload_chunks(ctx, &o);
                let water = upload_chunks(ctx, &w);
                self.loaded.insert((cx, cy), LoadedChunk { opaque, water, lod });
            }
        }
    }
}

/// Camera centre chunk + load radius (chunks) from the orthographic view span. `zoom` is
/// the visible world height; the iso tilt and aspect widen the ground footprint, so a
/// factor covers the corners, plus the margin and the hard cap.
fn load_window(cam: &IsoCam) -> ((i32, i32), i32) {
    let ccx = (cam.target.x / (CHUNK as f32 * VOX)).floor() as i32;
    let ccy = (cam.target.z / (CHUNK as f32 * VOX)).floor() as i32;
    let aspect = screen_width() / screen_height().max(1.0);
    let span_world = cam.zoom * aspect.max(1.0) * 1.6; // ground span incl. iso tilt
    let radius = ((span_world * 0.5) / (CHUNK as f32 * VOX)).ceil() as i32 + LOAD_MARGIN;
    ((ccx, ccy), radius.clamp(1, MAX_LOAD_RADIUS))
}

/// Max zoom-out (visible world height): chosen so the view span stays within the loaded
/// window (`MAX_LOAD_RADIUS`), so the camera never out-runs the streamed terrain.
fn max_zoom() -> f32 {
    let aspect = screen_width() / screen_height().max(1.0);
    // Invert load_window: radius = zoom*aspect*1.6*0.5/(CHUNK*VOX) + MARGIN ⇒ solve for zoom.
    let usable = (MAX_LOAD_RADIUS - LOAD_MARGIN).max(1) as f32;
    usable * (CHUNK as f32 * VOX) * 2.0 / (aspect.max(1.0) * 1.6)
}

#[macroquad::main(window_conf)]
async fn main() {
    let mut cam = IsoCam::new();
    let mut seed: u64 = 1;
    let mut terrain = VoxelTerrain::new(seed);

    // Chunk meshes are STREAMED around the camera (see `Streamer`) rather than all built
    // up front — the world model is fully resident but the meshes are not, so a ×16 map
    // stays within memory. The streamer fills in each frame from `terrain`.
    let pipeline;
    let mut streamer = Streamer::new();
    {
        let InternalGlContext { quad_context: ctx, .. } = unsafe { get_internal_gl() };
        pipeline = chunk_pipeline(ctx);
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
    // `G` toggles the TOPO debug view (height colourmap, water hidden) — reveals the cube
    // topology + underwater bed shape that the shaded/translucent normal view obscures.
    let mut topo = false;

    // Dev bridge: localhost JSON-RPC for driving/inspecting the viewer (see
    // DEV_BRIDGE.md). Off unless built with `--features dev`.
    #[cfg(feature = "dev")]
    let bridge = dev_bridge::spawn(8127);
    #[cfg(feature = "dev")]
    let mut pending_shots: Vec<(String, std::sync::mpsc::Sender<serde_json::Value>)> = Vec::new();

    loop {
        let dt = get_frame_time();
        // Smooth the frame-time readout so it doesn't jitter.
        frame_ms = 0.9 * frame_ms + 0.1 * dt * 1000.0;
        if dt > 0.0 {
            fps = 0.9 * fps + 0.1 / dt;
        }

        // ---- Input (no GUI) ----
        if is_key_pressed(KeyCode::I) {
            show_info = !show_info;
        }
        if is_key_pressed(KeyCode::G) {
            topo = !topo;
        }
        let wheel = mouse_wheel().1;
        if wheel != 0.0 {
            cam.zoom = (cam.zoom * (1.0 - wheel.signum() * 0.1)).clamp(8.0, max_zoom());
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
        // Regenerate the world with a fresh seed.
        if is_key_pressed(KeyCode::R) {
            seed = seed.wrapping_add(1);
            terrain = VoxelTerrain::new(seed);
            let InternalGlContext { quad_context: ctx, .. } = unsafe { get_internal_gl() };
            streamer.clear(ctx); // the streamer reloads around the camera next frame
        }

        // ---- Dev bridge: service queued commands on the main thread ----
        #[cfg(feature = "dev")]
        for req in dev_bridge::take(&bridge) {
            let dev_bridge::Req { cmd, reply } = req;
            match cmd {
                dev_bridge::Cmd::Status => {
                    let _ = reply.send(serde_json::json!({
                        "fps": fps,
                        "frame_ms": frame_ms,
                        "seed": seed,
                        "view": { "cx": cam.target.x, "cz": cam.target.z, "zoom": cam.zoom, "yaw": cam.yaw },
                        "map": { "cols": COLS, "rows": ROWS, "vox_m": VOX, "map_scale": MAP_SCALE,
                                 "loaded_chunks": streamer.loaded_count(), "chunks_total": terrain.chunks_x * terrain.chunks_y },
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
                    seed = s.unwrap_or(seed.wrapping_add(1));
                    terrain = VoxelTerrain::new(seed);
                    let InternalGlContext { quad_context: ctx, .. } = unsafe { get_internal_gl() };
                    streamer.clear(ctx);
                    let _ = reply.send(serde_json::json!({"seed": seed}));
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
        let (center, radius) = load_window(&cam);
        let mut drawn = 0usize;
        {
            let mut gl = unsafe { get_internal_gl() };
            gl.flush(); // flush any pending macroquad 2D before our own pass
            let ctx = gl.quad_context;
            // Stream: load chunks entering the window (budgeted), free those leaving.
            streamer.update(ctx, &terrain, center, radius);
            ctx.begin_pass(
                Some(scene_rt.render_pass.raw_miniquad_id()),
                PassAction::Clear {
                    color: Some((0.53, 0.62, 0.78, 1.0)), // sky
                    depth: Some(1.0),
                    stencil: None,
                },
            );
            ctx.apply_pipeline(&pipeline);
            let dbg = vec4(if topo { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0);
            ctx.apply_uniforms(UniformsSource::table(&ChunkUniforms { mvp: vp, dbg }));
            // Opaque first (fills the depth buffer), then the translucent water across all
            // loaded chunks (skipped in topo mode so the bed topology is visible). Each
            // GPU chunk is frustum-culled by its AABB.
            for lc in streamer.loaded.values() {
                for c in &lc.opaque {
                    if aabb_in_view(&vp, c.lo, c.hi) {
                        ctx.apply_bindings(&c.bindings);
                        ctx.draw(0, c.n_idx, 1);
                        drawn += 1;
                    }
                }
            }
            if !topo {
                for lc in streamer.loaded.values() {
                    for c in &lc.water {
                        if aabb_in_view(&vp, c.lo, c.hi) {
                            ctx.apply_bindings(&c.bindings);
                            ctx.draw(0, c.n_idx, 1);
                            drawn += 1;
                        }
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

        // Minimal debug readout (toggle `I`): fps + frame time. Drawn with a 1px
        // shadow so it stays legible over any terrain colour.
        // Build the readout unconditionally (reads `drawn` in every build config),
        // draw it only when toggled on.
        let loaded = streamer.loaded_count();
        let total_chunks = terrain.chunks_x * terrain.chunks_y;
        let mode = if topo { "   [TOPO: height/depth, G]" } else { "" };
        let line = format!(
            "{fps:.0} fps   {frame_ms:.2} ms   seed {seed}   {COLS}x{ROWS} m   draws {drawn}   chunks {loaded}/{total_chunks}{mode}"
        );
        if show_info {
            draw_text(&line, 9.0, 23.0, 24.0, Color::new(0.0, 0.0, 0.0, 0.6));
            draw_text(&line, 8.0, 22.0, 24.0, Color::new(0.95, 0.97, 1.0, 1.0));
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

// ---- Render-side palette: data-driven biome LUT (representation; kept out of the
// generator). A new biome = a new row here + a `BiomeKind` variant; the hot mesh loop
// just indexes `BIOME_DEFS[id]`, no match. Vegetation kind/density also live here. ----

#[derive(Clone, Copy, PartialEq)]
enum TreeKind {
    None,
    Broadleaf,
    Conifer,
}

#[derive(Clone, Copy)]
struct BiomeDef {
    surface: (f32, f32, f32),
    tree_density: f32,
    tree: TreeKind,
}

const fn def(surface: (f32, f32, f32), tree_density: f32, tree: TreeKind) -> BiomeDef {
    BiomeDef { surface, tree_density, tree }
}

/// Indexed by `BiomeKind::id()` (0..12 used, 12..16 padded). Order matches the enum.
static BIOME_DEFS: [BiomeDef; 16] = [
    def((0.13, 0.32, 0.55), 0.0, TreeKind::None),       // 0 Ocean
    def((0.84, 0.78, 0.54), 0.0, TreeKind::None),       // 1 Beach
    def((0.42, 0.62, 0.30), 0.04, TreeKind::Broadleaf), // 2 Plains
    def((0.20, 0.46, 0.24), 0.30, TreeKind::Broadleaf), // 3 Forest
    def((0.80, 0.70, 0.44), 0.0, TreeKind::None),       // 4 Desert
    def((0.48, 0.46, 0.45), 0.0, TreeKind::None),       // 5 Mountain
    def((0.93, 0.95, 0.98), 0.02, TreeKind::Conifer),   // 6 Snow
    def((0.17, 0.38, 0.29), 0.30, TreeKind::Conifer),   // 7 Taiga
    def((0.62, 0.64, 0.56), 0.0, TreeKind::None),       // 8 Tundra
    def((0.70, 0.66, 0.34), 0.03, TreeKind::Broadleaf), // 9 Savanna
    def((0.31, 0.40, 0.25), 0.14, TreeKind::Broadleaf), // 10 Swamp
    def((0.12, 0.43, 0.17), 0.50, TreeKind::Broadleaf), // 11 Jungle
    def((0.42, 0.62, 0.30), 0.0, TreeKind::None),       // 12-15 padding
    def((0.42, 0.62, 0.30), 0.0, TreeKind::None),
    def((0.42, 0.62, 0.30), 0.0, TreeKind::None),
    def((0.42, 0.62, 0.30), 0.0, TreeKind::None),
];

fn biome_def(biome: BiomeKind) -> &'static BiomeDef {
    &BIOME_DEFS[biome.id() as usize]
}

/// Surface (top-face) base colour per biome.
fn top_rgb(biome: BiomeKind) -> (f32, f32, f32) {
    biome_def(biome).surface
}

/// Side-wall colour for the exposed level `gz` of a column of height `h`: a thin
/// biome "lip" just under the surface, then topsoil, then stone deeper down.
fn strata_rgb(gz: u8, h: u8, biome: BiomeKind) -> (f32, f32, f32) {
    // Rocky biomes are bare stone all the way down — no brown topsoil band, which read as
    // out-of-place dirt specks on mountain/snow cliffs.
    let rocky = matches!(biome, BiomeKind::Mountain | BiomeKind::Snow);
    if gz + 1 == h {
        let (r, g, b) = top_rgb(biome);
        (r * 0.85, g * 0.85, b * 0.85)
    } else if !rocky && gz + 3 >= h {
        (0.42, 0.31, 0.20) // topsoil
    } else {
        (0.40, 0.38, 0.36) // stone
    }
}

// Baked directional face shading (fixed "sun"), so volume reads without lighting.
const SHADE_TOP: f32 = 1.0;
const SHADE_PX: f32 = 0.86;
const SHADE_NX: f32 = 0.62;
const SHADE_PZ: f32 = 0.74;
const SHADE_NZ: f32 = 0.54;

fn shaded(rgb: (f32, f32, f32), s: f32) -> Color {
    Color::new(rgb.0 * s, rgb.1 * s, rgb.2 * s, 1.0)
}

/// A built mesh plus its world-space AABB, so the renderer can frustum-cull it: with a
/// big map most chunks are off-screen, and macroquad re-batches every drawn mesh's
/// vertices each frame, so skipping off-screen ones keeps per-frame cost ∝ what's
/// visible, not ∝ the whole map.
struct Batch {
    mesh: Mesh,
    lo: Vec3,
    hi: Vec3,
}

/// macroquad's `draw_mesh` pushes through the immediate batch buffer, which **clamps**
/// (silently dropping geometry) at `>= 10000` vertices or `>= 5000` indices per call.
/// Indices bind first (6 per quad vs 4 verts), so we split meshes on the index count,
/// keeping a margin for the largest single-column burst (top + 4 cliff sides + a tree).
const MAX_MESH_INDICES: usize = 4800;
/// Worst-case indices a single column/LOD-block can add at once: a block can emit four
/// full-relief side faces (≈ `4 × MAX_H` strata quads) at a tall LOD step, plus the top.
const COLUMN_INDEX_BURST: usize = 1200;

/// Build the chunk meshes (one cached `Mesh` per chunk) plus the water plane. Each
/// land column emits its top quad and, for every lower horizontal neighbour, the
/// cliff side faces from the neighbour's height up to its own (one quad per level →
/// strata bands); neighbour heights come from the chunk's ghost ring, so this is
/// self-contained. Forest/Plains columns also grow a voxel tree. Water columns add a
/// single translucent surface quad at `SEA_ABS` to the separate water list.
/// Build the meshes for ONE chunk `(cx, cy)` — the unit the renderer streams. Returns the
/// opaque batches and the (translucent) water batches; a dense chunk may split into more
/// than one of each to stay under macroquad's per-draw limit.
fn build_chunk_mesh(t: &VoxelTerrain, cx: usize, cy: usize, lod: u32) -> (Vec<Batch>, Vec<Batch>) {
    // At LOD>0 the chunk is downsampled: columns are read on a `stride` grid and each
    // block emits one `stride×stride` footprint, sampled from its origin column. LOD0 is
    // the exact per-column build. Neighbour heights are read a stride away, so side faces
    // span the coarse block edge. Trees are full-detail only (too small to read far out).
    let stride = 1usize << lod;
    let si = stride as i32;
    let mut opaque = Vec::new();
    let mut water = Vec::new();
    let mut verts: Vec<Vertex> = Vec::new();
    let mut idx: Vec<u16> = Vec::new();
    let mut wv: Vec<Vertex> = Vec::new();
    let mut wi: Vec<u16> = Vec::new();
    let mut ly = 0;
    while ly < CHUNK {
        let mut lx = 0;
        while lx < CHUNK {
            let (gx, gy) = (cx * CHUNK + lx, cy * CHUNK + ly);
            lx += stride;
            if gx >= COLS || gy >= ROWS {
                continue; // partial edge chunk: outside the world
            }
            let cell = t.cell(gx as i32, gy as i32);
            let h = cell_height(cell);
            if h == 0 {
                continue; // air
            }
            // Split before macroquad's per-drawcall batch limit (see consts). The burst
            // margin must cover a tall LOD step's 4 full-height side faces.
            if idx.len() + COLUMN_INDEX_BURST > MAX_MESH_INDICES {
                flush_mesh(&mut verts, &mut idx, &mut opaque);
            }
            let biome = cell_biome(cell);
            push_top(&mut verts, &mut idx, gx, gy, stride, h, biome);
            let (ix, iy) = (gx as i32, gy as i32);
            let wl = t.water_level(ix, iy);
            // Skip the side faces of a SUBMERGED block (water above its top): those
            // underwater walls showed through shallow water as a dark basin ring; only the
            // flat bed tops remain. Shore (not submerged) keeps its bank faces.
            if wl <= h {
                let nb = [
                    (t.height(ix + si, iy), Face::Px),
                    (t.height(ix - si, iy), Face::Nx),
                    (t.height(ix, iy + si), Face::Pz),
                    (t.height(ix, iy - si), Face::Nz),
                ];
                for (nh, face) in nb {
                    if nh < h {
                        push_side(&mut verts, &mut idx, (gx, gy), stride, h, nh, face, biome);
                    }
                }
            }

            // Per-column (per-block) water: a translucent plane at the water level where it
            // stands above the terrain top. One quad per block ⇒ no overlap ⇒ no sort.
            if wl > h {
                if wi.len() + COLUMN_INDEX_BURST > MAX_MESH_INDICES {
                    flush_mesh(&mut wv, &mut wi, &mut water);
                }
                let depth = wl - h;
                push_water_top(&mut wv, &mut wi, gx, gy, stride, wl, depth);
                // Connective faces only toward a LOWER neighbouring WATER surface (river
                // step / water meeting lower water), never onto dry land.
                for (nx, ny, face) in [
                    (ix + si, iy, Face::Px),
                    (ix - si, iy, Face::Nx),
                    (ix, iy + si, Face::Pz),
                    (ix, iy - si, Face::Nz),
                ] {
                    let nwl = t.water_level(nx, ny);
                    if nwl > 0 && nwl < wl {
                        push_water_side(&mut wv, &mut wi, (gx, gy), stride, wl, nwl, depth, face);
                    }
                }
            } else if lod == 0 {
                let bd = biome_def(biome);
                if bd.tree != TreeKind::None
                    && feature_unit(t.seed, gx, gy, 101) < bd.tree_density
                {
                    push_tree(&mut verts, &mut idx, t, gx, gy, h, bd.tree);
                }
            }
        }
        ly += stride;
    }
    flush_mesh(&mut verts, &mut idx, &mut opaque);
    flush_mesh(&mut wv, &mut wi, &mut water);
    (opaque, water)
}

/// A voxel tree on column `(gx, gy)` standing on surface height `h`. **Broadleaf**: a
/// short brown trunk under a 3×3 leaf canopy + cap (rounded, deciduous). **Conifer**: a
/// taller trunk with a narrow tapering spire (1-cell tip over a + of leaves) — gives
/// taiga/snow a distinct boreal look. Per-column hashes keep it deterministic; canopy
/// blocks overhanging outside the world are skipped.
fn push_tree(verts: &mut Vec<Vertex>, idx: &mut Vec<u16>, t: &VoxelTerrain, gx: usize, gy: usize, h: u8, kind: TreeKind) {
    let seed = t.seed;
    let trunk = (0.36, 0.26, 0.16);
    let leaf = if kind == TreeKind::Conifer { (0.09, 0.24, 0.16) } else { (0.16, 0.42, 0.20) };
    // Skip canopy blocks that would overhang a WATER column (leaves floating over a
    // river/lake) or fall outside the world.
    let leaf_at = |verts: &mut Vec<Vertex>, idx: &mut Vec<u16>, lx: i32, ly: i32, lz: u8| {
        if (0..COLS as i32).contains(&lx)
            && (0..ROWS as i32).contains(&ly)
            && t.water_level(lx, ly) == 0
        {
            push_block(verts, idx, lx, ly, lz, leaf);
        }
    };
    let (gxi, gyi) = (gx as i32, gy as i32);
    if kind == TreeKind::Conifer {
        let th = 3 + (feature_unit(seed, gx, gy, 202) * 2.0) as u8; // 3 or 4
        for gz in h..h + th {
            push_block(verts, idx, gxi, gyi, gz, trunk);
        }
        // Two narrow tiers (+ shape) then a single tip — a spire.
        for (dx, dy) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
            leaf_at(verts, idx, gxi + dx, gyi + dy, h + th);
        }
        leaf_at(verts, idx, gxi, gyi, h + th + 1);
        leaf_at(verts, idx, gxi, gyi, h + th + 2);
    } else {
        let th = 2 + (feature_unit(seed, gx, gy, 202) * 2.0) as u8; // 2 or 3
        for gz in h..h + th {
            push_block(verts, idx, gxi, gyi, gz, trunk);
        }
        let top = h + th;
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                leaf_at(verts, idx, gxi + dx, gyi + dy, top);
            }
        }
        leaf_at(verts, idx, gxi, gyi, top + 1);
    }
}

#[derive(Clone, Copy)]
enum Face {
    Px,
    Nx,
    Pz,
    Nz,
}

fn flush_mesh(verts: &mut Vec<Vertex>, idx: &mut Vec<u16>, out: &mut Vec<Batch>) {
    if verts.is_empty() {
        return;
    }
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for v in verts.iter() {
        lo = lo.min(v.position);
        hi = hi.max(v.position);
    }
    out.push(Batch {
        mesh: Mesh {
            vertices: std::mem::take(verts),
            indices: std::mem::take(idx),
            texture: None,
        },
        lo,
        hi,
    });
}

fn push_quad(verts: &mut Vec<Vertex>, idx: &mut Vec<u16>, q: [Vec3; 4], col: Color) {
    let base = verts.len() as u16;
    for p in q {
        verts.push(Vertex::new(p.x, p.y, p.z, 0.0, 0.0, col));
    }
    idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// A whole standalone cube (top + 4 sides, no hidden bottom) at voxel `(gx, gy, gz)`,
/// with the same baked per-face shading as the terrain. `gx`/`gy` are `i32` so tree
/// canopies can overhang into (already bounds-checked) neighbour columns.
fn push_block(verts: &mut Vec<Vertex>, idx: &mut Vec<u16>, gx: i32, gy: i32, gz: u8, rgb: (f32, f32, f32)) {
    let (x0, x1) = (gx as f32 * VOX, (gx + 1) as f32 * VOX);
    let (z0, z1) = (gy as f32 * VOX, (gy + 1) as f32 * VOX);
    let (y0, y1) = (gz as f32 * VOX, (gz + 1) as f32 * VOX);
    push_quad(verts, idx, [vec3(x0, y1, z0), vec3(x1, y1, z0), vec3(x1, y1, z1), vec3(x0, y1, z1)], shaded(rgb, SHADE_TOP));
    push_quad(verts, idx, [vec3(x1, y0, z0), vec3(x1, y0, z1), vec3(x1, y1, z1), vec3(x1, y1, z0)], shaded(rgb, SHADE_PX));
    push_quad(verts, idx, [vec3(x0, y0, z1), vec3(x0, y0, z0), vec3(x0, y1, z0), vec3(x0, y1, z1)], shaded(rgb, SHADE_NX));
    push_quad(verts, idx, [vec3(x1, y0, z1), vec3(x0, y0, z1), vec3(x0, y1, z1), vec3(x1, y1, z1)], shaded(rgb, SHADE_PZ));
    push_quad(verts, idx, [vec3(x0, y0, z0), vec3(x1, y0, z0), vec3(x1, y1, z0), vec3(x0, y1, z0)], shaded(rgb, SHADE_NZ));
}

/// A translucent water-surface quad at sea level over column `(gx, gy)`. Drawn in the
/// water pass; the alpha lets the opaque sea floor show through.
/// Water surface colour by DEPTH (levels of water above the bed): shallows are light and
/// translucent (the bed shows through, as in clear shoreline water), deeps darken and turn
/// nearly opaque (hiding the bed, so a basin's sloped walls don't read as a harsh dark
/// ring through clear water). Standard shallow→deep gradient.
fn water_color(depth: u8) -> Color {
    let t = (depth as f32 / WATER_OPAQUE_DEPTH).clamp(0.0, 1.0);
    let lerp = |a: f32, b: f32| a + (b - a) * t;
    Color::new(
        lerp(0.28, 0.08),
        lerp(0.52, 0.21),
        lerp(0.68, 0.40),
        lerp(0.45, 0.94),
    )
}
/// Depth (levels) at which water reaches its deep, near-opaque colour.
const WATER_OPAQUE_DEPTH: f32 = 6.0;

fn push_water_top(verts: &mut Vec<Vertex>, idx: &mut Vec<u16>, gx: usize, gy: usize, s: usize, level: u8, depth: u8) {
    let (x0, x1) = (gx as f32 * VOX, (gx + s) as f32 * VOX);
    let (z0, z1) = (gy as f32 * VOX, (gy + s) as f32 * VOX);
    let y = level as f32 * VOX;
    let col = water_color(depth);
    push_quad(verts, idx, [vec3(x0, y, z0), vec3(x1, y, z0), vec3(x1, y, z1), vec3(x0, y, z1)], col);
}

/// A translucent water side face on one edge, spanning levels `lo..hi`. Where a river
/// steps down (or a water body abuts a lower one), this fills the vertical gap between the
/// two water-surface quads so the ribbon reads as continuous instead of dashed.
#[allow(clippy::too_many_arguments)]
fn push_water_side(verts: &mut Vec<Vertex>, idx: &mut Vec<u16>, (gx, gy): (usize, usize), s: usize, hi: u8, lo: u8, depth: u8, face: Face) {
    let (x0, x1) = (gx as f32 * VOX, (gx + s) as f32 * VOX);
    let (z0, z1) = (gy as f32 * VOX, (gy + s) as f32 * VOX);
    let (y0, y1) = (lo as f32 * VOX, hi as f32 * VOX);
    let shade = match face {
        Face::Px => SHADE_PX,
        Face::Nx => SHADE_NX,
        Face::Pz => SHADE_PZ,
        Face::Nz => SHADE_NZ,
    };
    let base = water_color(depth);
    let col = Color::new(base.r * shade, base.g * shade, base.b * shade, base.a);
    let q = match face {
        Face::Px => [vec3(x1, y0, z0), vec3(x1, y0, z1), vec3(x1, y1, z1), vec3(x1, y1, z0)],
        Face::Nx => [vec3(x0, y0, z1), vec3(x0, y0, z0), vec3(x0, y1, z0), vec3(x0, y1, z1)],
        Face::Pz => [vec3(x1, y0, z1), vec3(x0, y0, z1), vec3(x0, y1, z1), vec3(x1, y1, z1)],
        Face::Nz => [vec3(x0, y0, z0), vec3(x1, y0, z0), vec3(x1, y1, z0), vec3(x0, y1, z0)],
    };
    push_quad(verts, idx, q, col);
}

fn push_top(verts: &mut Vec<Vertex>, idx: &mut Vec<u16>, gx: usize, gy: usize, s: usize, h: u8, biome: BiomeKind) {
    let (x0, x1) = (gx as f32 * VOX, (gx + s) as f32 * VOX);
    let (z0, z1) = (gy as f32 * VOX, (gy + s) as f32 * VOX);
    let y = h as f32 * VOX;
    let col = shaded(top_rgb(biome), SHADE_TOP);
    push_quad(
        verts,
        idx,
        [
            vec3(x0, y, z0),
            vec3(x1, y, z0),
            vec3(x1, y, z1),
            vec3(x0, y, z1),
        ],
        col,
    );
}

#[allow(clippy::too_many_arguments)]
fn push_side(
    verts: &mut Vec<Vertex>,
    idx: &mut Vec<u16>,
    (gx, gy): (usize, usize),
    s: usize,
    h: u8,
    nh: u8,
    face: Face,
    biome: BiomeKind,
) {
    let (x0, x1) = (gx as f32 * VOX, (gx + s) as f32 * VOX);
    let (z0, z1) = (gy as f32 * VOX, (gy + s) as f32 * VOX);
    let shade = match face {
        Face::Px => SHADE_PX,
        Face::Nx => SHADE_NX,
        Face::Pz => SHADE_PZ,
        Face::Nz => SHADE_NZ,
    };
    for gz in nh..h {
        let (y0, y1) = (gz as f32 * VOX, (gz + 1) as f32 * VOX);
        let col = shaded(strata_rgb(gz, h, biome), shade);
        let q = match face {
            Face::Px => [
                vec3(x1, y0, z0),
                vec3(x1, y0, z1),
                vec3(x1, y1, z1),
                vec3(x1, y1, z0),
            ],
            Face::Nx => [
                vec3(x0, y0, z1),
                vec3(x0, y0, z0),
                vec3(x0, y1, z0),
                vec3(x0, y1, z1),
            ],
            Face::Pz => [
                vec3(x1, y0, z1),
                vec3(x0, y0, z1),
                vec3(x0, y1, z1),
                vec3(x1, y1, z1),
            ],
            Face::Nz => [
                vec3(x0, y0, z0),
                vec3(x1, y0, z0),
                vec3(x1, y1, z0),
                vec3(x0, y1, z0),
            ],
        };
        push_quad(verts, idx, q, col);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every built chunk mesh must stay strictly under macroquad's per-`draw_mesh` batch
    /// limits (`>= 10000` verts / `>= 5000` indices ⇒ silent clamping). Builds chunk by
    /// chunk (the streaming unit) so peak memory is one chunk even at ×16. Guards the
    /// splitter.
    #[test]
    fn meshes_stay_under_macroquad_drawcall_limits() {
        let t = VoxelTerrain::new(1);
        let mut any = false;
        for cy in 0..t.chunks_y {
            for cx in 0..t.chunks_x {
                let (op, wa) = build_chunk_mesh(&t, cx, cy, 0);
                for b in op.iter().chain(wa.iter()) {
                    any = true;
                    assert!(b.mesh.vertices.len() < 10_000, "verts {} at chunk ({cx},{cy})", b.mesh.vertices.len());
                    assert!(b.mesh.indices.len() < 5_000, "indices {} at chunk ({cx},{cy})", b.mesh.indices.len());
                }
            }
        }
        assert!(any, "no geometry built");
    }

    /// LOD must (a) stay under the draw-call limits at every level and (b) actually shrink
    /// the geometry as it coarsens (else the far rings buy nothing). Checks a spread of
    /// chunks across the map at LOD 0/1/2.
    #[test]
    fn lod_reduces_geometry_within_limits() {
        let t = VoxelTerrain::new(1);
        let sample = [
            (t.chunks_x / 4, t.chunks_y / 4),
            (t.chunks_x / 2, t.chunks_y / 2),
            (t.chunks_x * 3 / 4, t.chunks_y / 2),
        ];
        for (cx, cy) in sample {
            let mut prev = usize::MAX;
            for lod in 0..3u32 {
                let (op, wa) = build_chunk_mesh(&t, cx, cy, lod);
                let mut verts = 0;
                for b in op.iter().chain(wa.iter()) {
                    verts += b.mesh.vertices.len();
                    assert!(b.mesh.vertices.len() < 10_000, "lod {lod} verts overflow at ({cx},{cy})");
                    assert!(b.mesh.indices.len() < 5_000, "lod {lod} indices overflow at ({cx},{cy})");
                }
                assert!(verts <= prev, "lod {lod} not coarser at ({cx},{cy}): {verts} > {prev}");
                prev = verts;
            }
        }
    }

    /// Report the WHOLE-map mesh size at the current scale (built per chunk + dropped, so
    /// it doesn't hold it all) — this is the number the streaming exists to avoid holding
    /// resident. Informational, `--ignored` (it meshes the whole map).
    #[test]
    #[ignore]
    fn report_mesh_footprint() {
        let t = VoxelTerrain::new(1);
        let (mut verts, mut batches) = (0usize, 0usize);
        for cy in 0..t.chunks_y {
            for cx in 0..t.chunks_x {
                let (op, wa) = build_chunk_mesh(&t, cx, cy, 0);
                for b in op.iter().chain(wa.iter()) {
                    verts += b.mesh.vertices.len();
                    batches += 1;
                }
            }
        }
        let mb = (verts * std::mem::size_of::<Vertex>()) as f64 / (1024.0 * 1024.0);
        eprintln!("MAP_SCALE={MAP_SCALE} SURFACE_RANGE={SURFACE_RANGE}: {verts} verts, {batches} batches, ~{mb:.0} MB if all resident");
    }
}
