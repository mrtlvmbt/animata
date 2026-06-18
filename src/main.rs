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
        // The camera must sit BEYOND the map along the view direction, else the near half
        // of the ground falls behind the near plane and clips to a triangle when zoomed
        // out. Push back by the whole map extent (+ zoom), with a far plane to match.
        // Orthographic depth is linear, so a large range costs no precision.
        let reach = (COLS as f32 + ROWS as f32) * VOX + self.zoom;
        Camera3D {
            position: self.target + dir * reach,
            target: self.target,
            up: vec3(0.0, 1.0, 0.0),
            fovy: self.zoom,
            aspect: Some(screen_width() / screen_height()),
            projection: Projection::Orthographics,
            render_target: None,
            viewport: None,
            z_near: 0.1,
            z_far: reach * 2.5,
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
    lod: u32,
}

/// Detail-tier chunk meshes built+uploaded per frame, and coarse super-tiles per frame.
const BUILD_BUDGET: usize = 24;
const COARSE_BUDGET: usize = 16;
/// Within the detail tier, LOD by **Euclidean** chunk distance → concentric circular
/// rings (not square). The OUTER ring grades down to `COARSE_LOD` (stride 8) so the detail
/// edge meets the coarse tier at the SAME resolution → blocks align on the global stride
/// grid → no seam at the boundary.
const LOD0_RADIUS: i32 = 8;
const LOD1_RADIUS: i32 = 16;
const LOD2_RADIUS: i32 = 24;
/// Deadband (chunks) around each LOD ring boundary — see `lod_hyst`.
const LOD_HYSTERESIS: i32 = 2;
/// Skirt depth (levels): border columns drop their outward side face at least this far,
/// hiding sky cracks where adjacent meshes differ in LOD/height. Only applied on faces
/// that actually meet a DIFFERENT-LOD neighbour (see `SKIRT_*` mask), so same-LOD chunk
/// borders stay clean (no blanket of dark seams).
const SKIRT_LEVELS: u8 = 4;
const SKIRT_PX: u8 = 1;
const SKIRT_NX: u8 = 2;
const SKIRT_PZ: u8 = 4;
const SKIRT_NZ: u8 = 8;
/// Two-tier streaming. The DETAIL tier renders per-chunk (LOD by distance) the super-tiles
/// around the camera; the COARSE tier renders every OTHER super-tile as one merged buffer
/// at `COARSE_LOD`, covering the WHOLE map cheaply (so a full zoom-out shows all of ×16
/// at a few hundred draws). A super-tile is detail XOR coarse, so the two never overlap.
const SUPER: i32 = 8; // chunks per super-tile side
const DETAIL_SUPER_R: i32 = 2; // super-tiles around the camera kept at per-chunk detail
const COARSE_LOD: u32 = 3; // stride-8 overview
/// Past this zoom the detail tier is dropped entirely — pure coarse whole-map overview,
/// so a full zoom-out costs only the (few hundred) coarse super-tile draws.
const DETAIL_ZOOM_CUTOFF: f32 = 520.0;

/// LOD for a detail chunk at Euclidean distance `d` (chunks) from the camera centre,
/// grading 0→1→2→`COARSE_LOD` in concentric rings so the detail edge matches the coarse tier.
fn lod_for(d: i32) -> u32 {
    if d <= LOD0_RADIUS {
        0
    } else if d <= LOD1_RADIUS {
        1
    } else if d <= LOD2_RADIUS {
        2
    } else {
        COARSE_LOD
    }
}

/// LOD a detail chunk `(cx, cy)` resolves to, given the camera centre chunk `(ccx, ccy)`.
fn lod_at(cx: i32, cy: i32, ccx: i32, ccy: i32) -> u32 {
    let (dx, dy) = (cx - ccx, cy - ccy);
    lod_for(((dx * dx + dy * dy) as f32).sqrt() as i32)
}

/// Chunk distance (chunks) from the camera centre.
fn chunk_dist(cx: i32, cy: i32, ccx: i32, ccy: i32) -> i32 {
    let (dx, dy) = (cx - ccx, cy - ccy);
    ((dx * dx + dy * dy) as f32).sqrt() as i32
}

/// LOD with HYSTERESIS: a chunk already at `cur` only switches once the camera distance
/// clears the ring boundary by `LOD_HYSTERESIS` chunks, so a camera hovering on a ring
/// edge doesn't flip-flop the chunk's LOD (rebuild thrash + visible popping) every frame.
fn lod_hyst(d: i32, cur: Option<u32>) -> u32 {
    let raw = lod_for(d);
    match cur {
        // Coarsening (moved away): require d past the boundary by the margin.
        Some(c) if raw > c => if lod_for(d - LOD_HYSTERESIS) > c { raw } else { c },
        // Refining (moved closer): require d inside the boundary by the margin.
        Some(c) if raw < c => if lod_for(d + LOD_HYSTERESIS) < c { raw } else { c },
        _ => raw,
    }
}

type ChunkMap = std::collections::HashMap<(i32, i32), LoadedChunk>;

struct Streamer {
    detail: ChunkMap, // per-chunk, in the detail super-tiles
    coarse: ChunkMap, // per super-tile, whole-map overview (always resident)
    /// Super-tiles whose detail is FULLY built — the renderer draws their detail chunks
    /// and SKIPS their coarse twin. Until a super-tile is ready it shows coarse, so a
    /// detail→coarse swap never flashes empty and the tiers never overlap.
    ready: std::collections::HashSet<(i32, i32)>,
}

impl Streamer {
    fn new() -> Self {
        Streamer {
            detail: ChunkMap::new(),
            coarse: ChunkMap::new(),
            ready: std::collections::HashSet::new(),
        }
    }

    fn clear(&mut self, ctx: &mut dyn RenderingBackend) {
        for lc in self.detail.values().chain(self.coarse.values()) {
            free_chunks(ctx, &lc.opaque);
        }
        self.detail.clear();
        self.coarse.clear();
        self.ready.clear();
    }

    fn update(&mut self, ctx: &mut dyn RenderingBackend, t: &VoxelTerrain, center: (i32, i32), zoom: f32) {
        let (ccx, ccy) = center;
        let nsx = (t.chunks_x as i32 + SUPER - 1) / SUPER;
        let nsy = (t.chunks_y as i32 + SUPER - 1) / SUPER;
        let (scx, scy) = (ccx.div_euclid(SUPER), ccy.div_euclid(SUPER));
        let detail_on = zoom <= DETAIL_ZOOM_CUTOFF;

        // ---- DETAIL tier: per-chunk (LOD by distance) within the camera super-tiles ----
        if detail_on {
            let dr = DETAIL_SUPER_R;
            let dcx0 = (scx - dr).max(0) * SUPER;
            let dcx1 = ((scx + dr + 1).min(nsx) * SUPER).min(t.chunks_x as i32);
            let dcy0 = (scy - dr).max(0) * SUPER;
            let dcy1 = ((scy + dr + 1).min(nsy) * SUPER).min(t.chunks_y as i32);
            self.detail.retain(|&(cx, cy), lc| {
                let inside = (dcx0..dcx1).contains(&cx) && (dcy0..dcy1).contains(&cy);
                if !inside {
                    free_chunks(ctx, &lc.opaque);
                }
                inside
            });
            let mut todo: Vec<(i64, i32, i32, u32)> = Vec::new();
            for cy in dcy0..dcy1 {
                for cx in dcx0..dcx1 {
                    let cur = self.detail.get(&(cx, cy)).map(|lc| lc.lod);
                    let want = lod_hyst(chunk_dist(cx, cy, ccx, ccy), cur);
                    if cur != Some(want) {
                        let (dx, dy) = ((cx - ccx) as i64, (cy - ccy) as i64);
                        todo.push((dx * dx + dy * dy, cx, cy, want));
                    }
                }
            }
            todo.sort_unstable_by_key(|m| m.0);
            for &(_, cx, cy, lod) in todo.iter().take(BUILD_BUDGET) {
                if let Some(old) = self.detail.remove(&(cx, cy)) {
                    free_chunks(ctx, &old.opaque);
                }
                // Skirt only the faces meeting a DIFFERENT-LOD neighbour chunk (an actual
                // seam) — not every chunk border, which painted dark seams everywhere. Use
                // the neighbour's LOADED LOD (hysteresis means it may differ from the pure
                // distance LOD), falling back to the distance LOD if it isn't resident yet.
                let nlod = |nx: i32, ny: i32| {
                    self.detail.get(&(nx, ny)).map(|lc| lc.lod).unwrap_or_else(|| lod_at(nx, ny, ccx, ccy))
                };
                let mut sk = 0u8;
                if nlod(cx + 1, cy) != lod { sk |= SKIRT_PX; }
                if nlod(cx - 1, cy) != lod { sk |= SKIRT_NX; }
                if nlod(cx, cy + 1) != lod { sk |= SKIRT_PZ; }
                if nlod(cx, cy - 1) != lod { sk |= SKIRT_NZ; }
                let o = build_chunk_mesh(t, cx as usize, cy as usize, lod, sk);
                let lc = LoadedChunk { opaque: upload_chunks(ctx, &o), lod };
                self.detail.insert((cx, cy), lc);
            }
        } else if !self.detail.is_empty() {
            for lc in self.detail.values() {
                free_chunks(ctx, &lc.opaque);
            }
            self.detail.clear();
        }

        // ---- COARSE tier: the WHOLE map, ALWAYS resident (never freed for detail). The
        // detail tier draws on top of it with a depth bias, so freeing a detail chunk just
        // reveals the coarse underneath — no unload-before-load gap / flicker. ----
        let mut ctodo: Vec<(i64, i32, i32)> = Vec::new();
        for sy in 0..nsy {
            for sx in 0..nsx {
                if !self.coarse.contains_key(&(sx, sy)) {
                    let (dx, dy) = ((sx - scx) as i64, (sy - scy) as i64);
                    ctodo.push((dx * dx + dy * dy, sx, sy));
                }
            }
        }
        ctodo.sort_unstable_by_key(|m| m.0);
        for &(_, sx, sy) in ctodo.iter().take(COARSE_BUDGET) {
            let x0 = sx as usize * SUPER as usize * CHUNK;
            let y0 = sy as usize * SUPER as usize * CHUNK;
            let x1 = (x0 + SUPER as usize * CHUNK).min(COLS);
            let y1 = (y0 + SUPER as usize * CHUNK).min(ROWS);
            let o = build_region_mesh(t, x0, y0, x1, y1, COARSE_LOD, 0);
            let lc = LoadedChunk { opaque: upload_chunks(ctx, &o), lod: COARSE_LOD };
            self.coarse.insert((sx, sy), lc);
        }

        // ---- Readiness: a detail super-tile is ready once ALL its in-map chunks are
        // present (any LOD is drawable). The renderer draws detail for ready tiles and
        // coarse for the rest — so the swap is instant, never empty, never overlapping.
        self.ready.clear();
        if detail_on {
            for sy in (scy - DETAIL_SUPER_R)..=(scy + DETAIL_SUPER_R) {
                for sx in (scx - DETAIL_SUPER_R)..=(scx + DETAIL_SUPER_R) {
                    if sx < 0 || sy < 0 || sx >= nsx || sy >= nsy {
                        continue;
                    }
                    let cx0 = sx * SUPER;
                    let cx1 = ((sx + 1) * SUPER).min(t.chunks_x as i32);
                    let cy0 = sy * SUPER;
                    let cy1 = ((sy + 1) * SUPER).min(t.chunks_y as i32);
                    let mut all = true;
                    'tile: for cy in cy0..cy1 {
                        for cx in cx0..cx1 {
                            if !self.detail.contains_key(&(cx, cy)) {
                                all = false;
                                break 'tile;
                            }
                        }
                    }
                    if all {
                        self.ready.insert((sx, sy));
                    }
                }
            }
        }
    }
}

/// Camera centre chunk from its world target.
fn center_chunk(cam: &IsoCam) -> (i32, i32) {
    (
        (cam.target.x / (CHUNK as f32 * VOX)).floor() as i32,
        (cam.target.z / (CHUNK as f32 * VOX)).floor() as i32,
    )
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
                                 "detail_chunks": streamer.detail.len(), "coarse_tiles": streamer.coarse.len() },
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
        let center = center_chunk(&cam);
        let mut drawn = 0usize;
        {
            let mut gl = unsafe { get_internal_gl() };
            gl.flush(); // flush any pending macroquad 2D before our own pass
            let ctx = gl.quad_context;
            // Stream: detail tier around the camera + coarse super-tiles over the rest.
            streamer.update(ctx, &terrain, center, cam.zoom);
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
            // Per super-tile draw EITHER its detail chunks (if ready) OR its coarse buffer
            // (otherwise) — never both. So the tiers never overlap (no z-fight) and a
            // not-yet-ready tile shows coarse instead of flashing empty (no flicker). Opaque
            // across both tiers first, then translucent water (skipped in topo). Frustum-
            // culled by AABB.
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
        let (det, crs) = (streamer.detail.len(), streamer.coarse.len());
        let mode = if topo { "   [TOPO: height/depth, G]" } else { "" };
        let line = format!(
            "{fps:.0} fps   {frame_ms:.2} ms   seed {seed}   {COLS}x{ROWS} m   draws {drawn}   detail {det} coarse {crs}{mode}"
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

/// Side-wall colour for the exposed level `gz` of a column of height `h`: a thin lip of
/// the surface colour `top` just under the surface, then (unless `rocky`) topsoil, then
/// stone. `rocky` biomes (mountain/snow/seabed) skip the brown topsoil band.
fn strata_rgb(gz: u8, h: u8, top: (f32, f32, f32), rocky: bool) -> (f32, f32, f32) {
    if gz + 1 == h {
        (top.0 * 0.85, top.1 * 0.85, top.2 * 0.85)
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

/// Build the meshes for ONE chunk `(cx, cy)` at `lod` — the unit the detail tier streams.
fn build_chunk_mesh(t: &VoxelTerrain, cx: usize, cy: usize, lod: u32, skirt_faces: u8) -> Vec<Batch> {
    let x1 = (cx * CHUNK + CHUNK).min(COLS);
    let y1 = (cy * CHUNK + CHUNK).min(ROWS);
    build_region_mesh(t, cx * CHUNK, cy * CHUNK, x1, y1, lod, skirt_faces)
}

/// Build the opaque + water meshes for an arbitrary column rectangle `[x0,x1) × [y0,y1)`
/// at `lod`, merged into as few batches as the per-draw limit allows. A single chunk uses
/// this for the streamed detail tier; a whole super-tile uses it for the coarse overview
/// tier (many chunks → a handful of buffers, so the whole map is a few hundred draws).
///
/// At LOD>0 columns are read on a `stride` grid (blocks aligned globally because `x0/y0`
/// are stride multiples) and each block emits one `stride×stride` footprint sampled from
/// its origin column, with neighbour heights read a stride away. Trees are full-detail
/// only. Water is NOT rendered — submerged columns just show a sand/rock seabed.
#[allow(clippy::too_many_arguments)]
fn build_region_mesh(t: &VoxelTerrain, x0: usize, y0: usize, x1: usize, y1: usize, lod: u32, skirt_faces: u8) -> Vec<Batch> {
    let stride = 1usize << lod;
    let si = stride as i32;
    let mut opaque = Vec::new();
    let mut verts: Vec<Vertex> = Vec::new();
    let mut idx: Vec<u16> = Vec::new();
    let mut gyc = y0;
    while gyc < y1 {
        let mut gxc = x0;
        while gxc < x1 {
            let (gx, gy) = (gxc, gyc);
            gxc += stride;
            if gx >= COLS || gy >= ROWS {
                continue; // outside the world
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
            let (ix, iy) = (gx as i32, gy as i32);
            let wl = t.water_level(ix, iy);
            // Submerged columns get a sand/rock SEABED top (by depth), not the blue Ocean
            // biome colour — the floor reads as a bottom under the blue water surface.
            // Submerged columns are a sand/rock seabed (by depth); land uses its biome.
            // Either way the SIDE faces are drawn (the bed reads as 3D terrain like land,
            // and culling them left sky showing through the steps as blue edges).
            let submerged = wl > h;
            let top_col = if submerged {
                seabed_rgb(wl - h)
            } else if matches!(biome, BiomeKind::Mountain) {
                rock_rgb(gx, gy, t.seed)
            } else {
                top_rgb(biome)
            };
            let rocky = submerged || matches!(biome, BiomeKind::Mountain | BiomeKind::Snow);
            push_top(&mut verts, &mut idx, gx, gy, stride, h, top_col);
            // SKIRT: on a region-border column whose face meets a DIFFERENT-LOD neighbour
            // mesh (flagged in `skirt_faces`), drop the side down by ≥SKIRT_LEVELS so a
            // height mismatch across the seam can't show a sky crack. Same-LOD borders are
            // not flagged, so they stay clean. The dropped part is coloured with the SURFACE
            // tone (not dark stone strata) so the apron blends in instead of a dark seam.
            let (bpx, bnx) = (gx + stride >= x1 && skirt_faces & SKIRT_PX != 0, gx == x0 && skirt_faces & SKIRT_NX != 0);
            let (bpz, bnz) = (gy + stride >= y1 && skirt_faces & SKIRT_PZ != 0, gy == y0 && skirt_faces & SKIRT_NZ != 0);
            let nb = [
                (t.height(ix + si, iy), Face::Px, bpx),
                (t.height(ix - si, iy), Face::Nx, bnx),
                (t.height(ix, iy + si), Face::Pz, bpz),
                (t.height(ix, iy - si), Face::Nz, bnz),
            ];
            for (nh_real, face, is_skirt) in nb {
                let nh = if is_skirt { nh_real.min(h.saturating_sub(SKIRT_LEVELS)) } else { nh_real };
                if nh < h {
                    push_side(&mut verts, &mut idx, (gx, gy), stride, h, nh, face, top_col, rocky, is_skirt);
                }
            }

            // Trees on dry land (through LOD1, one per block, so the canopy fades a ring
            // out instead of a hard edge). Water itself is not rendered.
            if !submerged && lod <= 1 {
                let bd = biome_def(biome);
                if bd.tree != TreeKind::None && feature_unit(t.seed, gx, gy, 101) < bd.tree_density {
                    push_tree(&mut verts, &mut idx, t, gx, gy, h, bd.tree);
                }
            }
        }
        gyc += stride;
    }
    flush_mesh(&mut verts, &mut idx, &mut opaque);
    opaque
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

/// Sea/lake BED colour by water depth: a sandy shoal in the shallows grading to bare rock
/// in the deeps — submerged columns render this instead of a (removed) water surface.
fn seabed_rgb(depth: u8) -> (f32, f32, f32) {
    let t = (depth as f32 / 5.0).clamp(0.0, 1.0);
    let lerp = |a: f32, b: f32| a + (b - a) * t;
    (lerp(0.80, 0.40), lerp(0.72, 0.39), lerp(0.52, 0.37)) // sand → rock
}

/// Varied mountain rock: a coherent (low-frequency) brightness field over the bare grey,
/// plus greenish mossy patches — so a massif isn't a flat slab of one stone colour.
fn rock_rgb(gx: usize, gy: usize, seed: u64) -> (f32, f32, f32) {
    let v = terrain::fbm(seed, gx as f32 / 22.0, gy as f32 / 22.0, 303, 3); // brightness [0,1]
    let m = terrain::fbm(seed, gx as f32 / 34.0, gy as f32 / 34.0, 305, 2); // moss mask
    let g = 0.36 + 0.22 * v;
    let mut c = (g, g * 0.98, g * 0.93); // slightly warm, brightness-varied grey
    if m > 0.60 {
        let k = ((m - 0.60) / 0.40).min(1.0) * 0.5;
        c = (c.0 + (0.33 - c.0) * k, c.1 + (0.45 - c.1) * k, c.2 + (0.29 - c.2) * k); // → moss
    }
    c
}

fn push_top(verts: &mut Vec<Vertex>, idx: &mut Vec<u16>, gx: usize, gy: usize, s: usize, h: u8, rgb: (f32, f32, f32)) {
    let (x0, x1) = (gx as f32 * VOX, (gx + s) as f32 * VOX);
    let (z0, z1) = (gy as f32 * VOX, (gy + s) as f32 * VOX);
    let y = h as f32 * VOX;
    let col = shaded(rgb, SHADE_TOP);
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
    top: (f32, f32, f32),
    rocky: bool,
    skirt: bool,
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
        // A skirt face is a hidden gap-filler at a LOD seam, not real geology. Colour it
        // with the FLAT top tone (SHADE_TOP, no directional darkening) so the thin sliver
        // that pokes out on a "flat" seam matches the top exactly and disappears — with
        // side shading it read as a dark line tracing every LOD-ring boundary.
        let col = if skirt {
            shaded(top, SHADE_TOP)
        } else {
            shaded(strata_rgb(gz, h, top, rocky), shade)
        };
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
                let op = build_chunk_mesh(&t, cx, cy, 0, 0);
                for b in op.iter() {
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
                let op = build_chunk_mesh(&t, cx, cy, lod, 0);
                let mut verts = 0;
                for b in op.iter() {
                    verts += b.mesh.vertices.len();
                    assert!(b.mesh.vertices.len() < 10_000, "lod {lod} verts overflow at ({cx},{cy})");
                    assert!(b.mesh.indices.len() < 5_000, "lod {lod} indices overflow at ({cx},{cy})");
                }
                assert!(verts <= prev, "lod {lod} not coarser at ({cx},{cy}): {verts} > {prev}");
                prev = verts;
            }
        }
    }

    /// A coarse super-tile (a whole SUPER×SUPER chunk region merged into one mesh stream)
    /// must also stay under the per-draw limits and be far fewer batches than its chunks —
    /// that merge is what lets the whole map render in a few hundred draws.
    #[test]
    fn coarse_super_tiles_merge_within_limits() {
        let t = VoxelTerrain::new(1);
        let span = SUPER as usize * CHUNK;
        for &(sx, sy) in &[(0usize, 0usize), (2, 1)] {
            let (x0, y0) = (sx * span, sy * span);
            if x0 >= COLS || y0 >= ROWS {
                continue;
            }
            let (x1, y1) = ((x0 + span).min(COLS), (y0 + span).min(ROWS));
            let op = build_region_mesh(&t, x0, y0, x1, y1, COARSE_LOD, 0);
            let batches = op.len();
            for b in op.iter() {
                assert!(b.mesh.vertices.len() < 10_000, "coarse verts overflow");
                assert!(b.mesh.indices.len() < 5_000, "coarse indices overflow");
            }
            // A super-tile is SUPER² chunks; the merged coarse mesh must be far fewer
            // buffers than that (else the overview buys no draw-call reduction).
            assert!(batches < (SUPER * SUPER) as usize, "coarse not merged: {batches} batches");
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
                let op = build_chunk_mesh(&t, cx, cy, 0, 0);
                for b in op.iter() {
                    verts += b.mesh.vertices.len();
                    batches += 1;
                }
            }
        }
        let mb = (verts * std::mem::size_of::<Vertex>()) as f64 / (1024.0 * 1024.0);
        eprintln!("MAP_SCALE={MAP_SCALE} SURFACE_RANGE={SURFACE_RANGE}: {verts} verts, {batches} batches, ~{mb:.0} MB if all resident");
    }
}
