//! animata — voxel isometric world (environment viewer).
//!
//! Reset from the former a-life simulation (archived at git tag `sim-v1` / branch
//! `archive/sim-v1`). The simulation and all GUI are intentionally OFF: this is a
//! bare environment viewer that will grow a Minecraft-like voxel world on
//! macroquad's 3D pipeline (real cubes + GPU depth buffer).
//!
//! Phase 1: noise worldgen into a chunked `VoxelTerrain` (see `terrain.rs`), drawn
//! with a simple per-column pillar preview (immediate-mode `draw_cube`, culled to a
//! window around the camera). The proper batched, exposed-face chunk mesh comes in
//! phase 2 — this preview only exists to eyeball the generated heights/biomes.

mod config;
mod terrain;

use config::*;
use macroquad::prelude::*;
use terrain::{BiomeKind, VoxelTerrain};

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
            // Centre on the world grid.
            target: vec3(COLS as f32 * VOX * 0.5, 0.0, ROWS as f32 * VOX * 0.5),
            zoom: 48.0,
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
            position: self.target + dir * 200.0,
            target: self.target,
            up: vec3(0.0, 1.0, 0.0),
            fovy: self.zoom,
            aspect: Some(screen_width() / screen_height()),
            projection: Projection::Orthographics,
            render_target: None,
            viewport: None,
            z_near: 0.1,
            z_far: 2000.0,
        }
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    let mut cam = IsoCam::new();
    let mut seed: u64 = 1;
    let mut terrain = VoxelTerrain::new(seed);

    loop {
        let dt = get_frame_time();

        // ---- Input (no GUI) ----
        let wheel = mouse_wheel().1;
        if wheel != 0.0 {
            cam.zoom = (cam.zoom * (1.0 - wheel.signum() * 0.1)).clamp(6.0, 200.0);
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
            let speed = cam.zoom * dt; // pan faster when zoomed out
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
        }

        // ---- Render ----
        clear_background(Color::new(0.53, 0.62, 0.78, 1.0)); // sky
        set_camera(&cam.camera());
        draw_terrain_preview(&terrain, &cam);
        set_default_camera();

        next_frame().await;
    }
}

/// Phase-1 preview: draw each column in a window around the camera as a solid
/// pillar (one immediate-mode cube from ground to its surface height), coloured by
/// biome. Bounded by a column radius so immediate mode stays cheap; the real
/// batched exposed-face chunk mesh replaces this in phase 2.
fn draw_terrain_preview(t: &VoxelTerrain, cam: &IsoCam) {
    const R: i32 = 44; // preview window radius in columns
    let cx = (cam.target.x / VOX).round() as i32;
    let cy = (cam.target.z / VOX).round() as i32;
    let x0 = (cx - R).max(0);
    let x1 = (cx + R).min(COLS as i32);
    let y0 = (cy - R).max(0);
    let y1 = (cy + R).min(ROWS as i32);
    for gy in y0..y1 {
        for gx in x0..x1 {
            let (x, y) = (gx as usize, gy as usize);
            let h = t.height_at(x, y) as f32;
            let col = if t.is_water(x, y) {
                Color::new(0.11, 0.30, 0.57, 1.0) // flat sea (real water pass: phase 3)
            } else {
                block_color(t.biome_at(x, y), h)
            };
            let centre = vec3(gx as f32 * VOX, h * VOX * 0.5, gy as f32 * VOX);
            let size = vec3(VOX, h * VOX, VOX);
            draw_cube(centre, size, None, col);
            draw_cube_wires(centre, size, Color::new(0.0, 0.0, 0.0, 0.12));
        }
    }
}

/// Render-side palette: map an abstract `BiomeKind` to a colour, with a slight
/// brighten-by-height so taller terrain reads. (Representation lives here, not in
/// the generator.)
fn block_color(biome: BiomeKind, h: f32) -> Color {
    let (r, g, b) = match biome {
        BiomeKind::Ocean => (0.13, 0.32, 0.55),
        BiomeKind::Beach => (0.80, 0.74, 0.50),
        BiomeKind::Plains => (0.42, 0.62, 0.30),
        BiomeKind::Forest => (0.20, 0.46, 0.24),
        BiomeKind::Desert => (0.78, 0.68, 0.42),
        BiomeKind::Mountain => (0.45, 0.43, 0.42),
        BiomeKind::Snow => (0.92, 0.94, 0.97),
    };
    let s = (0.78 + 0.03 * h).min(1.0);
    Color::new((r * s).min(1.0), (g * s).min(1.0), (b * s).min(1.0), 1.0)
}
