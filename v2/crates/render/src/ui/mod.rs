pub mod theme;

use macroquad::prelude::*;
use sim_core::RenderSnapshot;
use std::collections::HashMap;

/// UiOut: egui's pointer/keyboard wants from this frame.
/// Used to gate camera input so UI interaction doesn't drive camera pan/rotate.
#[derive(Clone, Copy, Debug, Default)]
pub struct UiOut {
    pub wants_pointer: bool,
    pub wants_keyboard: bool,
}

/// Panel: a renderable UI element with a fixed anchor point and draw callback.
pub trait Panel {
    fn id(&self) -> &'static str;
    fn anchor(&self) -> Anchor;
    fn draw(&mut self, ctx: &egui::Context, ui_ctx: &mut UiCtx);
}

/// Anchor: position of a panel on screen (LeftTop/RightTop/LeftBottom/RightBottom + offset).
#[derive(Clone, Copy, Debug)]
pub enum Anchor {
    LeftTop(egui::Vec2),
    RightTop(egui::Vec2),
    LeftBottom(egui::Vec2),
    RightBottom(egui::Vec2),
}

impl Anchor {
    fn pos(&self, screen_size: egui::Vec2) -> egui::Pos2 {
        match self {
            Anchor::LeftTop(offset) => egui::pos2(offset.x, offset.y),
            Anchor::RightTop(offset) => egui::pos2(screen_size.x - offset.x, offset.y),
            Anchor::LeftBottom(offset) => egui::pos2(offset.x, screen_size.y - offset.y),
            Anchor::RightBottom(offset) => egui::pos2(screen_size.x - offset.x, screen_size.y - offset.y),
        }
    }
}

/// UiCtx: read-only frame state + action sink (passed to Panel::draw).
pub struct UiCtx<'a> {
    pub world_dim: i64,
    pub seed: u64,
    pub fps: i32,
    pub chunks_drawn: usize,
    pub verts: usize,
    pub snap: Option<&'a RenderSnapshot>,
    pub standalone_mode: bool,
    pub terrain_chunks_total: usize,
    pub actions: &'a mut Vec<UiAction>,
}

/// UiAction: commands from the UI that main.rs applies after the egui pass.
/// UI never mutates app state directly; all changes go through actions.
#[derive(Clone, Copy, Debug)]
pub enum UiAction {
    TogglePause,
    StepOnce,
    ToggleTerrainKind,
}

/// HudCache: texture caches and other UI-layer resources (for minimap, etc.).
pub struct HudCache {
    pub textures: HashMap<&'static str, egui::TextureHandle>,
}

impl HudCache {
    pub fn new() -> Self {
        HudCache {
            textures: HashMap::new(),
        }
    }
}

impl Default for HudCache {
    fn default() -> Self {
        Self::new()
    }
}

/// UiRoot: registry of all panels; drives the egui pass each frame.
pub struct UiRoot {
    panels: Vec<Box<dyn Panel>>,
    pub cache: HudCache,
}

impl UiRoot {
    pub fn new() -> Self {
        UiRoot {
            panels: Vec::new(),
            cache: HudCache::new(),
        }
    }

    /// Register a panel into the UI root.
    pub fn push(&mut self, panel: Box<dyn Panel>) {
        self.panels.push(panel);
    }

    /// Draw all panels and return pointer/keyboard wants.
    pub fn draw(&mut self, ctx: &egui::Context, ui_ctx: &mut UiCtx) -> UiOut {
        for panel in &mut self.panels {
            let anchor = panel.anchor();
            let screen_size = ctx.screen_rect().size();
            let pivot = anchor.pos(screen_size);

            egui::Area::new(panel.id().into())
                .fixed_pos(pivot)
                .pivot(egui::Align2::LEFT_TOP)
                .show(ctx, |ui| {
                    panel.draw(ctx, ui_ctx);
                });
        }

        // Compute pointer/keyboard wants from egui state.
        UiOut {
            wants_pointer: ctx.is_pointer_over_area() || ctx.wants_pointer_input(),
            wants_keyboard: ctx.wants_keyboard_input(),
        }
    }
}

impl Default for UiRoot {
    fn default() -> Self {
        Self::new()
    }
}

/// DebugPanel: the first Panel implementation — re-hosts draw_debug_hud content.
pub struct DebugPanel;

impl Panel for DebugPanel {
    fn id(&self) -> &'static str {
        "debug_hud"
    }

    fn anchor(&self) -> Anchor {
        Anchor::LeftTop(egui::vec2(10.0, 10.0))
    }

    fn draw(&mut self, ctx: &egui::Context, ui_ctx: &mut UiCtx) {
        let title = if ui_ctx.standalone_mode {
            "v2 render scaffold — R-8 standalone hex-map viewer"
        } else {
            "v2 render scaffold — R-7 biology coloring"
        };

        egui::Window::new(title)
            .frame(theme::themed_frame(theme::FrameKind::Vitals))
            .show(ctx, |ui| {
                match ui_ctx.snap {
                    Some(s) => {
                        ui.label(format!("tick: {}", s.tick));
                        ui.label(format!("population: {}", s.population));
                        ui.label(format!("species: {}", s.species_count));
                        ui.label(format!("creatures drawn: {}", s.creatures.len()));
                    }
                    None if ui_ctx.standalone_mode => {
                        ui.label("standalone mode — no sim, terrain only");
                    }
                    None => {
                        ui.label("waiting for the sim worker's first tick…");
                    }
                }
                ui.separator();
                if !ui_ctx.standalone_mode {
                    ui.label("─ Creature Coloring (uptake_layer / feeding guild) ─");
                    ui.colored_label(egui::Color32::from_rgb(255, 153, 51), "● Orange: Layer 0 (A-guild)");
                    ui.colored_label(egui::Color32::from_rgb(51, 204, 255), "● Cyan: Layer 1 (B-guild)");
                    ui.colored_label(egui::Color32::from_rgb(204, 51, 255), "● Magenta: Layer 2+");
                    ui.separator();
                }
                ui.label(format!("terrain: {}×{}, {} mesh chunks", ui_ctx.world_dim, ui_ctx.world_dim, ui_ctx.terrain_chunks_total));
                ui.label(format!("chunks drawn: {}/{}", ui_ctx.chunks_drawn, ui_ctx.terrain_chunks_total));
                ui.label(format!("fps: {}", ui_ctx.fps));
                ui.separator();

                // F1: Real UiAction flow — buttons push actions that main.rs consumes end-to-end
                if !ui_ctx.standalone_mode {
                    ui.horizontal(|ui| {
                        if ui.button("Pause").clicked() {
                            ui_ctx.actions.push(UiAction::TogglePause);
                        }
                        if ui.button("Step").clicked() {
                            ui_ctx.actions.push(UiAction::StepOnce);
                        }
                    });
                }

                if ui.button("Hex↔Cube").clicked() {
                    ui_ctx.actions.push(UiAction::ToggleTerrainKind);
                }

                ui.label("Keyboard: WASD/drag pan · wheel zoom · Q/E rotate");
                if !ui_ctx.standalone_mode {
                    ui.label("Space: toggle pause · Right/N: step once");
                }
            });
    }
}

/// Draw egui overlay to the screen after all scene rendering.
/// Call this after ui_root.draw() and before next_frame().
pub fn draw() {
    egui_macroquad::draw();
}
