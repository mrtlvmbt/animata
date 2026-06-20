/// LOD-based chunk streaming and mesh upload/deallocation.
use macroquad::prelude::*;
use macroquad::miniquad::RenderingBackend;
use animata_sim::config::*;
use animata_sim::terrain::VoxelTerrain;

use crate::render::camera::IsoCam;
use crate::render::gpu::{GpuChunk, upload_chunks, free_chunks};
use crate::render::mesh::{build_chunk_mesh, build_region_mesh};

pub struct LoadedChunk {
    pub opaque: Vec<GpuChunk>,
    pub water: Vec<GpuChunk>,
    pub lod: u32,
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
pub const SUPER: i32 = 8; // chunks per super-tile side
const DETAIL_SUPER_R: i32 = 2; // super-tiles around the camera kept at per-chunk detail
pub const COARSE_LOD: u32 = 3; // stride-8 overview
/// Past this zoom the detail tier is dropped entirely — pure coarse whole-map overview,
/// so a full zoom-out costs only the (few hundred) coarse super-tile draws.
const DETAIL_ZOOM_CUTOFF: f32 = 520.0;

/// LOD for a detail chunk at Euclidean distance `d` (chunks) from the camera centre,
/// grading 0→1→2→`COARSE_LOD` in concentric rings so the detail edge matches the coarse tier.
pub fn lod_for(d: i32) -> u32 {
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
pub fn chunk_dist(cx: i32, cy: i32, ccx: i32, ccy: i32) -> i32 {
    let (dx, dy) = (cx - ccx, cy - ccy);
    ((dx * dx + dy * dy) as f32).sqrt() as i32
}

/// LOD with HYSTERESIS: a chunk already at `cur` only switches once the camera distance
/// clears the ring boundary by `LOD_HYSTERESIS` chunks, so a camera hovering on a ring
/// edge doesn't flip-flop the chunk's LOD (rebuild thrash + visible popping) every frame.
pub fn lod_hyst(d: i32, cur: Option<u32>) -> u32 {
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

pub struct Streamer {
    pub detail: ChunkMap, // per-chunk, in the detail super-tiles
    pub coarse: ChunkMap, // per super-tile, whole-map overview (always resident)
    /// Super-tiles whose detail is FULLY built — the renderer draws their detail chunks
    /// and SKIPS their coarse twin. Until a super-tile is ready it shows coarse, so a
    /// detail→coarse swap never flashes empty and the tiers never overlap.
    pub ready: std::collections::HashSet<(i32, i32)>,
}

impl Streamer {
    pub fn new() -> Self {
        Streamer {
            detail: ChunkMap::new(),
            coarse: ChunkMap::new(),
            ready: std::collections::HashSet::new(),
        }
    }

    pub fn clear(&mut self, ctx: &mut dyn RenderingBackend) {
        for lc in self.detail.values().chain(self.coarse.values()) {
            free_chunks(ctx, &lc.opaque);
            free_chunks(ctx, &lc.water);
        }
        self.detail.clear();
        self.coarse.clear();
        self.ready.clear();
    }

    pub fn update(
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
pub fn center_chunk(cam: &IsoCam) -> (i32, i32) {
    (
        (cam.target.x / (CHUNK as f32 * VOX)).floor() as i32,
        (cam.target.z / (CHUNK as f32 * VOX)).floor() as i32,
    )
}

// `max_zoom` and `ground_under_cursor` live in `render::input` (the input pipeline owns
// cursor/zoom math); imported there and from `main`.

/// A world generation running on a background thread, so the render loop never blocks on
/// it. The worker produces a `Send` `VoxelTerrain` and ships it back over the channel; the
/// main thread polls `rx` each frame and reads `progress` (permille, 0..=1000) for the bar.
pub struct GenJob {
    pub rx: std::sync::mpsc::Receiver<VoxelTerrain>,
    pub progress: std::sync::Arc<std::sync::atomic::AtomicU32>,
}

/// Kick off background generation for `seed`. Generation is pure CPU (touches no GPU), so it
/// is safe off the main thread; meshes are still built on the main thread by the `Streamer`.
pub fn spawn_gen(seed: u64) -> GenJob {
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
    GenJob { rx, progress }
}

