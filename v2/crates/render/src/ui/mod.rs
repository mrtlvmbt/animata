pub mod loader;
pub mod theme;
pub mod minimap;
pub mod vitals;
pub mod transport;
pub mod rail;
pub mod toast;
pub mod legend;

use macroquad::prelude::*;
use sim_core::RenderSnapshot;
use std::collections::HashMap;
use sim_core::WorldView;
use egui::{Color32, RichText, Stroke};

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
#[derive(Clone, Debug)]
pub enum UiAction {
    TogglePause,
    StepOnce,
    ToggleTerrainKind,
    /// U-3: Regenerate the world with a new seed (only valid in Procgen+standalone mode).
    RegenSeed(u64),
    /// U-5: Jump camera to a world position (x, z).
    JumpCamera(glam::Vec2),
    /// U-9: H key toggle — hide/show all panels
    ToggleUiVisibility,
    /// U-9: Display a toast message (e.g., "World ready — seed 0x5")
    PushToast(String),
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
    /// U-9: true if panels are visible, false if hidden by H toggle
    pub panels_visible: bool,
    /// U-9: timer for hide-hint display (ms elapsed since hide)
    pub hide_hint_elapsed_ms: f32,
    /// U-9: toast message state (shared with ToastPanel via UiCtx)
    pub toast_message: Option<String>,
    pub toast_elapsed_ms: f32,
}

impl UiRoot {
    pub fn new() -> Self {
        UiRoot {
            panels: Vec::new(),
            cache: HudCache::new(),
            panels_visible: true,
            hide_hint_elapsed_ms: 0.0,
            toast_message: None,
            toast_elapsed_ms: 0.0,
        }
    }

    /// Register a panel into the UI root.
    pub fn push(&mut self, panel: Box<dyn Panel>) {
        self.panels.push(panel);
    }

    /// Toggle panel visibility (H key). Resets hide-hint timer.
    pub fn toggle_visibility(&mut self) {
        self.panels_visible = !self.panels_visible;
        self.hide_hint_elapsed_ms = 0.0;  // Reset timer when toggling
    }

    /// Draw all panels and return pointer/keyboard wants.
    /// Sets the cache pointer in UiCtx to our cache before drawing.
    pub fn draw(&mut self, ctx: &egui::Context, ui_ctx: &mut UiCtx) -> UiOut {
        // Set the cache pointer to point to our cache
        ui_ctx.cache = &mut self.cache as *mut HudCache;

        // Update toast timer
        if self.toast_message.is_some() {
            self.toast_elapsed_ms += 16.0;  // ~60fps
            if self.toast_elapsed_ms > 2600.0 {
                self.toast_message = None;  // Expire after 2.6s
            }
        }

        // Update hide-hint timer
        if !self.panels_visible {
            self.hide_hint_elapsed_ms += 16.0;  // ~60fps
        }

        // Draw regular panels only if visible
        if self.panels_visible {
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
        }

        // Draw toast message if active (always visible, not gated by panels_visible)
        if let Some(ref msg) = self.toast_message {
            let dt = self.toast_elapsed_ms;
            let opacity = if dt < 180.0 { dt / 180.0 } else if dt > 1900.0 { ((2600.0 - dt) / 700.0).max(0.0) } else { 1.0 };
            let shift_y = if dt < 180.0 { -(180.0 - dt) / 180.0 * 10.0 } else { 0.0 };
            let a = |c: Color32| Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (c.a() as f32 * opacity) as u8);

            egui::Area::new(egui::Id::new("toast"))
                .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 18.0 + shift_y))
                .interactable(false)
                .show(ctx, |ui| {
                    egui::Frame::NONE
                        .fill(a(Color32::from_rgba_unmultiplied(12, 15, 14, 209)))
                        .stroke(Stroke::new(1.0, a(Color32::from_rgba_unmultiplied(143, 209, 111, 77))))
                        .corner_radius(egui::CornerRadius::same(11))
                        .inner_margin(egui::Margin::symmetric(18, 10))
                        .shadow(egui::epaint::Shadow { offset: [0, 10], blur: 30, spread: 0, color: Color32::from_black_alpha((102.0 * opacity) as u8) })
                        .show(ui, |ui| {
                            ui.add(egui::Label::new(RichText::new(msg).font(theme::mono(12.0)).color(a(theme::TOAST_GREEN))).wrap_mode(egui::TextWrapMode::Extend));
                        });
                });
            ctx.request_repaint();
        }

        // Draw hide-hint if panels are hidden and hint hasn't expired (2.5s = 2500ms)
        if !self.panels_visible && self.hide_hint_elapsed_ms < 2500.0 {
            let alpha_factor = if self.hide_hint_elapsed_ms < 200.0 {
                self.hide_hint_elapsed_ms / 200.0
            } else if self.hide_hint_elapsed_ms > 1800.0 {
                ((2500.0 - self.hide_hint_elapsed_ms) / 700.0).max(0.0)
            } else {
                1.0
            };

            let hint_text = RichText::new("press H — интерфейс")
                .font(theme::mono(10.0))
                .color(theme::TEXT_LABEL);

            egui::Area::new(egui::Id::new("hide_hint"))
                .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(18.0, -18.0))
                .interactable(false)
                .show(ctx, |ui| {
                    let base_color = theme::straight(12, 15, 14, 153);
                    let alpha = (base_color.a() as f32 * alpha_factor) as u8;
                    let faded_bg = Color32::from_rgba_unmultiplied(
                        base_color.r(), base_color.g(), base_color.b(), alpha
                    );

                    let border_color = theme::straight(255, 255, 255, 20);
                    let border_alpha = (border_color.a() as f32 * alpha_factor) as u8;
                    let faded_border = Color32::from_rgba_unmultiplied(
                        border_color.r(), border_color.g(), border_color.b(), border_alpha
                    );

                    egui::Frame::NONE
                        .fill(faded_bg)
                        .stroke(Stroke::new(1.0, faded_border))
                        .corner_radius(egui::CornerRadius::same(9))
                        .inner_margin(egui::Margin::symmetric(12, 7))
                        .show(ui, |ui| {
                            ui.label(hint_text);
                        });
                });
        }

        // Compute pointer/keyboard wants from egui state.
        // When panels are hidden, don't capture input
        UiOut {
            wants_pointer: self.panels_visible && (ctx.is_pointer_over_area() || ctx.wants_pointer_input()),
            wants_keyboard: self.panels_visible && ctx.wants_keyboard_input(),
        }
    }
}

impl Default for UiRoot {
    fn default() -> Self {
        Self::new()
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
