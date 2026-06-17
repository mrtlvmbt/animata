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
use terrain::{cell_biome, cell_height, BiomeKind, VoxelTerrain};

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

#[macroquad::main(window_conf)]
async fn main() {
    let mut cam = IsoCam::new();
    let mut seed: u64 = 1;
    let mut terrain = VoxelTerrain::new(seed);
    let mut meshes = build_chunk_meshes(&terrain);

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
            meshes = build_chunk_meshes(&terrain);
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
                        "map": { "cols": COLS, "rows": ROWS, "vox_m": VOX, "map_scale": MAP_SCALE, "meshes": meshes.len() },
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
                    meshes = build_chunk_meshes(&terrain);
                    let _ = reply.send(serde_json::json!({"seed": seed}));
                }
                dev_bridge::Cmd::Screenshot(path) => {
                    pending_shots.push((path, reply)); // serviced post-draw below
                }
            }
        }

        // ---- Render ----
        clear_background(Color::new(0.53, 0.62, 0.78, 1.0)); // sky
        set_camera(&cam.camera());
        for m in &meshes {
            draw_mesh(m);
        }
        set_default_camera();

        // Minimal debug readout (toggle `I`): fps + frame time. Drawn with a 1px
        // shadow so it stays legible over any terrain colour.
        if show_info {
            let line = format!("{fps:.0} fps   {frame_ms:.2} ms   seed {seed}   {COLS}x{ROWS} m");
            draw_text(&line, 9.0, 23.0, 24.0, Color::new(0.0, 0.0, 0.0, 0.6));
            draw_text(&line, 8.0, 22.0, 24.0, Color::new(0.95, 0.97, 1.0, 1.0));
        }

        // Dev bridge: service deferred screenshots now the frame is fully drawn.
        #[cfg(feature = "dev")]
        for (path, reply) in pending_shots.drain(..) {
            let img = get_screen_data();
            img.export_png(&path);
            let _ = reply.send(serde_json::json!({"saved": path}));
        }

        next_frame().await;
    }
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

/// Build one cached `Mesh` per chunk from exposed faces only. Each column emits its
/// top quad plus, for every horizontal neighbour that is lower, the cliff side faces
/// from the neighbour's height up to its own (one quad per level → strata bands).
/// Neighbour heights come from the chunk's ghost ring, so this is self-contained.
fn build_chunk_meshes(t: &VoxelTerrain) -> Vec<Mesh> {
    let mut out = Vec::new();
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
                    // Keep each mesh under the u16 index/vertex ceiling.
                    if verts.len() + 4 * (h as usize + 1) > 60_000 {
                        flush_mesh(&mut verts, &mut idx, &mut out);
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
                }
            }
            flush_mesh(&mut verts, &mut idx, &mut out);
        }
    }
    out
}

#[derive(Clone, Copy)]
enum Face {
    Px,
    Nx,
    Pz,
    Nz,
}

fn flush_mesh(verts: &mut Vec<Vertex>, idx: &mut Vec<u16>, out: &mut Vec<Mesh>) {
    if verts.is_empty() {
        return;
    }
    out.push(Mesh {
        vertices: std::mem::take(verts),
        indices: std::mem::take(idx),
        texture: None,
    });
}

fn push_quad(verts: &mut Vec<Vertex>, idx: &mut Vec<u16>, q: [Vec3; 4], col: Color) {
    let base = verts.len() as u16;
    for p in q {
        verts.push(Vertex::new(p.x, p.y, p.z, 0.0, 0.0, col));
    }
    idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
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
