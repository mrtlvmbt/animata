//! Creature inspector — a selection-driven context panel (left, under vitals) plus the in-world
//! crosshair on the picked creature and ring markers on its conspecifics. NOT a rail tab: it lives
//! outside the "one panel at a time" rule, entered by clicking a creature in the world. The data
//! ([`CreatureView`]) is built in `main.rs` (so this module stays free of sim-getter knowledge, like
//! the rest of the HUD) and handed in via [`SimMetrics`]. Geometry/colours follow the mockup spec.

use egui::{Align, Align2, Color32, Id, Layout, LayerId, Order, Pos2, RichText, Sense, Stroke, StrokeKind, Vec2};

use super::hud::{bar_sized, caps_tracked, hairline, kv};
use super::theme;
use super::theme::FrameKind;
use super::{SimMetrics, UiState};

/// Trophic role of the inspected creature — the one place a non-amber data colour names a category
/// (a small dot beside a neutral label). Carnivore=terracotta, Herbivore=green, Autotroph=blue.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TrophicKind {
    Carnivore,
    Herbivore,
    Autotroph,
}

impl TrophicKind {
    fn color(self) -> Color32 {
        match self {
            TrophicKind::Carnivore => theme::DATA_CARN,
            TrophicKind::Herbivore => theme::GOOD_GREEN,
            TrophicKind::Autotroph => theme::DATA_AUTO,
        }
    }
    fn label(self) -> &'static str {
        match self {
            TrophicKind::Carnivore => "Carnivore",
            TrophicKind::Herbivore => "Herbivore",
            TrophicKind::Autotroph => "Autotroph",
        }
    }
}

/// A per-frame snapshot of the inspected creature, derived in `main.rs`. Strings are formatted there
/// (this struct only renders). Fields that the sim doesn't model (name / generation / health /
/// hydration / activity / offspring) are derived deterministically per-creature in `main.rs`.
pub struct CreatureView {
    pub id: String,       // morphotype tag, e.g. "AX-7" (Mono)
    pub name: String,     // species name (Sans)
    pub kind: TrophicKind,
    pub generation: u32,
    pub age: String,      // "6.2 d"
    pub diet: String,
    pub mass: String,     // "1.8 kg"
    pub locomotion: String,
    pub strata: String,
    pub energy: f32,      // 0..1
    pub health: f32,      // 0..1
    pub hydration: f32,   // 0..1
    pub traits: [(&'static str, f32); 6], // genome, fixed order (spec §4.3)
    pub activity: String,
    pub offspring: u32,
}

// straight-alpha tints local to the inspector
const TAG_BG: Color32 = theme::straight(12, 15, 14, 217); // .85 id-tag plate
const DOT_RING: Color32 = theme::straight(10, 12, 11, 217); // .85 dark dot outline
const DOT_HALO: Color32 = theme::straight(5, 7, 10, 77); // .30 outer halo
const RING_BRIGHT: Color32 = theme::straight(242, 166, 75, 230); // accent .90 inner ring
const CONSPEC: Color32 = theme::straight(242, 166, 75, 115); // accent .45 conspecific rings
const CHIP_BG: Color32 = theme::straight(255, 255, 255, 15); // .06 activity chip
const CHIP_STROKE: Color32 = theme::straight(255, 255, 255, 26); // .10
const LABEL_55: Color32 = theme::straight(233, 236, 230, 140); // .55 genome label
const VALUE_80: Color32 = theme::straight(233, 236, 230, 204); // .80 genome value
const CLOSE_GLYPH: Color32 = theme::straight(233, 236, 230, 128); // .50 ×

/// Whole inspector pass: world markers (background layer, under panels) + the panel Area.
pub fn draw_inspector(ctx: &egui::Context, st: &mut UiState, m: &SimMetrics, now: f32) {
    markers(ctx, m, now);
    let Some(view) = &m.inspect else { return };
    panel(ctx, st, view);
}

/// In-world crosshair on the selected creature + hollow rings on its conspecifics. Drawn on a
/// Background layer so they sit over the world but under the HUD panels, and are non-interactive
/// (they never capture the pointer → the world stays clickable through them).
fn markers(ctx: &egui::Context, m: &SimMetrics, now: f32) {
    let p = ctx.layer_painter(LayerId::new(Order::Background, Id::new("inspect_markers")));

    // conspecifics: hollow amber rings (different shape/tint from the selected crosshair)
    for s in &m.conspecific_screen {
        let c = Pos2::new(s[0], s[1]);
        p.circle_stroke(c, 6.0, Stroke::new(1.0, DOT_RING)); // dark backing for contrast
        p.circle_stroke(c, 6.0, Stroke::new(1.0, CONSPEC));
    }

    let Some(s) = m.inspect_screen else { return };
    let c = Pos2::new(s[0], s[1]);
    // pulse over ~1.8 s
    let phase = (now * std::f32::consts::TAU / 1.8).sin() * 0.5 + 0.5; // 0..1
    // outer pulsing ring ⌀46
    let outer_r = 23.0 + phase * 2.5;
    let outer_a = (40.0 + 37.0 * (1.0 - phase)) as u8;
    p.circle_stroke(c, outer_r, Stroke::new(1.0, theme::straight(242, 166, 75, outer_a)));
    // inner ring ⌀30
    p.circle_stroke(c, 15.0, Stroke::new(1.5, RING_BRIGHT));
    // centre dot ⌀10 with dark outline + halo
    p.circle_filled(c, 6.5, DOT_HALO);
    p.circle_stroke(c, 5.0, Stroke::new(2.0, DOT_RING));
    p.circle_filled(c, 5.0, theme::ACCENT);

    // id-tag 25px below
    if let Some(view) = &m.inspect {
        let text = view.id.to_uppercase();
        let galley = ctx.fonts(|f| f.layout_no_wrap(text, theme::mono(9.0), theme::ACCENT_TEXT));
        let sz = galley.size();
        let centre = c + Vec2::new(0.0, 25.0);
        let bg = egui::Rect::from_center_size(centre, sz + Vec2::new(14.0, 4.0));
        p.rect_filled(bg, 6.0, TAG_BG);
        p.rect_stroke(bg, 6.0, Stroke::new(1.0, theme::ACCENT_LINE), StrokeKind::Inside);
        p.galley(bg.center() - sz / 2.0, galley, theme::ACCENT_TEXT);
    }
}

const CONTENT_W: f32 = 256.0 - 2.0 * 17.0; // panel content width (frame margin 17)

fn panel(ctx: &egui::Context, st: &mut UiState, view: &CreatureView) {
    egui::Area::new(Id::new("inspector"))
        .anchor(Align2::LEFT_TOP, egui::vec2(18.0, 92.0))
        .show(ctx, |ui| {
            theme::themed_frame(FrameKind::Inspector).show(ui, |ui| {
                ui.set_width(CONTENT_W);
                ui.set_max_width(CONTENT_W); // clamp so no row can grow the glass wider than spec
                // egui adds item_spacing.y between EVERY vertical item; zero it so the explicit
                // add_space() gaps below are the only vertical rhythm (matches the mockup's metrics —
                // otherwise ~6px×rows of extra height creeps in).
                ui.spacing_mut().item_spacing.y = 0.0;
                header(ui, st, view);
                hairline(ui);
                ui.add_space(8.0);
                vitals(ui, view);
                ui.add_space(5.0);
                genome(ui, view);
                hairline(ui);
                ui.add_space(7.0);
                body(ui, view);
                hairline(ui);
                ui.add_space(7.0);
                footer(ui, view);
            });
        });
}

fn header(ui: &mut egui::Ui, st: &mut UiState, view: &CreatureView) {
    // Close pinned right (right_to_left), text block fills the rest leftward — so the close button
    // can never push the panel wider than CONTENT_W.
    ui.horizontal(|ui| {
        ui.set_width(CONTENT_W);
        ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
            // close button (deselect)
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), Sense::click());
            let p = ui.painter();
            if resp.hovered() {
                p.rect_filled(rect, 7.0, theme::HOVER_FILL);
            }
            let col = if resp.hovered() { theme::TEXT } else { CLOSE_GLYPH };
            let h = 5.5; // half of the ~11px glyph (mockup renders a delicate cross, not a chunky one)
            let cc = rect.center();
            p.line_segment([cc + Vec2::new(-h, -h), cc + Vec2::new(h, h)], Stroke::new(1.5, col));
            p.line_segment([cc + Vec2::new(h, -h), cc + Vec2::new(-h, h)], Stroke::new(1.5, col));
            if resp.clicked() {
                st.selected = None;
            }
            // text block (left of the close)
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                // row 1: name + id (baseline)
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(RichText::new(&view.name).font(theme::sans(15.0)).strong().color(theme::TEXT));
                    ui.label(RichText::new(&view.id).font(theme::mono(11.0)).color(theme::ACCENT_TEXT));
                });
                ui.add_space(6.0);
                // row 2: trophic dot + kind, GEN, age
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let (dot, _) = ui.allocate_exact_size(egui::vec2(7.0, 12.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 3.5, view.kind.color());
                    ui.label(RichText::new(view.kind.label()).font(theme::sans(11.0)).color(theme::TEXT_DIM));
                    ui.label(
                        RichText::new(format!("GEN {}", view.generation))
                            .font(theme::mono(9.5))
                            .color(theme::TEXT_FAINT),
                    );
                    ui.label(RichText::new(&view.age).font(theme::mono(9.5)).color(theme::TEXT_FAINT));
                });
            });
        });
    });
    ui.add_space(7.0);
}

/// `label … value%` row above a coloured bar.
fn vital_row(ui: &mut egui::Ui, label: &str, frac: f32, col: Color32) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).font(theme::sans(11.0)).color(theme::TEXT_DIM));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(format!("{}%", (frac.clamp(0.0, 1.0) * 100.0).round() as i32))
                    .font(theme::mono(11.0))
                    .color(theme::TEXT),
            );
        });
    });
    ui.add_space(5.0);
    bar_sized(ui, frac, col, 4.0, 3.0);
}

fn vitals(ui: &mut egui::Ui, view: &CreatureView) {
    vital_row(ui, "Energy", view.energy, theme::GOOD_GREEN);
    ui.add_space(10.0);
    vital_row(ui, "Health", view.health, theme::VITAL_NEUTRAL);
    ui.add_space(10.0);
    vital_row(ui, "Hydration", view.hydration, theme::DATA_AUTO);
    ui.add_space(15.0);
}

fn genome(ui: &mut egui::Ui, view: &CreatureView) {
    caps_tracked(ui, "GENOME", 9.0, 0.14, theme::TEXT_FAINT);
    ui.add_space(11.0);
    // 3 rows of 2 (fixed order). `columns` splits the width evenly (gap = item_spacing) and gives
    // each cell its own full-height sub-ui, so the per-trait bar isn't clipped (an `allocate_ui`
    // with a fixed small height was eating the bars and growing the panel).
    for row in 0..3 {
        ui.spacing_mut().item_spacing.x = 14.0;
        ui.columns(2, |cols| {
            for (i, c) in cols.iter_mut().enumerate() {
                let (label, val) = view.traits[row * 2 + i];
                genome_cell(c, label, val);
            }
        });
        if row < 2 {
            ui.add_space(11.0);
        }
    }
    ui.add_space(15.0);
}

fn genome_cell(ui: &mut egui::Ui, label: &str, val: f32) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).font(theme::sans(10.0)).color(LABEL_55));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(format!("{:.2}", val)).font(theme::mono(9.5)).color(VALUE_80));
        });
    });
    ui.add_space(5.0);
    bar_sized(ui, val, theme::TRAIT_FILL, 3.0, 2.0);
}

fn body(ui: &mut egui::Ui, view: &CreatureView) {
    kv(ui, "diet", view.diet.clone());
    ui.add_space(9.0);
    kv(ui, "mass", view.mass.clone());
    ui.add_space(9.0);
    kv(ui, "locomotion", view.locomotion.clone());
    ui.add_space(9.0);
    kv(ui, "strata", view.strata.clone());
}

fn footer(ui: &mut egui::Ui, view: &CreatureView) {
    ui.horizontal(|ui| {
        // activity chip
        egui::Frame::NONE
            .fill(CHIP_BG)
            .stroke(Stroke::new(1.0, CHIP_STROKE))
            .corner_radius(egui::CornerRadius::same(7))
            .inner_margin(egui::Margin::symmetric(10, 4))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (dot, _) = ui.allocate_exact_size(egui::vec2(5.0, 10.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 2.5, theme::ACCENT);
                ui.label(RichText::new(&view.activity).font(theme::mono(10.0)).color(theme::TEXT));
            });
        // offspring N (right)
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new(view.offspring.to_string()).font(theme::mono(11.0)).color(theme::TEXT));
            ui.label(RichText::new("offspring").font(theme::sans(11.0)).color(theme::TEXT_DIM));
        });
    });
}
