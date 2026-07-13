//! animata v2 renderer — R-ladder scaffold. R-2 (issue #223): the first REAL view — a hex-voxel
//! terrain mesh (`WorldView` → flat-top hex columns + cliff quads, biome-colored) under a minimal
//! fixed 3D iso camera, with R-1's creatures now projected into that same view as dots. R-1 (#219,
//! merged #220) built the seam: the worker-thread `Sim` driver, the read-only `RenderSnapshot`
//! double buffer, and a proof-of-life naive 2D projection — R-2 replaces that projection with the
//! real 3D hex view; the sim seam itself (driver.rs) is untouched.
//!
//! R-3 (merged #225): interactive pan/zoom/rotate IsoCam + box-frustum culling (terrain chunks +
//! creatures), minimal zoom-LOD, and the R-2 HMAX-literal footgun fix (cli consts now pub).
//!
//! R-4 (merged #227): creatures Tier-1 LOD — px_per_m-driven point/sphere/morphology tiers; snapshot
//! fields `size`/`uptake_layer`; consume R-3 dead-code warnings (render crate out of CI, local verify).
//!
//! R-5 (merged #228): cube-voxel toggle — a second terrain mesh builder (square columns vs hex prisms),
//! runtime key to switch hex↔cube, creature projection follows active layout. Golden-NEUTRAL (render-only).
//!
//! R-6 (merged #229): ProcgenWorld wiring — switches from the legacy `NoiseWorld` (f64 sin, arch-divergent)
//! to the full integer pipeline (W-1..W-6 reliefs + erosion + biome/edaphic + resource caps), enabling
//! hex-voxel visualization of the NOW-LIVE rich procedurally-generated world. Neutral read-only snapshot
//! consumer (render builds the same world the sim uses, no mutation path).
//!
//! R-7 (this slice): Biology coloring — creatures colored by uptake_layer (feeding guild) to visualize
//! A/B differentiation emergence. Layer 0 (A-guild) = orange; layer 1 (B-guild) = cyan; morphology
//! reflects cell_type (if available, from E-4 ontogenesis). HUD legend added. Render-only, golden-neutral.
//!
//! R-8 (this slice, #261): standalone hex-map viewer — `--standalone`/`--no-sim` skips spawning the
//! sim worker entirely (no `Sim`, no snapshot). Terrain/camera/culling/LOD are already sim-independent
//! (R-2/R-3/R-6); the only coupling point was the creature snapshot, which was already `Option`-typed
//! (`SimHandle::latest`) — standalone just keeps that `Option` at `None` for the whole run instead of
//! populating it from a worker thread. `--seed`/`--dim` are optional overrides (default: current pinned
//! values); `--dim` only takes effect in standalone (in sim mode it would desync the render's world from
//! the worker's, breaking the pinned-param contract below).
//!
//! Not part of the v2 CI workspace (`v2/Cargo.toml`'s `exclude`) — a leaf bin, verified LOCALLY:
//! `cargo build`/`cargo clippy` from this directory + a manual run (window opens, ProcgenWorld hex
//! terrain + colored creatures visible by feeding guild, HUD counts advance, T-key toggles cube terrain).

mod biome_palette;
mod camera;
mod driver;
mod dump_world;
mod hex;
mod terrain;
mod terrain_cube;

use camera::IsoCam;
use macroquad::prelude::*;
use sim_core::WorldView;
use std::time::Instant;

// ── R-4 LOD tier thresholds (px_per_m-driven) ──────────────────────────────────────────────────────
/// FAR tier: creatures are sub-pixel or nearly invisible (point/billboard). Triggers when px_per_m < 5.
/// At default 768px tall, this happens when ortho_span > ~154 world units (very far zoom).
const PX_PER_M_FAR_THRESHOLD: f32 = 5.0;

/// MID tier: creatures are cell-type-colored spheres (R-3 behavior). Active when 5 <= px_per_m < 20.
/// At default viewport, this is ortho_span in [38, 154] — a standard play range.
const PX_PER_M_MID_THRESHOLD: f32 = 20.0;

/// NEAR tier: creatures are minimal cell-type morphology (differentiated small shapes).
/// Triggers when px_per_m >= 20 (ortho_span <= ~38, zoomed in close).
fn window_conf() -> Conf {
    Conf {
        window_title: "animata v2 — render scaffold (R-8 standalone hex-map viewer)".to_owned(),
        window_width: 1024,
        window_height: 768,
        high_dpi: true,
        ..Default::default()
    }
}

/// The v2 demo/test seed used across the cli/telemetry suites — an arbitrary but consistent choice,
/// not load-bearing (the sim draws whatever the economy produces; the terrain is whatever this seed
/// generates).
const SEED: u64 = 0xA11A_2A11;

/// R-13 camera presets for deterministic evidence capture.
#[derive(Clone, Copy, Debug)]
enum CamPreset {
    /// Default isometric view: pitch ~41°, yaw 0°, centered, ortho_span 1.5x world span.
    IsoDefault,
    /// Zoomed-in isometric view: ortho_span 0.75x world span (close-up).
    IsoZoomClose,
    /// Zoomed-out isometric view: ortho_span 2.5x world span (wide view).
    IsoZoomFar,
}

impl CamPreset {
    /// Apply this preset to a camera, resetting it to a known state.
    fn apply_to_camera(&self, camera: &mut IsoCam, center: Vec3, world_span: f32) {
        match self {
            CamPreset::IsoDefault => {
                camera.focus = center;
                camera.yaw = 0.0;
                camera.ortho_span = world_span * 1.5;
            }
            CamPreset::IsoZoomClose => {
                camera.focus = center;
                camera.yaw = 0.0;
                camera.ortho_span = world_span * 0.75;
            }
            CamPreset::IsoZoomFar => {
                camera.focus = center;
                camera.yaw = 0.0;
                camera.ortho_span = world_span * 2.5;
            }
        }
    }
}

/// R-13 (#433): parsed CLI flags. `--standalone`/`--no-sim` are aliases for the same no-sim mode.
struct CliArgs {
    standalone: bool,
    seed: u64,
    /// Only honoured in standalone mode — see the module doc comment for why.
    dim_override: Option<i64>,
    /// `--v1-dump <path>`: draw a v1-generated world dump (ATDMP1) instead of `ProcgenWorld`, to
    /// compare v1 vs v2 worldgen in the SAME renderer. Implies standalone (no sim on a dump world).
    v1_dump: Option<String>,
    /// R-13: `--screenshot <path.png>`: render warmup frames and capture the framebuffer to PNG.
    screenshot: Option<String>,
    /// R-13: number of warmup frames before screenshot (default 30).
    screenshot_warmup: u32,
    /// R-13: `--bench`: run deterministic benchmark (300+ frames), print machine-readable line.
    bench: bool,
    /// R-13: camera preset (default: iso-default).
    cam_preset: CamPreset,
}

fn parse_args() -> CliArgs {
    let mut standalone = false;
    let mut seed = SEED;
    let mut dim_override = None;
    let mut v1_dump = None;
    let mut screenshot = None;
    let mut screenshot_warmup = 30;
    let mut bench = false;
    let mut cam_preset = CamPreset::IsoDefault;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--standalone" | "--no-sim" => standalone = true,
            "--v1-dump" => {
                v1_dump = Some(args.next().expect("--v1-dump requires a path"));
                standalone = true; // a dump world has no sim backend
            }
            "--seed" => {
                let v = args.next().expect("--seed requires a value");
                seed = v.parse().unwrap_or_else(|_| panic!("--seed expects a u64, got {v:?}"));
            }
            "--dim" => {
                let v = args.next().expect("--dim requires a value");
                dim_override = Some(v.parse().unwrap_or_else(|_| panic!("--dim expects an integer, got {v:?}")));
            }
            "--screenshot" => {
                screenshot = Some(args.next().expect("--screenshot requires a path"));
                standalone = true; // screenshot mode is standalone
            }
            "--screenshot-warmup" => {
                let v = args.next().expect("--screenshot-warmup requires a value");
                screenshot_warmup = v.parse().unwrap_or_else(|_| panic!("--screenshot-warmup expects a u32, got {v:?}"));
            }
            "--bench" => {
                bench = true;
                standalone = true; // benchmark mode is standalone
            }
            "--cam" => {
                let preset_name = args.next().expect("--cam requires a preset name");
                cam_preset = match preset_name.as_str() {
                    "iso-default" => CamPreset::IsoDefault,
                    "iso-zoom-close" => CamPreset::IsoZoomClose,
                    "iso-zoom-far" => CamPreset::IsoZoomFar,
                    other => panic!("unknown camera preset: {other:?}"),
                };
            }
            other => eprintln!("render: ignoring unknown arg {other:?}"),
        }
    }
    CliArgs { standalone, seed, dim_override, v1_dump, screenshot, screenshot_warmup, bench, cam_preset }
}

// ── Pinned-param contract (W-6 WIRE: ProcgenWorld; critic F3, issue #223 acceptance; R-6) ──────────
//
// The render's `WorldView` MUST resolve to the SAME terrain the sim worker runs on. `ProcgenWorld` is
// a pure function of `(world_dim, hmax, resource_base, seed)` — `cli::build_sim` constructs it as
// `ProcgenWorld::new(econ.world_dim, HMAX, econ.resource_base, config.seed ^ WORLD_SALT, .., false)` (`cli/src/lib.rs`).
// W-6 wiring makes `HMAX`, `RESOURCE_BASE`, `WORLD_SALT` all `pub` in `cli`, so this file IMPORTS them
// directly from `cli` rather than duplicating literals. This ensures the render's world matches the sim's
// — no divergence via stale consts (the R-2 footgun this contract guards). The three-const triple
// (`HMAX`, `RESOURCE_BASE`, `WORLD_SALT`) are load-bearing for deterministic world gen.
// (`HMAX`=relief spread, `RESOURCE_BASE`=cap rescale magnitude for biome+edaphic-driven richness,
// `WORLD_SALT`=seed permutation).

#[macroquad::main(window_conf)]
async fn main() {
    // macroquad's default per-draw-call buffer (10 000 verts / 5 000 indices) silently CLAMPS (drops
    // trailing geometry, logging "exceeded max drawcall size" every frame) a terrain chunk's worst
    // case (`ROWS_PER_CHUNK` rows × `world_dim` cols × ≤30 verts/≤48 indices per hex column,
    // `terrain.rs`). Raised above that worst case — a one-time buffer allocation, not a per-frame cost.
    //
    // BUT the capacity MUST stay ≤ u16::MAX: macroquad batches successive `draw_mesh` calls into one
    // draw-call and stores each index as `local_index + draw_call.vertices_count as u16`
    // (`quad_gl.rs:949`). It only breaks the batch when `vertices_count >= max_vertices`, so a large
    // capacity lets `vertices_count` grow past 65535 → the `as u16` cast wraps and the `+` overflows
    // (debug: panic "attempt to add with overflow"; release: silent geometry corruption). This bit us
    // on big/high-relief maps (dim≥256 with landforms). Cap at ≤65535 so macroquad auto-flushes each
    // batch before the u16 index space is exhausted. Each terrain chunk is kept < this bound by the
    // adaptive `rows_per_chunk` in `terrain.rs`/`terrain_cube.rs`.
    gl_set_drawcall_buffer_capacity(60_000, 120_000);

    let cli_args = parse_args();
    let config = cli::default_config(cli_args.seed);
    // R-8: `--dim` only applies in standalone — in sim mode the worker thread builds its OWN world
    // from `config.econ.world_dim` (via `cli::build_sim`), so overriding it here alone would desync
    // the render's terrain from the sim's, breaking the pinned-param contract below.
    // The render's OWN WorldView (boxed so it can be either the native `ProcgenWorld` or a v1 dump).
    // Built ONCE from the SAME (dim, hmax, resource_base, seed) tuple the sim worker uses — the single
    // source of provenance the pinned-param contract above requires. W-6 WIRE: use `cli::HMAX`,
    // `cli::RESOURCE_BASE`, `cli::WORLD_SALT` directly (now pub). The ProcgenWorld pipeline (integer
    // relief + erosion + biome/edaphic + caps) runs once here and caches all per-cell fields — never
    // re-run per frame or per query.
    //
    // Landform stages (tectonics/aeolian/volcanic/glacial/coastal, all default-off in the sim path):
    // turn ON in STANDALONE only, to preview the full diverse-relief terragen (map_dump does the same).
    // In sim mode they MUST stay off — the sim worker builds its world with all-off, and flipping only
    // the render side would desync the pinned-param contract above.
    //
    // `--v1-dump <path>` REPLACES the ProcgenWorld with a v1-generated dump (carrying its OWN `dim`),
    // holding THIS renderer constant to compare v1 vs v2 worldgen. On load error it falls back to
    // ProcgenWorld. `--dim` only applies in standalone — see the module doc comment for why.
    let make_procgen = |dim: i64| -> Box<dyn WorldView> {
        let lf = cli_args.standalone; // enable all 5 landforms for the standalone terragen preview
        Box::new(world::ProcgenWorld::new(dim, cli::HMAX, cli::RESOURCE_BASE, config.seed ^ cli::WORLD_SALT, None, lf, lf, lf, lf, lf))
    };
    let default_dim = if cli_args.standalone { cli_args.dim_override.unwrap_or(config.econ.world_dim) } else { config.econ.world_dim };
    let (world_dim, world): (i64, Box<dyn WorldView>) = match &cli_args.v1_dump {
        Some(path) => match dump_world::DumpWorld::load(path) {
            Ok(w) => {
                let dim = w.dim;
                println!("[v1-dump] loaded {path}: dim {dim} — drawing v1 worldgen in the v2 renderer");
                (dim, Box::new(w))
            }
            Err(e) => {
                eprintln!("[v1-dump] {e}; falling back to ProcgenWorld");
                (default_dim, make_procgen(default_dim))
            }
        },
        None => (default_dim, make_procgen(default_dim)),
    };
    // Terrain top-face coloring: 'C' toggles Height↔Material at runtime (rebuilds the baked meshes).
    // Default = Height (hypsometric relief ramp) so elevation shape reads at a glance.
    let mut color_mode = crate::biome_palette::ColorMode::Height;
    let mut hex_terrain_chunks = terrain::build_hex_terrain(world_dim, world.as_ref(), color_mode);
    let mut cube_terrain_chunks = terrain_cube::build_cube_terrain(world_dim, world.as_ref(), color_mode);

    // R-5: Runtime hex↔cube toggle state. Default = hex (R-2's established look).
    let mut use_cube_terrain = false;

    // R-3: Interactive isometric camera — pan (WASD/arrows + mouse drag), zoom (scroll),
    // rotate (yaw: Q/E or comma/period). Starts centered on the world at a standard iso view.
    let (span_x, _) = hex::hex_center(world_dim, 0);
    let (_, span_z) = hex::hex_center(0, world_dim);
    let world_span = span_x.max(span_z).max(1.0);
    let center = Vec3::new(span_x * 0.5, hex::HEIGHT_SCALE * cli::HMAX as f32 * 0.5, span_z * 0.5);
    let mut camera = IsoCam::new(center, 0.0, world_span * 1.5);

    // R-8: standalone mode spawns NO sim worker — `handle` stays `None` for the run, so `snap` below
    // stays `None` too (the render loop already tolerated a pre-first-tick `None`; standalone just
    // never leaves that state). Terrain/camera/culling/LOD are unaffected — they never read `handle`.
    let handle = if cli_args.standalone { None } else { Some(driver::spawn(cli_args.seed)) };

    // R-13: Apply camera preset to ensure deterministic view.
    let world_span = span_x.max(span_z).max(1.0);
    if cli_args.screenshot.is_some() || cli_args.bench {
        cli_args.cam_preset.apply_to_camera(&mut camera, center, world_span);
    }

    // R-13: Screenshot mode — render warmup frames, capture, exit.
    if let Some(screenshot_path) = &cli_args.screenshot {
        for _ in 0..cli_args.screenshot_warmup {
            if let Some(h) = &handle {
                if is_key_pressed(KeyCode::Space) {
                    h.toggle_pause();
                }
                if is_key_pressed(KeyCode::Right) || is_key_pressed(KeyCode::N) {
                    h.step_once();
                }
            }

            clear_background(Color::from_rgba(18, 18, 22, 255));
            let snap = handle.as_ref().and_then(|h| h.latest());

            camera.update();
            let cam3d = camera.to_camera3d();
            let frustum_planes = camera.frustum_planes();
            set_camera(&cam3d);

            let terrain_chunks = if use_cube_terrain { &cube_terrain_chunks } else { &hex_terrain_chunks };
            for chunk in terrain_chunks {
                let (min, max) = chunk.bounds;
                if frustum_planes.iter().all(|plane| plane.aabb_intersects(min, max)) {
                    draw_mesh(&chunk.mesh);
                }
            }

            if let Some(s) = snap.as_ref() {
                let px_per_m = camera.px_per_m();
                for c in &s.creatures {
                    let (cx, cz) = if use_cube_terrain {
                        (c.pos.0 as f32 + 0.5, c.pos.1 as f32 + 0.5)
                    } else {
                        hex::hex_center(c.pos.0, c.pos.1)
                    };
                    let h = world.height(c.pos.0, c.pos.1) as f32 * hex::HEIGHT_SCALE;
                    let creature_pos = vec3(cx, h + 0.15, cz);

                    if !camera.point_in_frustum(creature_pos) {
                        continue;
                    }

                    let color = match c.uptake_layer {
                        0 => Color::new(1.0, 0.6, 0.2, 1.0),
                        1 => Color::new(0.2, 0.8, 1.0, 1.0),
                        2 => Color::new(0.8, 0.2, 1.0, 1.0),
                        _ => Color::new(0.5, 0.5, 0.5, 1.0),
                    };

                    if px_per_m < PX_PER_M_FAR_THRESHOLD {
                        draw_sphere(creature_pos, 0.04, None, color);
                    } else if px_per_m < PX_PER_M_MID_THRESHOLD {
                        let body_count = c.body_size.max(1) as usize;
                        let grid_side = (body_count as f32).sqrt().ceil() as i32;
                        let cell_radius = 0.03;
                        let spacing = cell_radius * 2.2;
                        let grid_size = (grid_side - 1) as f32 * spacing;
                        let offset_x = -grid_size / 2.0;
                        let offset_z = -grid_size / 2.0;

                        let mut drawn = 0;
                        for row in 0..grid_side {
                            for col in 0..grid_side {
                                if drawn >= body_count {
                                    break;
                                }
                                let cell_x = offset_x + col as f32 * spacing;
                                let cell_z = offset_z + row as f32 * spacing;
                                let cell_pos = creature_pos + vec3(cell_x, 0.02, cell_z);
                                draw_sphere(cell_pos, cell_radius, None, color);
                                drawn += 1;
                            }
                            if drawn >= body_count {
                                break;
                            }
                        }
                    } else {
                        let size_scale = c.size as f32 / 16.0;
                        let base_size = 0.15 * size_scale;
                        let body_count = c.body_size.max(1) as usize;

                        if body_count > 1 {
                            let grid_side = (body_count as f32).sqrt().ceil() as i32;
                            let cell_radius = 0.04;
                            let spacing = cell_radius * 2.0;
                            let grid_size = (grid_side - 1) as f32 * spacing;
                            let offset_x = -grid_size / 2.0;
                            let offset_z = -grid_size / 2.0;

                            let mut drawn = 0;
                            for row in 0..grid_side {
                                for col in 0..grid_side {
                                    if drawn >= body_count {
                                        break;
                                    }
                                    let cell_x = offset_x + col as f32 * spacing;
                                    let cell_z = offset_z + row as f32 * spacing;
                                    let cell_pos = creature_pos + vec3(cell_x, 0.01, cell_z);
                                    draw_sphere(cell_pos, cell_radius, None, color);
                                    drawn += 1;
                                }
                                if drawn >= body_count {
                                    break;
                                }
                            }
                        } else {
                            match c.cell_type {
                                Some(sim_core::CellType::A) => {
                                    draw_sphere(creature_pos, base_size, None, color);
                                    let accent = Color::new(color.r.min(1.0), (color.g * 1.3).min(1.0), color.b, 1.0);
                                    draw_sphere(creature_pos + vec3(0.0, base_size * 1.2, 0.0), base_size * 0.5, None, accent);
                                }
                                Some(sim_core::CellType::B) => {
                                    draw_sphere(creature_pos, base_size, None, color);
                                    let accent = Color::new(color.r, (color.g * 1.3).min(1.0), color.b.min(1.0), 1.0);
                                    draw_sphere(creature_pos + vec3(base_size * 1.2, 0.0, 0.0), base_size * 0.5, None, accent);
                                }
                                Some(sim_core::CellType::Mixed) => {
                                    draw_sphere(creature_pos, base_size, None, color);
                                    let accent = Color::new((color.r * 1.3).min(1.0), color.g, color.b.min(1.0), 1.0);
                                    draw_sphere(creature_pos + vec3(0.0, 0.0, base_size * 1.2), base_size * 0.5, None, accent);
                                }
                                Some(sim_core::CellType::Diff(_)) => {
                                    draw_sphere(creature_pos, base_size, None, color);
                                }
                                None => {
                                    draw_sphere(creature_pos, base_size, None, color);
                                }
                            }
                        }
                    }
                }
            }
            set_default_camera();

            next_frame().await;
        }

        // Capture final frame to screenshot
        set_default_camera();
        let img = get_screen_data();
        img.export_png(screenshot_path);
        println!("[screenshot] captured to {}", screenshot_path);
        std::process::exit(0);
    }

    // R-13: Benchmark mode — time steady-state frames, print machine-readable line, exit.
    if cli_args.bench {
        // Warmup: 30 frames to reach steady state
        for _ in 0..30 {
            if let Some(h) = &handle {
                if is_key_pressed(KeyCode::Space) {
                    h.toggle_pause();
                }
                if is_key_pressed(KeyCode::Right) || is_key_pressed(KeyCode::N) {
                    h.step_once();
                }
            }

            clear_background(Color::from_rgba(18, 18, 22, 255));
            let snap = handle.as_ref().and_then(|h| h.latest());

            camera.update();
            let cam3d = camera.to_camera3d();
            let frustum_planes = camera.frustum_planes();
            set_camera(&cam3d);

            let terrain_chunks = if use_cube_terrain { &cube_terrain_chunks } else { &hex_terrain_chunks };
            for chunk in terrain_chunks {
                let (min, max) = chunk.bounds;
                if frustum_planes.iter().all(|plane| plane.aabb_intersects(min, max)) {
                    draw_mesh(&chunk.mesh);
                }
            }

            if let Some(s) = snap.as_ref() {
                let px_per_m = camera.px_per_m();
                for c in &s.creatures {
                    let (cx, cz) = if use_cube_terrain {
                        (c.pos.0 as f32 + 0.5, c.pos.1 as f32 + 0.5)
                    } else {
                        hex::hex_center(c.pos.0, c.pos.1)
                    };
                    let h = world.height(c.pos.0, c.pos.1) as f32 * hex::HEIGHT_SCALE;
                    let creature_pos = vec3(cx, h + 0.15, cz);

                    if !camera.point_in_frustum(creature_pos) {
                        continue;
                    }

                    let color = match c.uptake_layer {
                        0 => Color::new(1.0, 0.6, 0.2, 1.0),
                        1 => Color::new(0.2, 0.8, 1.0, 1.0),
                        2 => Color::new(0.8, 0.2, 1.0, 1.0),
                        _ => Color::new(0.5, 0.5, 0.5, 1.0),
                    };

                    if px_per_m < PX_PER_M_FAR_THRESHOLD {
                        draw_sphere(creature_pos, 0.04, None, color);
                    } else if px_per_m < PX_PER_M_MID_THRESHOLD {
                        let body_count = c.body_size.max(1) as usize;
                        let grid_side = (body_count as f32).sqrt().ceil() as i32;
                        let cell_radius = 0.03;
                        let spacing = cell_radius * 2.2;
                        let grid_size = (grid_side - 1) as f32 * spacing;
                        let offset_x = -grid_size / 2.0;
                        let offset_z = -grid_size / 2.0;

                        let mut drawn = 0;
                        for row in 0..grid_side {
                            for col in 0..grid_side {
                                if drawn >= body_count {
                                    break;
                                }
                                let cell_x = offset_x + col as f32 * spacing;
                                let cell_z = offset_z + row as f32 * spacing;
                                let cell_pos = creature_pos + vec3(cell_x, 0.02, cell_z);
                                draw_sphere(cell_pos, cell_radius, None, color);
                                drawn += 1;
                            }
                            if drawn >= body_count {
                                break;
                            }
                        }
                    } else {
                        let size_scale = c.size as f32 / 16.0;
                        let base_size = 0.15 * size_scale;
                        let body_count = c.body_size.max(1) as usize;

                        if body_count > 1 {
                            let grid_side = (body_count as f32).sqrt().ceil() as i32;
                            let cell_radius = 0.04;
                            let spacing = cell_radius * 2.0;
                            let grid_size = (grid_side - 1) as f32 * spacing;
                            let offset_x = -grid_size / 2.0;
                            let offset_z = -grid_size / 2.0;

                            let mut drawn = 0;
                            for row in 0..grid_side {
                                for col in 0..grid_side {
                                    if drawn >= body_count {
                                        break;
                                    }
                                    let cell_x = offset_x + col as f32 * spacing;
                                    let cell_z = offset_z + row as f32 * spacing;
                                    let cell_pos = creature_pos + vec3(cell_x, 0.01, cell_z);
                                    draw_sphere(cell_pos, cell_radius, None, color);
                                    drawn += 1;
                                }
                                if drawn >= body_count {
                                    break;
                                }
                            }
                        } else {
                            match c.cell_type {
                                Some(sim_core::CellType::A) => {
                                    draw_sphere(creature_pos, base_size, None, color);
                                    let accent = Color::new(color.r.min(1.0), (color.g * 1.3).min(1.0), color.b, 1.0);
                                    draw_sphere(creature_pos + vec3(0.0, base_size * 1.2, 0.0), base_size * 0.5, None, accent);
                                }
                                Some(sim_core::CellType::B) => {
                                    draw_sphere(creature_pos, base_size, None, color);
                                    let accent = Color::new(color.r, (color.g * 1.3).min(1.0), color.b.min(1.0), 1.0);
                                    draw_sphere(creature_pos + vec3(base_size * 1.2, 0.0, 0.0), base_size * 0.5, None, accent);
                                }
                                Some(sim_core::CellType::Mixed) => {
                                    draw_sphere(creature_pos, base_size, None, color);
                                    let accent = Color::new((color.r * 1.3).min(1.0), color.g, color.b.min(1.0), 1.0);
                                    draw_sphere(creature_pos + vec3(0.0, 0.0, base_size * 1.2), base_size * 0.5, None, accent);
                                }
                                Some(sim_core::CellType::Diff(_)) => {
                                    draw_sphere(creature_pos, base_size, None, color);
                                }
                                None => {
                                    draw_sphere(creature_pos, base_size, None, color);
                                }
                            }
                        }
                    }
                }
            }
            set_default_camera();

            next_frame().await;
        }

        // Timed frames: collect per-frame times for p95 calculation
        let mut frame_times_ms = Vec::with_capacity(300);
        let mut chunk_count = 0;
        let mut vert_count = 0;

        for _ in 0..300 {
            let frame_start = Instant::now();

            if let Some(h) = &handle {
                if is_key_pressed(KeyCode::Space) {
                    h.toggle_pause();
                }
                if is_key_pressed(KeyCode::Right) || is_key_pressed(KeyCode::N) {
                    h.step_once();
                }
            }

            clear_background(Color::from_rgba(18, 18, 22, 255));
            let snap = handle.as_ref().and_then(|h| h.latest());

            camera.update();
            let cam3d = camera.to_camera3d();
            let frustum_planes = camera.frustum_planes();
            set_camera(&cam3d);

            let terrain_chunks = if use_cube_terrain { &cube_terrain_chunks } else { &hex_terrain_chunks };
            let mut chunks_drawn = 0;
            for chunk in terrain_chunks {
                let (min, max) = chunk.bounds;
                if frustum_planes.iter().all(|plane| plane.aabb_intersects(min, max)) {
                    draw_mesh(&chunk.mesh);
                    chunks_drawn += 1;
                    vert_count = chunk.mesh.vertices.len();
                }
            }
            chunk_count = chunks_drawn;

            if let Some(s) = snap.as_ref() {
                let px_per_m = camera.px_per_m();
                for c in &s.creatures {
                    let (cx, cz) = if use_cube_terrain {
                        (c.pos.0 as f32 + 0.5, c.pos.1 as f32 + 0.5)
                    } else {
                        hex::hex_center(c.pos.0, c.pos.1)
                    };
                    let h = world.height(c.pos.0, c.pos.1) as f32 * hex::HEIGHT_SCALE;
                    let creature_pos = vec3(cx, h + 0.15, cz);

                    if !camera.point_in_frustum(creature_pos) {
                        continue;
                    }

                    let color = match c.uptake_layer {
                        0 => Color::new(1.0, 0.6, 0.2, 1.0),
                        1 => Color::new(0.2, 0.8, 1.0, 1.0),
                        2 => Color::new(0.8, 0.2, 1.0, 1.0),
                        _ => Color::new(0.5, 0.5, 0.5, 1.0),
                    };

                    if px_per_m < PX_PER_M_FAR_THRESHOLD {
                        draw_sphere(creature_pos, 0.04, None, color);
                    } else if px_per_m < PX_PER_M_MID_THRESHOLD {
                        let body_count = c.body_size.max(1) as usize;
                        let grid_side = (body_count as f32).sqrt().ceil() as i32;
                        let cell_radius = 0.03;
                        let spacing = cell_radius * 2.2;
                        let grid_size = (grid_side - 1) as f32 * spacing;
                        let offset_x = -grid_size / 2.0;
                        let offset_z = -grid_size / 2.0;

                        let mut drawn = 0;
                        for row in 0..grid_side {
                            for col in 0..grid_side {
                                if drawn >= body_count {
                                    break;
                                }
                                let cell_x = offset_x + col as f32 * spacing;
                                let cell_z = offset_z + row as f32 * spacing;
                                let cell_pos = creature_pos + vec3(cell_x, 0.02, cell_z);
                                draw_sphere(cell_pos, cell_radius, None, color);
                                drawn += 1;
                            }
                            if drawn >= body_count {
                                break;
                            }
                        }
                    } else {
                        let size_scale = c.size as f32 / 16.0;
                        let base_size = 0.15 * size_scale;
                        let body_count = c.body_size.max(1) as usize;

                        if body_count > 1 {
                            let grid_side = (body_count as f32).sqrt().ceil() as i32;
                            let cell_radius = 0.04;
                            let spacing = cell_radius * 2.0;
                            let grid_size = (grid_side - 1) as f32 * spacing;
                            let offset_x = -grid_size / 2.0;
                            let offset_z = -grid_size / 2.0;

                            let mut drawn = 0;
                            for row in 0..grid_side {
                                for col in 0..grid_side {
                                    if drawn >= body_count {
                                        break;
                                    }
                                    let cell_x = offset_x + col as f32 * spacing;
                                    let cell_z = offset_z + row as f32 * spacing;
                                    let cell_pos = creature_pos + vec3(cell_x, 0.01, cell_z);
                                    draw_sphere(cell_pos, cell_radius, None, color);
                                    drawn += 1;
                                }
                                if drawn >= body_count {
                                    break;
                                }
                            }
                        } else {
                            match c.cell_type {
                                Some(sim_core::CellType::A) => {
                                    draw_sphere(creature_pos, base_size, None, color);
                                    let accent = Color::new(color.r.min(1.0), (color.g * 1.3).min(1.0), color.b, 1.0);
                                    draw_sphere(creature_pos + vec3(0.0, base_size * 1.2, 0.0), base_size * 0.5, None, accent);
                                }
                                Some(sim_core::CellType::B) => {
                                    draw_sphere(creature_pos, base_size, None, color);
                                    let accent = Color::new(color.r, (color.g * 1.3).min(1.0), color.b.min(1.0), 1.0);
                                    draw_sphere(creature_pos + vec3(base_size * 1.2, 0.0, 0.0), base_size * 0.5, None, accent);
                                }
                                Some(sim_core::CellType::Mixed) => {
                                    draw_sphere(creature_pos, base_size, None, color);
                                    let accent = Color::new((color.r * 1.3).min(1.0), color.g, color.b.min(1.0), 1.0);
                                    draw_sphere(creature_pos + vec3(0.0, 0.0, base_size * 1.2), base_size * 0.5, None, accent);
                                }
                                Some(sim_core::CellType::Diff(_)) => {
                                    draw_sphere(creature_pos, base_size, None, color);
                                }
                                None => {
                                    draw_sphere(creature_pos, base_size, None, color);
                                }
                            }
                        }
                    }
                }
            }
            set_default_camera();

            next_frame().await;

            let frame_elapsed = frame_start.elapsed().as_secs_f32() * 1000.0;
            frame_times_ms.push(frame_elapsed);
        }

        // Calculate statistics
        let avg_ms = frame_times_ms.iter().sum::<f32>() / frame_times_ms.len() as f32;
        frame_times_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p95_idx = (frame_times_ms.len() * 95 / 100).max(1);
        let p95_ms = frame_times_ms[p95_idx - 1];

        println!("BENCH dim={} avg_ms={:.2} p95_ms={:.2} verts={} chunks={}", world_dim, avg_ms, p95_ms, vert_count, chunk_count);
        std::process::exit(0);
    }

    loop {
        if let Some(h) = &handle {
            if is_key_pressed(KeyCode::Space) {
                h.toggle_pause();
            }
            if is_key_pressed(KeyCode::Right) || is_key_pressed(KeyCode::N) {
                h.step_once();
            }
        }
        // R-5: Toggle hex↔cube terrain with 'T' key.
        if is_key_pressed(KeyCode::T) {
            use_cube_terrain = !use_cube_terrain;
        }
        // Toggle Height↔Material terrain coloring with 'C' (rebuilds the baked-color meshes).
        if is_key_pressed(KeyCode::C) {
            color_mode = match color_mode {
                crate::biome_palette::ColorMode::Height => crate::biome_palette::ColorMode::Material,
                crate::biome_palette::ColorMode::Material => crate::biome_palette::ColorMode::Height,
            };
            hex_terrain_chunks = terrain::build_hex_terrain(world_dim, world.as_ref(), color_mode);
            cube_terrain_chunks = terrain_cube::build_cube_terrain(world_dim, world.as_ref(), color_mode);
        }

        clear_background(Color::from_rgba(18, 18, 22, 255));

        let snap = handle.as_ref().and_then(|h| h.latest());

        // R-3: Update camera input and build frustum.
        camera.update();
        let cam3d = camera.to_camera3d();
        let frustum_planes = camera.frustum_planes();

        set_camera(&cam3d);

        // R-3: Frustum-cull terrain chunks — only draw chunks whose AABB intersects the frustum.
        // R-5: Works over both hex and cube layouts (same chunk AABB structure).
        let terrain_chunks = if use_cube_terrain { &cube_terrain_chunks } else { &hex_terrain_chunks };
        let mut chunks_drawn = 0;
        for chunk in terrain_chunks {
            let (min, max) = chunk.bounds;
            if frustum_planes.iter().all(|plane| plane.aabb_intersects(min, max)) {
                draw_mesh(&chunk.mesh);
                chunks_drawn += 1;
            }
        }

        // R-4/R-5: Creatures rendered by px_per_m LOD tier (point → sphere → morphology).
        // R-5: Projected into the ACTIVE view (hex or cube).
        // Tier selection is a pure function of camera zoom ONLY (RnD R21), never per-creature distance.
        if let Some(s) = snap.as_ref() {
            let px_per_m = camera.px_per_m(); // Pure fn of ortho_span + viewport; whole frame shares one tier.

            for c in &s.creatures {
                // R-5: Creature projection follows active terrain layout.
                let (cx, cz) = if use_cube_terrain {
                    // Cube mode: square cell center at (col + 0.5, row + 0.5)
                    (c.pos.0 as f32 + 0.5, c.pos.1 as f32 + 0.5)
                } else {
                    // Hex mode: hex center (R-2/R-4 behavior)
                    hex::hex_center(c.pos.0, c.pos.1)
                };
                let h = world.height(c.pos.0, c.pos.1) as f32 * hex::HEIGHT_SCALE;
                let creature_pos = vec3(cx, h + 0.15, cz);

                // R-3 frustum cull: skip creatures outside the view frustum.
                if !camera.point_in_frustum(creature_pos) {
                    continue;
                }

                // R-7 (biology coloring): Base color by uptake_layer (feeding guild).
                // Layer 0 (A-guild) = orange/red; layer 1 (B-guild) = cyan/blue; higher layers distinct.
                // This makes emergence visible: A/B differentiation is the primary visual signal.
                let color = match c.uptake_layer {
                    0 => Color::new(1.0, 0.6, 0.2, 1.0), // Orange (A-guild)
                    1 => Color::new(0.2, 0.8, 1.0, 1.0), // Cyan (B-guild)
                    2 => Color::new(0.8, 0.2, 1.0, 1.0), // Magenta (layer 2+)
                    _ => Color::new(0.5, 0.5, 0.5, 1.0), // Gray (undefined layers)
                };

                // R-4 LOD tier by px_per_m: FAR (point) < MID (sphere) < NEAR (morphology).
                if px_per_m < PX_PER_M_FAR_THRESHOLD {
                    // ─── FAR tier: sub-pixel point/billboard (cheapest) ───────────────────────────────
                    // Creatures so tiny they're unresolvable. Draw a minimal dot.
                    draw_sphere(creature_pos, 0.04, None, color);
                } else if px_per_m < PX_PER_M_MID_THRESHOLD {
                    // ─── MID tier: multicell cluster sphere (R-11 body_size rendering) ──────────────────
                    // R-11: Draw body as `body_size` cells in a packed cluster arrangement.
                    // Each cell is a small sphere; cluster is arranged in a square grid.
                    let body_count = c.body_size.max(1) as usize;
                    let grid_side = (body_count as f32).sqrt().ceil() as i32;
                    let cell_radius = 0.03; // Small sub-cell radius
                    let spacing = cell_radius * 2.2; // Slight spacing between cells

                    // Compute grid offset to center the cluster
                    let grid_size = (grid_side - 1) as f32 * spacing;
                    let offset_x = -grid_size / 2.0;
                    let offset_z = -grid_size / 2.0;

                    // Draw cells in a square grid pattern
                    let mut drawn = 0;
                    for row in 0..grid_side {
                        for col in 0..grid_side {
                            if drawn >= body_count {
                                break;
                            }
                            let cell_x = offset_x + col as f32 * spacing;
                            let cell_z = offset_z + row as f32 * spacing;
                            let cell_pos = creature_pos + vec3(cell_x, 0.02, cell_z);
                            draw_sphere(cell_pos, cell_radius, None, color);
                            drawn += 1;
                        }
                        if drawn >= body_count {
                            break;
                        }
                    }
                } else {
                    // ─── NEAR tier: cell-type morphology + multicell body representation ──────────────────
                    // R-4/R-11: Scale morphology by `size` (Kleiber); draw body as `body_size` cells.
                    // Each cell_type has a distinctive form; base color is uptake_layer (feeding guild).
                    let size_scale = c.size as f32 / 16.0;
                    let base_size = 0.15 * size_scale;
                    let body_count = c.body_size.max(1) as usize;

                    // For multicellular bodies (body_count > 1), render a small cluster around the main form.
                    // This makes multicellularity visible while preserving the cell_type morphology signal.
                    if body_count > 1 {
                        let grid_side = (body_count as f32).sqrt().ceil() as i32;
                        let cell_radius = 0.04;
                        let spacing = cell_radius * 2.0;
                        let grid_size = (grid_side - 1) as f32 * spacing;
                        let offset_x = -grid_size / 2.0;
                        let offset_z = -grid_size / 2.0;

                        // Draw cells in a compact grid, with cell_type morphology only on the main cell.
                        let mut drawn = 0;
                        for row in 0..grid_side {
                            for col in 0..grid_side {
                                if drawn >= body_count {
                                    break;
                                }
                                let cell_x = offset_x + col as f32 * spacing;
                                let cell_z = offset_z + row as f32 * spacing;
                                let cell_pos = creature_pos + vec3(cell_x, 0.01, cell_z);
                                // Render each cell as a small sphere in the base color
                                draw_sphere(cell_pos, cell_radius, None, color);
                                drawn += 1;
                            }
                            if drawn >= body_count {
                                break;
                            }
                        }
                    } else {
                        // Unicellular: draw the full cell_type morphology
                        match c.cell_type {
                            Some(sim_core::CellType::A) => {
                                // Type A: main body + upper accent sphere (a small top ball).
                                draw_sphere(creature_pos, base_size, None, color);
                                let accent = Color::new(color.r.min(1.0), (color.g * 1.3).min(1.0), color.b, 1.0);
                                draw_sphere(creature_pos + vec3(0.0, base_size * 1.2, 0.0), base_size * 0.5, None, accent);
                            }
                            Some(sim_core::CellType::B) => {
                                // Type B: main body + side accent sphere (a small offset ball).
                                draw_sphere(creature_pos, base_size, None, color);
                                let accent = Color::new(color.r, (color.g * 1.3).min(1.0), color.b.min(1.0), 1.0);
                                draw_sphere(creature_pos + vec3(base_size * 1.2, 0.0, 0.0), base_size * 0.5, None, accent);
                            }
                            Some(sim_core::CellType::Mixed) => {
                                // Type Mixed: main body + front accent sphere (a small forward ball).
                                draw_sphere(creature_pos, base_size, None, color);
                                let accent = Color::new((color.r * 1.3).min(1.0), color.g, color.b.min(1.0), 1.0);
                                draw_sphere(creature_pos + vec3(0.0, 0.0, base_size * 1.2), base_size * 0.5, None, accent);
                            }
                            Some(sim_core::CellType::Diff(_)) => {
                                // Diff: differentiated cell, render as neutral sphere (same as None for now)
                                draw_sphere(creature_pos, base_size, None, color);
                            }
                            None => {
                                // Neutral: single sphere (for non-morphogen configs).
                                draw_sphere(creature_pos, base_size, None, color);
                            }
                        }
                    }
                }
            }
        }
        set_default_camera();

        egui_macroquad::ui(|ctx| {
            let title = if cli_args.standalone {
                "v2 render scaffold — R-8 standalone hex-map viewer"
            } else {
                "v2 render scaffold — R-7 biology coloring"
            };
            egui::Window::new(title).show(ctx, |ui| {
                match snap.as_ref() {
                    Some(s) => {
                        ui.label(format!("tick: {}", s.tick));
                        ui.label(format!("population: {}", s.population));
                        ui.label(format!("species: {}", s.species_count));
                        ui.label(format!("creatures drawn: {}", s.creatures.len()));
                    }
                    None if cli_args.standalone => {
                        ui.label("standalone mode — no sim, terrain only");
                    }
                    None => {
                        ui.label("waiting for the sim worker's first tick…");
                    }
                }
                ui.separator();
                if !cli_args.standalone {
                    ui.label("─ Creature Coloring (uptake_layer / feeding guild) ─");
                    ui.colored_label(egui::Color32::from_rgb(255, 153, 51), "● Orange: Layer 0 (A-guild)");
                    ui.colored_label(egui::Color32::from_rgb(51, 204, 255), "● Cyan: Layer 1 (B-guild)");
                    ui.colored_label(egui::Color32::from_rgb(204, 51, 255), "● Magenta: Layer 2+");
                    ui.separator();
                }
                ui.label(format!("terrain: {world_dim}×{world_dim}, {} mesh chunks", terrain_chunks.len()));
                ui.label(format!("chunks drawn: {}/{}", chunks_drawn, terrain_chunks.len()));
                ui.label(format!("fps: {}", get_fps()));
                ui.separator();
                ui.label("Controls: WASD/drag pan · wheel zoom · Q/E rotate · T hex/cube");
                if let Some(h) = &handle {
                    ui.separator();
                    ui.label(if h.is_paused() {
                        "PAUSED — Space to resume"
                    } else {
                        "running — Space to pause"
                    });
                    ui.label("Right / N: step once while paused");
                }
            });
        });
        egui_macroquad::draw();

        next_frame().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R-8 teeth: the same world+terrain construction the standalone path uses (no `Sim`, no window)
    /// must produce non-empty meshes — a headless proxy for "renders terrain with a `None` snapshot
    /// without panicking," since the full visual check needs a window the CI/agent can't open.
    #[test]
    fn standalone_world_builds_nonempty_terrain() {
        let dim = 64;
        let world = world::ProcgenWorld::new(dim, cli::HMAX, cli::RESOURCE_BASE, SEED ^ cli::WORLD_SALT, None, false, false, false, false, false);
        let hex_chunks = terrain::build_hex_terrain(dim, &world, crate::biome_palette::ColorMode::Height);
        let cube_chunks = terrain_cube::build_cube_terrain(dim, &world, crate::biome_palette::ColorMode::Height);
        assert!(!hex_chunks.is_empty(), "hex terrain must produce at least one chunk");
        assert!(!cube_chunks.is_empty(), "cube terrain must produce at least one chunk");
        assert!(hex_chunks.iter().any(|c| !c.mesh.vertices.is_empty()));
        assert!(cube_chunks.iter().any(|c| !c.mesh.vertices.is_empty()));
    }
}
