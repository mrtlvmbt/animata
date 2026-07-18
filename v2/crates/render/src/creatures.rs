//! R-4/R-7: Creature rendering by px_per_m LOD tier (point → sphere → morphology).
//! Extracted from main.rs to eliminate duplication across screenshot/bench/main-loop paths.

use crate::hex;
use macroquad::prelude::*;
use sim_core::{RenderSnapshot, WorldView};
use std::sync::Arc;

/// FAR tier: creatures are sub-pixel or nearly invisible (point/billboard). Triggers when px_per_m < 5.
const PX_PER_M_FAR_THRESHOLD: f32 = 5.0;

/// MID tier: creatures are cell-type-colored spheres (R-3 behavior). Active when 5 <= px_per_m < 20.
const PX_PER_M_MID_THRESHOLD: f32 = 20.0;

/// Render creatures from a snapshot by px_per_m LOD tier.
///
/// Takes a snapshot (may be None for standalone mode), camera, world, and terrain layout.
/// Renders each creature according to its distance from the camera:
/// - FAR tier (px_per_m < 5): minimal point/billboard
/// - MID tier (5 <= px_per_m < 20): multicell sphere cluster
/// - NEAR tier (px_per_m >= 20): cell-type morphology + multicell body
///
/// This function captures the exact rendering logic from the three duplication sites
/// in main.rs (screenshot mode, benchmark mode, and main loop).
pub fn render_creatures_lod(
    snap: &Option<Arc<RenderSnapshot>>,
    camera: &crate::camera::IsoCam,
    world: &dyn WorldView,
    use_cube_terrain: bool,
    height_scale: f32,
) {
    if let Some(s) = snap.as_ref() {
        let px_per_m = camera.px_per_m();

        for c in &s.creatures {
            // R-5: Creature projection follows active terrain layout.
            let (cx, cz) = if use_cube_terrain {
                // Cube mode: square cell center at (col + 0.5, row + 0.5)
                (c.pos.0 as f32 + 0.5, c.pos.1 as f32 + 0.5)
            } else {
                // Hex mode: hex center (R-2/R-4 behavior)
                hex::hex_center(c.pos.0, c.pos.1)
            };
            let h = world.height(c.pos.0, c.pos.1) as f32 * height_scale;
            let creature_pos = vec3(cx, h + 0.15, cz);

            // R-3 frustum cull: skip creatures outside the view frustum.
            if !camera.point_in_frustum(creature_pos) {
                continue;
            }

            // R-7 (biology coloring): Base color by uptake_layer (feeding guild).
            // Layer 0 (A-guild) = orange/red; layer 1 (B-guild) = cyan/blue; higher layers distinct.
            // This makes emergence visible: A/B differentiation is the primary visual signal.
            let color = match c.uptake_layer {
                0 => Color::new(1.0, 0.6, 0.2, 1.0), // Orange (A-guild)
                1 => Color::new(0.2, 0.8, 1.0, 1.0), // Cyan (B-guild)
                2 => Color::new(0.8, 0.2, 1.0, 1.0), // Magenta (layer 2+)
                _ => Color::new(0.5, 0.5, 0.5, 1.0), // Gray (undefined layers)
            };

            // R-4 LOD tier by px_per_m: FAR (point) < MID (sphere) < NEAR (morphology).
            if px_per_m < PX_PER_M_FAR_THRESHOLD {
                // ─── FAR tier: sub-pixel point/billboard (cheapest) ───────────────────────────────
                // Creatures so tiny they're unresolvable. Draw a minimal dot.
                draw_sphere(creature_pos, 0.04, None, color);
            } else if px_per_m < PX_PER_M_MID_THRESHOLD {
                // ─── MID tier: multicell cluster sphere (R-11 body_size rendering) ──────────────────
                // R-11: Draw body as `body_size` cells in a packed cluster arrangement.
                // Each cell is a small sphere; cluster is arranged in a square grid.
                let body_count = c.body_size.max(1) as usize;
                let grid_side = (body_count as f32).sqrt().ceil() as i32;
                let cell_radius = 0.03; // Small sub-cell radius
                let spacing = cell_radius * 2.2; // Slight spacing between cells

                // Compute grid offset to center the cluster
                let grid_size = (grid_side - 1) as f32 * spacing;
                let offset_x = -grid_size / 2.0;
                let offset_z = -grid_size / 2.0;

                // Draw cells in a square grid pattern
                let mut drawn = 0;
                for row in 0..grid_side {
                    for col in 0..grid_side {
                        if drawn >= body_count {
                            break;
                        }
                        let cell_x = offset_x + col as f32 * spacing;
                        let cell_z = offset_z + row as f32 * spacing;
                        let cell_pos = creature_pos + vec3(cell_x, 0.02, cell_z);
                        draw_sphere(cell_pos, cell_radius, None, color);
                        drawn += 1;
                    }
                    if drawn >= body_count {
                        break;
                    }
                }
            } else {
                // ─── NEAR tier: cell-type morphology + multicell body representation ──────────────────
                // R-4/R-11: Scale morphology by `size` (Kleiber); draw body as `body_size` cells.
                // Each cell_type has a distinctive form; base color is uptake_layer (feeding guild).
                let size_scale = c.size as f32 / 16.0;
                let base_size = 0.15 * size_scale;
                let body_count = c.body_size.max(1) as usize;

                // For multicellular bodies (body_count > 1), render a small cluster around the main form.
                // This makes multicellularity visible while preserving the cell_type morphology signal.
                if body_count > 1 {
                    let grid_side = (body_count as f32).sqrt().ceil() as i32;
                    let cell_radius = 0.04;
                    let spacing = cell_radius * 2.0;
                    let grid_size = (grid_side - 1) as f32 * spacing;
                    let offset_x = -grid_size / 2.0;
                    let offset_z = -grid_size / 2.0;

                    // Draw cells in a compact grid, with cell_type morphology only on the main cell.
                    let mut drawn = 0;
                    for row in 0..grid_side {
                        for col in 0..grid_side {
                            if drawn >= body_count {
                                break;
                            }
                            let cell_x = offset_x + col as f32 * spacing;
                            let cell_z = offset_z + row as f32 * spacing;
                            let cell_pos = creature_pos + vec3(cell_x, 0.01, cell_z);
                            // Render each cell as a small sphere in the base color
                            draw_sphere(cell_pos, cell_radius, None, color);
                            drawn += 1;
                        }
                        if drawn >= body_count {
                            break;
                        }
                    }
                } else {
                    // Unicellular: draw the full cell_type morphology
                    match c.cell_type {
                        Some(sim_core::CellType::A) => {
                            // Type A: main body + upper accent sphere (a small top ball).
                            draw_sphere(creature_pos, base_size, None, color);
                            let accent = Color::new(color.r.min(1.0), (color.g * 1.3).min(1.0), color.b, 1.0);
                            draw_sphere(creature_pos + vec3(0.0, base_size * 1.2, 0.0), base_size * 0.5, None, accent);
                        }
                        Some(sim_core::CellType::B) => {
                            // Type B: main body + side accent sphere (a small offset ball).
                            draw_sphere(creature_pos, base_size, None, color);
                            let accent = Color::new(color.r, (color.g * 1.3).min(1.0), color.b.min(1.0), 1.0);
                            draw_sphere(creature_pos + vec3(base_size * 1.2, 0.0, 0.0), base_size * 0.5, None, accent);
                        }
                        Some(sim_core::CellType::Mixed) => {
                            // Type Mixed: main body + front accent sphere (a small forward ball).
                            draw_sphere(creature_pos, base_size, None, color);
                            let accent = Color::new((color.r * 1.3).min(1.0), color.g, color.b.min(1.0), 1.0);
                            draw_sphere(creature_pos + vec3(0.0, 0.0, base_size * 1.2), base_size * 0.5, None, accent);
                        }
                        Some(sim_core::CellType::Diff(_)) => {
                            // Diff: differentiated cell, render as neutral sphere (same as None for now)
                            draw_sphere(creature_pos, base_size, None, color);
                        }
                        None => {
                            // Neutral: single sphere (for non-morphogen configs).
                            draw_sphere(creature_pos, base_size, None, color);
                        }
                    }
                }
            }
        }
    }
}
