//! animata — voxel isometric world (environment viewer).
//!
//! Reset from the former a-life simulation (archived at git tag `sim-v1` / branch
//! `archive/sim-v1`). The simulation and all GUI are intentionally OFF: this is a
//! bare environment viewer that will grow a Minecraft-like voxel world on
//! macroquad's 3D pipeline (real cubes + GPU depth buffer).
//!
//! Phase 0: stand up an orthographic isometric `Camera3D` with pan/zoom/rotate and
//! a test height-field of cubes — a spike to confirm the depth buffer and the
//! orthographic camera behave in macroquad 0.4 before the real chunked terrain
//! (phase 1+) is built.

mod config;

use config::*;
use macroquad::prelude::*;

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
            // Centre on the (eventual) world grid.
            target: vec3(COLS as f32 * VOX * 0.5, 0.0, ROWS as f32 * VOX * 0.5),
            zoom: 24.0,
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

    loop {
        let dt = get_frame_time();

        // ---- Camera input (no GUI) ----
        let wheel = mouse_wheel().1;
        if wheel != 0.0 {
            cam.zoom = (cam.zoom * (1.0 - wheel.signum() * 0.1)).clamp(4.0, 160.0);
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

        // ---- Render ----
        clear_background(Color::new(0.53, 0.62, 0.78, 1.0)); // sky
        set_camera(&cam.camera());
        draw_spike_scene(cam.target);
        set_default_camera();

        next_frame().await;
    }
}

/// Test scene: an 8×8 height-field of unit cubes around `centre`, coloured by
/// height with wire edges — enough to confirm depth ordering and the iso look.
fn draw_spike_scene(centre: Vec3) {
    let n: i32 = 8;
    let ox = centre.x - n as f32 * VOX * 0.5;
    let oz = centre.z - n as f32 * VOX * 0.5;
    for gx in 0..n {
        for gz in 0..n {
            let h = (((gx as f32 * 0.8).sin() + (gz as f32 * 0.6).cos()) * 1.5 + 3.0) as i32;
            for gy in 0..h.max(1) {
                let world = vec3(
                    ox + gx as f32 * VOX,
                    gy as f32 * VOX + VOX * 0.5,
                    oz + gz as f32 * VOX,
                );
                let t = gy as f32 / 5.0;
                let col = Color::new(0.25 + 0.5 * t, 0.55 - 0.2 * t, 0.30, 1.0);
                draw_cube(world, vec3(VOX, VOX, VOX), None, col);
                draw_cube_wires(world, vec3(VOX, VOX, VOX), Color::new(0.0, 0.0, 0.0, 0.25));
            }
        }
    }
}
