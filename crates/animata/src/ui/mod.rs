//! Structured in-app GUI (egui) — replaces the old raw `draw_text` HUD.
//!
//! Four logically-grouped panels: Performance, World & Time, View & Debug, Population &
//! Evolution. Simple bool toggles are mutated DIRECTLY through `&mut UiState` (an egui checkbox
//! writes its `&mut bool`); only non-trivial intents (pause needs a clock sync, time-scale needs
//! clamping, save/load) and `wants_pointer` flow back via [`UiActions`]. The same fields are
//! flipped by keyboard hotkeys in `main.rs`, so widget and hotkey share one source of truth.

use crate::DebugView;

pub mod hud;
pub mod loader;
pub mod minimap;
pub mod theme;

pub use hud::draw_hud;

/// Cache key for the minimap texture: (seed, view, biomass-tick-bucket) — rebuilt only on change.
pub type MinimapKey = (u64, DebugView, u64);

/// Persistent HUD GPU resources, owned by `main` and passed `&mut` into [`draw_hud`]. The egui
/// `TextureHandle` is held across frames (valid for the context's lifetime) and only re-uploaded
/// when its key changes.
#[derive(Default)]
pub struct HudCache {
    pub minimap: Option<(MinimapKey, egui::TextureHandle)>,
}

/// Which flyout (if any) is open from the control rail. One at a time.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    World,
    View,
    Pop,
    Perf,
}

/// Toggle state owned here, snapshotted into Copy locals by the render loop each frame.
#[derive(Clone, Copy)]
pub struct UiState {
    pub show_info: bool,
    pub debug_view: DebugView,
    pub water_on: bool,
    pub mask: bool,
    pub outline: bool,
    /// Open flyout from the control rail (rail is the only entry point to detail panels).
    pub open_panel: Option<Panel>,
}

/// Things a panel widget asked for that don't reduce to a plain `&mut bool` toggle.
#[derive(Default)]
pub struct UiActions {
    pub toggle_pause: bool,
    pub set_time_scale: Option<f32>,
    pub save: bool,
    pub load: bool,
    /// `ctx.wants_pointer_input()` — gate world mouse interactions (zoom/pan/graze) on `!this`.
    pub wants_pointer: bool,
}

/// Population/evolution block — `None` until the world is ready. Pre-computed in `main.rs` so this
/// module stays free of sim-getter knowledge (and the metrics reflect the latest `sim.step`).
pub struct LifeStats {
    pub population: u64,
    pub avg_energy: f32,
    pub avg_biomass: f32,
    pub multi: f32,
    pub carn: f32,
    pub auto: f32,
    pub species: u64,
    pub niches: u64,
    pub allop: f32,
    pub crypsis: f32,
    pub strata: [f32; 4],
}

/// A read-only snapshot of everything the panels display, built once per frame.
pub struct SimMetrics {
    // Performance (perf counters lag one frame — they're produced during render, after the UI
    // pass; invisible on an fps/draw readout, and it keeps `wants_pointer` fresh for input gating).
    pub fps: f32,
    pub frame_ms: f32,
    pub drawn: usize,
    pub detail: usize,
    pub coarse: usize,
    pub on_screen: usize,
    // World & time
    pub seed: u64,
    pub cols: usize,
    pub rows: usize,
    pub tick: u64,
    pub sim_time: f32,
    pub day_frac: f32,
    pub time_scale: f32,
    pub paused: bool,
    // Population & evolution
    pub life: Option<LifeStats>,
    /// Last ~48 population samples (oldest→newest) for the vitals sparkline. Buffered in `main.rs`
    /// on a tick cadence (freezes when paused).
    pub pop_hist: Vec<f32>,
    /// Visible-world quad on the map for the minimap viewport frame: four corners as map-space
    /// fractions `[0,1]` (x,z), in screen order. Empty until the world is ready.
    pub minimap_view: Vec<[f32; 2]>,
    /// Transient top-centre system notice (message, elapsed milliseconds since it fired) — e.g.
    /// "Saved". The HUD derives the entry slide + fade from the elapsed time. `None` = nothing.
    pub toast: Option<(String, f32)>,
}

// Mirror of main.rs time-scale tuning (kept local to avoid a cross-module const dependency).
pub(crate) const MIN_TIME_SCALE: f32 = 0.1;
pub(crate) const MAX_TIME_SCALE: f32 = 64.0;

/// One-line ramp description for the active field map (mirrors `build_field_minimap`). The arrow is
/// the Phosphor `ARROW_RIGHT` glyph, not U+2192 — the vendored IBM Plex subset lacks U+2192 and would
/// render it as a tofu box; Phosphor is in both font families so it falls back cleanly.
pub(crate) fn legend_text(view: DebugView) -> String {
    let a = egui_phosphor::regular::ARROW_RIGHT;
    match view {
        DebugView::Temp => format!("cold (blue) {a} hot (red)"),
        DebugView::Moist => format!("dry (tan) {a} wet (teal)"),
        DebugView::WaterDist => format!("near water (bright) {a} far (dark)"),
        DebugView::Slope => format!("flat (dark) {a} steep (yellow)"),
        DebugView::Biomass => format!("barren (brown) {a} lush (green) · right-drag = graze"),
        _ => String::new(),
    }
}

/// Horizontal gradient strip painted from the SAME ramp math as `build_field_minimap`
/// (water special-cases omitted — this is just the colour legend). `h` is the bar height (mockup:
/// 9px in a flyout, 7px under the minimap); width fills the available space.
pub(crate) fn legend_bar(ui: &mut egui::Ui, view: DebugView, h: f32) {
    use egui::CornerRadius;
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
    let painter = ui.painter();
    // Rounded (pill) ends: solid end-caps with the outer corners rounded, square gradient slices in
    // between (a per-slice corner_radius can't round the bar's outer corners). The cap must be ≥2·r
    // wide or egui clamps the corner radius to capwidth/2 and the rounding comes out half-size.
    let r = (h * 0.5).round();
    let ru = r as u8;
    let capw = 2.0 * r;
    let y0 = rect.top();
    let y1 = rect.bottom();
    let cap = |p: &egui::Painter, x: f32, color, cr| {
        p.rect_filled(egui::Rect::from_min_max(egui::pos2(x, y0), egui::pos2(x + capw, y1)), cr, color)
    };
    cap(painter, rect.left(), ramp_color(view, 0.0),
        CornerRadius { nw: ru, sw: ru, ne: 0, se: 0 });
    cap(painter, rect.right() - capw, ramp_color(view, 1.0),
        CornerRadius { ne: ru, se: ru, nw: 0, sw: 0 });
    let (mx0, mx1) = (rect.left() + capw, rect.right() - capw);
    let mw = (mx1 - mx0).max(0.0);
    let n = 48usize;
    for i in 0..n {
        let v = (capw + mw * (i as f32 + 0.5) / n as f32) / w; // sample at slice centre
        let x0 = mx0 + mw * i as f32 / n as f32;
        let x1 = mx0 + mw * (i + 1) as f32 / n as f32;
        painter.rect_filled(
            egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1)),
            0.0,
            ramp_color(view, v.clamp(0.0, 1.0)),
        );
    }
}

pub(crate) fn ramp_color(view: DebugView, v: f32) -> egui::Color32 {
    let (r, g, b) = match view {
        DebugView::Temp => (v, 0.15, 1.0 - v),
        DebugView::Moist => (0.65 * (1.0 - v) + 0.1, 0.35 + 0.45 * v, 0.25 + 0.5 * v),
        DebugView::WaterDist => {
            let s = 1.0 - 0.85 * v;
            (s, s, s)
        }
        DebugView::Slope => (v, v, 0.25 * v),
        DebugView::Biomass => (0.45 * (1.0 - v) + 0.1, 0.25 + 0.6 * v, 0.12),
        _ => (0.0, 0.0, 0.0),
    };
    egui::Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}
