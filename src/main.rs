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
mod terrain;

use config::*;
use macroquad::prelude::*;
use terrain::{
    cell_biome, cell_flags, cell_height, feature_unit, BiomeKind, VoxelTerrain, FLAG_WATER, SEA_ABS,
};

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

#[macroquad::main(window_conf)]
async fn main() {
    let mut cam = IsoCam::new();
    let mut seed: u64 = 1;
    let mut terrain = VoxelTerrain::new(seed);
    let mut meshes = build_world_meshes(&terrain);

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
        let wheel = mouse_wheel().1;
        if wheel != 0.0 {
            cam.zoom = (cam.zoom * (1.0 - wheel.signum() * 0.1)).clamp(8.0, 600.0);
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
            meshes = build_world_meshes(&terrain);
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
                                 "meshes": meshes.opaque.len() + meshes.water.len(),
                                 "water_meshes": meshes.water.len() },
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
                        cam.zoom = v.clamp(8.0, 600.0);
                    }
                    if let Some(v) = yaw {
                        cam.yaw = v;
                    }
                    let _ = reply.send(serde_json::json!({"ok": true}));
                }
                dev_bridge::Cmd::Reseed { seed: s } => {
                    seed = s.unwrap_or(seed.wrapping_add(1));
                    terrain = VoxelTerrain::new(seed);
                    meshes = build_world_meshes(&terrain);
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

        // Pass 1: draw the scene into the offscreen target, frustum-culled per chunk.
        let mut scene_cam = cam.camera();
        scene_cam.render_target = Some(scene_rt.clone());
        set_camera(&scene_cam);
        clear_background(Color::new(0.53, 0.62, 0.78, 1.0)); // sky
        let vp = scene_cam.matrix();
        // Opaque first (terrain + trees) so the depth buffer is complete...
        let mut drawn = 0usize;
        for b in &meshes.opaque {
            if aabb_in_view(&vp, b.lo, b.hi) {
                draw_mesh(&b.mesh);
                drawn += 1;
            }
        }
        // ...then the translucent water plane on top. It's coplanar at SEA_ABS and one
        // quad per column, so no two water quads overlap on screen — no back-to-front
        // sort or depth-write toggle needed; the depth buffer hides water behind taller
        // land and the default alpha blend shows the sea floor through it.
        for b in &meshes.water {
            if aabb_in_view(&vp, b.lo, b.hi) {
                draw_mesh(&b.mesh);
                drawn += 1;
            }
        }
        set_default_camera();

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
        let total = meshes.opaque.len() + meshes.water.len();
        let line = format!(
            "{fps:.0} fps   {frame_ms:.2} ms   seed {seed}   {COLS}x{ROWS} m   chunks {drawn}/{total}"
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

// ---- Render-side palette (representation; kept out of the generator) ----

/// Surface (top-face) base colour per biome.
fn top_rgb(biome: BiomeKind) -> (f32, f32, f32) {
    match biome {
        BiomeKind::Ocean => (0.13, 0.32, 0.55),
        BiomeKind::Beach => (0.84, 0.78, 0.54),
        BiomeKind::Plains => (0.42, 0.62, 0.30),
        BiomeKind::Forest => (0.20, 0.46, 0.24),
        BiomeKind::Desert => (0.80, 0.70, 0.44),
        BiomeKind::Mountain => (0.48, 0.46, 0.45),
        BiomeKind::Snow => (0.93, 0.95, 0.98),
    }
}

/// Side-wall colour for the exposed level `gz` of a column of height `h`: a thin
/// biome "lip" just under the surface, then topsoil, then stone deeper down.
fn strata_rgb(gz: u8, h: u8, biome: BiomeKind) -> (f32, f32, f32) {
    if gz + 1 == h {
        let (r, g, b) = top_rgb(biome);
        (r * 0.85, g * 0.85, b * 0.85)
    } else if gz + 3 >= h {
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

/// The world split into two draw lists: solid geometry and the translucent water
/// plane. They are drawn in that order (opaque fills the depth buffer, water blends
/// over it) — see the render loop.
struct WorldMeshes {
    opaque: Vec<Batch>,
    water: Vec<Batch>,
}

/// macroquad's `draw_mesh` pushes through the immediate batch buffer, which **clamps**
/// (silently dropping geometry) at `>= 10000` vertices or `>= 5000` indices per call.
/// Indices bind first (6 per quad vs 4 verts), so we split meshes on the index count,
/// keeping a margin for the largest single-column burst (top + 4 cliff sides + a tree).
const MAX_MESH_INDICES: usize = 4800;
const COLUMN_INDEX_BURST: usize = 768;

/// Build the chunk meshes (one cached `Mesh` per chunk) plus the water plane. Each
/// land column emits its top quad and, for every lower horizontal neighbour, the
/// cliff side faces from the neighbour's height up to its own (one quad per level →
/// strata bands); neighbour heights come from the chunk's ghost ring, so this is
/// self-contained. Forest/Plains columns also grow a voxel tree. Water columns add a
/// single translucent surface quad at `SEA_ABS` to the separate water list.
fn build_world_meshes(t: &VoxelTerrain) -> WorldMeshes {
    let mut opaque = Vec::new();
    let mut water = Vec::new();
    // The water plane is coplanar everywhere, so it batches across chunks freely.
    let mut wv: Vec<Vertex> = Vec::new();
    let mut wi: Vec<u16> = Vec::new();
    for cy in 0..t.chunks_y {
        for cx in 0..t.chunks_x {
            let ch = t.chunk(cx, cy);
            let mut verts: Vec<Vertex> = Vec::new();
            let mut idx: Vec<u16> = Vec::new();
            for ly in 0..CHUNK {
                for lx in 0..CHUNK {
                    let (gx, gy) = (cx * CHUNK + lx, cy * CHUNK + ly);
                    if gx >= COLS || gy >= ROWS {
                        continue; // partial edge chunk: outside the world
                    }
                    let cell = ch.interior(lx, ly);
                    let h = cell_height(cell);
                    if h == 0 {
                        continue; // air
                    }
                    // Split before macroquad's per-drawcall batch limit (see consts).
                    if idx.len() + COLUMN_INDEX_BURST > MAX_MESH_INDICES {
                        flush_mesh(&mut verts, &mut idx, &mut opaque);
                    }
                    let biome = cell_biome(cell);
                    push_top(&mut verts, &mut idx, gx, gy, h, biome);
                    // Neighbour heights from the ghost ring (padded index = local+1).
                    let nb = [
                        (cell_height(ch.padded(lx + 2, ly + 1)), Face::Px),
                        (cell_height(ch.padded(lx, ly + 1)), Face::Nx),
                        (cell_height(ch.padded(lx + 1, ly + 2)), Face::Pz),
                        (cell_height(ch.padded(lx + 1, ly)), Face::Nz),
                    ];
                    for (nh, face) in nb {
                        if nh < h {
                            push_side(&mut verts, &mut idx, (gx, gy), h, nh, face, biome);
                        }
                    }

                    if cell_flags(cell) & FLAG_WATER != 0 {
                        // Translucent surface only where the basin actually has depth;
                        // coplanar with the floor (h == SEA_ABS) would z-fight.
                        if h < SEA_ABS {
                            if wi.len() + 6 > MAX_MESH_INDICES {
                                flush_mesh(&mut wv, &mut wi, &mut water);
                            }
                            push_water_top(&mut wv, &mut wi, gx, gy);
                        }
                    } else if tree_density(biome) > 0.0
                        && feature_unit(t.seed, gx, gy, 101) < tree_density(biome)
                    {
                        push_tree(&mut verts, &mut idx, gx, gy, h, t.seed);
                    }
                }
            }
            flush_mesh(&mut verts, &mut idx, &mut opaque);
        }
    }
    flush_mesh(&mut wv, &mut wi, &mut water);
    WorldMeshes { opaque, water }
}

/// Fraction of columns of a biome that grow a tree (0 = none).
fn tree_density(biome: BiomeKind) -> f32 {
    match biome {
        BiomeKind::Forest => 0.30,
        BiomeKind::Plains => 0.04,
        _ => 0.0,
    }
}

/// A voxel tree on column `(gx, gy)` standing on surface height `h`: a brown trunk
/// (2–3 cubes) topped by a 3×3 leaf canopy and a single cap cube. Per-column hashes
/// keep it deterministic. Leaves may overhang into neighbour columns (skipped if they
/// fall outside the world).
fn push_tree(verts: &mut Vec<Vertex>, idx: &mut Vec<u16>, gx: usize, gy: usize, h: u8, seed: u64) {
    let trunk = (0.36, 0.26, 0.16);
    let leaf = (0.16, 0.42, 0.20);
    let th = 2 + (feature_unit(seed, gx, gy, 202) * 2.0) as u8; // 2 or 3
    for gz in h..h + th {
        push_block(verts, idx, gx as i32, gy as i32, gz, trunk);
    }
    let top = h + th;
    for dy in -1i32..=1 {
        for dx in -1i32..=1 {
            let (lx, ly) = (gx as i32 + dx, gy as i32 + dy);
            if (0..COLS as i32).contains(&lx) && (0..ROWS as i32).contains(&ly) {
                push_block(verts, idx, lx, ly, top, leaf);
            }
        }
    }
    push_block(verts, idx, gx as i32, gy as i32, top + 1, leaf);
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
fn push_water_top(verts: &mut Vec<Vertex>, idx: &mut Vec<u16>, gx: usize, gy: usize) {
    let (x0, x1) = (gx as f32 * VOX, (gx + 1) as f32 * VOX);
    let (z0, z1) = (gy as f32 * VOX, (gy + 1) as f32 * VOX);
    let y = SEA_ABS as f32 * VOX;
    let col = Color::new(0.16, 0.40, 0.62, 0.55);
    push_quad(verts, idx, [vec3(x0, y, z0), vec3(x1, y, z0), vec3(x1, y, z1), vec3(x0, y, z1)], col);
}

fn push_top(verts: &mut Vec<Vertex>, idx: &mut Vec<u16>, gx: usize, gy: usize, h: u8, biome: BiomeKind) {
    let (x0, x1) = (gx as f32 * VOX, (gx + 1) as f32 * VOX);
    let (z0, z1) = (gy as f32 * VOX, (gy + 1) as f32 * VOX);
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

fn push_side(
    verts: &mut Vec<Vertex>,
    idx: &mut Vec<u16>,
    (gx, gy): (usize, usize),
    h: u8,
    nh: u8,
    face: Face,
    biome: BiomeKind,
) {
    let (x0, x1) = (gx as f32 * VOX, (gx + 1) as f32 * VOX);
    let (z0, z1) = (gy as f32 * VOX, (gy + 1) as f32 * VOX);
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

    /// Every built mesh must stay strictly under macroquad's per-`draw_mesh` batch
    /// limits (`>= 10000` verts / `>= 5000` indices ⇒ silent clamping). Builds plain
    /// `Mesh` structs with no GL context, so this runs headless. Guards the splitter.
    #[test]
    fn meshes_stay_under_macroquad_drawcall_limits() {
        for seed in 1..3 {
            let t = VoxelTerrain::new(seed);
            let m = build_world_meshes(&t);
            assert!(!m.opaque.is_empty(), "no opaque geometry for seed {seed}");
            for b in m.opaque.iter().chain(m.water.iter()) {
                assert!(b.mesh.vertices.len() < 10_000, "verts {} (seed {seed})", b.mesh.vertices.len());
                assert!(b.mesh.indices.len() < 5_000, "indices {} (seed {seed})", b.mesh.indices.len());
            }
        }
    }
}
