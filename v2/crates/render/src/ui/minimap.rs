//! U-5: Minimap raster builder and panel.
//!
//! The minimap is a downscaled top-down view of the world terrain, using the FLAT base colour
//! (no shading). It is palette-matched to the viewport by construction, using the shared
//! `biome_palette::cell_color()` for each pixel.
//!
//! Dimensions: 172×118 (v1 reference), covers the full world via downscaling.
//! Texture is cached in HudCache with key (seed, dim, bare_mode); rebuilds only on key change.
//! Viewport quad is drawn from screen→ground unprojection via camera.
//! Click-to-jump: click UV → world XZ → UiAction::JumpCamera.

use macroquad::prelude::*;
use sim_core::WorldView;
use sim_core::Vec2Fixed;

use crate::biome_palette;
use egui::Color32;

pub const MINIMAP_WIDTH: usize = 172;
pub const MINIMAP_HEIGHT: usize = 118;

// Camera constants (from camera.rs)
const ISO_PITCH: f32 = std::f32::consts::PI * 40.9 / 180.0;

/// View-projection matrix for isometric camera (copied logic from camera.rs for minimap use).
pub fn minimap_view_proj_matrix(focus: glam::Vec3, yaw: f32, ortho_span: f32, aspect: f32) -> glam::Mat4 {
    let distance = ortho_span * 1.4;
    let cos_yaw = yaw.cos();
    let sin_yaw = yaw.sin();
    let cos_pitch = ISO_PITCH.cos();
    let sin_pitch = ISO_PITCH.sin();

    let cam_x = cos_yaw * cos_pitch * distance;
    let cam_y = sin_pitch * distance;
    let cam_z = sin_yaw * cos_pitch * distance;

    let position = focus + glam::Vec3::new(cam_x, cam_y, cam_z);
    let up = glam::Vec3::new(0.0, 1.0, 0.0);

    let top = ortho_span / 2.0;
    let right = top * aspect;
    let z_near = 0.01;
    let z_far = 10000.0;

    glam::Mat4::orthographic_rh_gl(-right, right, -top, top, z_near, z_far)
        * glam::Mat4::look_at_rh(position, focus, up)
}

/// Unproject screen point through ortho VP matrix and intersect with y=0 ground plane.
pub fn minimap_ground_under_cursor(vp: glam::Mat4, screen_pos: (f32, f32), screen_dims: (f32, f32)) -> glam::Vec2 {
    let (mx, my) = screen_pos;
    let (sw, sh) = screen_dims;
    let sw = sw.max(1.0);
    let sh = sh.max(1.0);

    let nx = mx / sw * 2.0 - 1.0;
    let ny = 1.0 - my / sh * 2.0;

    let inv = vp.inverse();
    let near = inv.project_point3(glam::vec3(nx, ny, -1.0));
    let far = inv.project_point3(glam::vec3(nx, ny, 1.0));

    let d = far - near;
    let t = if d.y.abs() > 1e-6 { -near.y / d.y } else { 0.0 };
    let hit = near + d * t;

    glam::vec2(hit.x, hit.z)
}

/// Compute the per-map relief band [p2, p98] (same logic as terrain.rs::hypsometric_range).
/// This ensures minimap uses the exact same colour ramps as the viewport (D6 contract).
fn compute_relief_band(world: &dyn WorldView, dim: i64) -> (i64, i64) {
    let mut heights: Vec<i64> = Vec::with_capacity((dim * dim) as usize);
    for row in 0..dim {
        for col in 0..dim {
            heights.push(world.height(col, row));
        }
    }
    if heights.is_empty() {
        return (0, 1);
    }
    heights.sort_unstable();
    let n = heights.len();
    let percentile = |p: f64| heights[(((p * (n as f64 - 1.0)).round()) as usize).min(n - 1)];
    let lo = percentile(0.02);  // p2
    let hi = percentile(0.98);  // p98
    if hi > lo {
        (lo, hi)
    } else {
        (lo, lo + 1)
    }
}

/// Raster the minimap as an egui ColorImage from a WorldView.
/// Each pixel samples one world cell (downscaled grid).
/// Uses the FLAT base colour (no directional shading).
pub fn build_minimap_image(
    world: &dyn WorldView,
    dim: i64,
    seed: u64,
    bare_mode: bool,
) -> egui::ColorImage {
    let mut pixels = vec![Color32::BLACK; MINIMAP_WIDTH * MINIMAP_HEIGHT];

    // Compute the per-map relief band (same as terrain.rs for D6 palette-match)
    let (h_lo, h_hi) = compute_relief_band(world, dim);

    // Sample the world at downscaled coordinates.
    // For each minimap pixel, compute the corresponding world cell and sample its colour.
    for py in 0..MINIMAP_HEIGHT {
        let world_z = ((py as i64) * dim / (MINIMAP_HEIGHT as i64)).min(dim - 1);
        for px in 0..MINIMAP_WIDTH {
            let world_x = ((px as i64) * dim / (MINIMAP_WIDTH as i64)).min(dim - 1);

            // Query world height and material at this cell
            let height = world.height(world_x, world_z);
            // surface_material takes Vec2Fixed (tuple syntax); use direct world coordinates
            let material = world.surface_material(Vec2Fixed(world_x, world_z));

            // Use the shared cell_color helper to get the flat base colour (no shading).
            // This uses the exact same h_lo/h_hi as terrain.rs for D6 palette-match.
            let color = biome_palette::cell_color(material, height, h_lo, h_hi, world_x, world_z, seed, bare_mode);

            // Convert macroquad Color to egui Color32
            pixels[py * MINIMAP_WIDTH + px] = Color32::from_rgb(
                (color.r * 255.0) as u8,
                (color.g * 255.0) as u8,
                (color.b * 255.0) as u8,
            );
        }
    }

    egui::ColorImage {
        size: [MINIMAP_WIDTH, MINIMAP_HEIGHT],
        pixels,
    }
}

/// Project a world point to minimap UV coordinates [0, 1].
/// world_pos is (x, z) in world space; returns (u, v) in [0, 1].
pub fn world_to_minimap_uv(world_pos: glam::Vec2, dim: i64) -> glam::Vec2 {
    glam::vec2(
        (world_pos.x / dim as f32).clamp(0.0, 1.0),
        (world_pos.y / dim as f32).clamp(0.0, 1.0),
    )
}

/// Compute screen-space quad corners that map to world coordinates (for camera frustum).
/// Returns 4 (screen_x, screen_y) positions for corners at near plane.
pub fn screen_quad_corners(screen_dims: (f32, f32)) -> [(f32, f32); 4] {
    let (w, h) = screen_dims;
    [
        (0.0, 0.0),      // top-left
        (w, 0.0),        // top-right
        (w, h),          // bottom-right
        (0.0, h),        // bottom-left
    ]
}

/// Project minimap UV coordinates back to world position.
/// uv is in [0, 1]; returns (x, z) in world space.
pub fn minimap_uv_to_world(uv: glam::Vec2, dim: i64) -> glam::Vec2 {
    glam::vec2(
        (uv.x * dim as f32).clamp(0.0, (dim - 1) as f32),
        (uv.y * dim as f32).clamp(0.0, (dim - 1) as f32),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimap_uv_world_mapping_is_invertible() {
        let dim = 256i64;

        // Test several world points
        for wx in [0.0, 64.0, 128.0, 255.0] {
            for wz in [0.0, 64.0, 128.0, 255.0] {
                let world_pos = glam::vec2(wx, wz);

                // Forward: world → UV
                let uv = world_to_minimap_uv(world_pos, dim);

                // Backward: UV → world
                let recovered = minimap_uv_to_world(uv, dim);

                // Should be close (within 1 cell due to clamping)
                assert!((recovered.x - world_pos.x).abs() <= 1.0, "x mismatch at ({}, {})", wx, wz);
                assert!((recovered.y - world_pos.y).abs() <= 1.0, "z mismatch at ({}, {})", wx, wz);
            }
        }
    }
}
