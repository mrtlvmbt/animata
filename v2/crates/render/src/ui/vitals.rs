//! Vitals panel (top-left): compact always-on strip showing world seed, dim, and optionally population.

use egui::{Align, Layout, RichText};
use crate::ui::{Anchor, Panel, UiCtx};
use super::theme;

pub struct VitalsPanel;

impl Panel for VitalsPanel {
    fn id(&self) -> &'static str {
        "vitals"
    }

    fn anchor(&self) -> Anchor {
        Anchor::LeftTop(egui::vec2(18.0, 18.0))
    }

    fn draw(&mut self, ctx: &egui::Context, ui_ctx: &mut UiCtx) {
        egui::Area::new(egui::Id::new("vitals"))
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(18.0, 18.0))
            .show(ctx, |ui| {
                ui.set_min_width(180.0);  // Prevent SIZE value from wrapping
                theme::themed_frame(theme::FrameKind::Vitals).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 16.0;

                        // World seed (hex)
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 4.0;
                            caps_tracked(ui, "SEED", 9.0, 0.16, theme::TEXT_FAINT);
                            ui.label(
                                RichText::new(format!("0x{:X}", ui_ctx.seed))
                                    .font(theme::mono(12.0))
                                    .color(theme::TEXT),
                            );
                        });

                        vitals_hairline(ui);

                        // World dimensions
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 4.0;
                            caps_tracked(ui, "SIZE", 9.0, 0.16, theme::TEXT_FAINT);
                            ui.label(
                                RichText::new(format!("{}×{}", ui_ctx.world_dim, ui_ctx.world_dim))
                                    .font(theme::mono(12.0))
                                    .color(theme::TEXT),
                            );
                        });

                        // Population (only if sim snapshot is attached)
                        if let Some(snap) = ui_ctx.snap {
                            vitals_hairline(ui);
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 4.0;
                                caps_tracked(ui, "POP", 9.0, 0.16, theme::TEXT_FAINT);
                                ui.label(
                                    RichText::new(format!("{}", snap.population))
                                        .font(theme::mono(12.0))
                                        .color(theme::TEXT),
                                );
                            });

                            vitals_hairline(ui);

                            // Tick (sim only)
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 4.0;
                                caps_tracked(ui, "TICK", 9.0, 0.16, theme::TEXT_FAINT);
                                ui.label(
                                    RichText::new(format!("{}", snap.tick))
                                        .font(theme::mono(12.0))
                                        .color(theme::TEXT),
                                );
                            });
                        }
                    });
                });
            });
    }
}

fn vitals_hairline(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 26.0), egui::Sense::hover());
    ui.painter().vline(
        rect.center().x,
        rect.top()..=rect.bottom(),
        egui::Stroke::new(1.0, theme::HAIRLINE),
    );
}

/// Mono caps label with CSS letter-spacing
fn caps_tracked(ui: &mut egui::Ui, text: &str, size: f32, em: f32, color: egui::Color32) {
    let upper = text.to_uppercase();
    let font = theme::mono(size);
    let tr = theme::tracking_em(size, em);
    let w = theme::total_tracked_width(ui, &upper, &font, tr);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, size + 1.0), egui::Sense::hover());
    theme::paint_tracked(ui, rect.left_top(), egui::Align2::LEFT_TOP, &upper, font, color, tr);
}
