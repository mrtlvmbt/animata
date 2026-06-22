//! GPU erosion accelerator (OPT-IN, feature `gpu`). A wgpu compute path for the two heavy
//! worldgen erosion phases — hydraulic droplet + thermal talus — that the CPU `erosion::erode`
//! otherwise runs in ~9.5 s. This module is **never** part of the deterministic golden/CI build
//! (the CPU path is the reference); it produces a geomorphically-equivalent but NOT bit-identical
//! surface, which is acceptable only because worldgen geometry is regenerated-from-seed and is NOT
//! in `state_checksum` (see `terrain.rs`). See `crates/.../plans` / module docs in `erosion.rs`.
//!
//! Determinism note: the per-droplet spawn positions are precomputed on the CPU with the SAME
//! `seed_fold`/`Rng` stream as the CPU path and uploaded — so the shader carries no RNG (WGSL has
//! no 64-bit integers, so splitmix64 can't run there). The only CPU↔GPU divergence is then the
//! float trajectory math itself (sqrt/division/FMA reorder + non-atomic read-clamp), exactly the
//! accepted, measured divergence.

mod erosion;
pub use erosion::erode_gpu;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

/// Process-level toggle: when set, `erosion::erode` routes to the GPU path. Set ONLY by the
/// benchmark and the experimental in-app control — never by `generate()`'s default/load callers,
/// so saved worlds always re-erode on the CPU (a GPU-eroded world must not be persisted).
static GPU_ENABLED: AtomicBool = AtomicBool::new(false);

/// Enable/disable the GPU erosion path for this process. Returns the previous value.
pub fn set_gpu_erosion(on: bool) -> bool {
    GPU_ENABLED.swap(on, Ordering::Relaxed)
}

/// Whether the GPU erosion path is currently selected.
pub fn gpu_erosion_enabled() -> bool {
    GPU_ENABLED.load(Ordering::Relaxed)
}

/// A cached wgpu device+queue. Created once (the first time the GPU path runs) and reused across
/// every `generate()` — wgpu device init is not cheap, and there is no per-worldgen reason to
/// repeat it. `None` if no compute adapter is available (caller falls back to the CPU path).
pub struct GpuCtx {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

static CTX: OnceLock<Option<Arc<GpuCtx>>> = OnceLock::new();

/// The shared GPU context, or `None` if this machine has no usable compute adapter.
pub fn ctx() -> Option<Arc<GpuCtx>> {
    CTX.get_or_init(init_ctx).clone()
}

fn init_ctx() -> Option<Arc<GpuCtx>> {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await?;
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("animata-erosion"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_defaults(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await
            .ok()?;
        Some(Arc::new(GpuCtx { device, queue }))
    })
}

impl GpuCtx {
    /// A storage buffer initialised from `data` (usage STORAGE | COPY_SRC | COPY_DST).
    pub fn storage_from<T: bytemuck::Pod>(&self, label: &str, data: &[T]) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(data),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        })
    }

    /// A zeroed storage buffer of `len` elements of type `T`.
    pub fn storage_zeroed<T: bytemuck::Pod>(&self, label: &str, len: usize) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: (len * std::mem::size_of::<T>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// A small uniform buffer initialised from one `Pod` value.
    pub fn uniform_from<T: bytemuck::Pod>(&self, label: &str, value: &T) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::bytes_of(value),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    /// Block until the GPU is idle, then read `len` elements of type `T` back from `src`. Returns
    /// `None` on a map/channel failure (a driver error) rather than panicking — the caller treats
    /// that as "GPU declined" and falls back to the CPU path (the documented fallback contract).
    pub fn read_back<T: bytemuck::Pod>(&self, src: &wgpu::Buffer, len: usize) -> Option<Vec<T>> {
        let bytes = (len * std::mem::size_of::<T>()) as u64;
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback-staging"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        enc.copy_buffer_to_buffer(src, 0, &staging, 0, bytes);
        self.queue.submit(Some(enc.finish()));
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv().ok()?.ok()?; // channel dropped, or the map itself failed → fall back to CPU
        let out: Vec<T> = bytemuck::cast_slice(&slice.get_mapped_range()).to_vec();
        staging.unmap();
        Some(out)
    }
}

/// Pipeline smoke test (scaffold proof): multiply every element of `data` by `factor` on the GPU
/// and read it back. Exercises upload → bind group → dispatch → readback end-to-end. Used by the
/// scaffold test before the real erosion kernels are trusted.
#[cfg(test)]
pub fn round_trip_scale(ctx: &GpuCtx, data: &[f32], factor: f32) -> Option<Vec<f32>> {
    const SRC: &str = r#"
@group(0) @binding(0) var<storage, read_write> buf: array<f32>;
@group(0) @binding(1) var<uniform> factor: f32;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&buf)) { return; }
    buf[i] = buf[i] * factor;
}
"#;
    let dev = &ctx.device;
    let buf = ctx.storage_from("scale-buf", data);
    let fbuf = ctx.uniform_from("scale-factor", &factor);
    let shader = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("scale"),
        source: wgpu::ShaderSource::Wgsl(SRC.into()),
    });
    let pipeline = dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("scale"),
        layout: None,
        module: &shader,
        entry_point: "main",
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    let bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: fbuf.as_entire_binding() },
        ],
    });
    let mut enc = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    {
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None, timestamp_writes: None });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bg, &[]);
        let groups = (data.len() as u32).div_ceil(64);
        pass.dispatch_workgroups(groups, 1, 1);
    }
    ctx.queue.submit(Some(enc.finish()));
    ctx.read_back(&buf, data.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_round_trip_scale() {
        let Some(ctx) = ctx() else {
            eprintln!("no GPU adapter — skipping gpu_round_trip_scale");
            return;
        };
        let data: Vec<f32> = (0..1000).map(|i| i as f32).collect();
        let out = round_trip_scale(&ctx, &data, 3.0).expect("gpu readback");
        assert_eq!(out.len(), data.len());
        for (i, (&o, &d)) in out.iter().zip(&data).enumerate() {
            assert!((o - d * 3.0).abs() < 1e-3, "mismatch at {i}: {o} != {}", d * 3.0);
        }
    }
}
