//! GPU erosion host side — drives the WGSL kernels in `erosion.wgsl`. Mirrors the CPU
//! `erosion::erode` phase order (hydraulic batches → thermal passes → clamp) on a single
//! GPU-resident `elev` buffer: one upload in, N cheap on-GPU dispatches, one download out.

use crate::config::{COLS, ROWS};
use crate::erosion::{
    DEPOSIT_SPEED, DROPLET_BATCH, DROPLET_FRACTION, EROSION_RADIUS, ERODE_SPEED, EVAPORATE, GRAVITY,
    INERTIA, MAX_LIFETIME, MIN_CAPACITY, SALT_EROSION, SEDIMENT_CAPACITY, START_SPEED, START_WATER,
    TALUS, THERMAL_PASSES, THERMAL_RATE,
};
use crate::gpu::ctx;
use crate::rng::{seed_fold, Rng};

/// Fixed-point factor for the atomic edit buffer (Δheight × SCALE → i32). 1<<20 gives ~1e-6
/// resolution and ~2047 height-units of i32 headroom — far above any per-batch/per-cell Δ.
const SCALE: f32 = (1u32 << 20) as f32;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    cols: u32,
    rows: u32,
    max_lifetime: u32,
    brush_len: u32,
    inertia: f32,
    sediment_capacity: f32,
    min_capacity: f32,
    erode_speed: f32,
    deposit_speed: f32,
    evaporate: f32,
    gravity: f32,
    start_water: f32,
    start_speed: f32,
    scale: f32,
    talus: f32,
    thermal_rate: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Batch {
    start: u32,
    count: u32,
    _pad0: u32,
    _pad1: u32,
}

/// The circular deposit/erode brush — bit-for-bit the same construction as `hydraulic`'s inline
/// brush, packed as (bx, by, weight, 0) for upload.
fn build_brush() -> Vec<[f32; 4]> {
    let r = EROSION_RADIUS;
    let mut brush: Vec<[f32; 4]> = Vec::new();
    let mut wsum = 0.0f32;
    for by in -r..=r {
        for bx in -r..=r {
            let d = ((bx * bx + by * by) as f32).sqrt();
            if d <= r as f32 {
                let w = 1.0 - d / r as f32;
                brush.push([bx as f32, by as f32, w, 0.0]);
                wsum += w;
            }
        }
    }
    for e in brush.iter_mut() {
        e[2] /= wsum;
    }
    brush
}

/// Per-droplet spawn positions, drawn with the SAME `seed_fold`/`Rng` stream as the CPU
/// `simulate_droplet` (which draws exactly two units: px then py, and no RNG afterwards) — so the
/// shader needs no PRNG. `num` matches `hydraulic`'s droplet count.
fn build_spawns(seed: u64, num: u64) -> Vec<[f32; 2]> {
    (0..num)
        .map(|d| {
            let mut rng = Rng::new(seed_fold(seed, &[SALT_EROSION, d]));
            let px = rng.unit() * (COLS - 1) as f32;
            let py = rng.unit() * (ROWS - 1) as f32;
            [px, py]
        })
        .collect()
}

/// Erode `elev` in place on the GPU (hydraulic + thermal + clamp). Returns `false` (caller falls
/// back to the CPU) if no GPU adapter is available; panics are avoided in favour of that fallback.
pub fn erode_gpu(seed: u64, elev: &mut [f32]) -> bool {
    let Some(ctx) = ctx() else { return false };
    let dev = &ctx.device;
    let n = COLS * ROWS;
    let num = ((COLS * ROWS) as f32 * DROPLET_FRACTION) as u64;

    let spawns = build_spawns(seed, num);
    let brush = build_brush();
    let params = Params {
        cols: COLS as u32,
        rows: ROWS as u32,
        max_lifetime: MAX_LIFETIME,
        brush_len: brush.len() as u32,
        inertia: INERTIA,
        sediment_capacity: SEDIMENT_CAPACITY,
        min_capacity: MIN_CAPACITY,
        erode_speed: ERODE_SPEED,
        deposit_speed: DEPOSIT_SPEED,
        evaporate: EVAPORATE,
        gravity: GRAVITY,
        start_water: START_WATER,
        start_speed: START_SPEED,
        scale: SCALE,
        talus: TALUS,
        thermal_rate: THERMAL_RATE,
    };

    // Buffers (one upload of `elev`; everything else stays GPU-resident for the whole phase).
    let elev_buf = ctx.storage_from("elev", elev);
    let edit_buf = ctx.storage_zeroed::<i32>("edit", n);
    let spawn_buf = ctx.storage_from("spawns", &spawns);
    let brush_buf = ctx.storage_from("brush", &brush);
    let params_buf = ctx.uniform_from("params", &params);
    let batch_buf = ctx.uniform_from("batch", &Batch { start: 0, count: 0, _pad0: 0, _pad1: 0 });
    let overflow_buf = ctx.storage_zeroed::<u32>("overflow", 1);

    let shader = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("erosion"),
        source: wgpu::ShaderSource::Wgsl(include_str!("erosion.wgsl").into()),
    });
    let pipe = |entry: &str| {
        dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry),
            layout: None,
            module: &shader,
            entry_point: entry,
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        })
    };
    let droplet_pipe = pipe("droplet");
    let thermal_pipe = pipe("thermal");
    let resolve_pipe = pipe("resolve");
    let clamp_pipe = pipe("clamp_field");

    fn entry(b: u32, buf: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
        wgpu::BindGroupEntry { binding: b, resource: buf.as_entire_binding() }
    }
    // Auto-layout bind groups, one per pipeline — each lists only the bindings that entry point uses.
    let droplet_bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("droplet-bg"),
        layout: &droplet_pipe.get_bind_group_layout(0),
        entries: &[
            entry(0, &elev_buf),
            entry(1, &edit_buf),
            entry(2, &spawn_buf),
            entry(3, &brush_buf),
            entry(4, &params_buf),
            entry(5, &batch_buf),
        ],
    });
    let thermal_bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("thermal-bg"),
        layout: &thermal_pipe.get_bind_group_layout(0),
        entries: &[entry(0, &elev_buf), entry(1, &edit_buf), entry(4, &params_buf)],
    });
    let resolve_bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("resolve-bg"),
        layout: &resolve_pipe.get_bind_group_layout(0),
        entries: &[entry(0, &elev_buf), entry(1, &edit_buf), entry(4, &params_buf), entry(6, &overflow_buf)],
    });
    let clamp_bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("clamp-bg"),
        layout: &clamp_pipe.get_bind_group_layout(0),
        entries: &[entry(0, &elev_buf), entry(4, &params_buf)],
    });

    let cell_groups = (n as u32).div_ceil(64);

    // Hydraulic: one submit per snapshot batch (so the batch uniform write is ordered before its
    // dispatch); each batch = droplet scatter then resolve into `elev`. Matches the CPU's
    // DROPLET_BATCH partition so channels deepen the same way across batches.
    let mut done = 0u64;
    while done < num {
        let count = DROPLET_BATCH.min(num - done) as u32;
        ctx.queue.write_buffer(
            &batch_buf,
            0,
            bytemuck::bytes_of(&Batch { start: done as u32, count, _pad0: 0, _pad1: 0 }),
        );
        let mut enc = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("hydraulic") });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None, timestamp_writes: None });
            pass.set_pipeline(&droplet_pipe);
            pass.set_bind_group(0, &droplet_bg, &[]);
            pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
        }
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None, timestamp_writes: None });
            pass.set_pipeline(&resolve_pipe);
            pass.set_bind_group(0, &resolve_bg, &[]);
            pass.dispatch_workgroups(cell_groups, 1, 1);
        }
        ctx.queue.submit(Some(enc.finish()));
        done += count as u64;
    }

    // Thermal: each pass = scatter then resolve, on the surface left by hydraulic.
    for _ in 0..THERMAL_PASSES {
        let mut enc = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("thermal") });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None, timestamp_writes: None });
            pass.set_pipeline(&thermal_pipe);
            pass.set_bind_group(0, &thermal_bg, &[]);
            pass.dispatch_workgroups(cell_groups, 1, 1);
        }
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None, timestamp_writes: None });
            pass.set_pipeline(&resolve_pipe);
            pass.set_bind_group(0, &resolve_bg, &[]);
            pass.dispatch_workgroups(cell_groups, 1, 1);
        }
        ctx.queue.submit(Some(enc.finish()));
    }

    // Final clamp to [0,1].
    {
        let mut enc = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("clamp") });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None, timestamp_writes: None });
            pass.set_pipeline(&clamp_pipe);
            pass.set_bind_group(0, &clamp_bg, &[]);
            pass.dispatch_workgroups(cell_groups, 1, 1);
        }
        ctx.queue.submit(Some(enc.finish()));
    }

    // One download. On a readback failure (driver error), leave `elev` untouched and decline so the
    // caller re-runs the CPU path on the original field (the documented fallback — never panic).
    let Some(out) = ctx.read_back::<f32>(&elev_buf, n) else { return false };
    elev.copy_from_slice(&out);
    // Surface the overflow guard (the harness asserts it stayed clear); a failed read is non-fatal.
    if let Some(overflow) = ctx.read_back::<u32>(&overflow_buf, 1) {
        if overflow[0] != 0 {
            eprintln!("WARNING: gpu erosion fixed-point edit buffer neared i32 saturation (raise SCALE)");
        }
    }
    true
}
