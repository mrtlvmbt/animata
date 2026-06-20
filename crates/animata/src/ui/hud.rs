//! The HUD itself — nine floating `egui::Area`s composing the "naturalist's dashboard": vitals
//! (top-left), transport (bottom-left), control rail + one flyout (bottom-right), toast / hide-hint.
//! By default only vitals + transport + rail show; detail panels open from the rail, one at a time.
//! Minimap (top-right) is added in a later layer.

use egui::{Align, Align2, Layout, RichText, Sense, Stroke, StrokeKind, Vec2};
use egui_phosphor::regular as ph;

use animata_sim::terrain::VoxelTerrain;

use super::theme;
use super::{legend_bar, legend_text, minimap, MAX_TIME_SCALE, MIN_TIME_SCALE};
use super::{HudCache, Panel, SimMetrics, UiActions, UiState};
use crate::DebugView;

/// Build the whole HUD. Simple toggles mutate `st` directly; non-trivial intents + the pointer-gate
/// flow back via [`UiActions`].
pub fn draw_hud(
    ctx: &egui::Context,
    st: &mut UiState,
    m: &SimMetrics,
    cache: &mut HudCache,
    terrain: Option<&VoxelTerrain>,
) -> UiActions {
    let mut act = UiActions::default();

    toast(ctx, m);

    if !st.show_info {
        hide_hint(ctx);
        // Hint is non-interactable → it doesn't capture; world stays clickable while UI is hidden.
        act.wants_pointer = ctx.is_pointer_over_area();
        return act;
    }

    vitals(ctx, m);
    transport(ctx, m, &mut act);
    minimap_panel(ctx, st, m, cache, terrain);
    rail(ctx, st);
    flyout(ctx, st, m, &mut act);

    // F4: gate world mouse on "pointer over ANY interactive Area" — covers empty panel backgrounds
    // too (a click on the glass shouldn't reach the world), while non-interactable toast/hint pass.
    act.wants_pointer = ctx.is_pointer_over_area();
    act
}

// ---------- small shared widgets ----------

fn caps(text: &str) -> RichText {
    RichText::new(text.to_uppercase())
        .font(theme::mono(9.0))
        .color(theme::TEXT_FAINT)
}

/// `label … value` row (label sans dim left, value mono right).
fn kv(ui: &mut egui::Ui, label: &str, value: String) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(label)
                .font(theme::sans(11.5))
                .color(theme::TEXT_DIM),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(value).font(theme::mono(12.0)).color(theme::TEXT));
        });
    });
}

fn hairline(ui: &mut egui::Ui) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 1.0), Sense::hover());
    ui.painter()
        .hline(rect.left()..=rect.right(), rect.center().y, Stroke::new(1.0, theme::HAIRLINE));
}

/// Thin progress bar: track + metric-coloured fill.
fn bar(ui: &mut egui::Ui, frac: f32, col: egui::Color32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 4.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 3.0, theme::PANEL_STROKE);
    let mut fill = rect;
    fill.set_width(rect.width() * frac.clamp(0.0, 1.0));
    p.rect_filled(fill, 3.0, col);
}

/// Secondary (outline) button — white-ish frame, faint fill, hover lift.
fn secondary_button(ui: &mut egui::Ui, text: &str) -> bool {
    let resp = ui.add(
        egui::Button::new(RichText::new(text).font(theme::mono(11.0)).color(theme::TEXT))
            .fill(egui::Color32::from_white_alpha(8))
            .stroke(Stroke::new(1.0, egui::Color32::from_white_alpha(36))),
    );
    resp.clicked()
}

// ---------- toast / hide-hint ----------

fn toast(ctx: &egui::Context, m: &SimMetrics) {
    let Some((msg, alpha)) = &m.toast else { return };
    let a = (alpha.clamp(0.0, 1.0) * 255.0) as u8;
    egui::Area::new(egui::Id::new("toast"))
        .anchor(Align2::RIGHT_BOTTOM, egui::vec2(-18.0, -76.0))
        .interactable(false)
        .show(ctx, |ui| {
            theme::panel_frame()
                .stroke(Stroke::new(1.0, theme::TOAST_GREEN.gamma_multiply(0.30)))
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(msg)
                                .font(theme::mono(11.0))
                                .color(theme::TOAST_GREEN.gamma_multiply(alpha.clamp(0.0, 1.0))),
                        )
                        .wrap_mode(egui::TextWrapMode::Extend),
                    );
                    let _ = a;
                });
        });
}

fn hide_hint(ctx: &egui::Context) {
    egui::Area::new(egui::Id::new("hide_hint"))
        .anchor(Align2::LEFT_BOTTOM, egui::vec2(18.0, -18.0))
        .interactable(false)
        .show(ctx, |ui| {
            theme::panel_frame().show(ui, |ui| {
                ui.label(
                    RichText::new(format!("press  {}  for UI", ph::EYE))
                        .font(theme::mono(11.0))
                        .color(theme::TEXT_DIM),
                );
            });
        });
}

// ---------- vitals (top-left) ----------

fn vitals(ctx: &egui::Context, m: &SimMetrics) {
    egui::Area::new(egui::Id::new("vitals"))
        .anchor(Align2::LEFT_TOP, egui::vec2(18.0, 18.0))
        .show(ctx, |ui| {
            theme::panel_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;

                    // DAY + HH:MM
                    let day = (m.sim_time / 600.0).floor() as u64 + 1;
                    let hh = (m.day_frac * 24.0).floor() as u32;
                    let mm = (m.day_frac * 24.0 * 60.0) as u32 % 60;
                    ui.vertical(|ui| {
                        ui.label(caps("Day"));
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{day}")).font(theme::mono(15.0)).color(theme::TEXT));
                            ui.label(
                                RichText::new(format!("{hh:02}:{mm:02}"))
                                    .font(theme::mono(12.0))
                                    .color(theme::TEXT_DIM),
                            );
                        });
                    });

                    vitals_hairline(ui);
                    sun_dial(ui, m.day_frac);
                    vitals_hairline(ui);

                    // POPULATION + sparkline
                    let pop = m.life.as_ref().map(|l| l.population).unwrap_or(0);
                    ui.vertical(|ui| {
                        ui.label(caps("Population"));
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("{pop}"))
                                    .font(theme::mono(15.0))
                                    .color(theme::TEXT),
                            );
                            sparkline(ui, &m.pop_hist, egui::vec2(120.0, 22.0), theme::GOOD_GREEN);
                        });
                    });
                });
            });
        });
}

fn vitals_hairline(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 26.0), Sense::hover());
    ui.painter()
        .vline(rect.center().x, rect.top()..=rect.bottom(), Stroke::new(1.0, theme::PANEL_STROKE));
}

fn sun_dial(ui: &mut egui::Ui, frac: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), Sense::hover());
    let c = rect.center();
    let r = 10.0;
    let p = ui.painter();
    p.circle_stroke(c, r, Stroke::new(1.0, egui::Color32::from_white_alpha(46)));
    // angle: 0 at top, clockwise
    let ang = frac * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
    let tip = c + Vec2::new(ang.cos(), ang.sin()) * r;
    p.line_segment([c, tip], Stroke::new(1.5, theme::ACCENT));
}

fn sparkline(ui: &mut egui::Ui, data: &[f32], size: Vec2, col: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    if data.len() < 2 {
        return;
    }
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for &v in data {
        lo = lo.min(v);
        hi = hi.max(v);
    }
    let span = (hi - lo).max(1.0);
    let pts: Vec<egui::Pos2> = data
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = rect.left() + rect.width() * i as f32 / (data.len() - 1) as f32;
            let y = rect.bottom() - rect.height() * (v - lo) / span;
            egui::pos2(x, y)
        })
        .collect();
    ui.painter().add(egui::Shape::line(pts, Stroke::new(1.5, col)));
}

// ---------- transport (bottom-left) ----------

fn transport(ctx: &egui::Context, m: &SimMetrics, act: &mut UiActions) {
    egui::Area::new(egui::Id::new("transport"))
        .anchor(Align2::LEFT_BOTTOM, egui::vec2(18.0, -22.0))
        .show(ctx, |ui| {
            theme::panel_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;

                    // Pause/Play — accent button (custom painted for crisp contrast).
                    let glyph = if m.paused { ph::PLAY } else { ph::PAUSE };
                    if play_button(ui, glyph).clicked() {
                        act.toggle_pause = true;
                    }

                    vitals_hairline(ui);

                    // Log speed slider — thin track + amber handle (no +/- buttons, spec).
                    if let Some(v) = speed_slider(ui, m.time_scale) {
                        act.set_time_scale = Some(v);
                    }
                    let val = if m.time_scale < 10.0 {
                        format!("{:.1}×", m.time_scale)
                    } else {
                        format!("{}×", m.time_scale.round() as i32)
                    };
                    ui.label(RichText::new(val).font(theme::mono(14.0)).color(theme::ACCENT_TEXT));

                    if m.paused {
                        badge(ui, "PAUSED");
                    }
                });
            });
        });
}

/// Accent pause/play button: amber-filled rounded square, bright glyph (high contrast).
fn play_button(ui: &mut egui::Ui, glyph: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(34.0, 28.0), Sense::click());
    let p = ui.painter();
    // Solid amber fill + dark glyph = a crisp, unmistakable play/pause (amber-on-amber was invisible).
    let fill = if resp.hovered() {
        theme::ACCENT_TEXT
    } else {
        theme::ACCENT
    };
    p.rect_filled(rect, 8.0, fill);
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        theme::mono(16.0),
        egui::Color32::from_rgb(10, 12, 11),
    );
    resp
}

/// Custom logarithmic speed slider (0.1×–64×): thin track, filled amber lead, round amber handle.
/// Returns the new scale while being dragged/clicked.
fn speed_slider(ui: &mut egui::Ui, current: f32) -> Option<f32> {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(132.0, 24.0), Sense::click_and_drag());
    let cy = rect.center().y;
    let (lmin, lmax) = (MIN_TIME_SCALE.ln(), MAX_TIME_SCALE.ln());
    let t = ((current.ln() - lmin) / (lmax - lmin)).clamp(0.0, 1.0);
    let hx = rect.left() + t * rect.width();
    let p = ui.painter();
    p.line_segment(
        [egui::pos2(rect.left(), cy), egui::pos2(rect.right(), cy)],
        Stroke::new(3.0, egui::Color32::from_white_alpha(40)),
    );
    p.line_segment(
        [egui::pos2(rect.left(), cy), egui::pos2(hx, cy)],
        Stroke::new(3.0, theme::ACCENT_LINE),
    );
    p.circle_filled(egui::pos2(hx, cy), 6.0, theme::ACCENT);
    if resp.dragged() || resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let nt = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            return Some((lmin + nt * (lmax - lmin)).exp());
        }
    }
    None
}

fn badge(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new()
        .fill(theme::ACCENT_FILL)
        .stroke(Stroke::new(1.0, theme::ACCENT_LINE))
        .corner_radius(7)
        .inner_margin(egui::Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).font(theme::mono(9.5)).color(theme::ACCENT_TEXT));
        });
}

// ---------- minimap (top-right) ----------

fn minimap_panel(
    ctx: &egui::Context,
    st: &UiState,
    m: &SimMetrics,
    cache: &mut HudCache,
    terrain: Option<&VoxelTerrain>,
) {
    egui::Area::new(egui::Id::new("minimap"))
        .anchor(Align2::RIGHT_TOP, egui::vec2(-18.0, 18.0))
        .show(ctx, |ui| {
            theme::panel_frame()
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    let Some(t) = terrain else {
                        ui.label(caps("Map"));
                        ui.label(
                            RichText::new("generating…")
                                .font(theme::sans(11.0))
                                .color(theme::TEXT_DIM),
                        );
                        return;
                    };
                    // Rebuild the texture only when its key changes (biomass view buckets the tick
                    // so it refreshes a few times a second, not every frame).
                    let bucket = if st.debug_view == DebugView::Biomass {
                        m.tick / 30
                    } else {
                        0
                    };
                    let key = (m.seed, st.debug_view, bucket);
                    let stale = cache.minimap.as_ref().map(|(k, _)| *k != key).unwrap_or(true);
                    if stale {
                        let img = minimap::build_image(t, st.debug_view, m.tick);
                        let tex = ctx.load_texture("minimap", img, egui::TextureOptions::NEAREST);
                        cache.minimap = Some((key, tex));
                    }
                    let tex = &cache.minimap.as_ref().unwrap().1;
                    let size = egui::vec2(minimap::MW as f32, minimap::MH as f32);
                    let rect = ui.image(egui::load::SizedTexture::new(tex.id(), size)).rect;

                    // Viewport frame: the 4 ground-projected screen corners as a closed polygon
                    // (exact at any yaw — the iso view is a rotated quad on the map, not an AABB).
                    if m.minimap_view.len() == 4 {
                        let pts: Vec<egui::Pos2> = m
                            .minimap_view
                            .iter()
                            .map(|f| {
                                egui::pos2(
                                    rect.left() + f[0].clamp(0.0, 1.0) * rect.width(),
                                    rect.top() + f[1].clamp(0.0, 1.0) * rect.height(),
                                )
                            })
                            .collect();
                        ui.painter()
                            .add(egui::Shape::closed_line(pts, Stroke::new(1.5, theme::ACCENT)));
                    }

                    if st.debug_view.is_field_map() {
                        ui.label(
                            RichText::new(legend_text(st.debug_view))
                                .font(theme::sans(9.5))
                                .color(theme::TEXT_DIM),
                        );
                        legend_bar(ui, st.debug_view);
                    }
                });
        });
}

// ---------- control rail (bottom-right) ----------

fn rail(ctx: &egui::Context, st: &mut UiState) {
    egui::Area::new(egui::Id::new("rail"))
        .anchor(Align2::RIGHT_BOTTOM, egui::vec2(-18.0, -22.0))
        .show(ctx, |ui| {
            theme::panel_frame()
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    for (panel, glyph) in [
                        (Panel::World, ph::CLOCK),
                        (Panel::View, ph::STACK),
                        (Panel::Pop, ph::CIRCLES_THREE),
                        (Panel::Perf, ph::CHART_BAR),
                    ] {
                        if icon_tab(ui, glyph, st.open_panel == Some(panel)).clicked() {
                            st.open_panel = if st.open_panel == Some(panel) {
                                None
                            } else {
                                Some(panel)
                            };
                        }
                    }
                    let (r, _) = ui.allocate_exact_size(egui::vec2(24.0, 1.0), Sense::hover());
                    ui.painter()
                        .hline(r.left()..=r.right(), r.center().y, Stroke::new(1.0, theme::HAIRLINE));
                    if icon_tab(ui, ph::EYE_SLASH, false).clicked() {
                        st.show_info = false;
                    }
                });
        });
}

/// 40×40 rail tab. Hover → faint fill + white glyph; active → amber backing + frame + left tab.
fn icon_tab(ui: &mut egui::Ui, glyph: &str, active: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), Sense::click());
    let p = ui.painter();
    let hovered = resp.hovered();
    if active {
        p.rect_filled(rect, 10.0, theme::ACCENT_FILL);
        p.rect_stroke(rect, 10.0, Stroke::new(1.0, theme::ACCENT_LINE), StrokeKind::Inside);
        let tab = egui::Rect::from_center_size(
            egui::pos2(rect.left() + 2.0, rect.center().y),
            egui::vec2(3.0, 18.0),
        );
        p.rect_filled(tab, 3.0, theme::ACCENT);
    } else if hovered {
        p.rect_filled(rect, 10.0, theme::HOVER_FILL);
    }
    let col = if active {
        theme::ACCENT_TEXT
    } else if hovered {
        theme::TEXT
    } else {
        theme::TEXT_DIM
    };
    p.text(rect.center(), Align2::CENTER_CENTER, glyph, theme::mono(19.0), col);
    resp
}

// ---------- flyouts (bottom-right, left of the rail) ----------

fn flyout(ctx: &egui::Context, st: &mut UiState, m: &SimMetrics, act: &mut UiActions) {
    let Some(panel) = st.open_panel else { return };
    egui::Area::new(egui::Id::new("flyout"))
        .anchor(Align2::RIGHT_BOTTOM, egui::vec2(-84.0, -22.0))
        .show(ctx, |ui| {
            theme::flyout_frame().show(ui, |ui| {
                ui.set_width(220.0);
                ui.spacing_mut().item_spacing.y = 7.0;
                match panel {
                    Panel::World => world_panel(ui, m, act),
                    Panel::View => view_panel(ui, st, m),
                    Panel::Pop => pop_panel(ui, m),
                    Panel::Perf => perf_panel(ui, m),
                }
            });
        });
}

fn world_panel(ui: &mut egui::Ui, m: &SimMetrics, act: &mut UiActions) {
    ui.label(caps("World & Time"));
    kv(ui, "seed", format!("0x{:X}", m.seed));
    kv(ui, "size", format!("{}×{} m", m.cols, m.rows));
    hairline(ui);
    kv(ui, "tick", format!("{}", m.tick));
    kv(ui, "sim time", format!("{:.1} s", m.sim_time));
    kv(ui, "day fraction", format!("{:.2}", m.day_frac));
    hairline(ui);
    ui.horizontal(|ui| {
        if secondary_button(ui, &format!("{}  Save · F5", ph::FLOPPY_DISK)) {
            act.save = true;
        }
        if secondary_button(ui, &format!("{}  Load · F9", ph::FOLDER_OPEN)) {
            act.load = true;
        }
    });
}

fn view_panel(ui: &mut egui::Ui, st: &mut UiState, _m: &SimMetrics) {
    ui.horizontal(|ui| {
        ui.label(caps("Debug view"));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new("G cycles").font(theme::mono(9.0)).color(theme::TEXT_FAINT));
        });
    });
    for (v, name) in [
        (DebugView::None, "None"),
        (DebugView::Topo, "Topography"),
        (DebugView::Temp, "Temperature"),
        (DebugView::Moist, "Moisture"),
        (DebugView::WaterDist, "Water distance"),
        (DebugView::Slope, "Slope"),
        (DebugView::Biomass, "Biomass"),
    ] {
        ui.radio_value(&mut st.debug_view, v, RichText::new(name).font(theme::sans(11.5)));
    }
    hairline(ui);
    ui.checkbox(&mut st.outline, RichText::new("Step-edge outline   (O)").font(theme::sans(11.5)));
    ui.checkbox(&mut st.mask, RichText::new("Water/land mask   (J)").font(theme::sans(11.5)));
    ui.checkbox(&mut st.water_on, RichText::new("Water surface   (H)").font(theme::sans(11.5)));
    if st.debug_view.is_field_map() {
        hairline(ui);
        ui.label(RichText::new(legend_text(st.debug_view)).font(theme::sans(10.5)).color(theme::TEXT_DIM));
        legend_bar(ui, st.debug_view);
    }
}

fn pop_panel(ui: &mut egui::Ui, m: &SimMetrics) {
    ui.label(caps("Population & Evolution"));
    let Some(l) = m.life.as_ref() else {
        ui.label(RichText::new("world generating…").font(theme::sans(11.5)).color(theme::TEXT_DIM));
        return;
    };
    ui.horizontal(|ui| {
        big_stat(ui, "POP", format!("{}", l.population));
        big_stat(ui, "ENERGY", format!("{:.0}", l.avg_energy));
        big_stat(ui, "BIOMASS", format!("{:.2}", l.avg_biomass));
    });
    sparkline(ui, &m.pop_hist, egui::vec2(212.0, 38.0), theme::GOOD_GREEN);
    hairline(ui);
    ui.label(caps("Complexity"));
    labelled_bar(ui, "multicellular", l.multi, theme::GOOD_GREEN);
    labelled_bar(ui, "carnivory", l.carn, theme::DATA_CARN);
    labelled_bar(ui, "autotrophy", l.auto, theme::DATA_AUTO);
    hairline(ui);
    ui.label(caps("Diversity"));
    kv(ui, "species", format!("{}", l.species));
    kv(ui, "niches", format!("{}", l.niches));
    kv(ui, "allopatry", format!("{:.2}", l.allop));
    kv(ui, "crypsis", format!("{:.2}", l.crypsis));
    kv(ui, "nutrient", format!("{:.2}", l.nutri));
    hairline(ui);
    ui.label(caps("Strata mix"));
    strata_bar(ui, l.strata);
}

fn big_stat(ui: &mut egui::Ui, label: &str, value: String) {
    ui.vertical(|ui| {
        ui.label(caps(label));
        ui.label(RichText::new(value).font(theme::mono(18.0)).color(theme::TEXT));
    });
}

fn labelled_bar(ui: &mut egui::Ui, label: &str, frac: f32, col: egui::Color32) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).font(theme::sans(10.5)).color(theme::TEXT_DIM));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(format!("{:.0}%", frac * 100.0)).font(theme::mono(10.0)).color(theme::TEXT));
        });
    });
    bar(ui, frac, col);
}

fn strata_bar(ui: &mut egui::Ui, strata: [f32; 4]) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 10.0), Sense::hover());
    let cols = [theme::DATA_AUTO, theme::GOOD_GREEN, theme::ACCENT, theme::DATA_CARN];
    let p = ui.painter();
    let mut x = rect.left();
    for (i, &f) in strata.iter().enumerate() {
        let seg_w = rect.width() * f.clamp(0.0, 1.0);
        let seg = egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(seg_w, rect.height()));
        p.rect_filled(seg, 0.0, cols[i]);
        x += seg_w;
    }
    ui.label(
        RichText::new(format!(
            "under {:.0}  surf {:.0}  air {:.0}  water {:.0}",
            strata[0] * 100.0,
            strata[1] * 100.0,
            strata[2] * 100.0,
            strata[3] * 100.0
        ))
        .font(theme::mono(9.0))
        .color(theme::TEXT_FAINT),
    );
}

fn perf_panel(ui: &mut egui::Ui, m: &SimMetrics) {
    ui.label(caps("Performance"));
    ui.label(RichText::new(format!("{:.0}", m.fps)).font(theme::mono(26.0)).color(theme::TEXT));
    ui.label(
        RichText::new(format!("fps · {:.1} ms", m.frame_ms))
            .font(theme::mono(11.0))
            .color(theme::TEXT_DIM),
    );
    hairline(ui);
    kv(ui, "draws", format!("{}", m.drawn));
    kv(ui, "chunks · detail", format!("{}", m.detail));
    kv(ui, "chunks · coarse", format!("{}", m.coarse));
    kv(ui, "on-screen", format!("{}", m.on_screen));
}
