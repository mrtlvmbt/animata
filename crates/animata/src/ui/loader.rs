//! Full-screen modal loading overlay (world generation / session load) — drawn LAST in the egui
//! pass, above the whole HUD, and swallows all input while alive. Pixel-spec layout: scrim → brand
//! row → phase caps → step name → progress track → seed/percent meta → step checklist, all in a
//! fixed 384-px centred column. egui has no real letter-spacing or blur, so tracking is emulated by
//! per-glyph advance ([`paint_tracked`]) and the CSS glow is dropped (shape + colour carry it).

use egui::{Align2, Color32, FontId, Pos2, Shape, Stroke};

use super::theme;

/// Which long-running job the overlay is reporting (drives the phase caps, steps and finish toast).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LoadKind {
    Gen,
    Load,
}

// ---- colours (spec §1). These sit on the near-opaque dark scrim, so straight-alpha
// (`from_rgba_unmultiplied`) reads correctly without premultiply — same convention as `toast()`.
// Spec wants rgba(7,9,8,0.94). egui_macroquad's blend renders a single translucent dark fill far
// weaker than its alpha (≈0.45 effective, and it barely changes near the top), so we can't reach the
// near-opaque scrim with one pass. Painting it [`SCRIM_PASSES`] times compounds the coverage
// (residual ≈ 0.55ⁿ) and converges to the intended dark tint while keeping a faint world hint.
const SCRIM: Color32 = theme::straight(7, 9, 8, 240);
const SCRIM_PASSES: usize = 5;
const TXT_50: Color32 = theme::straight(233, 236, 230, 128); // seed line
const TXT_55: Color32 = theme::straight(233, 236, 230, 140); // done-step label
const TXT_32: Color32 = theme::straight(233, 236, 230, 82); // pending-step label
const TXT_28: Color32 = theme::straight(233, 236, 230, 71); // pending dash
const TRACK: Color32 = theme::straight(255, 255, 255, 26); // progress track (white 0.10)

const GEN_STEPS: [&str; 5] = [
    "Heightfield",
    "Hydrology & rivers",
    "Climate bands",
    "Biome assignment",
    "Seeding initial life",
];
const LOAD_STEPS: [&str; 4] = [
    "Reading snapshot",
    "World state",
    "Population census",
    "Restoring camera",
];

fn steps(kind: LoadKind) -> &'static [&'static str] {
    match kind {
        LoadKind::Gen => &GEN_STEPS,
        LoadKind::Load => &LOAD_STEPS,
    }
}

/// `amPulse` keyframe (spec §5): triangle wave, peak at the half-period. Returns `(opacity, scale)`
/// ramping 0.55→1.0 and 1.0→1.25.
fn pulse(t: f64, period: f64) -> (f32, f32) {
    let p = ((t % period) / period) as f32; // 0..1
    let tri = 1.0 - (2.0 * p - 1.0).abs(); // 0→1→0
    (0.55 + 0.45 * tri, 1.0 + 0.25 * tri)
}

/// Paint `text` centred on `center_x` at top `y` with CSS-style letter-spacing (`tracking` px between
/// glyphs). egui has no native tracking, so we lay out glyph-by-glyph using each glyph's advance.
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
    for (c, w) in text.chars().zip(widths) {
        painter.text(Pos2::new(x, y), Align2::LEFT_TOP, c, font.clone(), color);
        x += w + tracking;
    }
}

/// A done-step check mark in the 14×14 glyph box centred on `c` (spec §3 local points scaled).
fn paint_check(painter: &egui::Painter, c: Pos2) {
    // Local glyph coords (box ~12×12 around the centre): (2.5,6.5)→(5.0,9.0)→(9.5,3.5).
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

/// Draw the full-screen loader. `progress` is 0..=1, `step_index` the active step. Renders into a
/// foreground, interactable Area so the world/HUD beneath receive no pointer events.
pub fn draw(ctx: &egui::Context, kind: LoadKind, progress: f32, step_index: usize, seed: u64) {
    let steps = steps(kind);
    let t = ctx.input(|i| i.time);
    let screen = ctx.screen_rect();
    let col_w = 384.0;

    // Total block height (heights + bottom margins per spec §2) to vertically centre it.
    let n = steps.len() as f32;
    let block_h = 16.0 + 34.0   // brand row + margin
        + 10.0 + 13.0           // phase caps + margin
        + 24.0 + 26.0           // step name + margin
        + 4.0 + 11.0            // progress track + margin
        + 11.0 + 32.0           // meta row + margin
        + n * 14.0 + (n - 1.0) * 12.0; // checklist rows + gaps

    egui::Area::new(egui::Id::new("loader"))
        .order(egui::Order::Foreground)
        .interactable(true) // swallows pointer → background gets no events
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            for _ in 0..SCRIM_PASSES {
                ui.painter().rect_filled(screen, 0.0, SCRIM);
            }

            let cx = screen.center().x;
            let left = cx - col_w / 2.0;
            let mut y = screen.center().y - block_h / 2.0;

            // --- BRAND ROW: pulsing dot + "ANIMATA" (Mono 13, tracking 0.46em) ---
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
            let group_w = 8.0 + gap + brand_w; // dot + gap + text
            let group_left = cx - group_w / 2.0;
            let dot_c = Pos2::new(group_left + 4.0, y + 8.0);
            ui.painter()
                .circle_filled(dot_c, 4.0 * sc, theme::ACCENT.gamma_multiply(op));
            // Brand text, tracked, left-anchored after the dot.
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
            let phase = match kind {
                LoadKind::Gen => "GENERATING WORLD",
                LoadKind::Load => "LOADING SESSION",
            };
            paint_tracked(
                ui,
                cx,
                y,
                phase,
                theme::mono(10.0),
                theme::TEXT_FAINT,
                0.22 * 10.0,
            );
            y += 10.0 + 13.0;

            // --- STEP NAME ---
            ui.painter().text(
                Pos2::new(cx, y),
                Align2::CENTER_TOP,
                steps[step_index],
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
                format!("seed 0x{seed:08X}"),
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

            // --- STEP CHECKLIST ---
            let label_font = theme::sans(13.0);
            for (i, label) in steps.iter().enumerate() {
                let glyph_c = Pos2::new(left + 7.0, y + 7.0);
                let label_color = if i < step_index {
                    paint_check(ui.painter(), glyph_c);
                    TXT_55
                } else if i == step_index {
                    let (op2, sc2) = pulse(t, 1.4);
                    ui.painter().circle_filled(
                        glyph_c,
                        4.0 * sc2,
                        theme::ACCENT.gamma_multiply(op2),
                    );
                    theme::TEXT
                } else {
                    let dash = egui::Rect::from_center_size(glyph_c, egui::vec2(8.0, 1.5));
                    ui.painter().rect_filled(dash, 1.0, TXT_28);
                    TXT_32
                };
                ui.painter().text(
                    Pos2::new(left + 26.0, y),
                    Align2::LEFT_TOP,
                    label,
                    label_font.clone(),
                    label_color,
                );
                y += 14.0 + 12.0;
            }
        });

    ctx.request_repaint(); // pulse + progress animation
}
