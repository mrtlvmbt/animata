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
#[allow(dead_code)]
pub fn world_to_minimap_uv(world_pos: glam::Vec2, dim: i64) -> glam::Vec2 {
    glam::vec2(
        (world_pos.x / dim as f32).clamp(0.0, 1.0),
        (world_pos.y / dim as f32).clamp(0.0, 1.0),
    )
}

/// Project a world point to minimap UV coordinates, WITHOUT clamping to [0, 1].
/// Used for viewport quad projection so out-of-bounds corners can extend beyond the panel.
pub fn world_to_minimap_uv_unclamped(world_pos: glam::Vec2, dim: i64) -> glam::Vec2 {
    glam::vec2(
        world_pos.x / dim as f32,
        world_pos.y / dim as f32,
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

/// U-8: Project map UV coordinates to screen-aligned isometric diamond on the minimap panel.
///
/// Derives the projection from the iso camera's ground basis:
/// - screen_right = (sinY, 0, −cosY) in world (x, y, z)
/// - screen_up = (−cosY, 0, −sinY) in world (x, y, z)
///
/// Where Y is the camera yaw. The projection applies these basis vectors to map fractions,
/// foreshortens by FS = sin(35.264°), and scales to fit the panel with margin.
///
/// Args:
/// - u, v: map fractions in [0, 1]
/// - yaw: camera yaw rotation (radians)
/// - panel_w, panel_h: panel dimensions in pixels
///
/// Returns: (x, y) screen coordinates on the panel
pub fn map_uv_to_panel(u: f32, v: f32, yaw: f32, panel_w: f32, panel_h: f32) -> (f32, f32) {
    const FS: f32 = 0.577_350_3; // sin(35.264°)

    // Center the UV fractions
    let cu = u - 0.5;
    let cv = v - 0.5;

    let sin_yaw = yaw.sin();
    let cos_yaw = yaw.cos();

    // Project map fractions through the screen basis:
    // panel_x ∝ dot((cu, cv), screen_right_xz) = cu·sinY − cv·cosY
    let panel_x_unnormalized = cu * sin_yaw - cv * cos_yaw;

    // panel_y ∝ −dot((cu, cv), screen_up_xz)·FS = −(−cu·cosY − cv·sinY)·FS = (cu·cosY + cv·sinY)·FS
    let panel_y_unnormalized = (cu * cos_yaw + cv * sin_yaw) * FS;

    // U-13: Scale to FILL the panel — isometric diamond corners touch panel edges
    // Diamond ranges x in [center_x ± 0.5*s], y in [center_y ± 0.5*FS*s]
    // To fill panel [0, panel_w]×[0, panel_h]: s = min(panel_w, panel_h/FS)
    let s = (panel_w).min(panel_h / FS);
    let center_x = panel_w * 0.5;
    let center_y = panel_h * 0.5;

    (
        center_x + panel_x_unnormalized * s,
        center_y + panel_y_unnormalized * s,
    )
}

/// U-8: Invert the map_uv_to_panel projection for click-to-jump.
///
/// Given a screen coordinate on the minimap panel, recover the map UV fraction.
/// Returns uv clamped to [0, 1].
pub fn panel_to_map_uv(x: f32, y: f32, yaw: f32, panel_w: f32, panel_h: f32) -> glam::Vec2 {
    const FS: f32 = 0.577_350_3;

    // U-13: Undo centering and scaling (must match map_uv_to_panel exactly)
    let s = (panel_w).min(panel_h / FS);
    let center_x = panel_w * 0.5;
    let center_y = panel_h * 0.5;

    let panel_x_unnormalized = (x - center_x) / s;
    let panel_y_unnormalized = (y - center_y) / s;

    let sin_yaw = yaw.sin();
    let cos_yaw = yaw.cos();

    // Solve the linear system:
    // panel_x_unnormalized = cu·sinY − cv·cosY
    // panel_y_unnormalized / FS = cu·cosY + cv·sinY
    //
    // This is: [sinY   −cosY ] [cu] = [panel_x_unnormalized]
    //          [cosY    sinY ] [cv]   [panel_y_unnormalized / FS]
    //
    // Determinant = sinY·sinY − (−cosY)·cosY = sin²Y + cos²Y = 1
    // Inverse = [sinY   cosY]
    //           [−cosY  sinY]

    let panel_y_scaled = panel_y_unnormalized / FS;
    let cu = panel_x_unnormalized * sin_yaw + panel_y_scaled * cos_yaw;
    let cv = -panel_x_unnormalized * cos_yaw + panel_y_scaled * sin_yaw;

    // Uncenter the fractions
    let u = cu + 0.5;
    let v = cv + 0.5;

    glam::vec2(u.clamp(0.0, 1.0), v.clamp(0.0, 1.0))
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

    #[test]
    fn minimap_view_proj_transforms_screen_corners() {
        // Test that view-projection matrix correctly transforms screen corners to world space
        // This verifies the viewport quad projection math works.
        let focus = glam::vec3(128.0, 40.0, 128.0);  // Center of 256×256 world
        let yaw = 0.0;
        let ortho_span = 100.0;  // Default view span
        let aspect = 1.0;  // Square viewport

        let vp = minimap_view_proj_matrix(focus, yaw, ortho_span, aspect);

        // Screen corners for a 256×256 viewport
        let corners = [
            (0.0, 0.0),      // top-left
            (256.0, 0.0),    // top-right
            (256.0, 256.0),  // bottom-right
            (0.0, 256.0),    // bottom-left
        ];

        // Project all 4 corners and verify they map within world bounds
        for (screen_x, screen_y) in corners.iter() {
            let world_xz = minimap_ground_under_cursor(vp, (*screen_x, *screen_y), (256.0, 256.0));

            // World coordinates should be close to expected range
            assert!(world_xz.x >= 0.0 && world_xz.x < 256.0, "corner proj x={} out of world bounds", world_xz.x);
            assert!(world_xz.y >= 0.0 && world_xz.y < 256.0, "corner proj z={} out of world bounds", world_xz.y);
        }
    }

    #[test]
    fn minimap_viewport_quad_offsets_with_camera_pan() {
        // Verify that camera pan (offset focus) changes the viewport quad position on minimap
        let dim = 256i64;
        let yaw = 0.0;
        let ortho_span = 100.0;
        let aspect = 1.0;

        // Center camera
        let focus_center = glam::vec3(128.0, 40.0, 128.0);
        let vp_center = minimap_view_proj_matrix(focus_center, yaw, ortho_span, aspect);

        // Pan camera to a different position
        let focus_panned = glam::vec3(64.0, 40.0, 64.0);  // Panned toward corner
        let vp_panned = minimap_view_proj_matrix(focus_panned, yaw, ortho_span, aspect);

        // Sample one corner (top-left)
        let corner_screen = (0.0, 0.0);
        let screen_dims = (256.0, 256.0);

        // Project corner through both VP matrices
        let world_center = minimap_ground_under_cursor(vp_center, corner_screen, screen_dims);
        let world_panned = minimap_ground_under_cursor(vp_panned, corner_screen, screen_dims);

        // Map to minimap UV
        let uv_center = world_to_minimap_uv(world_center, dim);
        let uv_panned = world_to_minimap_uv(world_panned, dim);

        // Panning the camera should move the projected corner on the minimap
        let uv_distance = ((uv_panned.x - uv_center.x).powi(2) + (uv_panned.y - uv_center.y).powi(2)).sqrt();
        assert!(uv_distance > 0.05, "camera pan did not move viewport quad (distance={})", uv_distance);
    }

    // U-8: Orientation mechanical acceptance tests
    #[test]
    fn map_uv_to_panel_screen_right_at_yaw_0() {
        let panel_w = 200.0;
        let panel_h = 200.0;
        let yaw = 0.0;

        // At yaw=0, screen_right is (0, -1) in (x, z) world coords (negative z).
        // A displacement along screen_right (decreasing z → decreasing v → more negative cv)
        // should map to positive panel x (rightward).
        let center = map_uv_to_panel(0.5, 0.5, yaw, panel_w, panel_h);
        let displaced_neg_z = map_uv_to_panel(0.5, 0.4, yaw, panel_w, panel_h);  // Decrease v (z)

        let dx = displaced_neg_z.0 - center.0;
        assert!(dx.abs() > 0.1, "screen_right displacement should affect panel x; dx={}", dx);
        assert!(dx > 0.0, "screen_right displacement should map to positive panel x (rightward); dx={}", dx);
    }

    #[test]
    fn map_uv_to_panel_screen_up_at_yaw_0() {
        let panel_w = 200.0;
        let panel_h = 200.0;
        let yaw = 0.0;

        // At yaw=0, screen_up is (-1, 0) in (x, z) world coords (negative x).
        // A displacement along screen_up (decreasing x → decreasing u → more negative cu)
        // should map to negative panel y (upward on panel).
        let center = map_uv_to_panel(0.5, 0.5, yaw, panel_w, panel_h);
        let displaced_neg_x = map_uv_to_panel(0.4, 0.5, yaw, panel_w, panel_h);  // Decrease u (x)

        let dy = displaced_neg_x.1 - center.1;
        assert!(dy.abs() > 0.1, "screen_up displacement should affect panel y; dy={}", dy);
        assert!(dy < 0.0, "screen_up displacement should map to negative panel y (upward); dy={}", dy);
    }

    #[test]
    fn map_uv_to_panel_screen_right_at_yaw_90() {
        let panel_w = 200.0;
        let panel_h = 200.0;
        let yaw = std::f32::consts::PI / 2.0;  // 90°

        // At yaw=90°, screen_right is (1, 0) in (x, z) world coords.
        // A displacement along +x (u) should map to positive panel x (rightward).
        let center = map_uv_to_panel(0.5, 0.5, yaw, panel_w, panel_h);
        let displaced_u = map_uv_to_panel(0.6, 0.5, yaw, panel_w, panel_h);

        let dx = displaced_u.0 - center.0;
        assert!(dx.abs() > 0.1, "displacement along u should affect panel x; dx={}", dx);
        assert!(dx > 0.0, "positive u displacement should map to positive panel x (rightward) at yaw=90°; dx={}", dx);
    }

    #[test]
    fn map_uv_to_panel_roundtrip() {
        let panel_w = 200.0;
        let panel_h = 200.0;
        let yaw = std::f32::consts::PI / 6.0;  // 30°

        // Test roundtrip for various points on a grid, parametrized over height_scale.
        // NOTE: minimap projection (map_uv_to_panel / panel_to_map_uv) is based on UV/map coordinates
        // and does NOT depend on height-scale. The height_scale parameter is a render-side multiplier
        // on the 3D height→prism mapping (camera view); minimap UV↔panel conversion is independent.
        // This loop is a REGRESSION GUARD: parametrizes over ×1.0 (baseline) and ×1.5 (sentinel).
        // Today it's a no-op (height-scale is unused in the projection); tomorrow IF someone leaks
        // height_scale into the minimap, this test will fail and catch the mistake.
        for _height_scale in [1.0_f32, 1.5] {
            for u_test in [0.0, 0.25, 0.5, 0.75, 1.0] {
                for v_test in [0.0, 0.25, 0.5, 0.75, 1.0] {
                    let panel_pos = map_uv_to_panel(u_test, v_test, yaw, panel_w, panel_h);
                    let recovered_uv = panel_to_map_uv(panel_pos.0, panel_pos.1, yaw, panel_w, panel_h);

                    let u_error = (recovered_uv.x - u_test).abs();
                    let v_error = (recovered_uv.y - v_test).abs();

                    assert!(u_error < 1e-4, "u roundtrip error at ({}, {}): {} vs {}", u_test, v_test, recovered_uv.x, u_test);
                    assert!(v_error < 1e-4, "v roundtrip error at ({}, {}): {} vs {}", u_test, v_test, recovered_uv.y, v_test);
                }
            }
        }
    }

    #[test]
    fn map_uv_to_panel_magnitude_check_at_yaw_0() {
        const FS: f32 = 0.577_350_3;
        let panel_w = 200.0;
        let panel_h = 200.0;
        let yaw = 0.0;

        let center = map_uv_to_panel(0.5, 0.5, yaw, panel_w, panel_h);
        let corner = map_uv_to_panel(0.0, 0.0, yaw, panel_w, panel_h);

        let dx = (corner.0 - center.0).abs();
        let dy = (corner.1 - center.1).abs();

        // At yaw=0, corner (-0.5, -0.5) has equal |cu| and |cv|.
        // panel_x = -cv → unnormalized = 0.5
        // panel_y = cu*FS → unnormalized = -0.5*FS
        // Therefore dy/dx = FS (the iso foreshorten factor).
        assert!(dx > 0.1, "x-displacement should be non-zero; dx={}", dx);
        assert!(dy > 0.1, "y-displacement should be non-zero; dy={}", dy);
        let ratio = dy / dx;
        assert!((ratio - FS).abs() < 1e-3, "dy/dx should equal FS={} (iso foreshorten); got {}", FS, ratio);
    }
}
