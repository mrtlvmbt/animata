//! UI layer — debug HUD and future Panel abstraction.
//! Extracted from main.rs to establish seams for loading screen, minimap, and other panels.

use macroquad::prelude::*;
use sim_core::RenderSnapshot;

/// Draw the debug HUD window — stats display and control legend.
/// Shows snapshot stats (tick, population, species, creature count) if available,
/// creature coloring legend, terrain info, and input controls.
pub fn draw_debug_hud(
    snap: &Option<RenderSnapshot>,
    standalone_mode: bool,
    world_dim: i64,
    chunks_drawn: usize,
    terrain_chunks_total: usize,
) {
    let title = if standalone_mode {
        "v2 render scaffold — R-8 standalone hex-map viewer"
    } else {
        "v2 render scaffold — R-7 biology coloring"
    };

    egui_macroquad::ui(|ctx| {
        egui::Window::new(title).show(ctx, |ui| {
            match snap.as_ref() {
                Some(s) => {
                    ui.label(format!("tick: {}", s.tick));
                    ui.label(format!("population: {}", s.population));
                    ui.label(format!("species: {}", s.species_count));
                    ui.label(format!("creatures drawn: {}", s.creatures.len()));
                }
                None if standalone_mode => {
                    ui.label("standalone mode — no sim, terrain only");
                }
                None => {
                    ui.label("waiting for the sim worker's first tick…");
                }
            }
            ui.separator();
            if !standalone_mode {
                ui.label("─ Creature Coloring (uptake_layer / feeding guild) ─");
                ui.colored_label(egui::Color32::from_rgb(255, 153, 51), "● Orange: Layer 0 (A-guild)");
                ui.colored_label(egui::Color32::from_rgb(51, 204, 255), "● Cyan: Layer 1 (B-guild)");
                ui.colored_label(egui::Color32::from_rgb(204, 51, 255), "● Magenta: Layer 2+");
                ui.separator();
            }
            ui.label(format!("terrain: {world_dim}×{world_dim}, {terrain_chunks_total} mesh chunks"));
            ui.label(format!("chunks drawn: {chunks_drawn}/{terrain_chunks_total}"));
            ui.label(format!("fps: {}", get_fps()));
            ui.separator();
            ui.label("Controls: WASD/drag pan · wheel zoom · Q/E rotate · T hex/cube");
            if !standalone_mode {
                ui.separator();
                ui.label("Space: toggle pause · Right/N: step once");
            }
        });
    });
}

/// Draw egui overlay to the screen after all scene rendering.
/// Call this after draw_debug_hud() and before next_frame().
pub fn draw() {
    egui_macroquad::draw();
}
