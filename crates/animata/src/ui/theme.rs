//! HUD design tokens (colours + geometry) and small egui helpers — the "naturalist's dashboard"
//! look: dark glass panels, one warm amber accent, monospaced data. egui 0.31 uses INTEGER geometry
//! (`Margin`/`CornerRadius`/`Shadow` are i8/u8), so all values here are integers, not floats.

use egui::{Color32, CornerRadius, Frame, Margin, Shadow, Stroke};

// ---- colours (spec §1) ----
// Straight-alpha → premultiplied. egui stores Color32 PREMULTIPLIED; writing translucent tints as
// `from_rgba_premultiplied(fullRGB, lowAlpha)` is invalid (RGB ≫ alpha) and renders near-opaque
// (e.g. a "0.10 white" track came out solid white). `from_rgba_unmultiplied` isn't const, so:
pub(crate) const fn straight(r: u8, g: u8, b: u8, al: u8) -> Color32 {
    let a = al as u32;
    let pr = ((r as u32 * a + 127) / 255) as u8;
    let pg = ((g as u32 * a + 127) / 255) as u8;
    let pb = ((b as u32 * a + 127) / 255) as u8;
    Color32::from_rgba_premultiplied(pr, pg, pb, al)
}

pub const PANEL_BG: Color32 = straight(12, 15, 14, 184); // ~0.72 glass
pub const PANEL_BG_STRONG: Color32 = straight(12, 15, 14, 189); // flyouts
pub const PANEL_STROKE: Color32 = straight(255, 255, 255, 26); // 0.10 edge
pub const HAIRLINE: Color32 = straight(255, 255, 255, 26); // dividers

pub const TEXT: Color32 = Color32::from_rgb(233, 236, 230); // primary
pub const TEXT_DIM: Color32 = straight(233, 236, 230, 158); // secondary (.62)
pub const TEXT_LABEL: Color32 = straight(233, 236, 230, 140); // flyout kv labels / done-step (.55)
pub const TEXT_FAINT: Color32 = straight(233, 236, 230, 115); // caps labels (.45)

pub const ACCENT: Color32 = Color32::from_rgb(242, 166, 75); // amber — active states
pub const ACCENT_TEXT: Color32 = Color32::from_rgb(244, 184, 106); // amber text on dark
pub const ACCENT_FILL: Color32 = straight(242, 166, 75, 41); // 0.16 backing
pub const ACCENT_LINE: Color32 = straight(242, 166, 75, 128); // 0.50 frame

pub const GOOD_GREEN: Color32 = Color32::from_rgb(143, 209, 111); // population sparkline
pub const HOVER_FILL: Color32 = straight(255, 255, 255, 18); // 0.07 hover
pub const TOAST_GREEN: Color32 = Color32::from_rgb(166, 224, 140);

// data accents (charts only — never a second UI accent)
pub const DATA_CARN: Color32 = Color32::from_rgb(217, 122, 90); // #D97A5A terracotta
pub const DATA_AUTO: Color32 = Color32::from_rgb(95, 176, 201); // #5FB0C9 blue

// strata stack segments (under / surface / air / water) — spec §4.4
pub const STRATA_UNDER: Color32 = Color32::from_rgb(125, 106, 79); // #7D6A4F
pub const STRATA_SURF: Color32 = Color32::from_rgb(143, 176, 90); // #8FB05A
pub const STRATA_AIR: Color32 = Color32::from_rgb(95, 174, 122); // #5FAE7A
pub const STRATA_WATER: Color32 = Color32::from_rgb(95, 147, 201); // #5F93C9

// ---- type helpers ----
pub fn mono(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Monospace)
}
pub fn sans(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Proportional)
}

/// CSS `em` letter-spacing → pixels for a given font size (e.g. `.16em` at 9px = 1.44px).
pub fn tracking_em(size: f32, em: f32) -> f32 {
    size * em
}

/// Paint `text` with CSS-style letter-spacing (egui has no native tracking). Glyphs are laid out
/// one-by-one using each glyph's advance + `tracking` px; `align` positions the whole run around
/// `pos` (horizontal: Min/Center/Max; vertical handed to each glyph). Shared by the HUD caps and the
/// loader. `total_tracked_width` returns the run width so callers can pre-allocate layout space.
pub fn total_tracked_width(ui: &egui::Ui, text: &str, font: &egui::FontId, tracking: f32) -> f32 {
    let n = text.chars().count();
    let sum: f32 = text
        .chars()
        .map(|c| ui.ctx().fonts(|f| f.glyph_width(font, c)))
        .sum();
    sum + tracking * n.saturating_sub(1) as f32
}

pub fn paint_tracked(
    ui: &egui::Ui,
    pos: egui::Pos2,
    align: egui::Align2,
    text: &str,
    font: egui::FontId,
    color: Color32,
    tracking: f32,
) {
    let widths: Vec<f32> = text
        .chars()
        .map(|c| ui.ctx().fonts(|f| f.glyph_width(&font, c)))
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + tracking * widths.len().saturating_sub(1) as f32;
    let start_x = match align.x() {
        egui::Align::Min => pos.x,
        egui::Align::Center => pos.x - total / 2.0,
        egui::Align::Max => pos.x - total,
    };
    let glyph_align = egui::Align2([egui::Align::Min, align.y()]);
    let painter = ui.painter();
    let mut x = start_x;
    for (c, w) in text.chars().zip(widths) {
        painter.text(egui::pos2(x, pos.y), glyph_align, c, font.clone(), color);
        x += w + tracking;
    }
}

// ---- frames ----
/// Which floating panel a [`themed_frame`] dresses. One source of truth for the per-panel
/// padding / radius / fill / shadow (mockup §2): glass panels differ only by these.
#[derive(Clone, Copy)]
pub enum FrameKind {
    Vitals,    // top-left + (with margin override) minimap: r13, pad 10×16, glass .72
    Rail,      // control rail: r14, pad 7
    Transport, // bottom-left transport: r14, pad 9×14, stronger glass .74
    Flyout,    // detail flyouts: r14, pad 16×18, stronger glass .74
}

pub fn themed_frame(kind: FrameKind) -> Frame {
    // (inner_margin, radius, fill, shadow offset.y, shadow blur, shadow alpha)
    let (margin, radius, fill, off, blur, sa) = match kind {
        FrameKind::Vitals => (
            Margin { left: 16, right: 16, top: 10, bottom: 10 },
            13,
            PANEL_BG,
            10,
            34,
            102, // .40
        ),
        FrameKind::Rail => (Margin::same(7), 14, PANEL_BG, 10, 34, 102),
        FrameKind::Transport => (
            Margin { left: 14, right: 14, top: 9, bottom: 9 },
            14,
            PANEL_BG_STRONG,
            12,
            38,
            115, // .45
        ),
        FrameKind::Flyout => (
            Margin { left: 18, right: 18, top: 16, bottom: 16 },
            14,
            PANEL_BG_STRONG,
            14,
            40,
            115,
        ),
    };
    Frame {
        inner_margin: margin,
        fill,
        stroke: Stroke::new(1.0, PANEL_STROKE),
        corner_radius: CornerRadius::same(radius),
        shadow: Shadow {
            offset: [0, off],
            blur,
            spread: 0,
            color: Color32::from_black_alpha(sa),
        },
        ..Default::default()
    }
}

/// Register IBM Plex Sans (proportional) + Mono + the Phosphor icon glyphs. Call ONCE; egui keeps
/// the `FontDefinitions` for the lifetime of the context (`set_pixels_per_point` only re-rasterises,
/// it doesn't drop the data).
pub fn install_fonts(ctx: &egui::Context, sans_ttf: &'static [u8], mono_ttf: &'static [u8]) {
    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert("plex_sans".into(), egui::FontData::from_static(sans_ttf).into());
    fonts
        .font_data
        .insert("plex_mono".into(), egui::FontData::from_static(mono_ttf).into());
    fonts
        .families
        .get_mut(&egui::FontFamily::Proportional)
        .unwrap()
        .insert(0, "plex_sans".into());
    fonts
        .families
        .get_mut(&egui::FontFamily::Monospace)
        .unwrap()
        .insert(0, "plex_mono".into());
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    // add_to_fonts only wires Phosphor into the Proportional family; our icons render in the
    // Monospace family, so add it there too (else the glyphs come out as tofu boxes).
    if let Some(keys) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
        keys.push("phosphor".into());
    }
    ctx.set_fonts(fonts);
}

/// Global style: transparent widget backgrounds, amber selection, no default widget frames — so our
/// hand-styled panels/areas read as "glass over the world".
pub fn install_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let v = &mut style.visuals;
    v.window_fill = PANEL_BG;
    v.panel_fill = Color32::TRANSPARENT;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT_FILL;
    v.selection.stroke = Stroke::new(1.0, ACCENT_LINE);
    // Slider/handle + checkbox accent; flat widget backgrounds.
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.bg_fill = Color32::TRANSPARENT;
        w.weak_bg_fill = Color32::TRANSPARENT;
    }
    v.widgets.hovered.weak_bg_fill = HOVER_FILL;
    ctx.set_style(style);
}
