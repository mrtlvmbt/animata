//! Structured in-app GUI (egui) — replaces the old raw `draw_text` HUD.
//!
//! Four logically-grouped panels: Performance, World & Time, View & Debug, Population &
//! Evolution. Simple bool toggles are mutated DIRECTLY through `&mut UiState` (an egui checkbox
//! writes its `&mut bool`); only non-trivial intents (pause needs a clock sync, time-scale needs
//! clamping, save/load) and `wants_pointer` flow back via [`UiActions`]. The same fields are
//! flipped by keyboard hotkeys in `main.rs`, so widget and hotkey share one source of truth.

use crate::DebugView;

/// Toggle state owned here, snapshotted into Copy locals by the render loop each frame.
#[derive(Clone, Copy)]
pub struct UiState {
    pub show_info: bool,
    pub debug_view: DebugView,
    pub water_on: bool,
    pub mask: bool,
    pub outline: bool,
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
    pub nutri: f32,
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
    /// Transient bottom-right notice (message, alpha 0..1) — e.g. "saved". `None` = nothing.
    pub toast: Option<(String, f32)>,
}

// Mirror of main.rs time-scale tuning (kept local to avoid a cross-module const dependency).
const MIN_TIME_SCALE: f32 = 0.1;
const MAX_TIME_SCALE: f32 = 64.0;
const TIME_SCALE_STEP: f32 = 1.5;

/// Render all panels. Mutates `st` in place for simple toggles; returns the non-trivial intents.
pub fn draw_ui(ctx: &egui::Context, st: &mut UiState, m: &SimMetrics) -> UiActions {
    let mut act = UiActions::default();

    // Transient bottom-right notice (e.g. "saved"). Non-interactable, fades via its alpha; shown
    // even when the panels are hidden (`I`).
    if let Some((msg, alpha)) = &m.toast {
        let a = (alpha.clamp(0.0, 1.0) * 255.0) as u8;
        egui::Area::new(egui::Id::new("toast"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-14.0, -14.0))
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(msg)
                                .color(egui::Color32::from_rgba_unmultiplied(180, 230, 180, a)),
                        )
                        // Anchored at the right edge the available width is ~0, so egui would wrap
                        // "Loaded" onto two lines; Extend keeps it one line and grows leftward.
                        .wrap_mode(egui::TextWrapMode::Extend),
                    );
                });
            });
    }

    // `I` hides the whole GUI; nothing else to draw, no panel to capture the pointer.
    if !st.show_info {
        act.wants_pointer = ctx.wants_pointer_input();
        return act;
    }

    let sr = ctx.screen_rect();

    egui::Window::new("Performance")
        .default_pos(egui::pos2(8.0, 8.0))
        .resizable(false)
        .show(ctx, |ui| {
            ui.label(format!("{:.0} fps   {:.2} ms", m.fps, m.frame_ms));
            ui.label(format!("draws {}", m.drawn));
            ui.label(format!("chunks  detail {}  coarse {}", m.detail, m.coarse));
            ui.label(format!("on-screen {}", m.on_screen));
        });

    egui::Window::new("World & Time")
        .default_pos(egui::pos2(8.0, 132.0))
        .resizable(false)
        .show(ctx, |ui| {
            ui.label(format!("seed {}", m.seed));
            ui.label(format!("{}×{} m", m.cols, m.rows));
            ui.separator();
            ui.label(format!("tick {}   sim {:.1}s", m.tick, m.sim_time));
            ui.label(format!("day {:.2}", m.day_frac));
            ui.horizontal(|ui| {
                let label = if m.paused { "▶ Resume (P)" } else { "⏸ Pause (P)" };
                if ui.button(label).clicked() {
                    act.toggle_pause = true;
                }
                if m.paused {
                    ui.colored_label(egui::Color32::from_rgb(255, 180, 80), "PAUSED");
                }
            });
            ui.horizontal(|ui| {
                ui.label("speed");
                if ui.small_button("[").clicked() {
                    act.set_time_scale =
                        Some((m.time_scale / TIME_SCALE_STEP).max(MIN_TIME_SCALE));
                }
                let mut ts = m.time_scale;
                if ui
                    .add(
                        egui::Slider::new(&mut ts, MIN_TIME_SCALE..=MAX_TIME_SCALE)
                            .logarithmic(true)
                            .suffix("×"),
                    )
                    .changed()
                {
                    act.set_time_scale = Some(ts);
                }
                if ui.small_button("]").clicked() {
                    act.set_time_scale =
                        Some((m.time_scale * TIME_SCALE_STEP).min(MAX_TIME_SCALE));
                }
            });
        });

    egui::Window::new("View & Debug")
        .default_pos(egui::pos2(sr.right() - 248.0, 8.0))
        .resizable(false)
        .show(ctx, |ui| {
            ui.label("Debug view (G cycles)");
            for (v, name) in [
                (DebugView::None, "None"),
                (DebugView::Topo, "Topo (height)"),
                (DebugView::Temp, "Temperature"),
                (DebugView::Moist, "Moisture"),
                (DebugView::WaterDist, "Water dist"),
                (DebugView::Slope, "Slope"),
                (DebugView::Biomass, "Biomass"),
            ] {
                ui.radio_value(&mut st.debug_view, v, name);
            }
            ui.separator();
            ui.checkbox(&mut st.outline, "Step-edge outline (O)");
            ui.checkbox(&mut st.mask, "Water/land mask (J)");
            ui.checkbox(&mut st.water_on, "Water surface (H)");
            if st.debug_view.is_field_map() {
                ui.separator();
                ui.label(legend_text(st.debug_view));
                legend_bar(ui, st.debug_view);
                ui.label("(minimap top-right)");
            }
        });

    egui::Window::new("Population & Evolution")
        .default_pos(egui::pos2(8.0, sr.bottom() - 232.0))
        .resizable(false)
        .show(ctx, |ui| match &m.life {
            None => {
                ui.label("world generating…");
            }
            Some(l) => {
                ui.label(format!(
                    "pop {}   E {:.0}   bm {:.2}",
                    l.population, l.avg_energy, l.avg_biomass
                ));
                egui::CollapsingHeader::new("Complexity")
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.label(format!(
                            "multi {:.0}%   carn {:.0}%   auto {:.0}%",
                            l.multi * 100.0,
                            l.carn * 100.0,
                            l.auto * 100.0
                        ));
                    });
                egui::CollapsingHeader::new("Diversity")
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.label(format!("species {}   niches {}", l.species, l.niches));
                        ui.label(format!(
                            "allop {:.2}   crypsis {:.2}   nutri {:.2}",
                            l.allop, l.crypsis, l.nutri
                        ));
                    });
                egui::CollapsingHeader::new("Ecology")
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.label(format!(
                            "strata  u{:.0}/s{:.0}/a{:.0}/w{:.0}",
                            l.strata[0] * 100.0,
                            l.strata[1] * 100.0,
                            l.strata[2] * 100.0,
                            l.strata[3] * 100.0
                        ));
                    });
            }
        });

    act.wants_pointer = ctx.wants_pointer_input();
    act
}

/// One-line ramp description for the active field map (mirrors `build_field_minimap`).
fn legend_text(view: DebugView) -> &'static str {
    match view {
        DebugView::Temp => "cold (blue) → hot (red)",
        DebugView::Moist => "dry (tan) → wet (teal)",
        DebugView::WaterDist => "near water (bright) → far (dark)",
        DebugView::Slope => "flat (dark) → steep (yellow)",
        DebugView::Biomass => "barren (brown) → lush (green) · right-drag = graze",
        _ => "",
    }
}

/// Horizontal gradient strip painted from the SAME ramp math as `build_field_minimap`
/// (water special-cases omitted — this is just the colour legend).
fn legend_bar(ui: &mut egui::Ui, view: DebugView) {
    let (w, h) = (180.0, 12.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
    let painter = ui.painter();
    let n = 48usize;
    for i in 0..n {
        let v = i as f32 / (n - 1) as f32;
        let x0 = rect.left() + w * i as f32 / n as f32;
        let x1 = rect.left() + w * (i + 1) as f32 / n as f32;
        painter.rect_filled(
            egui::Rect::from_min_max(egui::pos2(x0, rect.top()), egui::pos2(x1, rect.bottom())),
            0.0,
            ramp_color(view, v),
        );
    }
}

fn ramp_color(view: DebugView, v: f32) -> egui::Color32 {
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
