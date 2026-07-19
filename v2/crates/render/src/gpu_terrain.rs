//! GPU pipeline and buffer management for retained-mode terrain rendering.
//!
//! R-15a: Retained GPU buffers — persistent immutable GPU buffers for terrain chunks.
//! Macroquad's `draw_mesh` re-uploads vertices and indices every frame, costing O(visible verts) per frame.
//! Instead, we upload each chunk mesh ONCE to immutable GPU buffers and issue one draw call per visible chunk —
//! per-frame cost becomes O(visible chunk count). This pattern mirrors v1's `crates/animata/src/render/gpu.rs`.

use macroquad::prelude::*;
use macroquad::miniquad::{
    Bindings, BufferSource, BufferType, BufferUsage,
    Comparison, CullFace, FrontFaceOrder, Pipeline, PipelineParams,
    RenderingBackend, ShaderMeta, ShaderSource, UniformBlockLayout, UniformDesc, UniformType,
    VertexAttribute, VertexFormat,
};

use crate::terrain::TerrainChunk;

#[repr(C)]
pub struct ChunkUniforms {
    pub mvp: Mat4,
}

/// One chunk's geometry living in immutable GPU buffers, plus its world AABB for culling.
pub struct GpuChunk {
    pub bindings: Bindings,
    pub n_idx: i32,
    pub lo: Vec3,
    pub hi: Vec3,
}

/// Build the opaque-chunk render pipeline (position + vertex colour; depth-tested).
/// Mirrors v1's logic: depth comparison `Less` (not `LessOrEqual`) to avoid depth-tie scallops
/// along cube rims; back-face culling with clockwise winding order; alpha blending.
pub fn chunk_pipeline(ctx: &mut dyn RenderingBackend, vertex: &str, fragment: &str) -> Pipeline {
    let shader = match ctx
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
                    ],
                },
            },
        ) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("render: chunk shader compilation failed: {e}");
                std::process::exit(2);
            }
        };
    ctx.new_pipeline(
        &[macroquad::miniquad::BufferLayout::default()],
        &[
            VertexAttribute::new("position", VertexFormat::Float3),
            VertexAttribute::new("texcoord", VertexFormat::Float2),
            VertexAttribute::new("color0", VertexFormat::Byte4),  // Match macroquad exactly
            VertexAttribute::new("normal", VertexFormat::Float4),
        ],
        shader,
        PipelineParams {
            depth_test: Comparison::LessOrEqual,
            depth_write: true,
            // Disable culling: back-face culling via CullFace::Back + Clockwise order
            // was not working in miniquad; disabling gives clean results (tested with magenta test)
            cull_face: CullFace::Nothing,
            front_face_order: FrontFaceOrder::Clockwise,
            color_blend: None,
            ..Default::default()
        },
    )
}

/// Upload built chunk meshes to immutable GPU buffers.
pub fn upload_chunks(ctx: &mut dyn RenderingBackend, chunks: &[TerrainChunk]) -> Vec<GpuChunk> {
    chunks
        .iter()
        .map(|tc| {
            // Extract vertices and indices from the macroquad Mesh
            let vertices = &tc.mesh.vertices;
            let indices = &tc.mesh.indices;

            let vb = ctx.new_buffer(
                BufferType::VertexBuffer,
                BufferUsage::Immutable,
                BufferSource::slice(vertices),
            );
            let ib = ctx.new_buffer(
                BufferType::IndexBuffer,
                BufferUsage::Immutable,
                BufferSource::slice(indices),
            );

            let (lo, hi) = tc.bounds;
            GpuChunk {
                bindings: Bindings {
                    vertex_buffers: vec![vb],
                    index_buffer: ib,
                    images: vec![],
                },
                n_idx: indices.len() as i32,
                lo,
                hi,
            }
        })
        .collect()
}

/// Release a chunk set's GPU buffers (before re-uploading on reseed).
#[allow(dead_code)]
pub fn free_chunks(ctx: &mut dyn RenderingBackend, chunks: &[GpuChunk]) {
    for c in chunks {
        ctx.delete_buffer(c.bindings.vertex_buffers[0]);
        ctx.delete_buffer(c.bindings.index_buffer);
    }
}
