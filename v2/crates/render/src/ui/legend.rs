//! Material legend panel: swatch + name for each material type.

use egui::{Color32, RichText};
use crate::ui::{Anchor, Panel, UiCtx};
use super::theme;

pub struct LegendPanel;

impl Panel for LegendPanel {
    fn id(&self) -> &'static str { "legend" }
    fn anchor(&self) -> Anchor { Anchor::RightTop(egui::vec2(18.0, 420.0)) }

    fn draw(&mut self, ctx: &egui::Context, _ui_ctx: &mut UiCtx) {
        egui::Area::new(egui::Id::new("legend"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(18.0, 420.0))
            .show(ctx, |ui| {
                theme::themed_frame(theme::FrameKind::Vitals).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    let upper = "МАТЕРИАЛЫ";
                    let font = theme::mono(10.0);
                    let tr = theme::tracking_em(10.0, 0.18);
                    let w = theme::total_tracked_width(ui, upper, &font, tr);
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 11.0), egui::Sense::hover());
                    theme::paint_tracked(ui, rect.left_top(), egui::Align2::LEFT_TOP, upper, font, theme::TEXT_FAINT, tr);
                    ui.add_space(7.0);

                    let colors = world::palette::MATERIAL_COLORS;
                    let names = world::palette::MATERIAL_NAMES;
                    for i in 0..colors.len() {
                        let rgb = colors[i];
                        let color = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                        let name = names[i];
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                            ui.painter().rect_filled(rect, 3.0, color);
                            ui.label(RichText::new(name).font(theme::sans(11.0)).color(theme::TEXT));
                        });
                    }
                });
            });
    }
}
