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

// The GLSL shaders live in `assets/shaders/{chunk,water}.{vert,frag}` (loaded at startup, with an
// `include_str!` fallback baked in — see `main`). The pipeline builders take the sources as args.

/// Build the opaque-chunk render pipeline (position + vertex colour; depth-tested).
pub fn chunk_pipeline(ctx: &mut dyn RenderingBackend, vertex: &str, fragment: &str) -> Pipeline {
    let shader = ctx
        .new_shader(
            ShaderSource::Glsl {
                vertex,
                fragment,
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
pub fn water_pipeline(ctx: &mut dyn RenderingBackend, vertex: &str, fragment: &str) -> Pipeline {
    let shader = ctx
        .new_shader(
            ShaderSource::Glsl {
                vertex,
                fragment,
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
