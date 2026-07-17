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

    // Sample the world at downscaled coordinates.
    // For each minimap pixel, compute the corresponding world cell and sample its colour.
    for py in 0..MINIMAP_HEIGHT {
        let world_z = ((py as i64) * dim / (MINIMAP_HEIGHT as i64)).min(dim - 1);
        for px in 0..MINIMAP_WIDTH {
            let world_x = ((px as i64) * dim / (MINIMAP_WIDTH as i64)).min(dim - 1);

            // Query world height and material at this cell
            let height = world.height(world_x, world_z);
            // surface_material takes Vec2Fixed (tuple syntax); multiply by 2 since Vec2Fixed is in "fixed" units
            let material = world.surface_material(Vec2Fixed(world_x * 2, world_z * 2));

            // For now, use a placeholder relief band. In a real scenario,
            // the world would store this, but we'll derive it from typical terrain bounds.
            // A reasonable default for most terrains is h_lo=40, h_hi=180 (can be tuned).
            const H_LO: i64 = 40;
            const H_HI: i64 = 180;

            // Use the shared cell_color helper to get the flat base colour (no shading).
            let color = biome_palette::cell_color(material, height, H_LO, H_HI, world_x, world_z, seed, bare_mode);

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

/// Project a world point to minimap UV coordinates.
/// world_pos is (x, z) in world space; returns (u, v) in [0, 1].
pub fn world_to_minimap_uv(world_pos: glam::Vec2, dim: i64) -> glam::Vec2 {
    glam::vec2(
        (world_pos.x / dim as f32).clamp(0.0, 1.0),
        (world_pos.y / dim as f32).clamp(0.0, 1.0),
    )
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
