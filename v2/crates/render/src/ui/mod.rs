pub mod loader;
pub mod theme;
pub mod minimap;

use macroquad::prelude::*;
use sim_core::RenderSnapshot;
use std::collections::HashMap;
use sim_core::WorldView;

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
    /// U-3: true if source is Procgen (needed for reseed button gating per F12)
    pub is_procgen: bool,
    /// U-3: optional LoadState for in-flight reseed progress tracking
    pub regen_load_state: Option<&'a crate::loader_state::LoadState>,
    /// U-5: reference to the world for minimap rendering
    pub world: Option<&'a (dyn sim_core::WorldView + Sync)>,
    /// U-5: bare_mode flag for minimap colouring
    pub bare_mode: bool,
    /// U-5: mutable reference to HudCache for texture management (raw pointer to work around borrow checker)
    pub cache: *mut HudCache,
    /// U-5: camera focus point (for viewport quad in minimap)
    pub camera_focus: glam::Vec3,
    /// U-5: camera ortho_span (for viewport quad in minimap)
    pub camera_ortho_span: f32,
    /// U-5: camera yaw (for viewport quad in minimap)
    pub camera_yaw: f32,
    /// U-5: screen dimensions for camera math
    pub screen_dims: (f32, f32),
}

/// UiAction: commands from the UI that main.rs applies after the egui pass.
/// UI never mutates app state directly; all changes go through actions.
#[derive(Clone, Copy, Debug)]
pub enum UiAction {
    TogglePause,
    StepOnce,
    ToggleTerrainKind,
    /// U-3: Regenerate the world with a new seed (only valid in Procgen+standalone mode).
    RegenSeed(u64),
    /// U-5: Jump camera to a world position (x, z).
    JumpCamera(glam::Vec2),
}

/// HudCache: texture caches and other UI-layer resources (for minimap, etc.).
pub struct HudCache {
    pub textures: HashMap<&'static str, egui::TextureHandle>,
    /// Minimap texture cache: (seed, dim, bare_mode) → (key_tuple, texture_handle)
    pub minimap: Option<((u64, i64, bool), egui::TextureHandle)>,
}

impl HudCache {
    pub fn new() -> Self {
        HudCache {
            textures: HashMap::new(),
            minimap: None,
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
    /// Sets the cache pointer in UiCtx to our cache before drawing.
    pub fn draw(&mut self, ctx: &egui::Context, ui_ctx: &mut UiCtx) -> UiOut {
        // Set the cache pointer to point to our cache
        ui_ctx.cache = &mut self.cache as *mut HudCache;

        for panel in &mut self.panels {
            let anchor = panel.anchor();
            let screen_size = ctx.screen_rect().size();
            let pivot = anchor.pos(screen_size);

            egui::Area::new(panel.id().into())
                .fixed_pos(pivot)
                .pivot(egui::Align2::LEFT_TOP)
                .show(ctx, |_| {
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

                // U-3: "New world" button — only shown in Procgen+standalone mode (F12/F15)
                // When clicked, regenerate with next seed (current_seed + 1)
                if ui_ctx.is_procgen && ui_ctx.standalone_mode {
                    if ui.button("New world (N)").clicked() {
                        ui_ctx.actions.push(UiAction::RegenSeed(ui_ctx.seed.wrapping_add(1)));
                    }
                }

                ui.label("Keyboard: WASD/drag pan · wheel zoom · Q/E rotate");
                if !ui_ctx.standalone_mode {
                    ui.label("Space: toggle pause · Right: step once");
                } else if ui_ctx.is_procgen {
                    ui.label("N: new world");
                }
            });
    }
}

/// Dim the panel OUTSIDE the viewport rectangle (four bands around the frame's bounding box, clamped
/// to the panel). Bands collapse to nothing once the view encloses the whole map.
/// U-8: Veil alpha adjusted to 130 (.51) for darker appearance per user request.
fn veil_outside(painter: &egui::Painter, rect: egui::Rect, quad: &[egui::Pos2]) {
    const VEIL_ALPHA: u8 = 130; // User-taste dial: darker veil for visual separation
    let mut lo = rect.max;
    let mut hi = rect.min;
    for p in quad {
        let x = p.x.clamp(rect.left(), rect.right());
        let y = p.y.clamp(rect.top(), rect.bottom());
        lo = egui::pos2(lo.x.min(x), lo.y.min(y));
        hi = egui::pos2(hi.x.max(x), hi.y.max(y));
    }
    let veil = egui::Color32::from_rgba_unmultiplied(5, 7, 10, VEIL_ALPHA);
    let band = |a: egui::Pos2, b: egui::Pos2| {
        let r = egui::Rect::from_two_pos(a, b);
        if r.width() > 0.5 && r.height() > 0.5 {
            painter.rect_filled(r, 0.0, veil);
        }
    };
    band(rect.left_top(), egui::pos2(rect.right(), lo.y)); // top
    band(egui::pos2(rect.left(), hi.y), rect.right_bottom()); // bottom
    band(egui::pos2(rect.left(), lo.y), egui::pos2(lo.x, hi.y)); // left
    band(egui::pos2(hi.x, lo.y), egui::pos2(rect.right(), hi.y)); // right
}

/// U-5: MinimapPanel — isometric minimap with viewport quad and click-to-jump.
pub struct MinimapPanel;

impl Panel for MinimapPanel {
    fn id(&self) -> &'static str {
        "minimap_panel"
    }

    fn anchor(&self) -> Anchor {
        Anchor::RightTop(egui::vec2(16.0, 16.0))
    }

    fn draw(&mut self, ctx: &egui::Context, ui_ctx: &mut UiCtx) {
        // Only draw if we have a world
        let Some(world) = ui_ctx.world else {
            return;
        };

        let cache_key = (ui_ctx.seed, ui_ctx.world_dim, ui_ctx.bare_mode);

        // SAFETY: ui_root.draw() (ui/mod.rs:138) sets ui_ctx.cache to point to &mut self.cache
        // before calling Panel::draw(). The raw pointer remains valid for the duration of the
        // draw call (the entire Panel::draw frame) because:
        // 1. ui_ctx.cache is set immediately before the loop over panels (no drop/mutation of self)
        // 2. self (UiRoot) is borrowed mutably only during draw(), keeping the cache allocation stable
        // 3. Panel::draw() is called synchronously within the same call stack, before draw() returns
        // The invariant FAILS if: (a) draw() is called concurrently, (b) ui_ctx is reused across
        // draw() calls without re-setting cache, or (c) self is moved/dropped during the call.
        // All three are prevented by the calling convention (main.rs creates UiCtx, calls
        // ui_root.draw(ctx, &mut ui_ctx) once per frame, ui_root is not shared).
        let cache = unsafe { &mut *ui_ctx.cache };

        // Check if we need to rebuild the minimap texture
        let stale = cache.minimap.as_ref().map(|(k, _)| *k != cache_key).unwrap_or(true);
        if stale {
            // Build the minimap image from the world
            let img = minimap::build_minimap_image(world, ui_ctx.world_dim, ui_ctx.seed, ui_ctx.bare_mode);
            let tex = ctx.load_texture("minimap", img, egui::TextureOptions::NEAREST);
            cache.minimap = Some((cache_key, tex));
        }

        // Get the texture from cache
        let tex = &cache.minimap.as_ref().unwrap().1;

        egui::Area::new(egui::Id::new("minimap"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-16.0, 16.0))
            .show(ctx, |ui| {
                theme::themed_frame(theme::FrameKind::Vitals)
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        let size = egui::vec2(minimap::MINIMAP_WIDTH as f32, minimap::MINIMAP_HEIGHT as f32);
                        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());

                        // U-8: Draw the minimap texture as a screen-aligned iso diamond through the new projection
                        let painter = ui.painter_at(rect);
                        let mut mesh = egui::Mesh::with_texture(tex.id());
                        for &(u, v) in &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
                            let (panel_x, panel_y) = minimap::map_uv_to_panel(
                                u,
                                v,
                                ui_ctx.camera_yaw,
                                rect.width(),
                                rect.height(),
                            );
                            mesh.vertices.push(egui::epaint::Vertex {
                                pos: egui::Pos2::new(
                                    rect.left() + panel_x,
                                    rect.top() + panel_y,
                                ),
                                uv: egui::pos2(u, v),
                                color: egui::Color32::WHITE,
                            });
                        }
                        mesh.add_triangle(0, 1, 2);
                        mesh.add_triangle(0, 2, 3);
                        painter.add(egui::Shape::mesh(mesh));

                        // U-8: Draw viewport quad through the same screen-aligned projection
                        let aspect = ui_ctx.screen_dims.0 / ui_ctx.screen_dims.1;
                        let cam_vp = minimap::minimap_view_proj_matrix(
                            ui_ctx.camera_focus,
                            ui_ctx.camera_yaw,
                            ui_ctx.camera_ortho_span,
                            aspect,
                        );
                        let corners = minimap::screen_quad_corners(ui_ctx.screen_dims);
                        let mut viewport_pts: Vec<egui::Pos2> = Vec::new();
                        for corner_screen in corners.iter() {
                            let world_xz = minimap::minimap_ground_under_cursor(cam_vp, *corner_screen, ui_ctx.screen_dims);
                            let uv = minimap::world_to_minimap_uv(world_xz, ui_ctx.world_dim);
                            let (panel_x, panel_y) = minimap::map_uv_to_panel(
                                uv.x,
                                uv.y,
                                ui_ctx.camera_yaw,
                                rect.width(),
                                rect.height(),
                            );
                            viewport_pts.push(egui::Pos2::new(rect.left() + panel_x, rect.top() + panel_y));
                        }
                        // Draw veil outside the viewport quad (v1 parity)
                        if viewport_pts.len() == 4 {
                            veil_outside(&painter, rect, &viewport_pts);
                        }

                        // Draw closed quad outline
                        if viewport_pts.len() == 4 {
                            let stroke = egui::Stroke::new(1.5, theme::ACCENT);
                            painter.add(egui::Shape::closed_line(viewport_pts, stroke));
                        }

                        // U-8: Handle click to jump using the inverted screen-aligned projection
                        if response.clicked() {
                            if let Some(pos) = response.interact_pointer_pos() {
                                let panel_x = pos.x - rect.left();
                                let panel_y = pos.y - rect.top();
                                let uv = minimap::panel_to_map_uv(
                                    panel_x,
                                    panel_y,
                                    ui_ctx.camera_yaw,
                                    rect.width(),
                                    rect.height(),
                                );
                                let world_pos = minimap::minimap_uv_to_world(uv, ui_ctx.world_dim);
                                ui_ctx.actions.push(UiAction::JumpCamera(world_pos));
                            }
                        }
                    });
            });
    }
}

/// Draw egui overlay to the screen after all scene rendering.
/// Call this after ui_root.draw() and before next_frame().
pub fn draw() {
    egui_macroquad::draw();
}
