//! Toast and hide-hint panels.

use egui::{Color32, RichText, Stroke};
use crate::ui::{Anchor, Panel, UiCtx};
use super::theme;

#[derive(Default)]
pub struct ToastState { pub message: Option<String>, pub elapsed_ms: f32 }

pub struct ToastPanel { pub state: ToastState }
impl ToastPanel { pub fn new() -> Self { ToastPanel { state: ToastState::default() } } }

impl Panel for ToastPanel {
    fn id(&self) -> &'static str { "toast" }
    fn anchor(&self) -> Anchor { Anchor::LeftTop(egui::vec2(0.0, 0.0)) }

    fn draw(&mut self, ctx: &egui::Context, _ui_ctx: &mut UiCtx) {
        self.state.elapsed_ms += 16.0;
        let Some(ref msg) = self.state.message else { return };
        if self.state.elapsed_ms > 2600.0 { self.state.message = None; return; }
        let dt = self.state.elapsed_ms;
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
}

pub struct HideHintPanel;

impl Panel for HideHintPanel {
    fn id(&self) -> &'static str { "hide_hint" }
    fn anchor(&self) -> Anchor { Anchor::LeftBottom(egui::vec2(18.0, 18.0)) }

    fn draw(&mut self, ctx: &egui::Context, _ui_ctx: &mut UiCtx) {
        egui::Area::new(egui::Id::new("hide_hint"))
            .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(18.0, -18.0))
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(theme::straight(12, 15, 14, 153))
                    .stroke(Stroke::new(1.0, theme::straight(255, 255, 255, 20)))
                    .corner_radius(egui::CornerRadius::same(9))
                    .inner_margin(egui::Margin::symmetric(12, 7))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            ui.label(RichText::new("press").font(theme::mono(10.0)).color(theme::TEXT_LABEL));
                            egui::Frame::NONE.fill(theme::straight(255, 255, 255, 26)).corner_radius(egui::CornerRadius::same(5)).inner_margin(egui::Margin::symmetric(6, 2)).show(ui, |ui| {
                                ui.label(RichText::new("H").font(theme::mono(10.0)).color(theme::TEXT));
                            });
                            ui.label(RichText::new("— интерфейс").font(theme::mono(10.0)).color(theme::TEXT_LABEL));
                        });
                    });
            });
    }
}
