//! Control rail (bottom-right): vertical icon buttons with flyout panels.

use egui::{Align, Layout, RichText, Shape, Stroke, StrokeKind};
use crate::ui::{Anchor, Panel, UiAction, UiCtx};
use super::theme;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RailPanel { World, View, Perf, Pop }

pub struct ControlRail { pub open_panel: Option<RailPanel> }

impl ControlRail { pub fn new() -> Self { ControlRail { open_panel: None } } }

impl Panel for ControlRail {
    fn id(&self) -> &'static str { "rail" }
    fn anchor(&self) -> Anchor { Anchor::RightBottom(egui::vec2(18.0, 22.0)) }

    fn draw(&mut self, ctx: &egui::Context, ui_ctx: &mut UiCtx) {
        egui::Area::new(egui::Id::new("rail"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-18.0, -22.0))
            .show(ctx, |ui| {
                theme::themed_frame(theme::FrameKind::Rail).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    for (p, i) in [(RailPanel::World, RailIcon::Clock), (RailPanel::View, RailIcon::Layers), (RailPanel::Perf, RailIcon::Bars)] {
                        if icon_tab(ui, "", i, self.open_panel == Some(p)).clicked() {
                            self.open_panel = if self.open_panel == Some(p) { None } else { Some(p) };
                        }
                    }
                    if ui_ctx.snap.is_some() {
                        if icon_tab(ui, "", RailIcon::Circles, self.open_panel == Some(RailPanel::Pop)).clicked() {
                            self.open_panel = if self.open_panel == Some(RailPanel::Pop) { None } else { Some(RailPanel::Pop) };
                        }
                    }
                    ui.add_space(2.0);
                    let (r, _) = ui.allocate_exact_size(egui::vec2(28.0, 1.0), egui::Sense::hover());
                    ui.painter().hline(r.left()..=r.right(), r.center().y, Stroke::new(1.0, theme::HAIRLINE));
                    ui.add_space(2.0);
                    if icon_tab(ui, "", RailIcon::Eye, false).clicked() {}
                });
            });

        if let Some(panel) = self.open_panel {
            draw_flyout(ctx, panel, ui_ctx);
        }
    }
}

#[derive(Clone, Copy)]
enum RailIcon { Clock, Layers, Circles, Bars, Eye }

fn icon_tab(ui: &mut egui::Ui, _id: &str, icon: RailIcon, active: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::click());
    let p = ui.painter();
    if active {
        p.rect_filled(rect, 10.0, theme::ACCENT_FILL);
        p.rect_stroke(rect, 10.0, Stroke::new(1.0, theme::ACCENT_LINE_50), StrokeKind::Inside);
        p.rect_filled(egui::Rect::from_center_size(egui::pos2(rect.left() - 5.5, rect.center().y), egui::vec2(3.0, 18.0)), 3.0, theme::ACCENT);
    } else if resp.hovered() {
        p.rect_filled(rect, 10.0, theme::HOVER_FILL);
    }
    let col = if active { theme::ACCENT_TEXT } else if resp.hovered() { theme::TEXT } else { theme::TEXT_LABEL };
    let ic = egui::Rect::from_center_size(rect.center(), egui::vec2(19.0, 19.0));
    paint_rail_icon(p, icon, ic, col);
    resp
}

fn paint_rail_icon(p: &egui::Painter, icon: RailIcon, r: egui::Rect, col: egui::Color32) {
    let s = Stroke::new(1.6, col);
    match icon {
        RailIcon::Clock => { p.circle_stroke(vb(r, 12.0, 12.0), vbr(r, 8.5), s); p.add(Shape::line(vec![vb(r, 12.0, 7.0), vb(r, 12.0, 12.0), vb(r, 15.2, 13.8)], s)); }
        RailIcon::Layers => { p.add(Shape::closed_line(vec![vb(r, 12.0, 4.0), vb(r, 20.0, 8.0), vb(r, 12.0, 12.0), vb(r, 4.0, 8.0)], s)); p.add(Shape::line(vec![vb(r, 4.0, 12.0), vb(r, 12.0, 16.0), vb(r, 20.0, 12.0)], s)); }
        RailIcon::Circles => { p.circle_stroke(vb(r, 8.0, 9.0), vbr(r, 2.4), s); p.circle_stroke(vb(r, 15.5, 7.0), vbr(r, 2.0), s); p.circle_stroke(vb(r, 13.0, 15.0), vbr(r, 2.8), s); }
        RailIcon::Bars => { for (x, y, h) in [(4.0, 13.0, 7.0), (10.2, 8.0, 12.0), (16.5, 5.0, 15.0)] { let br = egui::Rect::from_min_max(vb(r, x, y), vb(r, x + 3.5, y + h)); p.rect_stroke(br, vbr(r, 1.0), s, StrokeKind::Inside); } }
        RailIcon::Eye => { let c = vb(r, 12.0, 12.0); let (rx, ry) = (vbr(r, 10.0), vbr(r, 6.0)); let pts: Vec<_> = (0..=28).map(|i| { let a = i as f32 / 28.0 * std::f32::consts::TAU; egui::Pos2::new(c.x + rx * a.cos(), c.y + ry * a.sin()) }).collect(); p.add(Shape::closed_line(pts, s)); p.circle_stroke(c, vbr(r, 2.6), s); }
    }
}

fn draw_flyout(ctx: &egui::Context, panel: RailPanel, ui_ctx: &mut UiCtx) {
    let width = match panel { RailPanel::World | RailPanel::View => 238.0, RailPanel::Pop => 248.0, RailPanel::Perf => 222.0 };
    egui::Area::new(egui::Id::new("flyout")).anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-84.0, -22.0)).show(ctx, |ui| {
        theme::themed_frame(theme::FrameKind::Flyout).show(ui, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 7.0;
            match panel {
                RailPanel::World => { caps_tracked(ui, "WORLD", 10.0, 0.18, theme::TEXT_FAINT); ui.add_space(7.0); kv(ui, "seed", format!("0x{:X}", ui_ctx.seed)); kv(ui, "size", format!("{}×{}", ui_ctx.world_dim, ui_ctx.world_dim)); if let Some(snap) = ui_ctx.snap { hairline(ui); kv(ui, "tick", format!("{}", snap.tick)); } if ui_ctx.is_procgen && ui_ctx.standalone_mode { hairline(ui); if ui.button("New world").clicked() { ui_ctx.actions.push(UiAction::RegenSeed(ui_ctx.seed.wrapping_add(1))); } } }
                RailPanel::View => { caps_tracked(ui, "VIEW", 10.0, 0.18, theme::TEXT_FAINT); ui.add_space(7.0); if ui.button("Hex ↔ Cube").clicked() { ui_ctx.actions.push(UiAction::ToggleTerrainKind); } }
                RailPanel::Perf => { caps_tracked(ui, "FPS", 10.0, 0.18, theme::TEXT_FAINT); ui.add_space(7.0); ui.label(RichText::new(format!("{:.0}", ui_ctx.fps)).font(theme::mono(26.0)).color(theme::TEXT)); hairline(ui); kv(ui, "chunks", format!("{}", ui_ctx.chunks_drawn)); kv(ui, "verts", format!("{}", ui_ctx.verts)); }
                RailPanel::Pop => { caps_tracked(ui, "POP", 10.0, 0.18, theme::TEXT_FAINT); ui.add_space(7.0); if let Some(snap) = ui_ctx.snap { kv(ui, "count", format!("{}", snap.population)); kv(ui, "species", format!("{}", snap.species_count)); } }
            }
        });
    });
}

fn kv(ui: &mut egui::Ui, label: &str, value: String) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).font(theme::sans(12.0)).color(theme::TEXT_LABEL));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(value).font(theme::mono(12.0)).color(theme::TEXT));
        });
    });
}

fn hairline(ui: &mut egui::Ui) {
    ui.add_space(5.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().hline(rect.left()..=rect.right(), rect.center().y, Stroke::new(1.0, theme::HAIRLINE));
    ui.add_space(5.0);
}

fn caps_tracked(ui: &mut egui::Ui, text: &str, size: f32, em: f32, color: egui::Color32) {
    let upper = text.to_uppercase();
    let font = theme::mono(size);
    let tr = theme::tracking_em(size, em);
    let w = theme::total_tracked_width(ui, &upper, &font, tr);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, size + 1.0), egui::Sense::hover());
    theme::paint_tracked(ui, rect.left_top(), egui::Align2::LEFT_TOP, &upper, font, color, tr);
}

fn vb(r: egui::Rect, x: f32, y: f32) -> egui::Pos2 { egui::Pos2::new(r.left() + x / 24.0 * r.width(), r.top() + y / 24.0 * r.height()) }
fn vbr(r: egui::Rect, rad: f32) -> f32 { rad / 24.0 * r.width() }
