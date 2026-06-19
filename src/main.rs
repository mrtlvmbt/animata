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

mod clock;
mod config;
#[cfg(feature = "dev")]
mod dev_bridge;
mod erosion;
mod genome;
mod grid;
mod hydrology;
mod rng;
mod sim;
mod tectonics;
mod terrain;

use clock::WorldClock;
use config::*;
use sim::Sim;
use macroquad::miniquad::{
    Bindings, BlendFactor, BlendState, BlendValue, BufferSource, BufferType, BufferUsage,
    Comparison, CullFace, Equation, FrontFaceOrder, PassAction, Pipeline, PipelineParams,
    RenderingBackend, ShaderMeta, ShaderSource, UniformBlockLayout, UniformDesc, UniformType,
    UniformsSource, VertexAttribute, VertexFormat,
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
        // Sit BEYOND the map along the view dir so all geometry has positive depth (ortho, so
        // distance doesn't affect size).
        let reach = (COLS as f32 + ROWS as f32) * VOX + self.zoom;
        let position = self.target + dir * reach;
        // Depth precision = (z_far - z_near) / depth-buffer-steps, and ortho depth is LINEAR.
        // The OLD range was the whole-map diagonal (~2700 m at ×16), so a single voxel spanned
        // only a handful of depth steps — faces tied and the water pass had to paper over it
        // with `LessOrEqual` + a hand-tuned z-bias (which then bled water over the shore).
        // Instead fit [z_near, z_far] to ONLY the geometry actually on screen: intersect the
        // four screen corners' view rays with the ground (`y=0`) and the tallest possible
        // column, and bracket the resulting depths. This tracks `zoom`, so precision stays
        // per-voxel-fine at any zoom AND any MAP_SCALE — no magic constants.
        let fwd = (self.target - position).normalize();
        let right = fwd.cross(vec3(0.0, 1.0, 0.0)).normalize();
        let up = right.cross(fwd);
        let half_h = self.zoom * 0.5;
        let half_w = half_h * (screen_width() / screen_height().max(1.0));
        // Vertical span of drawable geometry: ground (0) up to the tallest column + a little
        // headroom for tree canopies.
        let top_y = (UNDERGROUND_LEVELS + SEA_LEVEL + 1 + SURFACE_RANGE) as f32 * VOX + 8.0;
        let (mut z_near, mut z_far) = (f32::MAX, f32::MIN);
        for sx in [-half_w, half_w] {
            for sy in [-half_h, half_h] {
                let corner = position + right * sx + up * sy;
                for y in [0.0_f32, top_y] {
                    let t = (y - corner.y) / fwd.y; // distance along the (downward) view ray to y
                    z_near = z_near.min(t);
                    z_far = z_far.max(t);
                }
            }
        }
        Camera3D {
            position,
            target: self.target,
            up: vec3(0.0, 1.0, 0.0),
            fovy: self.zoom,
            aspect: Some(screen_width() / screen_height()),
            projection: Projection::Orthographics,
            render_target: None,
            viewport: None,
            z_near: (z_near - 1.0).max(1.0),
            z_far: z_far + 1.0,
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
attribute vec2 texcoord;
attribute vec4 color0;
uniform mat4 mvp;
varying lowp vec4 color;
varying highp float vy;
varying lowp float rim;
void main() {
    gl_Position = mvp * vec4(position, 1.0);
    // texcoord.y flags a contour-overlay vert (1.0 = the dark edge strips); the fragment
    // shader hides them when the outline toggle is off. Terrain/tree faces leave it 0.0.
    rim = texcoord.y;
    // texcoord.x is a per-vertex depth nudge flag (-1/0/+1): a face's TOP edge is pushed a
    // hair toward the far plane (+1) so the column's own top face deterministically wins
    // their shared rim (otherwise they z-fight into dark corner speckle, whatever the
    // depth precision); a tree's BOTTOM edge is nudged forward (-1) so the trunk wins the
    // tie against the ground it stands on. The nudge is far below one voxel.
    gl_Position.z += texcoord.x * 0.00012 * gl_Position.w;
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
varying lowp float rim;
uniform highp vec4 dbg;
void main() {
    // dbg.z = outline on. A contour-overlay frag (rim) is discarded when it's off, baring the
    // face behind it (the strip is a nudged overlay, so there is always geometry under it).
    if (rim > 0.5 && dbg.z < 0.5) discard;
    if (dbg.y > 0.5) {
        // WATER/LAND mask debug (key J): every opaque column flat grey = "land". The water
        // pass paints flat blue over the columns generation flagged as water, so a dry cell
        // that SHOULD be flooded shows through as a grey hole inside the blue.
        gl_FragColor = vec4(0.62, 0.60, 0.54, 1.0);
    } else if (dbg.x > 0.5) {
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

// ---- Water surface (separate, animated, translucent pass) ----------------------
//
// Submerged columns render a sand/rock seabed (opaque) PLUS a translucent water
// surface drawn on top in a second pass. The surface is stylised (Minecraft-like),
// not photoreal: depth-shaded (deeper = darker + more opaque, Beer-Lambert), with
// two world-space sine waves animating the vertices and a bright foam rim at the
// shore. Depth (`water_level - terrain_height`, in voxel levels) is carried per
// vertex in `uv.y`; the wave is a pure function of WORLD xz + time so it matches
// bit-for-bit across chunk / LOD boundaries (no seam).
// FLAT geometry (no vertex displacement — that jittered on the low-tessellation per-column
// quads). All motion lives in the fragment shader. Passes world xz + depth to the fragment.
const WATER_VERT: &str = r#"#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;
uniform mat4 mvp;
varying highp float vDepth;
varying highp vec2 vWorld;
void main() {
    gl_Position = mvp * vec4(position, 1.0);
    vDepth = texcoord.y;     // water depth in voxel levels
    vWorld = position.xz;    // world coords → animation is seamless across chunks
}"#;

// Toon / cel-shaded water, fully procedural (no textures): a domain-warped scrolling
// value-noise field (pure function of WORLD xz + time → seamless across chunk/LOD edges)
// is quantised into a few brightness steps with bright contour lines on the band edges
// (the "crests"), over a depth-banded blue (deeper = darker + more opaque) with a broken
// foam rim at the shore. No geometry moves, so nothing jitters on the coarse quads.
const WATER_FRAG: &str = r#"#version 100
uniform highp vec4 params; // params.x = time
varying highp float vDepth;
varying highp vec2 vWorld;

highp float hash(highp vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453123);
}
highp float vnoise(highp vec2 p) {
    highp vec2 i = floor(p);
    highp vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    highp float a = hash(i);
    highp float b = hash(i + vec2(1.0, 0.0));
    highp float c = hash(i + vec2(0.0, 1.0));
    highp float d = hash(i + vec2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

void main() {
    if (params.y > 0.5) {
        // WATER/LAND mask debug (key J): flat OPAQUE blue over every flagged-water column.
        gl_FragColor = vec4(0.16, 0.42, 0.70, 1.0);
        return;
    }
    highp float t = params.x;
    highp vec2 p = vWorld * 0.20; // ripple scale (world units per noise cell)
    // Domain warp: two slow scrolling noise layers warp a third → organic, living surface.
    highp vec2 q = vec2(vnoise(p + vec2(0.0, t * 0.10)),
                        vnoise(p + vec2(5.2, t * 0.12)));
    highp float n = vnoise(p + 1.3 * q + vec2(t * 0.06, -t * 0.05));

    // Depth colour, QUANTISED into bands (cel look): deeper = darker + more opaque.
    // NB depth is in INTEGER voxel levels, minimum 1 (a submerged column has wl > h), so
    // there is no depth→0 shore ramp — the shallowest water is depth 1.
    highp float absorb = 1.0 - exp(-vDepth * 0.16);
    highp vec3 shallow = vec3(0.24, 0.62, 0.74);
    highp vec3 deep    = vec3(0.02, 0.17, 0.40);
    highp float db = floor(absorb * 4.0) / 4.0;
    highp vec3 col = mix(shallow, deep, db);

    // Toon ripple: quantise the noise into 3 brightness steps.
    highp float lv = floor(n * 3.0) / 3.0;
    col *= mix(0.86, 1.14, lv);

    // Bright contour "crest" lines on the band boundaries (animated, since n moves).
    highp float edge = abs(fract(n * 3.0) - 0.5);
    highp float crest = smoothstep(0.47, 0.5, edge);
    col = mix(col, vec3(0.82, 0.93, 0.99), crest * 0.35);

    // Broken foam: ONLY the shallowest shore band (depth ≈ 1), and only on noise crests, so
    // it reads as scattered flecks at the water's edge — not a white wash over shallow lakes.
    highp float shoreband = smoothstep(2.4, 1.0, vDepth); // 1 at depth 1 → 0 by depth ~2.4
    highp float foamy = shoreband * smoothstep(0.58, 0.82, n);
    col = mix(col, vec3(0.92, 0.96, 1.0), foamy * 0.6);

    highp float alpha = mix(0.62, 0.94, db);
    gl_FragColor = vec4(col, alpha);
}"#;

#[repr(C)]
struct WaterUniforms {
    mvp: Mat4,
    params: Vec4, // params.x = time
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
            ShaderSource::Glsl {
                vertex: CHUNK_VERT,
                fragment: CHUNK_FRAG,
            },
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
            // `Less` (not `LessOrEqual`): a cube's top face is meshed BEFORE its side
            // faces, which share the top edge at the same y. With limited depth-buffer
            // precision the edge band rounds to equal depth; `LessOrEqual` let the
            // later-drawn (darker) side overwrite the top there → a dark scalloped fringe
            // along every cube rim. `Less` keeps the first-drawn top on ties → clean rim.
            depth_test: Comparison::Less,
            depth_write: true,
            // Back-face culling. Faces are wound clockwise as seen from OUTSIDE (the top
            // face's vertex order yields a -y geometric normal), so front = Clockwise.
            // This drops the inward/back faces of stacked & adjacent cubes (tree canopy),
            // whose coincident, differently-shaded quads z-fought into dashed seams.
            cull_face: CullFace::Back,
            front_face_order: FrontFaceOrder::Clockwise,
            color_blend: Some(BlendState::new(
                Equation::Add,
                BlendFactor::Value(BlendValue::SourceAlpha),
                BlendFactor::OneMinusValue(BlendValue::SourceAlpha),
            )),
            ..Default::default()
        },
    )
}

/// Pipeline for the translucent water surface (second pass). Same vertex layout as the
/// terrain (so `upload_chunks` feeds it unchanged); strict `Less` depth test against the
/// terrain depth already in the buffer, `depth_write: true` (see below), no face culling
/// (waves tilt the surface, and it's viewed from above). Alpha blended.
///
/// `depth_write: true`: water is one flat layer per pixel, so writing depth costs nothing,
/// and on the GL-on-Metal backend (macOS) a blended pass with `depth_write: false` had its
/// depth TEST mis-applied — water leaked over the far shore/forest (view-dependent). Writing
/// depth makes the occlusion correct.
///
/// `Less` (not `LessOrEqual`): a water surface sits a full voxel above its own seabed (many
/// depth steps with the visible-slab depth range), so it still draws over the bed while
/// losing depth TIES to any other terrain.
fn water_pipeline(ctx: &mut dyn RenderingBackend) -> Pipeline {
    let shader = ctx
        .new_shader(
            ShaderSource::Glsl {
                vertex: WATER_VERT,
                fragment: WATER_FRAG,
            },
            ShaderMeta {
                images: vec![],
                uniforms: UniformBlockLayout {
                    uniforms: vec![
                        UniformDesc::new("mvp", UniformType::Mat4),
                        UniformDesc::new("params", UniformType::Float4),
                    ],
                },
            },
        )
        .expect("water shader");
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
            depth_test: Comparison::Less,
            // depth_write MUST stay true. With `depth_write: false` the GL-on-Metal backend
            // (Apple's GL→Metal layer, what miniquad runs on macOS) mis-applies the depth
            // TEST for this blended pass: water fragments that are behind opaque terrain pass
            // anyway and the surface bleeds over the far shore/forest (view-dependent — only
            // the shore facing away from the camera). The water surface is a single layer per
            // pixel, so writing depth is harmless (no self-sorting artifacts) and makes the
            // occlusion correct.
            depth_write: true,
            cull_face: CullFace::Nothing,
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
        Some(c) if raw > c => {
            if lod_for(d - LOD_HYSTERESIS) > c {
                raw
            } else {
                c
            }
        }
        // Refining (moved closer): require d inside the boundary by the margin.
        Some(c) if raw < c => {
            if lod_for(d + LOD_HYSTERESIS) < c {
                raw
            } else {
                c
            }
        }
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
            free_chunks(ctx, &lc.water);
        }
        self.detail.clear();
        self.coarse.clear();
        self.ready.clear();
    }

    fn update(
        &mut self,
        ctx: &mut dyn RenderingBackend,
        t: &VoxelTerrain,
        center: (i32, i32),
        zoom: f32,
    ) {
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
                    free_chunks(ctx, &lc.water);
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
                    free_chunks(ctx, &old.water);
                }
                let (o, w) = build_chunk_mesh(t, cx as usize, cy as usize, lod);
                let lc = LoadedChunk {
                    opaque: upload_chunks(ctx, &o),
                    water: upload_chunks(ctx, &w),
                    lod,
                };
                self.detail.insert((cx, cy), lc);
            }
        } else if !self.detail.is_empty() {
            for lc in self.detail.values() {
                free_chunks(ctx, &lc.opaque);
                free_chunks(ctx, &lc.water);
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
            let (o, w) = build_region_mesh(t, x0, y0, x1, y1, COARSE_LOD);
            let lc = LoadedChunk {
                opaque: upload_chunks(ctx, &o),
                water: upload_chunks(ctx, &w),
                lod: COARSE_LOD,
            };
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

/// A world generation running on a background thread, so the render loop never blocks on
/// it. The worker produces a `Send` `VoxelTerrain` and ships it back over the channel; the
/// main thread polls `rx` each frame and reads `progress` (permille, 0..=1000) for the bar.
struct GenJob {
    rx: std::sync::mpsc::Receiver<VoxelTerrain>,
    progress: std::sync::Arc<std::sync::atomic::AtomicU32>,
    seed: u64,
}

/// Kick off background generation for `seed`. Generation is pure CPU (touches no GPU), so it
/// is safe off the main thread; meshes are still built on the main thread by the `Streamer`.
fn spawn_gen(seed: u64) -> GenJob {
    use std::sync::atomic::Ordering;
    let progress = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let (tx, rx) = std::sync::mpsc::channel();
    let p = progress.clone();
    std::thread::spawn(move || {
        let t = VoxelTerrain::generate(seed, &|f| {
            p.store((f.clamp(0.0, 1.0) * 1000.0) as u32, Ordering::Relaxed);
        });
        let _ = tx.send(t); // receiver may be gone if the app exited mid-gen — ignore
    });
    GenJob { rx, progress, seed }
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
                    "   pop {}   E {:.0}   bm {:.2}   multi {:.0}% carn {:.0}% auto {:.0}%   allop {:.2} crypsis {:.2}   nutri {:.2}   strata u{:.0}/s{:.0}/a{:.0}/w{:.0}   on-scr {on_screen}",
                    s.population(), s.avg_energy(), s.avg_biomass(), multi * 100.0,
                    s.frac_carnivore() * 100.0, s.frac_autotroph() * 100.0, s.thermal_correlation(t),
                    s.crypsis_correlation(t), s.avg_nutrient(t, clock.tick()),
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
    BiomeDef {
        surface,
        tree_density,
        tree,
    }
}

/// Indexed by `BiomeKind::id()` (0..12 used, 12..16 padded). Order matches the enum.
static BIOME_DEFS: [BiomeDef; 16] = [
    def((0.13, 0.32, 0.55), 0.0, TreeKind::None), // 0 Ocean
    def((0.84, 0.78, 0.54), 0.0, TreeKind::None), // 1 Beach
    def((0.42, 0.62, 0.30), 0.04, TreeKind::Broadleaf), // 2 Plains
    def((0.20, 0.46, 0.24), 0.30, TreeKind::Broadleaf), // 3 Forest
    def((0.80, 0.70, 0.44), 0.0, TreeKind::None), // 4 Desert
    def((0.48, 0.46, 0.45), 0.0, TreeKind::None), // 5 Mountain
    def((0.93, 0.95, 0.98), 0.02, TreeKind::Conifer), // 6 Snow
    def((0.17, 0.38, 0.29), 0.30, TreeKind::Conifer), // 7 Taiga
    def((0.62, 0.64, 0.56), 0.0, TreeKind::None), // 8 Tundra
    def((0.70, 0.66, 0.34), 0.03, TreeKind::Broadleaf), // 9 Savanna
    def((0.31, 0.40, 0.25), 0.14, TreeKind::Broadleaf), // 10 Swamp
    def((0.12, 0.43, 0.17), 0.50, TreeKind::Broadleaf), // 11 Jungle
    def((0.42, 0.62, 0.30), 0.0, TreeKind::None), // 12-15 padding
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
/// Returns `(opaque, water)`.
fn build_chunk_mesh(t: &VoxelTerrain, cx: usize, cy: usize, lod: u32) -> (Vec<Batch>, Vec<Batch>) {
    let x1 = (cx * CHUNK + CHUNK).min(COLS);
    let y1 = (cy * CHUNK + CHUNK).min(ROWS);
    build_region_mesh(t, cx * CHUNK, cy * CHUNK, x1, y1, lod)
}

/// Build the opaque + water meshes for an arbitrary column rectangle `[x0,x1) × [y0,y1)`
/// at `lod`, merged into as few batches as the per-draw limit allows. A single chunk uses
/// this for the streamed detail tier; a whole super-tile uses it for the coarse overview
/// tier (many chunks → a handful of buffers, so the whole map is a few hundred draws).
///
/// At LOD>0 columns are read on a `stride` grid (blocks aligned globally because `x0/y0`
/// are stride multiples) and each block emits one `stride×stride` footprint sampled from
/// its origin column, with neighbour heights read a stride away. Trees are full-detail
/// only. Returns `(opaque, water)`: the opaque seabed/land/trees, plus a translucent
/// water surface (one quad per submerged column + connective faces for river steps),
/// drawn in a second, animated pass.
fn build_region_mesh(
    t: &VoxelTerrain,
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    lod: u32,
) -> (Vec<Batch>, Vec<Batch>) {
    let stride = 1usize << lod;
    let si = stride as i32;
    let mut opaque = Vec::new();
    let mut verts: Vec<Vertex> = Vec::new();
    let mut idx: Vec<u16> = Vec::new();
    let mut water = Vec::new();
    let mut wv: Vec<Vertex> = Vec::new();
    let mut wi: Vec<u16> = Vec::new();
    // Trees are voxelised into a set first, then meshed with EXPOSED faces only (like the
    // terrain) — so overlapping canopies and adjacent cubes don't leave coincident,
    // differently-shaded faces that z-fight into dashed seams.
    let mut tvox: VoxMap = std::collections::HashMap::new();
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
            let nb = [
                (t.height(ix + si, iy), t.water_level(ix + si, iy), Face::Px),
                (t.height(ix - si, iy), t.water_level(ix - si, iy), Face::Nx),
                (t.height(ix, iy + si), t.water_level(ix, iy + si), Face::Pz),
                (t.height(ix, iy - si), t.water_level(ix, iy - si), Face::Nz),
            ];
            // A face that fronts a LOWER neighbour is a step edge; the top's rim verts on that
            // edge get darkened (fake AO), so a 1-cell dark band traces every height boundary —
            // otherwise two same-biome plateaus at different heights read as one flat tone.
            let drops = [nb[0].0 < h, nb[1].0 < h, nb[2].0 < h, nb[3].0 < h]; // [Px,Nx,Pz,Nz]
            push_top(&mut verts, &mut idx, gx, gy, stride, h, top_col, drops);
            for (nh, nwl, face) in nb {
                if nh < h {
                    // `nwl` = the neighbour's water surface: levels of this face below it are
                    // underwater (this neighbour is the water body fronting the face), so the
                    // mesher colours them as seabed rather than land strata.
                    push_side(
                        &mut verts,
                        &mut idx,
                        (gx, gy),
                        stride,
                        h,
                        nh,
                        face,
                        top_col,
                        rocky,
                        nwl,
                        lod == 0,
                    );
                }
            }

            // Translucent water surface over this column: a quad at the water level, depth
            // (`wl - h`) carried per vertex for the shader's depth shading. Connective side
            // faces ONLY toward a slightly LOWER water neighbour (a river step ≤ WATER_STEP_MAX)
            // so a descending river reads continuous; a BIG drop is two separate bodies (e.g.
            // mountain lake beside the sea) — no face there, which avoids the old "water walls".
            if submerged {
                let depth = wl - h;
                if wi.len() + COLUMN_INDEX_BURST > MAX_MESH_INDICES {
                    flush_mesh(&mut wv, &mut wi, &mut water);
                }
                push_water_top(&mut wv, &mut wi, gx, gy, stride, wl, depth);
                for (nx, ny, face) in [
                    (ix + si, iy, Face::Px),
                    (ix - si, iy, Face::Nx),
                    (ix, iy + si, Face::Pz),
                    (ix, iy - si, Face::Nz),
                ] {
                    let nwl = t.water_level(nx, ny);
                    if nwl > 0 && nwl < wl && wl - nwl <= WATER_STEP_MAX {
                        push_water_side(&mut wv, &mut wi, (gx, gy), stride, wl, nwl, depth, face);
                    }
                }
            }

            // Trees on dry land (through LOD1, one per block, so the canopy fades a ring
            // out instead of a hard edge). Water itself is a separate translucent pass.
            if !submerged && lod <= 1 {
                let bd = biome_def(biome);
                if bd.tree != TreeKind::None && feature_unit(t.seed, gx, gy, 101) < bd.tree_density
                {
                    collect_tree(&mut tvox, t, gx, gy, h, bd.tree);
                }
            }
        }
        gyc += stride;
    }
    mesh_tree_voxels(&mut verts, &mut idx, &mut opaque, &tvox);
    flush_mesh(&mut verts, &mut idx, &mut opaque);
    flush_mesh(&mut wv, &mut wi, &mut water);
    (opaque, water)
}

/// A sparse voxel set (position → colour) the trees are rasterised into before meshing.
type VoxMap = std::collections::HashMap<(i32, i32, u8), (f32, f32, f32)>;

/// Voxelise a tree on column `(gx, gy)` standing on surface height `h` into `vox`.
/// **Broadleaf**: short brown trunk under a 3×3 leaf canopy + cap (rounded, deciduous).
/// **Conifer**: taller trunk with a narrow tapering spire (1-cell tip over a + of leaves).
/// Per-column hashes keep it deterministic; canopy blocks overhanging the world / water are
/// skipped. Writing into a SET de-duplicates overlapping canopies (no coincident faces).
fn collect_tree(vox: &mut VoxMap, t: &VoxelTerrain, gx: usize, gy: usize, h: u8, kind: TreeKind) {
    let seed = t.seed;
    let trunk = (0.36, 0.26, 0.16);
    let leaf = if kind == TreeKind::Conifer {
        (0.09, 0.24, 0.16)
    } else {
        (0.16, 0.42, 0.20)
    };
    let (gxi, gyi) = (gx as i32, gy as i32);
    // Leaves are skipped over water / off-map; the trunk sits on the tree's own (valid) column.
    let leaf_at = |vox: &mut VoxMap, lx: i32, ly: i32, lz: u8| {
        if (0..COLS as i32).contains(&lx)
            && (0..ROWS as i32).contains(&ly)
            && t.water_level(lx, ly) == 0
        {
            vox.insert((lx, ly, lz), leaf);
        }
    };
    if kind == TreeKind::Conifer {
        let th = 3 + (feature_unit(seed, gx, gy, 202) * 2.0) as u8; // 3 or 4
        for gz in h..h + th {
            vox.insert((gxi, gyi, gz), trunk);
        }
        for (dx, dy) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
            leaf_at(vox, gxi + dx, gyi + dy, h + th);
        }
        leaf_at(vox, gxi, gyi, h + th + 1);
        leaf_at(vox, gxi, gyi, h + th + 2);
    } else {
        let th = 2 + (feature_unit(seed, gx, gy, 202) * 2.0) as u8; // 2 or 3
        for gz in h..h + th {
            vox.insert((gxi, gyi, gz), trunk);
        }
        let top = h + th;
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                leaf_at(vox, gxi + dx, gyi + dy, top);
            }
        }
        leaf_at(vox, gxi, gyi, top + 1);
    }
}

/// Mesh the tree voxel set, emitting only EXPOSED faces (a face is drawn only where the
/// neighbour voxel is absent) — exactly like the terrain mesher, so there are no interior
/// or coincident faces to z-fight. Bottom faces are omitted (unseen from the iso top-down
/// view), matching the terrain. Side faces bias their top edge back (the column's own top
/// wins the rim, no dark speckle).
fn mesh_tree_voxels(
    verts: &mut Vec<Vertex>,
    idx: &mut Vec<u16>,
    opaque: &mut Vec<Batch>,
    vox: &VoxMap,
) {
    for (&(gx, gy, gz), &rgb) in vox {
        if idx.len() + COLUMN_INDEX_BURST > MAX_MESH_INDICES {
            flush_mesh(verts, idx, opaque);
        }
        let (x0, x1) = (gx as f32 * VOX, (gx + 1) as f32 * VOX);
        let (z0, z1) = (gy as f32 * VOX, (gy + 1) as f32 * VOX);
        let (y0, y1) = (gz as f32 * VOX, (gz + 1) as f32 * VOX);
        // Side verts are bottom (0,1) then top (2,3). Bias the TOP edge back (the voxel's
        // own top wins the rim) AND the BOTTOM edge forward (toward camera): a tree is a
        // SEPARATE mesh sitting on the terrain, so its base edge ties the ground's top and
        // would otherwise be eaten ("saw") — the forward nudge makes the trunk win there.
        let top_back = [-1.0, -1.0, 1.0, 1.0];
        if !vox.contains_key(&(gx, gy, gz + 1)) {
            push_quad(
                verts,
                idx,
                [
                    vec3(x0, y1, z0),
                    vec3(x1, y1, z0),
                    vec3(x1, y1, z1),
                    vec3(x0, y1, z1),
                ],
                shaded(rgb, SHADE_TOP),
                0.0,
            );
        }
        if !vox.contains_key(&(gx + 1, gy, gz)) {
            push_quad_v(
                verts,
                idx,
                [
                    vec3(x1, y0, z0),
                    vec3(x1, y0, z1),
                    vec3(x1, y1, z1),
                    vec3(x1, y1, z0),
                ],
                shaded(rgb, SHADE_PX),
                top_back,
            );
        }
        if !vox.contains_key(&(gx - 1, gy, gz)) {
            push_quad_v(
                verts,
                idx,
                [
                    vec3(x0, y0, z1),
                    vec3(x0, y0, z0),
                    vec3(x0, y1, z0),
                    vec3(x0, y1, z1),
                ],
                shaded(rgb, SHADE_NX),
                top_back,
            );
        }
        if !vox.contains_key(&(gx, gy + 1, gz)) {
            push_quad_v(
                verts,
                idx,
                [
                    vec3(x1, y0, z1),
                    vec3(x0, y0, z1),
                    vec3(x0, y1, z1),
                    vec3(x1, y1, z1),
                ],
                shaded(rgb, SHADE_PZ),
                top_back,
            );
        }
        if !vox.contains_key(&(gx, gy - 1, gz)) {
            push_quad_v(
                verts,
                idx,
                [
                    vec3(x0, y0, z0),
                    vec3(x1, y0, z0),
                    vec3(x1, y1, z0),
                    vec3(x0, y1, z0),
                ],
                shaded(rgb, SHADE_NZ),
                top_back,
            );
        }
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

/// Per-vertex `back` flags (0/1) → uv.x; the shader nudges back=1 verts a hair toward the
/// far plane. Only the TOP edge of a side wall is flagged: that edge is shared with the
/// column's own top face (which must win the rim → no dark z-fight speckle), while the
/// wall's BOTTOM edge stays unbiased so it isn't eaten by the lower neighbour's top face.
fn push_quad_v(
    verts: &mut Vec<Vertex>,
    idx: &mut Vec<u16>,
    q: [Vec3; 4],
    col: Color,
    backs: [f32; 4],
) {
    push_quad_c(verts, idx, q, [col; 4], backs);
}

/// Quad with a PER-VERTEX colour (`cols`) and per-vertex `back` flag — used by the top face
/// to bake rim AO into individual corners.
fn push_quad_c(
    verts: &mut Vec<Vertex>,
    idx: &mut Vec<u16>,
    q: [Vec3; 4],
    cols: [Color; 4],
    backs: [f32; 4],
) {
    let base = verts.len() as u16;
    for ((p, b), c) in q.into_iter().zip(backs).zip(cols) {
        verts.push(Vertex::new(p.x, p.y, p.z, b, 0.0, c));
    }
    idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// Quad with a uniform `back` flag on all four verts.
fn push_quad(verts: &mut Vec<Vertex>, idx: &mut Vec<u16>, q: [Vec3; 4], col: Color, back: f32) {
    push_quad_v(verts, idx, q, col, [back; 4]);
}

/// Max level drop across which a connective water SIDE face is drawn (a river step). A
/// bigger drop means two separate water bodies (e.g. a mountain lake beside the sea), where
/// a face would stand as a tall spurious "water wall" — so it's capped.
const WATER_STEP_MAX: u8 = 2;

/// A water-surface quad covering column `(gx, gy)`'s `stride×stride` footprint at level
/// `wl`. `depth` (= `wl - terrain_h`, voxel levels) goes in every vertex's `uv.y` for the
/// water shader's depth shading; vertex colour is unused by the shader (placeholder WHITE).
fn push_water_top(
    verts: &mut Vec<Vertex>,
    idx: &mut Vec<u16>,
    gx: usize,
    gy: usize,
    s: usize,
    wl: u8,
    depth: u8,
) {
    let (x0, x1) = (gx as f32 * VOX, (gx + s) as f32 * VOX);
    let (z0, z1) = (gy as f32 * VOX, (gy + s) as f32 * VOX);
    let y = wl as f32 * VOX;
    let q = [
        vec3(x0, y, z0),
        vec3(x1, y, z0),
        vec3(x1, y, z1),
        vec3(x0, y, z1),
    ];
    push_water_quad(verts, idx, q, depth);
}

/// A water side face on one edge, from the lower neighbour surface `lo` up to this water
/// level `hi` — fills the vertical gap at a river step so the ribbon reads continuous.
#[allow(clippy::too_many_arguments)]
fn push_water_side(
    verts: &mut Vec<Vertex>,
    idx: &mut Vec<u16>,
    (gx, gy): (usize, usize),
    s: usize,
    hi: u8,
    lo: u8,
    depth: u8,
    face: Face,
) {
    let (x0, x1) = (gx as f32 * VOX, (gx + s) as f32 * VOX);
    let (z0, z1) = (gy as f32 * VOX, (gy + s) as f32 * VOX);
    let (y0, y1) = (lo as f32 * VOX, hi as f32 * VOX);
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
    push_water_quad(verts, idx, q, depth);
}

/// Emit a water quad: `uv.x = 0` (no terrain depth-nudge), `uv.y = depth`, colour WHITE
/// (the water shader computes colour/alpha from depth, ignoring vertex colour).
fn push_water_quad(verts: &mut Vec<Vertex>, idx: &mut Vec<u16>, q: [Vec3; 4], depth: u8) {
    let base = verts.len() as u16;
    let d = depth as f32;
    for p in q {
        verts.push(Vertex::new(p.x, p.y, p.z, 0.0, d, WHITE));
    }
    idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
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
        c = (
            c.0 + (0.33 - c.0) * k,
            c.1 + (0.45 - c.1) * k,
            c.2 + (0.29 - c.2) * k,
        ); // → moss
    }
    c
}

/// Contour line: width of the dark rim strip overlaid along a terrace edge, as a fraction
/// of ONE voxel (kept constant in world space, not scaled by LOD stride, so the line stays
/// thin on coarse far tiles). And how much it darkens the top colour.
const RIM_LINE_W: f32 = 0.02;
const RIM_LINE_SHADE: f32 = 0.6;

/// `drops` = [Px, Nx, Pz, Nz]: whether each neighbour edge steps DOWN from this column. A
/// thin dark strip is overlaid along every dropping edge (nudged toward the camera via
/// `uv.x=-1` so it wins the top plane without z-fight). Only step edges get it and the
/// strips run the full edge, so adjacent rim cells join into ONE continuous contour around
/// the terrace — interior cube joins (same height, no drop) stay unmarked.
#[allow(clippy::too_many_arguments)]
fn push_top(
    verts: &mut Vec<Vertex>,
    idx: &mut Vec<u16>,
    gx: usize,
    gy: usize,
    s: usize,
    h: u8,
    rgb: (f32, f32, f32),
    drops: [bool; 4],
) {
    let (x0, x1) = (gx as f32 * VOX, (gx + s) as f32 * VOX);
    let (z0, z1) = (gy as f32 * VOX, (gy + s) as f32 * VOX);
    let y = h as f32 * VOX;
    push_quad(
        verts,
        idx,
        [
            vec3(x0, y, z0),
            vec3(x1, y, z0),
            vec3(x1, y, z1),
            vec3(x0, y, z1),
        ],
        shaded(rgb, SHADE_TOP),
        0.0,
    );
    let [dpx, dnx, dpz, dnz] = drops;
    if dpx || dnx || dpz || dnz {
        let line = shaded(rgb, RIM_LINE_SHADE);
        let w = RIM_LINE_W * VOX;
        // Each strip keeps the top winding [(-,-),(+,-),(+,+),(-,+)]; `back=-1` nudges it
        // toward the camera so it deterministically wins the shared top plane.
        let mut strip = |ax0: f32, ax1: f32, az0: f32, az1: f32| {
            push_rim(
                verts,
                idx,
                [
                    vec3(ax0, y, az0),
                    vec3(ax1, y, az0),
                    vec3(ax1, y, az1),
                    vec3(ax0, y, az1),
                ],
                line,
            );
        };
        if dpx {
            strip(x1 - w, x1, z0, z1);
        }
        if dnx {
            strip(x0, x0 + w, z0, z1);
        }
        if dpz {
            strip(x0, x1, z1 - w, z1);
        }
        if dnz {
            strip(x0, x1, z0, z0 + w);
        }
    }
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
    nwl: u8,
    rim: bool,
) {
    let (x0, x1) = (gx as f32 * VOX, (gx + s) as f32 * VOX);
    let (z0, z1) = (gy as f32 * VOX, (gy + s) as f32 * VOX);
    // Face quad for a vertical [y0,y1] band, winding outward per face (shared by the strata
    // quads and the rim strip below).
    let wall = |y0: f32, y1: f32| match face {
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
    let shade = match face {
        Face::Px => SHADE_PX,
        Face::Nx => SHADE_NX,
        Face::Pz => SHADE_PZ,
        Face::Nz => SHADE_NZ,
    };
    for gz in nh..h {
        let (y0, y1) = (gz as f32 * VOX, (gz + 1) as f32 * VOX);
        // Levels below the fronting water's surface (`nwl`) are seabed, not land strata: a dry
        // shore column dropping into a lake/sea exposes a side face whose underwater part would
        // otherwise show the land (grass/dirt) colour and, through the translucent water, read
        // as "water drawn over land" (and only from the angle the face points at the camera —
        // hence view-dependent). `nwl` is the NEIGHBOUR's level, so it works for high lakes too,
        // not just the global sea. Colour by depth below that surface, matching submerged tops.
        let col = if gz < nwl {
            shaded(seabed_rgb(nwl - gz), shade)
        } else {
            shaded(strata_rgb(gz, h, top, rocky), shade)
        };
        let q = wall(y0, y1);
        // Bias only the wall's TOP edge (the topmost level quad's top verts 2,3) back, so
        // the column's top face wins that rim; the rest of the wall is unbiased.
        let backs = if gz + 1 == h {
            [0.0, 0.0, 1.0, 1.0]
        } else {
            [0.0; 4]
        };
        push_quad_v(verts, idx, q, col, backs);
    }
    // Vertical leg of the contour: a thin dark strip down the TOP of the wall from the rim,
    // overlaid (nudged toward the camera) so it wins the strata face. Together with the top
    // strip it wraps the edge in an L, so the step reads from the side too. LOD0 only.
    if rim {
        let yt = h as f32 * VOX;
        let yb = (yt - RIM_LINE_W * VOX).max(nh as f32 * VOX);
        push_rim(verts, idx, wall(yb, yt), shaded(top, RIM_LINE_SHADE));
    }
}

/// Emit a contour-overlay quad: `uv.x = -1` nudges it toward the camera so it wins the face
/// it sits on (no z-fight), `uv.y = 1` flags it so the fragment shader can hide the whole
/// outline when toggled off (key `O`), baring the face underneath.
fn push_rim(verts: &mut Vec<Vertex>, idx: &mut Vec<u16>, q: [Vec3; 4], col: Color) {
    let base = verts.len() as u16;
    for p in q {
        verts.push(Vertex::new(p.x, p.y, p.z, -1.0, 1.0, col));
    }
    idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
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
                let (op, wt) = build_chunk_mesh(&t, cx, cy, 0);
                for b in op.iter().chain(wt.iter()) {
                    any = true;
                    assert!(
                        b.mesh.vertices.len() < 10_000,
                        "verts {} at chunk ({cx},{cy})",
                        b.mesh.vertices.len()
                    );
                    assert!(
                        b.mesh.indices.len() < 5_000,
                        "indices {} at chunk ({cx},{cy})",
                        b.mesh.indices.len()
                    );
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
                let (op, wt) = build_chunk_mesh(&t, cx, cy, lod);
                let mut verts = 0;
                for b in op.iter().chain(wt.iter()) {
                    verts += b.mesh.vertices.len();
                    assert!(
                        b.mesh.vertices.len() < 10_000,
                        "lod {lod} verts overflow at ({cx},{cy})"
                    );
                    assert!(
                        b.mesh.indices.len() < 5_000,
                        "lod {lod} indices overflow at ({cx},{cy})"
                    );
                }
                assert!(
                    verts <= prev,
                    "lod {lod} not coarser at ({cx},{cy}): {verts} > {prev}"
                );
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
            let (op, wt) = build_region_mesh(&t, x0, y0, x1, y1, COARSE_LOD);
            let batches = op.len();
            for b in op.iter().chain(wt.iter()) {
                assert!(b.mesh.vertices.len() < 10_000, "coarse verts overflow");
                assert!(b.mesh.indices.len() < 5_000, "coarse indices overflow");
            }
            // A super-tile is SUPER² chunks; the merged coarse mesh must be far fewer
            // buffers than that (else the overview buys no draw-call reduction).
            assert!(
                batches < (SUPER * SUPER) as usize,
                "coarse not merged: {batches} batches"
            );
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
                let (op, wt) = build_chunk_mesh(&t, cx, cy, 0);
                for b in op.iter().chain(wt.iter()) {
                    verts += b.mesh.vertices.len();
                    batches += 1;
                }
            }
        }
        let mb = (verts * std::mem::size_of::<Vertex>()) as f64 / (1024.0 * 1024.0);
        eprintln!("MAP_SCALE={MAP_SCALE} SURFACE_RANGE={SURFACE_RANGE}: {verts} verts, {batches} batches, ~{mb:.0} MB if all resident");
    }
}
