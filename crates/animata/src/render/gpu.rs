/// GPU pipeline and buffer management for terrain and water rendering.
use macroquad::prelude::*;
use macroquad::miniquad::{
    Bindings, BlendFactor, BlendState, BlendValue, BufferSource, BufferType, BufferUsage,
    Comparison, CullFace, Equation, FrontFaceOrder, Pipeline, PipelineParams,
    RenderingBackend, ShaderMeta, ShaderSource, UniformBlockLayout, UniformDesc, UniformType,
    VertexAttribute, VertexFormat,
};

use super::mesh::Batch;

#[repr(C)]
pub struct ChunkUniforms {
    pub mvp: Mat4,
    pub dbg: Vec4,
}

#[repr(C)]
pub struct WaterUniforms {
    pub mvp: Mat4,
    pub params: Vec4, // params.x = time
}

/// One chunk's geometry living in immutable GPU buffers, plus its world AABB for culling.
pub struct GpuChunk {
    pub bindings: Bindings,
    pub n_idx: i32,
    pub lo: Vec3,
    pub hi: Vec3,
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

/// Build the opaque-chunk render pipeline (position + vertex colour; depth-tested).
pub fn chunk_pipeline(ctx: &mut dyn RenderingBackend) -> Pipeline {
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
pub fn water_pipeline(ctx: &mut dyn RenderingBackend) -> Pipeline {
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
pub fn upload_chunks(ctx: &mut dyn RenderingBackend, batches: &[Batch]) -> Vec<GpuChunk> {
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
pub fn free_chunks(ctx: &mut dyn RenderingBackend, chunks: &[GpuChunk]) {
    for c in chunks {
        ctx.delete_buffer(c.bindings.vertex_buffers[0]);
        ctx.delete_buffer(c.bindings.index_buffer);
    }
}

// ---- Chunk streaming -------------------------------------------------------------
