//! The HUD itself — nine floating `egui::Area`s composing the "naturalist's dashboard": vitals
//! (top-left), transport (bottom-left), control rail + one flyout (bottom-right), minimap
//! (top-right), toast (top-centre) / hide-hint. Pixel-spec'd against the `Animata GUI` mockup:
//! glass panels, one warm amber accent, IBM Plex type. egui has no native letter-spacing or SVG, so
//! caps tracking is emulated per-glyph ([`theme::paint_tracked`]) and every icon / control marker is
//! hand-painted from the mockup's 24-px viewBox vectors.
//! By default only vitals + transport + rail + minimap show; detail panels open from the rail.

use egui::{Align, Align2, Color32, Layout, Pos2, RichText, Sense, Shape, Stroke, StrokeKind, Vec2};

use animata_sim::terrain::VoxelTerrain;

use super::theme;
use super::theme::FrameKind;
use super::{inspector, legend_text, minimap, ramp_color, MAX_TIME_SCALE, MIN_TIME_SCALE};
use super::{HudCache, Panel, SimMetrics, UiActions, UiState};
use crate::DebugView;

// ---- local tints (straight-alpha via theme::straight; bytes = round(frac*255)) ----
const ACTIVE_ROW: Color32 = theme::straight(242, 166, 75, 36); // accent .14 — selected option row
const HOVER_ROW: Color32 = theme::straight(255, 255, 255, 13); // .05 — row hover
const RING: Color32 = theme::straight(255, 255, 255, 77); // .30 — radio/checkbox marker edge
const KEYCAP_TXT: Color32 = theme::straight(233, 236, 230, 102); // .40 — keycap letters
const HAIR_12: Color32 = theme::straight(255, 255, 255, 31); // .12 — vitals/transport dividers
const ACCENT_LINE_50: Color32 = theme::straight(242, 166, 75, 128); // accent .50 — frames/rings
const DARK_GLYPH: Color32 = Color32::from_rgb(10, 12, 11); // glyph on amber / checkmark

/// Build the whole HUD. Simple toggles mutate `st` directly; non-trivial intents + the pointer-gate
/// flow back via [`UiActions`].
pub fn draw_hud(
    ctx: &egui::Context,
    st: &mut UiState,
    m: &SimMetrics,
    cache: &mut HudCache,
    terrain: Option<&VoxelTerrain>,
    now: f32,
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
    flyout(ctx, st, m, cache, &mut act);
    // Inspector last (after the rail flyout) but before the modal loader: it coexists with any
    // flyout and is driven by world-selection, not the rail.
    inspector::draw_inspector(ctx, st, m, now);

    // F4: gate world mouse on "pointer over ANY interactive Area" — covers empty panel backgrounds
    // too (a click on the glass shouldn't reach the world), while non-interactable toast/hint pass.
    act.wants_pointer = ctx.is_pointer_over_area();
    act
}

// ---------- small shared widgets ----------

/// Mono caps label with CSS letter-spacing, laid out left-aligned in the current vertical flow.
pub(crate) fn caps_tracked(ui: &mut egui::Ui, text: &str, size: f32, em: f32, color: Color32) {
    let upper = text.to_uppercase();
    let font = theme::mono(size);
    let tr = theme::tracking_em(size, em);
    let w = theme::total_tracked_width(ui, &upper, &font, tr);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, size + 1.0), Sense::hover());
    theme::paint_tracked(ui, rect.left_top(), Align2::LEFT_TOP, &upper, font, color, tr);
}

/// Human name of a debug field (minimap caption).
fn field_name(view: DebugView) -> &'static str {
    match view {
        DebugView::Topo => "Topography",
        DebugView::Temp => "Temperature",
        DebugView::Moist => "Moisture",
        DebugView::WaterDist => "Water distance",
        DebugView::Slope => "Slope",
        DebugView::Biomass => "Biomass",
        DebugView::None => "",
    }
}

/// Two-decimal format that snaps a near-zero magnitude to a clean `0.00` (avoids `-0.00`).
fn fmt2(v: f32) -> String {
    format!("{:.2}", if v.abs() < 0.005 { 0.0 } else { v })
}

/// `label … value` row (label sans dim left, value mono right) — mockup flyout row.
pub(crate) fn kv(ui: &mut egui::Ui, label: &str, value: String) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(label)
                .font(theme::sans(12.0))
                .color(theme::TEXT_LABEL),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(value).font(theme::mono(12.0)).color(theme::TEXT));
        });
    });
}

pub(crate) fn hairline(ui: &mut egui::Ui) {
    ui.add_space(5.0);
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 1.0), Sense::hover());
    ui.painter()
        .hline(rect.left()..=rect.right(), rect.center().y, Stroke::new(1.0, theme::HAIRLINE));
    ui.add_space(5.0);
}

/// Thin progress bar: track + metric-coloured fill. Default h4 r3 (flyout/pop bars).
fn bar(ui: &mut egui::Ui, frac: f32, col: Color32) {
    bar_sized(ui, frac, col, 4.0, 3.0);
}

/// Progress bar with explicit height/radius (inspector reuses h4 vitals, h3 genome).
pub(crate) fn bar_sized(ui: &mut egui::Ui, frac: f32, col: Color32, h: f32, r: f32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, r, theme::straight(255, 255, 255, 26)); // .10 track
    let mut fill = rect;
    fill.set_width(rect.width() * frac.clamp(0.0, 1.0));
    p.rect_filled(fill, r, col);
}

/// Outline button (Save / Load): faint fill, white-ish frame, hover lift. Fixed `w`, no icon.
fn secondary_button(ui: &mut egui::Ui, text: &str, w: f32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 30.0), Sense::click());
    let p = ui.painter();
    let fill = if resp.hovered() {
        theme::straight(255, 255, 255, 23) // .09
    } else {
        theme::straight(255, 255, 255, 8) // .03
    };
    p.rect_filled(rect, 9.0, fill);
    p.rect_stroke(rect, 9.0, Stroke::new(1.0, theme::straight(255, 255, 255, 36)), StrokeKind::Inside);
    p.text(rect.center(), Align2::CENTER_CENTER, text, theme::mono(11.0), theme::TEXT);
    resp.clicked()
}

/// Draw a horizontal bar with rounded ends via a 1-D colour texture on a rounded `RectShape` — the
/// rounded geometry itself masks the corners (a true mask, not end-cap hacks). `colors` is the
/// left→right colour profile; `smooth` picks LINEAR (gradients) vs NEAREST (hard segments). The tiny
/// texture is cached in `HudCache::bars` (keyed by content) so the handle outlives the paint.
fn rounded_bar(ui: &mut egui::Ui, cache: &mut HudCache, rect: egui::Rect, colors: &[Color32], smooth: bool) {
    use std::hash::{Hash, Hasher};
    let mut hsh = std::collections::hash_map::DefaultHasher::new();
    smooth.hash(&mut hsh);
    for c in colors {
        c.to_array().hash(&mut hsh);
    }
    let key = hsh.finish();
    if cache.bars.len() > 64 {
        cache.bars.clear(); // bound growth — the strata key varies with live data
    }
    let tex = cache.bars.entry(key).or_insert_with(|| {
        let img = egui::ColorImage { size: [colors.len().max(1), 1], pixels: colors.to_vec() };
        let opt = if smooth { egui::TextureOptions::LINEAR } else { egui::TextureOptions::NEAREST };
        ui.ctx().load_texture("hbar", img, opt)
    });
    let r = (rect.height() * 0.5).round() as u8;
    let uv = egui::Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));
    ui.painter().add(
        egui::epaint::RectShape::filled(rect, egui::CornerRadius::same(r), Color32::WHITE)
            .with_texture(tex.id(), uv),
    );
}

/// Field-map colour-ramp legend with rounded ends. `h` = bar height (flyout 9, minimap 7).
fn legend_bar(ui: &mut egui::Ui, cache: &mut HudCache, view: DebugView, h: f32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), Sense::hover());
    let n = 64usize;
    let colors: Vec<Color32> = (0..n).map(|i| ramp_color(view, i as f32 / (n - 1) as f32)).collect();
    rounded_bar(ui, cache, rect, &colors, true);
}

// ---- 24-px viewBox → rect mapping (mockup icons) ----
fn vb(r: egui::Rect, x: f32, y: f32) -> Pos2 {
    Pos2::new(r.left() + x / 24.0 * r.width(), r.top() + y / 24.0 * r.height())
}
fn vbr(r: egui::Rect, rad: f32) -> f32 {
    rad / 24.0 * r.width()
}

// ---------- toast / hide-hint ----------

/// System confirmation toast, anchored top-centre. `dt` = elapsed ms since it fired; over its
/// ~2600 ms life it slides in (0–180 ms), holds, then fades (1900–2600 ms). Opacity multiplies every
/// layer so nothing leaves a hard edge while fading.
fn toast(ctx: &egui::Context, m: &SimMetrics) {
    let Some((msg, dt)) = &m.toast else { return };
    let dt = *dt;
    let opacity = if dt < 180.0 {
        dt / 180.0
    } else if dt > 1900.0 {
        ((2600.0 - dt) / 700.0).max(0.0)
    } else {
        1.0
    };
    let shift_y = if dt < 180.0 { -(180.0 - dt) / 180.0 * 10.0 } else { 0.0 };
    let a = |c: Color32| {
        Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (c.a() as f32 * opacity) as u8)
    };
    egui::Area::new(egui::Id::new("toast"))
        .anchor(Align2::CENTER_TOP, egui::vec2(0.0, 18.0 + shift_y))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(a(Color32::from_rgba_unmultiplied(12, 15, 14, 209))) // .82
                .stroke(Stroke::new(1.0, a(Color32::from_rgba_unmultiplied(143, 209, 111, 77)))) // green .30
                .corner_radius(egui::CornerRadius::same(11))
                .inner_margin(egui::Margin::symmetric(18, 10))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 10],
                    blur: 30,
                    spread: 0,
                    color: Color32::from_black_alpha((102.0 * opacity) as u8),
                })
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(msg)
                                .font(theme::mono(12.0))
                                .color(a(theme::TOAST_GREEN)),
                        )
                        .wrap_mode(egui::TextWrapMode::Extend),
                    );
                });
        });
    ctx.request_repaint(); // keep the animation advancing between frames
}

/// `press [I] for UI` chip, bottom-left (mockup): faint glass + a keycap box, no icon.
fn hide_hint(ctx: &egui::Context) {
    egui::Area::new(egui::Id::new("hide_hint"))
        .anchor(Align2::LEFT_BOTTOM, egui::vec2(18.0, -18.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(theme::straight(12, 15, 14, 153)) // .60
                .stroke(Stroke::new(1.0, theme::straight(255, 255, 255, 20))) // .08
                .corner_radius(egui::CornerRadius::same(9))
                .inner_margin(egui::Margin::symmetric(12, 7))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.label(RichText::new("press").font(theme::mono(10.0)).color(theme::TEXT_LABEL));
                        egui::Frame::NONE
                            .fill(theme::straight(255, 255, 255, 26)) // .10
                            .corner_radius(egui::CornerRadius::same(5))
                            .inner_margin(egui::Margin::symmetric(6, 2))
                            .show(ui, |ui| {
                                ui.label(RichText::new("I").font(theme::mono(10.0)).color(theme::TEXT));
                            });
                        ui.label(RichText::new("for UI").font(theme::mono(10.0)).color(theme::TEXT_LABEL));
                    });
                });
        });
}

// ---------- vitals (top-left) ----------

fn vitals(ctx: &egui::Context, m: &SimMetrics) {
    egui::Area::new(egui::Id::new("vitals"))
        .anchor(Align2::LEFT_TOP, egui::vec2(18.0, 18.0))
        .show(ctx, |ui| {
            theme::themed_frame(FrameKind::Vitals).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 16.0;

                    // DAY + HH:MM
                    let day = (m.sim_time / 600.0).floor() as u64 + 1;
                    let hh = (m.day_frac * 24.0).floor() as u32;
                    let mm = (m.day_frac * 24.0 * 60.0) as u32 % 60;
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        caps_tracked(ui, "Day", 9.0, 0.16, theme::TEXT_FAINT);
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
                        ui.spacing_mut().item_spacing.y = 4.0;
                        caps_tracked(ui, "Population", 9.0, 0.16, theme::TEXT_FAINT);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("{pop}"))
                                    .font(theme::mono(15.0))
                                    .color(theme::TEXT),
                            );
                            sparkline(ui, &m.pop_hist, egui::vec2(120.0, 26.0), theme::GOOD_GREEN);
                        });
                    });
                });
            });
        });
}

fn vitals_hairline(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 26.0), Sense::hover());
    ui.painter()
        .vline(rect.center().x, rect.top()..=rect.bottom(), Stroke::new(1.0, HAIR_12));
}

/// Day-cycle dial: a thin ring + a single amber hand from the centre pointing "up", swept clockwise
/// by `frac` (mockup: 20×20 ring, 8-px hand). No hub/sun-bead.
fn sun_dial(ui: &mut egui::Ui, frac: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), Sense::hover());
    let c = rect.center();
    let p = ui.painter();
    p.circle_stroke(c, 10.0, Stroke::new(1.0, theme::straight(255, 255, 255, 46))); // .18
    let ang = frac * std::f32::consts::TAU; // 0 = up, clockwise
    let dir = Vec2::new(ang.sin(), -ang.cos());
    p.line_segment([c, c + dir * 8.0], Stroke::new(1.5, theme::ACCENT));
}

fn sparkline(ui: &mut egui::Ui, data: &[f32], size: Vec2, col: Color32) {
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
    let pts: Vec<Pos2> = data
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = rect.left() + rect.width() * i as f32 / (data.len() - 1) as f32;
            let y = rect.bottom() - rect.height() * (v - lo) / span;
            egui::pos2(x, y)
        })
        .collect();
    ui.painter().add(Shape::line(pts, Stroke::new(1.5, col)));
}

// ---------- transport (bottom-left) ----------

fn transport(ctx: &egui::Context, m: &SimMetrics, act: &mut UiActions) {
    egui::Area::new(egui::Id::new("transport"))
        .anchor(Align2::LEFT_BOTTOM, egui::vec2(18.0, -22.0))
        .show(ctx, |ui| {
            theme::themed_frame(FrameKind::Transport).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;

                    if play_button(ui, m.paused).clicked() {
                        act.toggle_pause = true;
                    }

                    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 24.0), Sense::hover());
                    ui.painter()
                        .vline(rect.center().x, rect.top()..=rect.bottom(), Stroke::new(1.0, HAIR_12));

                    if let Some(v) = speed_slider(ui, m.time_scale) {
                        act.set_time_scale = Some(v);
                    }
                    let val = if m.time_scale < 10.0 {
                        format!("{:.1}×", m.time_scale)
                    } else {
                        format!("{}×", m.time_scale.round() as i32)
                    };
                    let (lr, _) = ui.allocate_exact_size(egui::vec2(46.0, 16.0), Sense::hover());
                    ui.painter().text(
                        lr.right_center(),
                        Align2::RIGHT_CENTER,
                        val,
                        theme::mono(14.0),
                        theme::ACCENT_TEXT,
                    );

                    if m.paused {
                        badge(ui, "PAUSED");
                    }
                });
            });
        });
}

/// Pause/play button (mockup): amber-tint square, amber frame, vector glyph (triangle / two bars).
fn play_button(ui: &mut egui::Ui, paused: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), Sense::click());
    let p = ui.painter();
    let fill = if resp.hovered() {
        theme::straight(242, 166, 75, 66) // .26
    } else {
        theme::ACCENT_FILL // .16
    };
    p.rect_filled(rect, 11.0, fill);
    p.rect_stroke(rect, 11.0, Stroke::new(1.0, ACCENT_LINE_50), StrokeKind::Inside);
    let ic = egui::Rect::from_center_size(rect.center(), egui::vec2(18.0, 18.0));
    if paused {
        // play triangle M8 5 v14 l11-7 Z
        p.add(Shape::convex_polygon(
            vec![vb(ic, 8.0, 5.0), vb(ic, 8.0, 19.0), vb(ic, 19.0, 12.0)],
            theme::ACCENT_TEXT,
            Stroke::NONE,
        ));
    } else {
        // pause: two bars
        for x in [6.0_f32, 14.0] {
            let br = egui::Rect::from_min_max(vb(ic, x, 5.0), vb(ic, x + 4.0, 19.0));
            p.rect_filled(br, vbr(ic, 1.0), theme::ACCENT_TEXT);
        }
    }
    resp
}

/// Logarithmic speed slider (0.1×–64×): uniform track + ringed amber thumb (mockup amSlider).
fn speed_slider(ui: &mut egui::Ui, current: f32) -> Option<f32> {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(150.0, 24.0), Sense::click_and_drag());
    let cy = rect.center().y;
    let (lmin, lmax) = (MIN_TIME_SCALE.ln(), MAX_TIME_SCALE.ln());
    let t = ((current.ln() - lmin) / (lmax - lmin)).clamp(0.0, 1.0);
    let hx = rect.left() + t * rect.width();
    let p = ui.painter();
    p.line_segment(
        [egui::pos2(rect.left(), cy), egui::pos2(rect.right(), cy)],
        Stroke::new(3.0, theme::straight(255, 255, 255, 41)), // .16 uniform track
    );
    let h = egui::pos2(hx, cy);
    p.circle_filled(h, 6.5, theme::ACCENT);
    p.circle_stroke(h, 6.5, Stroke::new(2.0, theme::straight(10, 12, 11, 230))); // .9 dark border
    p.circle_stroke(h, 7.5, Stroke::new(1.0, ACCENT_LINE_50));
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
        .fill(theme::straight(242, 166, 75, 46)) // .18
        .stroke(Stroke::new(1.0, theme::straight(242, 166, 75, 102))) // .40
        .corner_radius(7)
        .inner_margin(egui::Margin::symmetric(9, 4))
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
            theme::themed_frame(FrameKind::Vitals)
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    let Some(t) = terrain else {
                        caps_tracked(ui, "Map", 10.0, 0.18, theme::TEXT_FAINT);
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
                    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                    let painter = ui.painter_at(rect);

                    // Project map fractions (u,v) into the panel the same way the iso camera frames
                    // the world on screen, so the minimap reads as the on-screen diamond rather than
                    // a top-down square. Azimuth 45° → screen-x ∝ (x−z); the ~35.26° elevation
                    // foreshortens depth → screen-y ∝ (x+z)·FS (a wider-than-tall diamond).
                    const FS: f32 = 0.577_350_3; // sin(35.264°)
                    let s = ((rect.width() * 0.5 - 6.0) / 1.0).min((rect.height() * 0.5 - 6.0) / FS);
                    let c = rect.center();
                    let proj = |u: f32, v: f32| {
                        let (cu, cv) = (u - 0.5, v - 0.5);
                        egui::pos2(c.x + (cu - cv) * s, c.y + (cu + cv) * FS * s)
                    };

                    // Map texture as a rotated/foreshortened quad (the four map corners → diamond).
                    let mut mesh = egui::Mesh::with_texture(tex.id());
                    for &(u, v) in &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
                        mesh.vertices.push(egui::epaint::Vertex {
                            pos: proj(u, v),
                            uv: egui::pos2(u, v),
                            color: Color32::WHITE,
                        });
                    }
                    mesh.add_triangle(0, 1, 2);
                    mesh.add_triangle(0, 2, 3);
                    painter.add(Shape::mesh(mesh));

                    // Viewport frame: the 4 ground-projected screen corners through the SAME iso
                    // projection. `proj` exactly inverts the camera's ground unprojection, so the
                    // footprint maps back to the upright screen rectangle. The painter clips it.
                    if m.minimap_view.len() == 4 {
                        let pts: Vec<Pos2> =
                            m.minimap_view.iter().map(|f| proj(f[0], f[1])).collect();
                        veil_outside(&painter, rect, &pts);
                        painter.add(Shape::closed_line(pts, Stroke::new(1.5, theme::ACCENT)));
                    }

                    if st.debug_view.is_field_map() {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            let fname = field_name(st.debug_view);
                            let font = theme::mono(10.0);
                            let tr = theme::tracking_em(10.0, 0.04);
                            let w = theme::total_tracked_width(ui, fname, &font, tr);
                            let (r, _) = ui.allocate_exact_size(egui::vec2(w, 12.0), Sense::hover());
                            theme::paint_tracked(
                                ui,
                                r.left_center(),
                                Align2::LEFT_CENTER,
                                fname,
                                font,
                                theme::straight(233, 236, 230, 166), // .65
                                tr,
                            );
                            legend_bar(ui, cache, st.debug_view, 7.0);
                        });
                    }
                });
        });
}

/// Dim the panel OUTSIDE the viewport rectangle (four bands around the frame's bounding box, clamped
/// to the panel). Bands collapse to nothing once the view encloses the whole map.
fn veil_outside(painter: &egui::Painter, rect: egui::Rect, quad: &[Pos2]) {
    let mut lo = rect.max;
    let mut hi = rect.min;
    for p in quad {
        let x = p.x.clamp(rect.left(), rect.right());
        let y = p.y.clamp(rect.top(), rect.bottom());
        lo = egui::pos2(lo.x.min(x), lo.y.min(y));
        hi = egui::pos2(hi.x.max(x), hi.y.max(y));
    }
    let veil = Color32::from_rgba_unmultiplied(5, 7, 10, 71); // .28
    let band = |a: Pos2, b: Pos2| {
        let r = egui::Rect::from_two_pos(a, b);
        if r.width() > 0.5 && r.height() > 0.5 {
            painter.rect_filled(r, 0.0, veil);
        }
    };
    band(rect.left_top(), egui::pos2(rect.right(), lo.y)); // top
    band(egui::pos2(rect.left(), hi.y), rect.right_bottom()); // bottom
    band(egui::pos2(rect.left(), lo.y), egui::pos2(lo.x, hi.y)); // left
    band(egui::pos2(hi.x, lo.y), egui::pos2(rect.right(), hi.y)); // right
}

// ---------- control rail (bottom-right) ----------

fn rail(ctx: &egui::Context, st: &mut UiState) {
    egui::Area::new(egui::Id::new("rail"))
        .anchor(Align2::RIGHT_BOTTOM, egui::vec2(-18.0, -22.0))
        .show(ctx, |ui| {
            theme::themed_frame(FrameKind::Rail).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                for (panel, icon) in [
                    (Panel::World, RailIcon::Clock),
                    (Panel::View, RailIcon::Layers),
                    (Panel::Pop, RailIcon::Circles),
                    (Panel::Perf, RailIcon::Bars),
                ] {
                    if icon_tab(ui, icon, st.open_panel == Some(panel)).clicked() {
                        st.open_panel = if st.open_panel == Some(panel) {
                            None
                        } else {
                            Some(panel)
                        };
                    }
                }
                ui.add_space(2.0);
                let (r, _) = ui.allocate_exact_size(egui::vec2(28.0, 1.0), Sense::hover());
                ui.painter()
                    .hline(r.left()..=r.right(), r.center().y, Stroke::new(1.0, theme::HAIRLINE));
                ui.add_space(2.0);
                if icon_tab(ui, RailIcon::Eye, false).clicked() {
                    st.show_info = false;
                }
            });
        });
}

#[derive(Clone, Copy)]
enum RailIcon {
    Clock,
    Layers,
    Circles,
    Bars,
    Eye,
}

/// 40×40 rail tab. Hover → faint fill + white glyph; active → amber backing + frame + an OUTER left
/// tab. Icon drawn from the mockup's 24-px vectors.
fn icon_tab(ui: &mut egui::Ui, icon: RailIcon, active: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), Sense::click());
    let p = ui.painter();
    let hovered = resp.hovered();
    if active {
        p.rect_filled(rect, 10.0, theme::ACCENT_FILL);
        p.rect_stroke(rect, 10.0, Stroke::new(1.0, ACCENT_LINE_50), StrokeKind::Inside);
        let tab = egui::Rect::from_center_size(
            egui::pos2(rect.left() - 5.5, rect.center().y), // left:-7px (3-wide tab centre)
            egui::vec2(3.0, 18.0),
        );
        p.rect_filled(tab, 3.0, theme::ACCENT);
    } else if hovered {
        p.rect_filled(rect, 10.0, theme::HOVER_FILL); // .07
    }
    let col = if active {
        theme::ACCENT_TEXT
    } else if hovered {
        theme::TEXT
    } else if matches!(icon, RailIcon::Eye) {
        theme::TEXT_LABEL // .55 — hide control sits quieter
    } else {
        theme::straight(233, 236, 230, 179) // .70
    };
    let ic = egui::Rect::from_center_size(rect.center(), egui::vec2(19.0, 19.0));
    paint_rail_icon(p, icon, ic, col);
    resp
}

fn paint_rail_icon(p: &egui::Painter, icon: RailIcon, r: egui::Rect, col: Color32) {
    let s = Stroke::new(1.6, col);
    match icon {
        RailIcon::Clock => {
            p.circle_stroke(vb(r, 12.0, 12.0), vbr(r, 8.5), s);
            p.add(Shape::line(
                vec![vb(r, 12.0, 7.0), vb(r, 12.0, 12.0), vb(r, 15.2, 13.8)],
                s,
            ));
        }
        RailIcon::Layers => {
            p.add(Shape::closed_line(
                vec![vb(r, 12.0, 4.0), vb(r, 20.0, 8.0), vb(r, 12.0, 12.0), vb(r, 4.0, 8.0)],
                s,
            ));
            p.add(Shape::line(
                vec![vb(r, 4.0, 12.0), vb(r, 12.0, 16.0), vb(r, 20.0, 12.0)],
                s,
            ));
        }
        RailIcon::Circles => {
            p.circle_stroke(vb(r, 8.0, 9.0), vbr(r, 2.4), s);
            p.circle_stroke(vb(r, 15.5, 7.0), vbr(r, 2.0), s);
            p.circle_stroke(vb(r, 13.0, 15.0), vbr(r, 2.8), s);
        }
        RailIcon::Bars => {
            for (x, y, h) in [(4.0, 13.0, 7.0), (10.2, 8.0, 12.0), (16.5, 5.0, 15.0)] {
                let br = egui::Rect::from_min_max(vb(r, x, y), vb(r, x + 3.5, y + h));
                p.rect_stroke(br, vbr(r, 1.0), s, StrokeKind::Inside);
            }
        }
        RailIcon::Eye => {
            // Lens approximated by an ellipse (rx10 ry6) + pupil — egui has no SVG bézier.
            let c = vb(r, 12.0, 12.0);
            let (rx, ry) = (vbr(r, 10.0), vbr(r, 6.0));
            let pts: Vec<Pos2> = (0..=28)
                .map(|i| {
                    let a = i as f32 / 28.0 * std::f32::consts::TAU;
                    Pos2::new(c.x + rx * a.cos(), c.y + ry * a.sin())
                })
                .collect();
            p.add(Shape::closed_line(pts, s));
            p.circle_stroke(c, vbr(r, 2.6), s);
        }
    }
}

// ---------- flyouts (bottom-right, left of the rail) ----------

fn flyout_width(panel: Panel) -> f32 {
    match panel {
        Panel::World | Panel::View => 238.0,
        Panel::Pop => 248.0,
        Panel::Perf => 222.0,
    }
}

fn flyout(ctx: &egui::Context, st: &mut UiState, m: &SimMetrics, cache: &mut HudCache, act: &mut UiActions) {
    let Some(panel) = st.open_panel else { return };
    egui::Area::new(egui::Id::new("flyout"))
        .anchor(Align2::RIGHT_BOTTOM, egui::vec2(-84.0, -22.0))
        .show(ctx, |ui| {
            theme::themed_frame(FrameKind::Flyout).show(ui, |ui| {
                ui.set_width(flyout_width(panel));
                ui.spacing_mut().item_spacing.y = 7.0;
                match panel {
                    Panel::World => world_panel(ui, m, act),
                    Panel::View => view_panel(ui, cache, st, m),
                    Panel::Pop => pop_panel(ui, cache, m),
                    Panel::Perf => perf_panel(ui, m),
                }
            });
        });
}

/// Flyout caps header (mono 10, .18em) with the 14-px bottom gap from the mockup.
fn flyout_header(ui: &mut egui::Ui, text: &str) {
    caps_tracked(ui, text, 10.0, 0.18, theme::TEXT_FAINT);
    ui.add_space(7.0);
}

fn world_panel(ui: &mut egui::Ui, m: &SimMetrics, act: &mut UiActions) {
    flyout_header(ui, "World & Time");
    ui.spacing_mut().item_spacing.y = 9.0;
    kv(ui, "seed", format!("0x{:X}", m.seed));
    kv(ui, "size", format!("{}×{} m", m.cols, m.rows));
    hairline(ui);
    kv(ui, "tick", format!("{}", m.tick));
    kv(ui, "sim time", format!("{:.1} s", m.sim_time));
    kv(ui, "day fraction", format!("{:.2}", m.day_frac));
    hairline(ui);
    let bw = (ui.available_width() - 8.0) / 2.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if secondary_button(ui, "Save · F5", bw) {
            act.save = true;
        }
        if secondary_button(ui, "Load · F9", bw) {
            act.load = true;
        }
    });
}

fn view_panel(ui: &mut egui::Ui, cache: &mut HudCache, st: &mut UiState, _m: &SimMetrics) {
    ui.horizontal(|ui| {
        caps_tracked(ui, "Debug view", 10.0, 0.18, theme::TEXT_FAINT);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new("G cycles").font(theme::mono(9.0)).color(KEYCAP_TXT));
        });
    });
    ui.add_space(8.0); // header margin-bottom 12 (≈ caps gap + this)
    ui.spacing_mut().item_spacing.y = 1.0;
    for (v, name) in [
        (DebugView::None, "None"),
        (DebugView::Topo, "Topography"),
        (DebugView::Temp, "Temperature"),
        (DebugView::Moist, "Moisture"),
        (DebugView::WaterDist, "Water distance"),
        (DebugView::Slope, "Slope"),
        (DebugView::Biomass, "Biomass"),
    ] {
        if radio_row(ui, st.debug_view == v, name).clicked() {
            st.debug_view = v;
        }
    }
    hairline(ui);
    ui.spacing_mut().item_spacing.y = 2.0;
    if checkbox_row(ui, st.outline, "Step-edge outline", "O").clicked() {
        st.outline = !st.outline;
    }
    if checkbox_row(ui, st.mask, "Water / land mask", "J").clicked() {
        st.mask = !st.mask;
    }
    if checkbox_row(ui, st.water_on, "Water surface", "H").clicked() {
        st.water_on = !st.water_on;
    }
    if st.debug_view.is_field_map() {
        hairline(ui);
        ui.label(RichText::new(legend_text(st.debug_view)).font(theme::sans(11.0)).color(theme::straight(233, 236, 230, 153)));
        ui.add_space(3.0);
        legend_bar(ui, cache, st.debug_view, 9.0);
    }
}

/// Full-row clickable option with a ring marker (mockup radio). `active` fills the row amber-faint.
fn radio_row(ui: &mut egui::Ui, active: bool, label: &str) -> egui::Response {
    let w = ui.available_width();
    // 26.5 = mockup row box (12.5 line + 7+7 padding); column gap stays 1px → 27.5 pitch.
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 26.5), Sense::click());
    let p = ui.painter();
    if active {
        p.rect_filled(rect, 8.0, ACTIVE_ROW);
    } else if resp.hovered() {
        p.rect_filled(rect, 8.0, HOVER_ROW);
    }
    let mc = egui::pos2(rect.left() + 9.0 + 6.0, rect.center().y);
    p.circle_stroke(mc, 6.0, Stroke::new(1.5, RING));
    if active {
        p.circle_filled(mc, 3.0, theme::ACCENT);
    }
    p.text(
        egui::pos2(rect.left() + 31.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        theme::sans(12.5),
        theme::TEXT,
    );
    resp
}

/// Full-row clickable toggle with a square check box + right-aligned keycap (mockup checkbox).
fn checkbox_row(ui: &mut egui::Ui, checked: bool, label: &str, key: &str) -> egui::Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 28.0), Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 8.0, HOVER_ROW);
    }
    let bx = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 9.0, rect.center().y - 7.0),
        egui::vec2(14.0, 14.0),
    );
    if checked {
        p.rect_filled(bx, 4.0, theme::ACCENT);
        // checkmark M2.5 6.5 l2.5 2.5 l4.5-5.5 in a 12-vbox, mapped into the 14-px box.
        let s = bx.width() / 12.0;
        let m = |x: f32, y: f32| Pos2::new(bx.left() + x * s, bx.top() + y * s);
        p.add(Shape::line(
            vec![m(2.5, 6.5), m(5.0, 9.0), m(9.5, 3.5)],
            Stroke::new(2.2, DARK_GLYPH),
        ));
    } else {
        p.rect_stroke(bx, 4.0, Stroke::new(1.5, RING), StrokeKind::Inside);
    }
    p.text(
        egui::pos2(rect.left() + 33.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        theme::sans(12.5),
        theme::TEXT,
    );
    p.text(
        egui::pos2(rect.right() - 4.0, rect.center().y),
        Align2::RIGHT_CENTER,
        key,
        theme::mono(9.0),
        KEYCAP_TXT,
    );
    resp
}

fn pop_panel(ui: &mut egui::Ui, cache: &mut HudCache, m: &SimMetrics) {
    flyout_header(ui, "Population & Evolution");
    let Some(l) = m.life.as_ref() else {
        ui.label(RichText::new("world generating…").font(theme::sans(11.5)).color(theme::TEXT_DIM));
        return;
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let cw = (ui.available_width() - 20.0) / 3.0;
        big_stat(ui, cw, format!("{}", l.population), "POP");
        big_stat(ui, cw, format!("{:.0}", l.avg_energy), "ENERGY");
        big_stat(ui, cw, format!("{:.2}", l.avg_biomass), "BIOMASS");
    });
    ui.add_space(7.0);
    sparkline(ui, &m.pop_hist, egui::vec2(ui.available_width(), 40.0), theme::GOOD_GREEN);
    ui.add_space(7.0);
    caps_tracked(ui, "Complexity", 9.0, 0.14, KEYCAP_TXT);
    ui.add_space(2.0);
    ui.spacing_mut().item_spacing.y = 9.0;
    labelled_bar(ui, "multicellular", l.multi, theme::GOOD_GREEN);
    labelled_bar(ui, "carnivory", l.carn, theme::DATA_CARN);
    labelled_bar(ui, "autotrophy", l.auto, theme::DATA_AUTO);
    ui.add_space(5.0);
    caps_tracked(ui, "Diversity", 9.0, 0.14, KEYCAP_TXT);
    ui.add_space(4.0);
    diversity_row(ui, l);
    ui.add_space(5.0);
    caps_tracked(ui, "Strata mix", 9.0, 0.14, KEYCAP_TXT);
    ui.add_space(4.0);
    strata_bar(ui, cache, l.strata);
}

/// Big metric: value on top (mono 18), caps label beneath (mono 9, .1em) — mockup order.
fn big_stat(ui: &mut egui::Ui, w: f32, value: String, label: &str) {
    ui.allocate_ui(egui::vec2(w, 32.0), |ui| {
        ui.spacing_mut().item_spacing.y = 5.0;
        ui.label(RichText::new(value).font(theme::mono(18.0)).color(theme::TEXT));
        caps_tracked(ui, label, 9.0, 0.1, theme::TEXT_FAINT);
    });
}

fn labelled_bar(ui: &mut egui::Ui, label: &str, frac: f32, col: Color32) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).font(theme::sans(11.0)).color(theme::TEXT_DIM));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(format!("{:.0}%", frac * 100.0)).font(theme::mono(11.0)).color(theme::TEXT));
        });
    });
    ui.add_space(5.0);
    bar(ui, frac, col);
}

/// Inline diversity field list (mockup wrap), 4 fields — `nutrient` is omitted on screen.
fn diversity_row(ui: &mut egui::Ui, l: &super::LifeStats) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(16.0, 7.0);
        let field = |ui: &mut egui::Ui, name: &str, val: String| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                ui.label(RichText::new(name).font(theme::sans(11.5)).color(theme::TEXT_DIM));
                ui.label(RichText::new(val).font(theme::mono(11.5)).color(theme::TEXT));
            });
        };
        field(ui, "species", format!("{}", l.species));
        field(ui, "niches", format!("{}", l.niches));
        field(ui, "allopatry", fmt2(l.allop));
        field(ui, "crypsis", fmt2(l.crypsis));
    });
}

fn strata_bar(ui: &mut egui::Ui, cache: &mut HudCache, strata: [f32; 4]) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 10.0), Sense::hover());
    let cols = [
        theme::STRATA_UNDER,
        theme::STRATA_SURF,
        theme::STRATA_AIR,
        theme::STRATA_WATER,
    ];
    // Rounded ends via the texture-mask helper: build the segment colour profile at 256 columns
    // (NEAREST → crisp segment edges) and let the rounded RectShape clip the corners.
    const N: usize = 256;
    let mut acc = [0.0f32; 5];
    for i in 0..4 {
        acc[i + 1] = acc[i] + strata[i].clamp(0.0, 1.0);
    }
    let total = acc[4].max(1e-3);
    let profile: Vec<Color32> = (0..N)
        .map(|j| {
            let f = (j as f32 + 0.5) / N as f32 * total;
            let seg = (0..4).find(|&i| f < acc[i + 1]).unwrap_or(3);
            cols[seg]
        })
        .collect();
    rounded_bar(ui, cache, rect, &profile, false);
    ui.add_space(7.0);
    ui.horizontal(|ui| {
        let labels = [
            ("under", strata[0]),
            ("surf", strata[1]),
            ("air", strata[2]),
            ("water", strata[3]),
        ];
        let cw = ui.available_width() / 4.0;
        for (name, f) in labels {
            ui.allocate_ui(egui::vec2(cw, 12.0), |ui| {
                ui.label(
                    RichText::new(format!("{name} {:.0}", f * 100.0))
                        .font(theme::mono(9.5))
                        .color(theme::straight(233, 236, 230, 128)), // .50
                );
            });
        }
    });
}

fn perf_panel(ui: &mut egui::Ui, m: &SimMetrics) {
    flyout_header(ui, "Performance");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(RichText::new(format!("{:.0}", m.fps)).font(theme::mono(26.0)).color(theme::TEXT));
        ui.label(
            RichText::new(format!("fps · {:.1} ms", m.frame_ms))
                .font(theme::mono(11.0))
                .color(theme::straight(233, 236, 230, 128)),
        );
    });
    hairline(ui);
    ui.spacing_mut().item_spacing.y = 9.0;
    kv(ui, "draws", format!("{}", m.drawn));
    kv(ui, "chunks · detail", format!("{}", m.detail));
    kv(ui, "chunks · coarse", format!("{}", m.coarse));
    kv(ui, "on-screen", format!("{}", m.on_screen));
}
