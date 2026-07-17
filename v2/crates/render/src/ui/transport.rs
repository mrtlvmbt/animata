//! Transport panel (bottom-left): play/pause + step controls, sim-mode only.

use egui::{RichText, Shape, Stroke, StrokeKind};
use crate::ui::{Panel, UiAction, UiCtx};
use super::theme;

pub struct TransportPanel;

impl Panel for TransportPanel {
    fn id(&self) -> &'static str {
        "transport"
    }

    fn draw(&mut self, ctx: &egui::Context, ui_ctx: &mut UiCtx) {
        // Only show in sim mode (snapshot present)
        let Some(_snap) = ui_ctx.snap else { return };

        egui::Area::new(egui::Id::new("transport"))
            .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(18.0, -22.0))
            .show(ctx, |ui| {
                theme::themed_frame(theme::FrameKind::Transport).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 14.0;

                        // Play/pause button
                        if play_button(ui, false).clicked() {
                            ui_ctx.actions.push(UiAction::TogglePause);
                        }

                        // Divider
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 24.0), egui::Sense::hover());
                        ui.painter().vline(
                            rect.center().x,
                            rect.top()..=rect.bottom(),
                            Stroke::new(1.0, theme::HAIRLINE),
                        );

                        // Step button
                        if step_button(ui).clicked() {
                            ui_ctx.actions.push(UiAction::StepOnce);
                        }

                        ui.label(
                            RichText::new("Step")
                                .font(theme::mono(11.0))
                                .color(theme::TEXT_LABEL),
                        );
                    });
                });
            });
    }
}

/// Pause/play button: amber-tint square with vector glyph (triangle / two bars)
fn play_button(ui: &mut egui::Ui, paused: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::click());
    let p = ui.painter();
    let fill = if resp.hovered() {
        theme::straight(242, 166, 75, 66) // .26
    } else {
        theme::ACCENT_FILL // .16
    };
    p.rect_filled(rect, 11.0, fill);
    p.rect_stroke(rect, 11.0, Stroke::new(1.0, theme::ACCENT_LINE_50), StrokeKind::Inside);
    let ic = egui::Rect::from_center_size(rect.center(), egui::vec2(18.0, 18.0));
    if paused {
        // play triangle M8 5 v14 l11-7 Z
        p.add(Shape::convex_polygon(
            vec![vb(ic, 8.0, 5.0), vb(ic, 8.0, 19.0), vb(ic, 19.0, 12.0)],
            theme::ACCENT_TEXT,
            Stroke::NONE,
        ));
    } else {
        // pause: two bars
        for x in [6.0_f32, 14.0] {
            let br = egui::Rect::from_min_max(vb(ic, x, 5.0), vb(ic, x + 4.0, 19.0));
            p.rect_filled(br, vbr(ic, 1.0), theme::ACCENT_TEXT);
        }
    }
    resp
}

/// Step button: small square button
fn step_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::click());
    let p = ui.painter();
    let fill = if resp.hovered() {
        theme::straight(255, 255, 255, 26) // .10
    } else {
        theme::straight(255, 255, 255, 13) // .05
    };
    p.rect_filled(rect, 9.0, fill);
    p.rect_stroke(
        rect,
        9.0,
        Stroke::new(1.0, theme::straight(255, 255, 255, 26)),
        StrokeKind::Inside,
    );
    resp
}

// Helper functions for 24-px viewBox → rect mapping
fn vb(r: egui::Rect, x: f32, y: f32) -> egui::Pos2 {
    egui::Pos2::new(
        r.left() + x / 24.0 * r.width(),
        r.top() + y / 24.0 * r.height(),
    )
}

fn vbr(r: egui::Rect, rad: f32) -> f32 {
    rad / 24.0 * r.width()
}
