//! HUD design tokens (colours + geometry) and small egui helpers — the "naturalist's dashboard"
//! look: dark glass panels, one warm amber accent, monospaced data. egui 0.31 uses INTEGER geometry
//! (`Margin`/`CornerRadius`/`Shadow` are i8/u8), so all values here are integers, not floats.

use egui::{Color32, CornerRadius, Frame, Margin, Shadow, Stroke};

// ---- colours (spec §1) ----
// Straight-alpha → premultiplied. egui stores Color32 PREMULTIPLIED; writing translucent tints as
// `from_rgba_premultiplied(fullRGB, lowAlpha)` is invalid (RGB ≫ alpha) and renders near-opaque
// (e.g. a "0.10 white" track came out solid white). `from_rgba_unmultiplied` isn't const, so:
const fn straight(r: u8, g: u8, b: u8, al: u8) -> Color32 {
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
pub const TEXT_DIM: Color32 = straight(233, 236, 230, 158); // secondary
pub const TEXT_FAINT: Color32 = straight(233, 236, 230, 115); // caps labels

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

// ---- frames ----
pub fn panel_frame() -> Frame {
    Frame {
        inner_margin: Margin::symmetric(16, 12),
        fill: PANEL_BG,
        stroke: Stroke::new(1.0, PANEL_STROKE),
        corner_radius: CornerRadius::same(13),
        shadow: Shadow {
            offset: [0, 10],
            blur: 34,
            spread: 0,
            color: Color32::from_black_alpha(102),
        },
        ..Default::default()
    }
}

pub fn flyout_frame() -> Frame {
    panel_frame().fill(PANEL_BG_STRONG)
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
