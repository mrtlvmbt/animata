//! DNA-based artificial life simulation — entry point, rendering, input.
//!
//! Creatures carry an ACGT genome that decodes into body traits and the weights
//! of a small neural-net brain. They seek food, spend energy, split when fed and
//! die when starved. Over time mutation + selection shift the population's genes.

mod behavior;
mod biome;
mod body;
mod brain;
mod config;
mod creature;
mod genome;
mod grid;
mod phylo;
mod save;
mod speciation;
mod stats;
mod world;

use behavior::DEFAULT_BEHAVIOR;
use config::*;
use macroquad::prelude::*;
use macroquad::ui::{hash, root_ui, widgets};
use stats::Snapshot;
use std::collections::HashMap;
use std::io::Write as _;
use world::World;

const PANEL_H: f32 = 150.0;

/// How creatures are colored, cycled with `G`.
#[derive(Clone, Copy, PartialEq)]
enum ColorMode {
    Diet,
    Lineage,
    Species,
}

impl ColorMode {
    fn next(self) -> Self {
        match self {
            ColorMode::Diet => ColorMode::Lineage,
            ColorMode::Lineage => ColorMode::Species,
            ColorMode::Species => ColorMode::Diet,
        }
    }
    fn label(self) -> &'static str {
        match self {
            ColorMode::Diet => "diet",
            ColorMode::Lineage => "lineage",
            ColorMode::Species => "species",
        }
    }
}
const SAVE_PATH: &str = "life_save.txt";
const CSV_PATH: &str = "life_stats.csv";
const TREE_PATH: &str = "life_tree.csv";
/// LOD threshold: a creature rendering smaller than this many on-screen pixels is
/// drawn as a fixed-size dot instead of a heading triangle (overview/giant-map).
const LOD_POINT_PX: f32 = 2.5;

/// Pan/zoom view: maps world coordinates into the fixed on-screen viewport
/// (`VIEW_W`×`VIEW_H`), decoupled from the (larger) world size. Manual transform
/// so rendering stays in screen space (no camera/viewport/high-dpi fiddliness).
struct View {
    /// Zoom factor; at `fit_scale()` the whole world fits the viewport.
    scale: f32,
    /// World point shown at the center of the viewport.
    center: Vec2,
}

impl View {
    /// Scale at which the entire world fits inside the viewport.
    fn fit_scale() -> f32 {
        (VIEW_W / WORLD_W).min(VIEW_H / WORLD_H)
    }

    fn new() -> Self {
        View { scale: Self::fit_scale(), center: vec2(WORLD_W * 0.5, WORLD_H * 0.5) }
    }

    fn viewport_half() -> Vec2 {
        vec2(VIEW_W * 0.5, VIEW_H * 0.5)
    }

    fn world_to_screen(&self, w: Vec2) -> Vec2 {
        (w - self.center) * self.scale + Self::viewport_half()
    }

    fn screen_to_world(&self, s: Vec2) -> Vec2 {
        (s - Self::viewport_half()) / self.scale + self.center
    }

    /// Clamp zoom to [fit, 12] and keep the visible rectangle inside the world.
    fn clamp(&mut self) {
        self.scale = self.scale.clamp(Self::fit_scale(), 12.0);
        let hx = VIEW_W * 0.5 / self.scale;
        let hy = VIEW_H * 0.5 / self.scale;
        self.center.x = if hx * 2.0 >= WORLD_W { WORLD_W * 0.5 } else { self.center.x.clamp(hx, WORLD_W - hx) };
        self.center.y = if hy * 2.0 >= WORLD_H { WORLD_H * 0.5 } else { self.center.y.clamp(hy, WORLD_H - hy) };
    }
}

fn window_conf() -> Conf {
    Conf {
        window_title: "life — DNA evolution sim".to_owned(),
        window_width: VIEW_W as i32,
        window_height: (VIEW_H + PANEL_H) as i32,
        high_dpi: true,
        ..Default::default()
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    let mut seed_counter: u64 = 1;
    let mut behavior = DEFAULT_BEHAVIOR;
    let mut world = World::new(seed_counter, behavior);
    let mut paused = false;
    let mut speed: u32 = 1; // sim steps per frame
    // Transient on-screen notice (message, expiry time).
    let mut notice: Option<(String, f64)> = None;
    // Id of the creature being inspected, if any.
    let mut selected: Option<u64> = None;
    // Live-tunable params (owned here; copied into the world each frame so they
    // survive a load) and whether the tuning panel is open.
    let mut params = Params::default();
    let mut tune_open = false;
    // Camera (pan/zoom) and middle-drag panning state.
    let mut view = View::new();
    let mut pan_anchor: Option<Vec2> = None;
    // Optional CSV stats log (toggled with O) and the last tick written.
    let mut csv: Option<std::fs::File> = None;
    let mut last_logged: u64 = u64::MAX;
    // How creatures are colored (diet tint / lineage clade / detected species).
    let mut color_mode = ColorMode::Diet;
    // Show the coalescent phylogeny tree overlay.
    let mut show_tree = false;
    // Show the per-diet trait breakdown panel.
    let mut show_diet = false;
    // Show the Muller plot (stacked lineage shares over time).
    let mut show_muller = false;
    // Cached static biome background texture, rebuilt when the biome seed changes.
    let mut biome_tex: Option<Texture2D> = None;
    let mut biome_tex_seed = u64::MAX;

    loop {
        // ---- Input ----
        if is_key_pressed(KeyCode::Space) {
            paused = !paused;
        }
        if is_key_pressed(KeyCode::Up) || is_key_pressed(KeyCode::Equal) {
            speed = (speed + 1).min(40);
        }
        if is_key_pressed(KeyCode::Down) || is_key_pressed(KeyCode::Minus) {
            speed = speed.saturating_sub(1).max(1);
        }
        if is_key_pressed(KeyCode::R) {
            seed_counter += 1;
            world = World::new(seed_counter, behavior);
        }
        if is_key_pressed(KeyCode::B) {
            // Swap behavior strategy; needs a fresh world to rebuild minds.
            behavior = behavior.next();
            seed_counter += 1;
            world = World::new(seed_counter, behavior);
        }
        if is_key_pressed(KeyCode::T) {
            tune_open = !tune_open;
        }
        if is_key_pressed(KeyCode::C) {
            view = View::new();
        }
        if is_key_pressed(KeyCode::G) {
            color_mode = color_mode.next();
        }
        if is_key_pressed(KeyCode::P) {
            show_tree = !show_tree;
        }
        if is_key_pressed(KeyCode::D) {
            show_diet = !show_diet;
        }
        if is_key_pressed(KeyCode::M) {
            show_muller = !show_muller;
        }
        if is_key_pressed(KeyCode::Y) {
            let msg = match world.ancestry.export_csv(TREE_PATH) {
                Ok(()) => format!("tree ({} nodes) -> {TREE_PATH}", world.ancestry.len()),
                Err(e) => format!("tree export failed: {e}"),
            };
            notice = Some((msg, get_time() + 2.5));
        }
        if is_key_pressed(KeyCode::O) {
            let msg = if csv.is_some() {
                csv = None;
                "csv logging off".to_string()
            } else {
                match std::fs::File::create(CSV_PATH) {
                    Ok(mut f) => {
                        let _ = writeln!(f, "tick,population,herbivores,carnivores,avg_speed,avg_sense,avg_radius,avg_metabolism,avg_carnivory,diversity,lineages,max_generation");
                        csv = Some(f);
                        last_logged = u64::MAX;
                        format!("csv logging -> {CSV_PATH}")
                    }
                    Err(e) => format!("csv failed: {e}"),
                }
            };
            notice = Some((msg, get_time() + 2.5));
        }

        // ---- Camera: zoom to cursor + middle-drag pan ----
        let mouse = Vec2::from(mouse_position());
        let (_, wheel) = mouse_wheel();
        if wheel != 0.0 && mouse.y < VIEW_H {
            let before = view.screen_to_world(mouse);
            view.scale *= 1.0 + wheel.signum() * 0.15;
            view.clamp();
            let after = view.screen_to_world(mouse);
            view.center += before - after; // keep the point under the cursor fixed
            view.clamp();
        }
        if is_mouse_button_down(MouseButton::Middle) && mouse.y < VIEW_H {
            if let Some(prev) = pan_anchor {
                view.center -= (mouse - prev) / view.scale;
                view.clamp();
            }
            pan_anchor = Some(mouse);
        } else {
            pan_anchor = None;
        }
        if is_key_pressed(KeyCode::S) {
            let msg = match save::save(&world, SAVE_PATH) {
                Ok(()) => format!("saved -> {SAVE_PATH}"),
                Err(e) => format!("save failed: {e}"),
            };
            notice = Some((msg, get_time() + 2.5));
        }
        if is_key_pressed(KeyCode::L) {
            let msg = match save::load(SAVE_PATH) {
                Ok(w) => {
                    behavior = w.behavior;
                    world = w;
                    format!("loaded <- {SAVE_PATH}")
                }
                Err(e) => format!("load failed: {e}"),
            };
            notice = Some((msg, get_time() + 2.5));
        }
        if is_mouse_button_pressed(MouseButton::Left) && mouse.y < VIEW_H {
            selected = pick_creature(&world, view.screen_to_world(mouse));
        }
        if is_mouse_button_down(MouseButton::Right) && mouse.y < VIEW_H {
            world.add_food_at(view.screen_to_world(mouse));
        }

        // ---- Update ----
        // Tuning panel mutates our params; push them into the world so a load
        // (which resets world.params) doesn't lose the user's settings.
        if tune_open {
            draw_tuning(&mut params);
        }
        world.params = params;
        if !paused {
            for _ in 0..speed {
                world.step();
                if world.creatures.is_empty() {
                    break; // extinction; freeze until reset
                }
            }
        }

        // Append a CSV row once per recorded snapshot (every 5 ticks).
        if let Some(f) = &mut csv {
            if world.tick % 5 == 0 && world.tick != last_logged {
                let s = world.stats.latest();
                let _ = writeln!(
                    f,
                    "{},{},{},{},{:.4},{:.2},{:.3},{:.4},{:.4},{:.4},{},{}",
                    world.tick, s.population, s.herbivores, s.predators, s.avg_speed,
                    s.avg_sense, s.avg_radius, s.avg_metabolism, s.avg_carnivory,
                    s.diversity, s.lineages, s.max_generation
                );
                last_logged = world.tick;
            }
        }

        // ---- Render ----
        // Rebuild the static biome texture only when the map changes (reset/load).
        if biome_tex_seed != world.biome_seed {
            biome_tex = Some(build_biome_texture(&world));
            biome_tex_seed = world.biome_seed;
        }
        clear_background(Color::new(0.06, 0.07, 0.09, 1.0));
        draw_biome_texture(biome_tex.as_ref().unwrap(), &view);
        // All food + creatures in a single batched mesh (one draw call).
        let drawn = draw_entities(&world, &view, color_mode);
        // Subtle parched tint over the world during a drought.
        if world.in_drought() {
            draw_rectangle(0.0, 0.0, VIEW_W, VIEW_H, Color::new(0.55, 0.4, 0.12, 0.12));
        }
        draw_signals(&world, &view);
        // Inspector: ring the selected creature and show its detail panel.
        // Drop the selection if that creature has died.
        if let Some(id) = selected {
            match world.creature_by_id(id) {
                Some(c) => draw_selection_ring(c, &view),
                None => selected = None,
            }
        }
        draw_panel(&world);
        draw_hud(&world, paused, speed, drawn, color_mode);
        if let Some(id) = selected {
            if let Some(c) = world.creature_by_id(id) {
                draw_inspector(c, &world.ancestry);
            }
        }
        if show_tree {
            draw_tree(&world);
        }
        if show_diet {
            draw_diet_breakdown(&world);
        }
        if show_muller {
            draw_muller(&world);
        }

        // Transient save/load notice.
        if let Some((msg, until)) = &notice {
            if get_time() < *until {
                draw_text(msg, 10.0, 90.0, 22.0, Color::new(0.5, 1.0, 0.6, 1.0));
            } else {
                notice = None;
            }
        }

        next_frame().await;
    }
}

/// Build the static biome tint as a texture: one texel per ~36px cell (Nearest
/// filter keeps the blocky look). Rebuilt only when the biome seed changes.
fn build_biome_texture(world: &World) -> Texture2D {
    let cell = 36.0;
    let tw = (WORLD_W / cell).ceil() as u16;
    let th = (WORLD_H / cell).ceil() as u16;
    let mut img = Image::gen_image_color(tw, th, Color::new(0.0, 0.0, 0.0, 1.0));
    for ty in 0..th {
        for tx in 0..tw {
            let wx = tx as f32 * cell + cell * 0.5;
            let wy = ty as f32 * cell + cell * 0.5;
            let (r, g, b) = world.biome.props_at(vec2(wx, wy)).tint;
            img.set_pixel(tx as u32, ty as u32, Color::new(r, g, b, 1.0));
        }
    }
    let tex = Texture2D::from_image(&img);
    tex.set_filter(FilterMode::Nearest);
    tex
}

/// One textured quad for the whole biome map, transformed by the camera.
fn draw_biome_texture(tex: &Texture2D, view: &View) {
    let origin = view.world_to_screen(vec2(0.0, 0.0));
    draw_texture_ex(
        tex,
        origin.x,
        origin.y,
        Color::new(1.0, 1.0, 1.0, 0.5), // same translucency as before
        DrawTextureParams {
            dest_size: Some(vec2(WORLD_W * view.scale, WORLD_H * view.scale)),
            ..Default::default()
        },
    );
}

/// Draw all food + creatures as a single batched mesh (one draw call). Food are
/// quads (cheaper than tessellated circles), creatures are triangles. Off-screen
/// entities are culled. Returns the number of entities drawn.
/// Food color along the flavor/niche axis (0..1): sandy desert -> green plains
/// -> deep forest -> teal swamp, so the food-type map reads at a glance.
fn flavor_color(f: f32) -> Color {
    let stops = [
        (0.12, (0.80, 0.72, 0.35)), // desert
        (0.38, (0.45, 0.80, 0.35)), // plains
        (0.64, (0.20, 0.70, 0.30)), // forest
        (0.88, (0.25, 0.75, 0.80)), // swamp
    ];
    let f = f.clamp(stops[0].0, stops[3].0);
    for w in stops.windows(2) {
        let (fa, ca) = w[0];
        let (fb, cb) = w[1];
        if f <= fb {
            let t = (f - fa) / (fb - fa);
            return Color::new(
                ca.0 + (cb.0 - ca.0) * t,
                ca.1 + (cb.1 - ca.1) * t,
                ca.2 + (cb.2 - ca.2) * t,
                1.0,
            );
        }
    }
    let (_, c) = stops[3];
    Color::new(c.0, c.1, c.2, 1.0)
}

fn draw_entities(world: &World, view: &View, mode: ColorMode) -> usize {
    let m = 24.0; // cull margin
    let on_screen = |p: Vec2| p.x >= -m && p.x <= VIEW_W + m && p.y >= -m && p.y <= VIEW_H + m;
    let uv = Vec2::ZERO;
    let mut drawn = 0;

    // Batched into a reused mesh, flushed in chunks: a single draw_mesh that
    // exceeds macroquad's per-draw geometry limit (10000 verts / 5000 indices)
    // gets clamped (dropped), so cap each chunk under both — indices bind first.
    const MAX_V: usize = 9000;
    const MAX_I: usize = 4800;
    let mut mesh = Mesh {
        vertices: Vec::with_capacity(MAX_V + 4),
        indices: Vec::with_capacity(MAX_V * 2),
        texture: None,
    };
    let flush = |mesh: &mut Mesh| {
        if !mesh.vertices.is_empty() {
            draw_mesh(mesh);
            mesh.vertices.clear();
            mesh.indices.clear();
        }
    };

    // Food: small quads, tinted by flavor (the niche/food-type map).
    let fr = (FOOD_RADIUS * view.scale).max(1.0);
    for (f, &fl) in world.food.iter().zip(&world.flavor) {
        let s = view.world_to_screen(*f);
        if !on_screen(s) {
            continue;
        }
        let food_c = flavor_color(fl);
        if mesh.vertices.len() + 4 > MAX_V || mesh.indices.len() + 6 > MAX_I {
            flush(&mut mesh);
        }
        let base = mesh.vertices.len() as u16;
        for (dx, dy) in [(-fr, -fr), (fr, -fr), (fr, fr), (-fr, fr)] {
            mesh.vertices.push(Vertex::new(s.x + dx, s.y + dy, 0.0, uv.x, uv.y, food_c));
        }
        mesh.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        drawn += 1;
    }

    // Creatures: triangles, colored by diet / lineage / species.
    let lerp = |a: f32, t: f32, w: f32| a + (t - a) * w;
    for cr in &world.creatures {
        let center = view.world_to_screen(cr.pos);
        if !on_screen(center) {
            continue;
        }
        let (r, g, b) = cr.pheno.color;
        let c = cr.carnivory();
        let color = match mode {
            ColorMode::Lineage => lineage_color(cr.lineage),
            ColorMode::Species => lineage_color(cr.species_id),
            ColorMode::Diet => Color::new(
                lerp(r.max(0.25), 0.95, c),
                lerp(g.max(0.25), 0.2, c),
                lerp(b.max(0.25), 0.2, c),
                1.0,
            ),
        };
        let size = cr.pheno.radius * lerp(1.7, 2.2, c) * view.scale;
        // LOD: below ~LOD_POINT_PX a heading triangle is sub-pixel (invisible) and
        // costs trig per creature. At overview/giant-map zoom draw a fixed-size dot
        // instead — the "simplified overview". (Phase 0 render seam; later the
        // near-zoom branch grows the full segmented body here.)
        if size < LOD_POINT_PX {
            let r = LOD_POINT_PX * 0.5;
            if mesh.vertices.len() + 4 > MAX_V || mesh.indices.len() + 6 > MAX_I {
                flush(&mut mesh);
            }
            let base = mesh.vertices.len() as u16;
            for (dx, dy) in [(-r, -r), (r, -r), (r, r), (-r, r)] {
                mesh.vertices.push(Vertex::new(center.x + dx, center.y + dy, 0.0, uv.x, uv.y, color));
            }
            mesh.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            drawn += 1;
            continue;
        }
        // Segmented body: draw the chain as a row of quads (one per segment),
        // appendage-bearing segments tinted. Empty chain falls through to the
        // original heading triangle (single implicit segment).
        if !cr.pheno.segments.is_empty() {
            for (wc, wr, app) in cr.pheno.segment_layout(cr.pos, cr.heading) {
                let sc = view.world_to_screen(wc);
                let r = (wr * view.scale).max(1.0);
                let seg_color = appendage_tint(color, app);
                if mesh.vertices.len() + 4 > MAX_V || mesh.indices.len() + 6 > MAX_I {
                    flush(&mut mesh);
                }
                let base = mesh.vertices.len() as u16;
                for (dx, dy) in [(-r, -r), (r, -r), (r, r), (-r, r)] {
                    mesh.vertices.push(Vertex::new(sc.x + dx, sc.y + dy, 0.0, uv.x, uv.y, seg_color));
                }
                mesh.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            }
            drawn += 1;
            continue;
        }
        let nose = center + Vec2::from_angle(cr.heading) * size;
        let back_l = center + Vec2::from_angle(cr.heading + 2.4) * size * 0.8;
        let back_r = center + Vec2::from_angle(cr.heading - 2.4) * size * 0.8;
        if mesh.vertices.len() + 3 > MAX_V || mesh.indices.len() + 3 > MAX_I {
            flush(&mut mesh);
        }
        let base = mesh.vertices.len() as u16;
        mesh.vertices.push(Vertex::new(nose.x, nose.y, 0.0, uv.x, uv.y, color));
        mesh.vertices.push(Vertex::new(back_l.x, back_l.y, 0.0, uv.x, uv.y, color));
        mesh.vertices.push(Vertex::new(back_r.x, back_r.y, 0.0, uv.x, uv.y, color));
        mesh.indices.extend_from_slice(&[base, base + 1, base + 2]);
        drawn += 1;
    }

    flush(&mut mesh);
    drawn
}

/// Blend a creature's base color toward an appendage's signature tint, so a body
/// plan reads at a glance (fins aquatic-blue, wings pale, legs earthy, burrow dark).
fn appendage_tint(base: Color, app: genome::Appendage) -> Color {
    let (t, w) = match app {
        genome::Appendage::None => return base,
        genome::Appendage::Fin => ((0.30, 0.60, 0.95), 0.45),
        genome::Appendage::Wing => ((0.95, 0.95, 0.98), 0.45),
        genome::Appendage::Leg => ((0.70, 0.50, 0.30), 0.40),
        genome::Appendage::Burrow => ((0.35, 0.28, 0.22), 0.45),
    };
    Color::new(
        base.r + (t.0 - base.r) * w,
        base.g + (t.1 - base.g) * w,
        base.b + (t.2 - base.b) * w,
        1.0,
    )
}

/// Faint ring around creatures emitting a loud signal (a visible "call").
fn draw_signals(world: &World, view: &View) {
    for c in &world.creatures {
        let s = view.world_to_screen(c.pos);
        if s.x < 0.0 || s.x > VIEW_W || s.y < 0.0 || s.y > VIEW_H {
            continue;
        }
        if c.signal > 0.5 {
            let r = (c.pheno.radius * 2.0 * view.scale + 4.0).max(3.0);
            draw_circle_lines(s.x, s.y, r, 1.5, Color::new(1.0, 1.0, 1.0, 0.5 * c.signal));
        }
        // Infected hosts get a sickly magenta halo (hue keyed to the strain).
        if let Some(strain) = c.infection {
            let r = (c.pheno.radius * 1.6 * view.scale + 2.5).max(2.5);
            draw_circle_lines(s.x, s.y, r, 1.5, Color::new(0.85, 0.2, 0.7, 0.4 + 0.4 * strain));
        }
    }
}

/// Bottom panel with the live trend graph.
fn draw_panel(world: &World) {
    let top = VIEW_H;
    draw_rectangle(0.0, top, VIEW_W, PANEL_H, Color::new(0.03, 0.03, 0.05, 1.0));
    draw_line(0.0, top, VIEW_W, top, 1.0, Color::new(0.2, 0.2, 0.25, 1.0));

    let hist = &world.stats.history;
    if hist.len() < 2 {
        return;
    }
    let pad = 8.0;
    let gx = pad;
    let gw = VIEW_W - pad * 2.0;
    let gy = top + pad;
    let gh = PANEL_H - pad * 2.0;

    // Normalized trait curves (0..1) + populations scaled to caps.
    let series: [(Color, fn(&Snapshot) -> f32); 16] = [
        (Color::new(0.9, 0.9, 0.9, 1.0), |s| {
            (s.herbivores as f32 / POP_CAP as f32).min(1.0)
        }),
        (Color::new(0.95, 0.25, 0.25, 1.0), |s| {
            (s.predators as f32 / 200.0).min(1.0)
        }),
        (Color::new(0.95, 0.5, 0.3, 1.0), |s| {
            norm(s.avg_speed, SPEED_RANGE)
        }),
        (Color::new(0.4, 0.7, 1.0, 1.0), |s| {
            norm(s.avg_sense, SENSE_RANGE)
        }),
        (Color::new(0.8, 0.8, 0.3, 1.0), |s| {
            norm(s.avg_radius, RADIUS_RANGE)
        }),
        (Color::new(0.7, 0.45, 0.9, 1.0), |s| {
            norm(s.avg_metabolism, METAB_RANGE)
        }),
        // Diversity is a std-dev (0..~0.5); scale ×2 to use the panel height.
        (Color::new(0.55, 0.85, 0.95, 1.0), |s| (s.diversity * 2.0).min(1.0)),
        // Mean carnivory (0..1): the population's average diet.
        (Color::new(0.95, 0.35, 0.55, 1.0), |s| s.avg_carnivory),
        // Surviving founder lineages as a fraction of the initial count: drops
        // toward 0 as clades die out (coalescence).
        (Color::new(0.6, 0.95, 0.6, 1.0), |s| {
            s.lineages as f32 / (START_CREATURES + START_PREDATORS) as f32
        }),
        // Detected species count, scaled to a soft cap.
        (Color::new(1.0, 0.6, 0.2, 1.0), |s| (s.species as f32 / 20.0).min(1.0)),
        // Mean ornament (0..1): sexual-display trait.
        (Color::new(0.85, 0.5, 1.0, 1.0), |s| s.avg_ornament),
        // Mean emitted signal (0..1): communication / alarm calls.
        (Color::new(0.95, 0.95, 0.95, 1.0), |s| s.avg_signal),
        // Mean disease-resistance allele (0..1): the Red Queen's host side.
        (Color::new(0.4, 0.95, 0.8, 1.0), |s| s.avg_resistance),
        // Infected fraction (0..1): the pathogen's reach.
        (Color::new(0.7, 0.2, 0.5, 1.0), |s| s.infected_frac),
        // Realized recurrent-memory reliance (0..1): how much brains lean on
        // their hidden state vs current inputs while behaving.
        (Color::new(1.0, 0.85, 0.4, 1.0), |s| s.avg_memory),
        // Diet-niche spread (std-dev, ×3 to fill the panel): rises/goes bimodal
        // as the population splits into food specialists (ecological speciation).
        (Color::new(0.5, 0.9, 0.6, 1.0), |s| (s.niche_spread * 3.0).min(1.0)),
    ];

    let n = hist.len();
    for (color, get) in series {
        let mut prev: Option<(f32, f32)> = None;
        for (i, s) in hist.iter().enumerate() {
            let x = gx + gw * (i as f32 / (n - 1) as f32);
            let y = gy + gh * (1.0 - get(s).clamp(0.0, 1.0));
            if let Some((px, py)) = prev {
                draw_line(px, py, x, y, 1.4, color);
            }
            prev = Some((x, y));
        }
    }

    // Legend.
    let labels = [
        ("herb", Color::new(0.9, 0.9, 0.9, 1.0)),
        ("pred", Color::new(0.95, 0.25, 0.25, 1.0)),
        ("speed", Color::new(0.95, 0.5, 0.3, 1.0)),
        ("sense", Color::new(0.4, 0.7, 1.0, 1.0)),
        ("size", Color::new(0.8, 0.8, 0.3, 1.0)),
        ("metab", Color::new(0.7, 0.45, 0.9, 1.0)),
        ("div", Color::new(0.55, 0.85, 0.95, 1.0)),
        ("diet", Color::new(0.95, 0.35, 0.55, 1.0)),
        ("clade", Color::new(0.6, 0.95, 0.6, 1.0)),
        ("spec", Color::new(1.0, 0.6, 0.2, 1.0)),
        ("ornm", Color::new(0.85, 0.5, 1.0, 1.0)),
        ("sig", Color::new(0.95, 0.95, 0.95, 1.0)),
        ("resist", Color::new(0.4, 0.95, 0.8, 1.0)),
        ("infect", Color::new(0.7, 0.2, 0.5, 1.0)),
        ("mem", Color::new(1.0, 0.85, 0.4, 1.0)),
        ("niche", Color::new(0.5, 0.9, 0.6, 1.0)),
    ];
    let mut lx = gx + 4.0;
    for (label, color) in labels {
        draw_text(label, lx, top + 16.0, 16.0, color);
        lx += measure_text(label, None, 16, 1.0).width + 14.0;
    }
}

fn draw_hud(world: &World, paused: bool, speed: u32, drawn: usize, color_mode: ColorMode) {
    let s = world.stats.latest();
    let white = Color::new(0.92, 0.92, 0.92, 1.0);
    let env = if world.in_drought() {
        "DROUGHT"
    } else if world.season_phase() > 0.15 {
        "bounty"
    } else if world.season_phase() < -0.15 {
        "lean"
    } else {
        "mild"
    };
    let lines = [
        format!("pop {} (herb {} / pred {})   tick {}   gen {}   clades {}   species {}   brain: {}", s.population, s.herbivores, s.predators, world.tick, s.max_generation, s.lineages, s.species, world.behavior.label()),
        format!("fps {}   {:.1} ms   drawn {}   speed x{}   color: {}   env: {}{}", get_fps(), get_frame_time() * 1000.0, drawn, speed, color_mode.label(), env, if paused { "  [PAUSED]" } else { "" }),
        format!(
            "avg  speed {:.2}  sense {:.0}  size {:.1}  metab {:.2}   diversity {:.3}   resist {:.2}  infected {:.0}%   niche {:.2} (spread {:.2})",
            s.avg_speed, s.avg_sense, s.avg_radius, s.avg_metabolism, s.diversity, s.avg_resistance, s.infected_frac * 100.0, s.avg_niche, s.niche_spread
        ),
    ];
    let mut y = 20.0;
    for line in lines {
        draw_text(&line, 10.0, y, 20.0, white);
        y += 22.0;
    }
    if world.creatures.is_empty() {
        draw_text(
            "EXTINCTION - press R to restart",
            VIEW_W * 0.5 - 150.0,
            VIEW_H * 0.5,
            28.0,
            Color::new(1.0, 0.4, 0.4, 1.0),
        );
    }
    draw_text(
        "Space pause  Up/Down speed  R reset  B brain  T tune  S/L save/load  O csv  G color  Y export  P tree  D diet  M muller  wheel zoom  mid-drag pan  C recenter  L-click inspect  R-click food",
        10.0,
        VIEW_H - 12.0,
        16.0,
        Color::new(0.6, 0.6, 0.65, 1.0),
    );
}

/// Live-tuning panel (immediate-mode UI). Mutates `params` in place.
fn draw_tuning(params: &mut Params) {
    widgets::Window::new(hash!(), vec2(VIEW_W - 250.0, 110.0), vec2(238.0, 130.0))
        .label("tuning (T)")
        .titlebar(true)
        .ui(&mut root_ui(), |ui| {
            ui.slider(hash!(), "food/step", 0.0..6.0, &mut params.food_per_step);
            ui.slider(hash!(), "pred gain", 0.0..120.0, &mut params.predator_gain);
            let mut mr = params.mutation_rate as f32;
            ui.slider(hash!(), "mut rate", 0.0..0.05, &mut mr);
            params.mutation_rate = mr as f64;
        });
}

/// Nearest creature to `p` within a small pick radius, by id.
fn pick_creature(world: &World, p: Vec2) -> Option<u64> {
    let mut best: Option<(u64, f32)> = None;
    for c in &world.creatures {
        let scale = 1.7 + 0.5 * c.carnivory();
        let pick = (c.pheno.radius * scale + 6.0).max(10.0);
        let d2 = (c.pos - p).length_squared();
        if d2 <= pick * pick && best.map_or(true, |(_, bd)| d2 < bd) {
            best = Some((c.id, d2));
        }
    }
    best.map(|(id, _)| id)
}

fn draw_selection_ring(c: &creature::Creature, view: &View) {
    let shape = 1.7 + 0.5 * c.carnivory();
    let center = view.world_to_screen(c.pos);
    let r = c.pheno.radius * shape * view.scale + 5.0;
    draw_circle_lines(center.x, center.y, r, 2.0, Color::new(1.0, 0.95, 0.4, 1.0));
}

/// Right-side overlay describing the selected creature: stats, traits, brain
/// diagram and genome.
fn draw_inspector(c: &creature::Creature, anc: &phylo::Ancestry) {
    let w = 330.0;
    let x = VIEW_W - w;
    let h = 670.0;
    draw_rectangle(x, 0.0, w, h, Color::new(0.04, 0.05, 0.07, 0.92));
    draw_line(x, 0.0, x, h, 1.0, Color::new(0.3, 0.3, 0.36, 1.0));

    let tx = x + 12.0;
    let white = Color::new(0.92, 0.92, 0.94, 1.0);
    let dim = Color::new(0.62, 0.64, 0.7, 1.0);
    let species = match c.diet() {
        creature::Diet::Herbivore => "herbivore",
        creature::Diet::Omnivore => "omnivore",
        creature::Diet::Carnivore => "carnivore",
    };
    let parent = c.parent_id.map(|p| p.to_string()).unwrap_or_else(|| "-".into());
    let lines = [
        format!("creature #{}", c.id),
        format!("{species}   brain: {}", c.kind.label()),
        format!("parent: {parent}   gen: {}   lineage: {}   species: {}", c.generation, c.lineage, c.species_id),
        {
            // Ancestor chain (nearest first), from the full ancestry log.
            let chain = anc.ancestors(c.id, 4);
            let depth = anc.depth(c.id);
            let ids: Vec<String> = chain.iter().map(|a| a.to_string()).collect();
            let tail = if depth > chain.len() { " ..." } else { "" };
            format!("ancestors({depth}): {}{tail}", ids.join(" <- "))
        },
        format!("age: {}   energy: {:.1}", c.age, c.energy),
        format!(
            "longevity: {:.0}  mature@{:.0}  senesc {:.0}%",
            c.pheno.prime,
            c.pheno.prime * MATURITY_FRAC,
            c.senescence() * 100.0
        ),
        format!("genome: {} nt", c.genome.nt.len()),
        String::new(),
        format!("speed  {:.2}", c.pheno.max_speed),
        format!("sense  {:.0}", c.pheno.sense_range),
        format!("size   {:.1}", c.pheno.radius),
        format!("metab  {:.2}", c.pheno.metabolism),
        format!("carnivory  {:.2}", c.carnivory()),
        format!("ornament {:.2}  pref {:.2}", c.pheno.ornament, c.pheno.preference),
        format!("signal {:.2}", c.signal),
        format!("resistance {:.2}", c.pheno.resistance),
        match c.infection {
            Some(s) => format!("INFECTED  strain {:.2}", s),
            None => "healthy".into(),
        },
        format!("memory use {:.2}  (cap {:.2})", c.memory_use(), c.pheno.recurrent_gain()),
        format!("mem leak gamma {:.2}", c.pheno.leak),
        format!("diet niche {:.2}", c.pheno.diet_niche),
    ];
    let mut y = 22.0;
    for line in &lines {
        draw_text(line, tx, y, 18.0, white);
        y += 21.0;
    }

    // Color swatch (the creature's gene color).
    let (r, g, b) = c.pheno.color;
    draw_text("color", tx, y, 18.0, dim);
    draw_rectangle(tx + 56.0, y - 13.0, 18.0, 14.0, Color::new(r, g, b, 1.0));
    y += 16.0;

    draw_text("brain (inputs -> hidden -> outputs)", tx, y, 16.0, dim);
    draw_brain(c, x + 14.0, y + 8.0, w - 28.0, 120.0);

    // Genome, wrapped to a few lines.
    let dna = c.genome.to_string();
    let per_line = 40;
    let mut gy = h - 78.0;
    draw_text("DNA", tx, gy, 16.0, dim);
    gy += 16.0;
    for chunk in dna.as_bytes().chunks(per_line).take(4) {
        let s = std::str::from_utf8(chunk).unwrap_or("");
        draw_text(s, tx, gy, 14.0, Color::new(0.55, 0.8, 0.6, 1.0));
        gy += 15.0;
    }
}

/// Node-link diagram of the brain using the decoded weights. Lines are teal for
/// positive weights, orange for negative, with opacity by magnitude.
fn draw_brain(c: &creature::Creature, x: f32, y: f32, w: f32, h: f32) {
    let cols = [NN_INPUTS, NN_HIDDEN, NN_OUTPUTS];
    let col_x = [x, x + w * 0.5, x + w];
    let node = |col: usize, i: usize| -> Vec2 {
        let n = cols[col];
        let step = h / (n as f32 + 1.0);
        Vec2::new(col_x[col], y + step * (i as f32 + 1.0))
    };

    // Rebuild the dense weight matrices from the synapse list (same routing as
    // the brain builder) so the inspector can draw the connection strengths.
    let mut w_ih = vec![0.0f32; NN_INPUTS * NN_HIDDEN];
    let mut w_ho = vec![0.0f32; NN_HIDDEN * NN_OUTPUTS];
    for s in &c.pheno.synapses {
        let (src, dst) = (s.src as usize, s.dst as usize);
        if src < NN_INPUTS {
            if dst < NN_HIDDEN {
                w_ih[dst * NN_INPUTS + src] += s.w;
            }
        } else if dst >= NN_HIDDEN {
            w_ho[(dst - NN_HIDDEN) * NN_HIDDEN + (src - NN_INPUTS)] += s.w;
        }
    }
    let edge = |wgt: f32| {
        let a = (wgt.abs() / WEIGHT_SCALE).clamp(0.05, 1.0);
        if wgt >= 0.0 {
            Color::new(0.3, 0.85, 0.8, a)
        } else {
            Color::new(0.95, 0.55, 0.3, a)
        }
    };

    // input -> hidden
    for hdn in 0..NN_HIDDEN {
        for inp in 0..NN_INPUTS {
            let a = node(0, inp);
            let bn = node(1, hdn);
            draw_line(a.x, a.y, bn.x, bn.y, 1.0, edge(w_ih[hdn * NN_INPUTS + inp]));
        }
    }
    // hidden -> output
    for out in 0..NN_OUTPUTS {
        for hdn in 0..NN_HIDDEN {
            let a = node(1, hdn);
            let bn = node(2, out);
            draw_line(a.x, a.y, bn.x, bn.y, 1.0, edge(w_ho[out * NN_HIDDEN + hdn]));
        }
    }
    // nodes on top
    for (col, &n) in cols.iter().enumerate() {
        for i in 0..n {
            let p = node(col, i);
            draw_circle(p.x, p.y, 3.0, Color::new(0.9, 0.9, 0.95, 1.0));
        }
    }
}

fn norm(v: f32, range: (f32, f32)) -> f32 {
    ((v - range.0) / (range.1 - range.0)).clamp(0.0, 1.0)
}

/// Overlay: the coalescent phylogeny of the living population — every living
/// creature's ancestor paths back to founders, drawn time (birth) left→right,
/// leaves spread vertically, edges colored by lineage.
fn draw_tree(world: &World) {
    let pad = 36.0;
    let x0 = pad;
    let y0 = pad + 14.0;
    let w = VIEW_W - pad * 2.0;
    let h = VIEW_H - pad * 2.0 - 14.0;
    draw_rectangle(0.0, 0.0, VIEW_W, VIEW_H, Color::new(0.03, 0.03, 0.05, 0.92));

    // Sample living creatures so huge populations stay legible/fast.
    let ids: Vec<u64> = world.creatures.iter().map(|c| c.id).collect();
    let step = (ids.len() / 400).max(1);
    let sample: Vec<u64> = ids.iter().step_by(step).copied().collect();
    let nodes = world.ancestry.coalescent(&sample);

    if nodes.len() < 2 {
        draw_text("phylogeny: not enough history yet", x0, y0, 20.0, Color::new(0.8, 0.8, 0.8, 1.0));
        return;
    }

    // Index nodes; build child lists.
    let mut info: HashMap<u64, &phylo::TreeNode> = HashMap::new();
    let mut children: HashMap<u64, Vec<u64>> = HashMap::new();
    for n in &nodes {
        info.insert(n.id, n);
        if let Some(p) = n.parent {
            children.entry(p).or_default().push(n.id);
        }
    }
    // X position by *rank* of birth (still time-ordered, but equal spacing) so the
    // recent radiation isn't crushed into a sliver while a sparse trunk hogs width.
    let mut order: Vec<(u64, u64)> = nodes.iter().map(|n| (n.birth, n.id)).collect();
    order.sort_unstable();
    let denom = (order.len() - 1).max(1) as f32;
    let mut x_rank: HashMap<u64, f32> = HashMap::new();
    for (rank, (_, id)) in order.iter().enumerate() {
        x_rank.insert(*id, rank as f32 / denom);
    }

    // Vertical layout: leaves get sequential rows, parents sit at their mean.
    let mut y_of: HashMap<u64, f32> = HashMap::new();
    let mut counter = 0.0f32;
    // Roots = founders (no parent) or nodes whose parent isn't in this set
    // (e.g. just after a prune) — so the layout always has something to start from.
    let roots: Vec<u64> = nodes
        .iter()
        .filter(|n| n.parent.map_or(true, |p| !info.contains_key(&p)))
        .map(|n| n.id)
        .collect();
    for &r in &roots {
        assign_y(r, &children, &mut y_of, &mut counter);
    }
    let leaves = counter.max(1.0);

    let sx = |id: u64| x0 + w * x_rank.get(&id).copied().unwrap_or(0.0);
    let sy = |id: u64| y0 + h * (y_of.get(&id).copied().unwrap_or(0.0) / leaves);

    // Edges parent→child, colored by the child's lineage.
    for n in &nodes {
        if let Some(p) = n.parent {
            if info.contains_key(&p) {
                let mut col = lineage_color(n.lineage);
                col.a = 0.5;
                draw_line(sx(p), sy(p), sx(n.id), sy(n.id), 1.0, col);
            }
        }
    }

    draw_text(
        &format!(
            "phylogeny: {} living sampled, {} ancestors, {} surviving founder lines   (P to close)",
            sample.len(),
            nodes.len(),
            roots.len()
        ),
        x0,
        pad,
        18.0,
        Color::new(0.85, 0.9, 0.95, 1.0),
    );
}

/// Post-order y assignment: leaves take the next row, internal nodes average
/// their children. Returns the node's row.
fn assign_y(id: u64, children: &HashMap<u64, Vec<u64>>, y_of: &mut HashMap<u64, f32>, counter: &mut f32) -> f32 {
    let y = match children.get(&id) {
        Some(kids) if !kids.is_empty() => {
            let mut sum = 0.0;
            for &k in kids {
                sum += assign_y(k, children, y_of, counter);
            }
            sum / kids.len() as f32
        }
        _ => {
            let v = *counter;
            *counter += 1.0;
            v
        }
    };
    y_of.insert(id, y);
    y
}

/// Overlay: Muller plot — each species' share of the population stacked over
/// time, colored by species. Bands appear/widen/vanish as species rise and die;
/// the gray top band is everything outside the tracked top species.
fn draw_muller(world: &World) {
    let hist = &world.stats.lineage_history;
    let pops = &world.stats.history;
    draw_rectangle(0.0, 0.0, VIEW_W, VIEW_H, Color::new(0.03, 0.03, 0.05, 0.92));
    if hist.len() < 2 {
        draw_text("muller plot: not enough history yet", 40.0, 50.0, 20.0, Color::new(0.8, 0.8, 0.8, 1.0));
        return;
    }
    let pad = 36.0;
    let x0 = pad;
    let y0 = pad + 14.0;
    let w = VIEW_W - pad * 2.0;
    let h = VIEW_H - pad * 2.0 - 14.0;
    let n = hist.len();
    let col_w = (w / (n - 1) as f32).max(1.0);

    for i in 0..n {
        let total = pops[i].population.max(1) as f32;
        let x = x0 + w * i as f32 / (n - 1) as f32;
        // Stable stacking order: by lineage id.
        let mut rows = hist[i].clone();
        rows.sort_unstable_by_key(|(l, _)| *l);
        let mut acc = 0.0f32; // cumulative fraction from the bottom
        for (lin, cnt) in rows {
            let frac = cnt as f32 / total;
            let y1 = y0 + h * acc;
            let y2 = y0 + h * (acc + frac);
            draw_line(x, y1, x, y2, col_w, lineage_color(lin));
            acc += frac;
        }
        // Remainder (untracked lineages) as a gray band on top.
        if acc < 1.0 {
            draw_line(x, y0 + h * acc, x, y0 + h, col_w, Color::new(0.3, 0.3, 0.33, 1.0));
        }
    }

    draw_text(
        "muller plot - lineage shares over time (left=old, right=now)   (M to close)",
        x0,
        pad,
        18.0,
        Color::new(0.85, 0.9, 0.95, 1.0),
    );
}

/// Panel: average traits split by diet class (herbivore / omnivore / carnivore),
/// computed live — shows e.g. whether carnivores evolve faster or larger.
fn draw_diet_breakdown(world: &World) {
    // [bucket] sums of (speed, sense, size, metab, carnivory) and counts.
    let mut sum = [[0.0f32; 5]; 3];
    let mut cnt = [0u32; 3];
    for c in &world.creatures {
        let b = match c.diet() {
            creature::Diet::Herbivore => 0,
            creature::Diet::Omnivore => 1,
            creature::Diet::Carnivore => 2,
        };
        cnt[b] += 1;
        let p = &c.pheno;
        sum[b][0] += p.max_speed;
        sum[b][1] += p.sense_range;
        sum[b][2] += p.radius;
        sum[b][3] += p.metabolism;
        sum[b][4] += c.carnivory();
    }

    let x = 10.0;
    let y = 108.0;
    let w = 360.0;
    let h = 104.0;
    draw_rectangle(x, y, w, h, Color::new(0.04, 0.05, 0.07, 0.9));
    draw_line(x, y, x + w, y, 1.0, Color::new(0.3, 0.3, 0.36, 1.0));
    let dim = Color::new(0.62, 0.64, 0.7, 1.0);
    let white = Color::new(0.9, 0.9, 0.94, 1.0);

    draw_text("diet breakdown   n   speed sense size metab carn", x + 8.0, y + 18.0, 16.0, dim);
    let names = ["herbivore", "omnivore ", "carnivore"];
    let mut ry = y + 40.0;
    for b in 0..3 {
        let n = cnt[b].max(1) as f32;
        let line = if cnt[b] == 0 {
            format!("{}    0    -", names[b])
        } else {
            format!(
                "{}  {:4}  {:.2}  {:3.0}  {:.1}  {:.2}  {:.2}",
                names[b], cnt[b], sum[b][0] / n, sum[b][1] / n, sum[b][2] / n, sum[b][3] / n, sum[b][4] / n
            )
        };
        draw_text(&line, x + 8.0, ry, 16.0, white);
        ry += 20.0;
    }
}

/// Distinct, stable hue per lineage id (golden-ratio spacing).
fn lineage_color(lineage: u32) -> Color {
    let h = (lineage as f32 * 0.618_034).fract();
    hsv(h, 0.65, 0.95)
}

fn hsv(h: f32, s: f32, v: f32) -> Color {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match (i as i32).rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    Color::new(r, g, b, 1.0)
}
