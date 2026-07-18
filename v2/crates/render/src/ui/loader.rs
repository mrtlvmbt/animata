//! U-2: Loading screen modal (ported from v1).
//!
//! Full-screen egui modal showing world-generation progress:
//! - Dark scrim (blocks interaction with world beneath)
//! - Progress bar (0–1000 permille)
//! - Step checklist (✓ done / ○ active / – pending)
//! - Pulsing animation on active step
//! - Seed + progress percent meta row

use egui::{Align2, Color32, FontId, Pos2, Shape, Stroke};
use super::theme;
use crate::loader_state::LoadState;
use crate::world_spec::{Stage, Phase};

/// Pulse keyframe for active step (triangle wave, peak at half-period).
/// Returns (opacity, scale) ramping 0.55→1.0 and 1.0→1.25.
fn pulse(t: f64, period: f64) -> (f32, f32) {
    let p = ((t % period) / period) as f32; // 0..1
    let tri = 1.0 - (2.0 * p - 1.0).abs(); // 0→1→0
    (0.55 + 0.45 * tri, 1.0 + 0.25 * tri)
}

/// Paint text centred on `center_x` at top `y` with CSS-style letter-spacing.
fn paint_tracked(
    ui: &egui::Ui,
    center_x: f32,
    y: f32,
    text: &str,
    font: FontId,
    color: Color32,
    tracking: f32,
) {
    let widths: Vec<f32> = text
        .chars()
        .map(|c| ui.ctx().fonts(|f| f.glyph_width(&font, c)))
        .collect();
    let n = widths.len();
    let total: f32 = widths.iter().sum::<f32>() + tracking * n.saturating_sub(1) as f32;
    let mut x = center_x - total / 2.0;
    let painter = ui.painter();
    for (c, w) in text.chars().zip(&widths) {
        painter.text(Pos2::new(x, y), Align2::LEFT_TOP, c, font.clone(), color);
        x += w + tracking;
    }
}

/// Paint a done-step check mark in the 14×14 glyph box centred on `c`.
fn paint_check(painter: &egui::Painter, c: Pos2) {
    let o = Pos2::new(c.x - 6.0, c.y - 6.0);
    let pts = [
        Pos2::new(o.x + 2.5, o.y + 6.5),
        Pos2::new(o.x + 5.0, o.y + 9.0),
        Pos2::new(o.x + 9.5, o.y + 3.5),
    ];
    painter.add(Shape::line(
        pts.to_vec(),
        Stroke::new(2.0, theme::GOOD_GREEN),
    ));
}

const SCRIM: Color32 = theme::straight(7, 9, 8, 240);
const SCRIM_PASSES: usize = 5;
const TXT_50: Color32 = theme::straight(233, 236, 230, 128); // seed line
const TXT_55: Color32 = theme::straight(233, 236, 230, 140); // done-step label
const TXT_32: Color32 = theme::straight(233, 236, 230, 82); // pending-step label
const TXT_28: Color32 = theme::straight(233, 236, 230, 71); // pending dash
const TRACK: Color32 = theme::straight(255, 255, 255, 26); // progress track (white 0.10)

/// Draw the full-screen loader modal. Call only when in Loading phase.
pub fn draw(ctx: &egui::Context, load_state: &LoadState) {
    let progress = (load_state.get_progress() as f32) / 1000.0;
    let step_index = load_state.get_stage() as usize;

    let t = ctx.input(|i| i.time);
    let screen = ctx.screen_rect();
    let col_w = 384.0;

    // Total block height to vertically centre it.
    // U-12: 3 coarse phases (GenerateWorld, BuildMesh, Done)
    let n = 3.0;
    let block_h = 16.0 + 34.0   // brand row + margin
        + 10.0 + 13.0           // phase caps + margin
        + 24.0 + 26.0           // detail line + margin
        + 4.0 + 11.0            // progress track + margin
        + 11.0 + 32.0           // meta row + margin
        + n * 14.0 + (n - 1.0) * 12.0; // phase checklist rows + gaps

    egui::Area::new(egui::Id::new("loader"))
        .order(egui::Order::Foreground)
        .interactable(true)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            // Dark scrim
            for _ in 0..SCRIM_PASSES {
                ui.painter().rect_filled(screen, 0.0, SCRIM);
            }

            let cx = screen.center().x;
            let left = cx - col_w / 2.0;
            let mut y = screen.center().y - block_h / 2.0;

            // --- BRAND ROW: pulsing dot + "ANIMATA" ---
            let (op, sc) = pulse(t, 1.6);
            let brand = "ANIMATA";
            let brand_font = theme::mono(13.0);
            let tracking = 0.46 * 13.0;
            let widths: Vec<f32> = brand
                .chars()
                .map(|c| ui.ctx().fonts(|f| f.glyph_width(&brand_font, c)))
                .collect();
            let brand_w: f32 = widths.iter().sum::<f32>()
                + tracking * brand.chars().count().saturating_sub(1) as f32;
            let gap = 10.0;
            let group_w = 8.0 + gap + brand_w;
            let group_left = cx - group_w / 2.0;
            let dot_c = Pos2::new(group_left + 4.0, y + 8.0);
            ui.painter()
                .circle_filled(dot_c, 4.0 * sc, theme::ACCENT.gamma_multiply(op));

            // Brand text
            let mut tx = group_left + 8.0 + gap;
            {
                let painter = ui.painter();
                for (c, w) in brand.chars().zip(&widths) {
                    painter.text(
                        Pos2::new(tx, y + 1.0),
                        Align2::LEFT_TOP,
                        c,
                        brand_font.clone(),
                        theme::TEXT,
                    );
                    tx += w + tracking;
                }
            }
            y += 16.0 + 34.0;

            // --- PHASE CAPS ---
            paint_tracked(
                ui,
                cx,
                y,
                "GENERATING WORLD",
                theme::mono(10.0),
                theme::TEXT_FAINT,
                0.22 * 10.0,
            );
            y += 10.0 + 13.0;

            // --- DETAIL LINE: current stage label (mutating in place) ---
            let stage_label = if step_index < 14 {
                Stage::from_u8(step_index as u8)
                    .map(|s| s.label_ru())
                    .unwrap_or("Неизвестно")
            } else {
                "Готово"
            };
            ui.painter().text(
                Pos2::new(cx, y),
                Align2::CENTER_TOP,
                stage_label,
                theme::sans(20.0),
                theme::TEXT,
            );
            y += 24.0 + 26.0;

            // --- PROGRESS TRACK + FILL ---
            let track = egui::Rect::from_min_size(Pos2::new(left, y), egui::vec2(col_w, 4.0));
            ui.painter().rect_filled(track, 2.0, TRACK);
            let fill_w = (col_w * progress.clamp(0.0, 1.0)).max(0.0);
            let fill = egui::Rect::from_min_size(track.min, egui::vec2(fill_w, 4.0));
            ui.painter().rect_filled(fill, 2.0, theme::ACCENT);
            y += 4.0 + 11.0;

            // --- META ROW: seed (left) · percent (right) ---
            ui.painter().text(
                Pos2::new(left, y),
                Align2::LEFT_TOP,
                format!("seed 0x{:08X}", load_state.seed),
                theme::mono(11.0),
                TXT_50,
            );
            ui.painter().text(
                Pos2::new(left + col_w, y),
                Align2::RIGHT_TOP,
                format!("{}%", (progress * 100.0).round()),
                theme::mono(11.0),
                theme::ACCENT_TEXT,
            );
            y += 11.0 + 32.0;

            // --- PHASE CHECKLIST (coarse phases only) ---
            // U-12: Show 3 coarse phases (GenerateWorld, BuildMesh, Done).
            // Map current stage to its phase for monotone ticking.
            let label_font = theme::sans(13.0);
            let current_stage = Stage::from_u8(step_index as u8);
            let current_phase = current_stage.map(|s| s.phase()).or(
                // F3: If step_index >= 14 (out of bounds), treat as Phase::Done for coherent UI state
                if step_index >= 14 { Some(Phase::Done) } else { None }
            );

            // Display all 3 phases in order
            for phase in [Phase::GenerateWorld, Phase::BuildMesh, Phase::Done].iter() {
                let phase_label = phase.label_ru();
                let glyph_c = Pos2::new(left + 7.0, y + 7.0);

                // Determine phase state based on current stage's phase and execution position.
                // Phases tick monotonically: GenerateWorld → BuildMesh → Done.
                let label_color = match (current_phase, phase) {
                    // Current stage's phase is this phase or beyond it (in execution order)
                    (Some(curr_p), _) if curr_p >= *phase => {
                        if curr_p == *phase {
                            // This is the active phase
                            let (op2, sc2) = pulse(t, 1.4);
                            ui.painter().circle_filled(
                                glyph_c,
                                4.0 * sc2,
                                theme::ACCENT.gamma_multiply(op2),
                            );
                            theme::TEXT
                        } else {
                            // This phase is done (current phase is after this one)
                            paint_check(ui.painter(), glyph_c);
                            TXT_55
                        }
                    }
                    _ => {
                        // This phase hasn't started yet
                        let dash = egui::Rect::from_center_size(glyph_c, egui::vec2(8.0, 1.5));
                        ui.painter().rect_filled(dash, 1.0, TXT_28);
                        TXT_32
                    }
                };
                ui.painter().text(
                    Pos2::new(left + 26.0, y),
                    Align2::LEFT_TOP,
                    phase_label,
                    label_font.clone(),
                    label_color,
                );
                y += 14.0 + 12.0;
            }
        });

    ctx.request_repaint(); // pulse + progress animation
}
