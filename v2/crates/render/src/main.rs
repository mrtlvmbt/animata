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
mod creatures;
mod draw;
mod driver;
mod dump_world;
mod gpu_terrain;
mod hex;
mod input;
mod terrain;
mod terrain_cube;
mod ui;

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
    // R-13: Pre-parse args to detect --bench mode for vsync configuration
    let is_bench = std::env::args().any(|arg| arg == "--bench");

    Conf {
        window_title: "animata v2 — render scaffold (R-8 standalone hex-map viewer)".to_owned(),
        window_width: 1024,
        window_height: 768,
        high_dpi: true,
        platform: macroquad::miniquad::conf::Platform {
            swap_interval: if is_bench { Some(0) } else { None },
            ..Default::default()
        },
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
    /// R-15a: `--retained`: use retained-buffer GPU rendering for terrain (default OFF).
    retained: bool,
    /// R-14: `--bare`: water renders as dry-bed (default OFF).
    bare_mode: bool,
    /// R-14: `--height-scale <f32>`: override the height scale (default 0.2).
    height_scale_override: Option<f32>,
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
    let mut retained = false;
    let mut bare_mode = false;
    let mut height_scale_override = None;

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
            "--retained" => retained = true,
            "--bare" => bare_mode = true,
            "--height-scale" => {
                let v = args.next().expect("--height-scale requires a value");
                height_scale_override = Some(v.parse().unwrap_or_else(|_| panic!("--height-scale expects f32, got {v:?}")));
            }
            other => eprintln!("render: ignoring unknown arg {other:?}"),
        }
    }
    CliArgs { standalone, seed, dim_override, v1_dump, screenshot, screenshot_warmup, bench, cam_preset, retained, bare_mode, height_scale_override }
}

// ── R-15a: Retained-buffer GPU rendering helpers ──────────────────────────────────────────────────
/// Load a shader source from file. Tries multiple paths to find assets/shaders/ relative to the repo root.
fn load_shader(filename: &str) -> String {
    // Try v2 local paths first, then fallback to v1 paths (for compatibility)
    let candidate_paths = [
        format!("v2/crates/render/assets/shaders/{}", filename),  // From repo root
        format!("crates/render/assets/shaders/{}", filename),     // From v2/
        format!("assets/shaders/{}", filename),                   // From repo root (v1 fallback)
        format!("../../../../assets/shaders/{}", filename),       // From target/release
        format!("../../../assets/shaders/{}", filename),          // From v2/crates/render
    ];

    for path in &candidate_paths {
        if let Ok(content) = std::fs::read_to_string(path) {
            return content;
        }
    }

    panic!("[gpu_terrain] FATAL: shader {} not found in any candidate path (tried: {:?})", filename, candidate_paths);
}

/// Helper to draw terrain using retained GPU buffers.
pub fn draw_gpu_terrain(
    gpu_chunks: &[gpu_terrain::GpuChunk],
    pipeline: macroquad::miniquad::Pipeline,
    camera: &IsoCam,
    frustum_planes: &[camera::FrustumPlane],
) {
    use macroquad::prelude::get_internal_gl;
    use macroquad::miniquad::UniformsSource;

    let mut gl = unsafe { get_internal_gl() };
    gl.flush(); // Flush any pending macroquad 2D before our draw calls
    let ctx = gl.quad_context;

    let mvp = camera.to_camera3d().matrix();
    let uniforms = gpu_terrain::ChunkUniforms {
        mvp,
    };

    ctx.apply_pipeline(&pipeline);
    ctx.apply_uniforms(UniformsSource::table(&uniforms));

    for chunk in gpu_chunks {
        // Frustum culling
        if !frustum_planes.iter().all(|plane| plane.aabb_intersects(chunk.lo, chunk.hi)) {
            continue;
        }

        ctx.apply_bindings(&chunk.bindings);
        ctx.draw(0, chunk.n_idx, 1);
    }
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

/// R-17: Per-seed landform variety — each landform toggles independently from seed bits,
/// allowing free mixing (volcanic + coastal for volcanic islands, volcanic + tectonic, etc).
/// Uses splitmix64 hash with independent bit positions for each landform.
/// Guard: never all-off (ensures maps are never flat/featureless).
fn landform_flags(seed: u64) -> (bool, bool, bool, bool, bool) {
    // Deterministic hash: splitmix64
    let mut x = seed;
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^= x >> 31;

    // Extract independent bits for each landform (well-spaced bit positions)
    let tect = (x >> 3) & 1 == 1;
    let aeol = (x >> 13) & 1 == 1;
    let volc = (x >> 23) & 1 == 1;
    let glac = (x >> 33) & 1 == 1;
    let coast = (x >> 43) & 1 == 1;

    // Guard: never all-off (avoid flat/boring maps)
    let (tect, aeol, volc, glac, coast) = if !(tect || aeol || volc || glac || coast) {
        (true, aeol, volc, glac, coast)  // force tectonic if all others are off
    } else {
        (tect, aeol, volc, glac, coast)
    };

    (tect, aeol, volc, glac, coast)
}

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
    // R-17: Standalone uses per-seed landform profiles for variety (6 archetypes via landform_profile).
    // `--v1-dump <path>` REPLACES the ProcgenWorld with a v1-generated dump (carrying its OWN `dim`),
    // holding THIS renderer constant to compare v1 vs v2 worldgen. On load error it falls back to
    // ProcgenWorld. `--dim` only applies in standalone — see the module doc comment for why.
    let make_procgen = |dim: i64| -> Box<dyn WorldView> {
        let (tect, aeol, volc, glac, coast) = if cli_args.standalone {
            landform_flags(config.seed)   // deterministic, seed-derived variety with free landform mixing
        } else {
            (false, false, false, false, false)  // sim mode: all-off (unchanged)
        };
        Box::new(world::ProcgenWorld::new(dim, cli::HMAX, cli::RESOURCE_BASE, config.seed ^ cli::WORLD_SALT, None, tect, aeol, volc, glac, coast))
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
    // Build terrain meshes (palette v2: material HUE × height VALUE + jitter).
    let mut hex_terrain_chunks = terrain::build_hex_terrain(world_dim, world.as_ref(), config.seed, cli_args.bare_mode);
    let mut cube_terrain_chunks = terrain_cube::build_cube_terrain(world_dim, world.as_ref(), config.seed, cli_args.bare_mode);

    // R-15a: Retained-buffer GPU terrain initialization (if --retained).
    let (mut gpu_hex_chunks, mut gpu_cube_chunks, gpu_pipeline) = if cli_args.retained {
        use macroquad::prelude::get_internal_gl;

        let chunk_vert = load_shader("chunk_v2.vert");
        let chunk_frag = load_shader("chunk_v2.frag");

        let mut gl = unsafe { get_internal_gl() };
        let ctx = gl.quad_context;

        let pipeline = gpu_terrain::chunk_pipeline(ctx, &chunk_vert, &chunk_frag);
        let gpu_hex = gpu_terrain::upload_chunks(ctx, &hex_terrain_chunks);
        let gpu_cube = gpu_terrain::upload_chunks(ctx, &cube_terrain_chunks);
        (gpu_hex, gpu_cube, Some(pipeline))
    } else {
        (Vec::new(), Vec::new(), None)
    };

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

    // U-1: Initialize UI root with DebugPanel.
    let mut ui_root = ui::UiRoot::new();
    ui_root.push(Box::new(ui::DebugPanel));

    // R-13: Apply camera preset to ensure deterministic view.
    let world_span = span_x.max(span_z).max(1.0);
    if cli_args.screenshot.is_some() || cli_args.bench {
        cli_args.cam_preset.apply_to_camera(&mut camera, center, world_span);
    }

    // R-13: Screenshot mode — render warmup frames, then capture on the final frame.
    if let Some(screenshot_path) = &cli_args.screenshot {
        for frame_num in 0..=cli_args.screenshot_warmup {
            // Screenshot mode: no UI, so gating is off (empty UiOut)
            let no_ui = ui::UiOut::default();
            // Process input events (screenshot mode: only sim controls)
            for ev in input::collect(&no_ui) {
                match ev {
                    input::InputEvent::TogglePause => {
                        if let Some(h) = &handle {
                            h.toggle_pause();
                        }
                    }
                    input::InputEvent::StepOnce => {
                        if let Some(h) = &handle {
                            h.step_once();
                        }
                    }
                    input::InputEvent::ToggleTerrainKind => {
                        // Not used in screenshot mode
                    }
                }
            }

            clear_background(Color::from_rgba(18, 18, 22, 255));
            let snap = handle.as_ref().and_then(|h| h.latest());

            camera.update();
            let cam3d = camera.to_camera3d();
            let frustum_planes = camera.frustum_planes();
            set_camera(&cam3d);

            // R-15a: Draw terrain (CPU or GPU path)
            let _ = draw::draw_terrain(
                &hex_terrain_chunks,
                &cube_terrain_chunks,
                &gpu_hex_chunks,
                &gpu_cube_chunks,
                gpu_pipeline,
                &camera,
                use_cube_terrain,
                cli_args.retained,
            );

            // Render creatures by LOD tier
            creatures::render_creatures_lod(&snap, &camera, world.as_ref(), use_cube_terrain);
            set_default_camera();

            // R-13 F-B5: Capture on final frame (frame_num == cli_args.screenshot_warmup)
            // AFTER all rendering, BEFORE next_frame() to read the full backbuffer
            if frame_num == cli_args.screenshot_warmup {
                let img = get_screen_data();
                img.export_png(screenshot_path);
                println!("[screenshot] captured to {}", screenshot_path);
                std::process::exit(0);
            }

            next_frame().await;
        }
    }

    // R-13: Benchmark mode — time steady-state frames, print machine-readable line, exit.
    if cli_args.bench {
        // Warmup: 30 frames to reach steady state
        for _ in 0..30 {
            // Benchmark mode: no UI, so gating is off (empty UiOut)
            let no_ui = ui::UiOut::default();
            // Process input events (benchmark warmup: only sim controls)
            for ev in input::collect(&no_ui) {
                match ev {
                    input::InputEvent::TogglePause => {
                        if let Some(h) = &handle {
                            h.toggle_pause();
                        }
                    }
                    input::InputEvent::StepOnce => {
                        if let Some(h) = &handle {
                            h.step_once();
                        }
                    }
                    input::InputEvent::ToggleTerrainKind => {
                        // Not used in benchmark mode
                    }
                }
            }

            clear_background(Color::from_rgba(18, 18, 22, 255));
            let snap = handle.as_ref().and_then(|h| h.latest());

            camera.update();
            let cam3d = camera.to_camera3d();
            set_camera(&cam3d);

            let _ = draw::draw_terrain(
                &hex_terrain_chunks,
                &cube_terrain_chunks,
                &gpu_hex_chunks,
                &gpu_cube_chunks,
                gpu_pipeline,
                &camera,
                use_cube_terrain,
                cli_args.retained,
            );

            creatures::render_creatures_lod(&snap, &camera, world.as_ref(), use_cube_terrain);
            set_default_camera();

            next_frame().await;
        }

        // Timed frames: collect per-frame times for p95 calculation
        // R-13 F-B2b: Measure BOTH CPU (pre-flush) and wall (including GPU flush/present) times.
        // cpu_ms tracks R-15 headroom improvement; wall_ms is the user-facing 60fps gate.
        // Timing structure: [input] clear_background [cpu_start] render [cpu_end/wall_start] next_frame [wall_end]
        let mut cpu_times_ms = Vec::with_capacity(300);
        let mut wall_times_ms = Vec::with_capacity(300);
        let mut chunk_count = 0;
        let mut vert_count = 0;

        for _ in 0..300 {
            let wall_start = Instant::now();

            // Benchmark mode: no UI, so gating is off (empty UiOut)
            let no_ui = ui::UiOut::default();
            // Process input events (benchmark timed: only sim controls)
            for ev in input::collect(&no_ui) {
                match ev {
                    input::InputEvent::TogglePause => {
                        if let Some(h) = &handle {
                            h.toggle_pause();
                        }
                    }
                    input::InputEvent::StepOnce => {
                        if let Some(h) = &handle {
                            h.step_once();
                        }
                    }
                    input::InputEvent::ToggleTerrainKind => {
                        // Not used in benchmark mode
                    }
                }
            }

            clear_background(Color::from_rgba(18, 18, 22, 255));
            let snap = handle.as_ref().and_then(|h| h.latest());

            let cpu_start = Instant::now();

            camera.update();
            let cam3d = camera.to_camera3d();
            set_camera(&cam3d);

            let draw_stats = draw::draw_terrain(
                &hex_terrain_chunks,
                &cube_terrain_chunks,
                &gpu_hex_chunks,
                &gpu_cube_chunks,
                gpu_pipeline,
                &camera,
                use_cube_terrain,
                cli_args.retained,
            );
            chunk_count = draw_stats.chunks_drawn;
            vert_count = draw_stats.verts_drawn;

            creatures::render_creatures_lod(&snap, &camera, world.as_ref(), use_cube_terrain);
            set_default_camera();

            let cpu_elapsed = cpu_start.elapsed().as_secs_f32() * 1000.0;

            next_frame().await;

            let wall_elapsed = wall_start.elapsed().as_secs_f32() * 1000.0;

            cpu_times_ms.push(cpu_elapsed);
            wall_times_ms.push(wall_elapsed);
        }

        // Calculate statistics for both metrics
        let cpu_avg = cpu_times_ms.iter().sum::<f32>() / cpu_times_ms.len() as f32;
        cpu_times_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let cpu_p95_idx = (cpu_times_ms.len() * 95 / 100).max(1);
        let cpu_p95 = cpu_times_ms[cpu_p95_idx - 1];

        let wall_avg = wall_times_ms.iter().sum::<f32>() / wall_times_ms.len() as f32;
        wall_times_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let wall_p95_idx = (wall_times_ms.len() * 95 / 100).max(1);
        let wall_p95 = wall_times_ms[wall_p95_idx - 1];

        println!("BENCH dim={} cpu_ms={:.2}/{:.2} wall_ms={:.2}/{:.2} verts={} chunks={}", world_dim, cpu_avg, cpu_p95, wall_avg, wall_p95, vert_count, chunk_count);
        std::process::exit(0);
    }

    loop {
        // U-1: Draw UI first to get pointer/keyboard gating state before processing input.
        let snap = handle.as_ref().and_then(|h| h.latest());
        let terrain_chunks = if use_cube_terrain { &cube_terrain_chunks } else { &hex_terrain_chunks };

        // F1: Collect all commands (UiActions + InputEvents) and apply them in one place
        let mut ui_actions: Vec<ui::UiAction> = Vec::new();

        egui_macroquad::ui(|ctx| {
            // Plan D3: UI draws BEFORE world input to read previous-frame egui state.
            // This causes a 1-frame lag (acceptable, identical to v1 behavior).
            let mut ui_ctx = ui::UiCtx {
                world_dim,
                seed: config.seed,
                fps: get_fps(),
                chunks_drawn: 0, // Will be updated after drawing
                verts: 0,
                snap: snap.as_ref().map(|v| &**v),
                standalone_mode: cli_args.standalone,
                terrain_chunks_total: terrain_chunks.len(),
                actions: &mut ui_actions,
            };
            let ui_out = ui_root.draw(ctx, &mut ui_ctx);

            // Apply camera update with gating.
            camera.update_gated(ui_out.wants_pointer, ui_out.wants_keyboard);

            // Collect keyboard input events with gating.
            for ev in input::collect(&ui_out) {
                // Convert InputEvent to UiAction and collect for unified handling
                match ev {
                    input::InputEvent::TogglePause => {
                        ui_actions.push(ui::UiAction::TogglePause);
                    }
                    input::InputEvent::StepOnce => {
                        ui_actions.push(ui::UiAction::StepOnce);
                    }
                    input::InputEvent::ToggleTerrainKind => {
                        ui_actions.push(ui::UiAction::ToggleTerrainKind);
                    }
                }
            }
        });

        // F1: Apply all actions (from UI buttons and keyboard) in unified handler
        for action in ui_actions {
            match action {
                ui::UiAction::TogglePause => {
                    if let Some(h) = &handle {
                        h.toggle_pause();
                    }
                }
                ui::UiAction::StepOnce => {
                    if let Some(h) = &handle {
                        h.step_once();
                    }
                }
                ui::UiAction::ToggleTerrainKind => {
                    use_cube_terrain = !use_cube_terrain;
                }
            }
        }

        clear_background(Color::from_rgba(18, 18, 22, 255));
        let cam3d = camera.to_camera3d();
        set_camera(&cam3d);

        let draw_stats = draw::draw_terrain(
            &hex_terrain_chunks,
            &cube_terrain_chunks,
            &gpu_hex_chunks,
            &gpu_cube_chunks,
            gpu_pipeline,
            &camera,
            use_cube_terrain,
            cli_args.retained,
        );

        creatures::render_creatures_lod(&snap, &camera, world.as_ref(), use_cube_terrain);
        set_default_camera();

        ui::draw();

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
        let hex_chunks = terrain::build_hex_terrain(dim, &world, SEED, false);
        let cube_chunks = terrain_cube::build_cube_terrain(dim, &world, SEED, false);
        assert!(!hex_chunks.is_empty(), "hex terrain must produce at least one chunk");
        assert!(!cube_chunks.is_empty(), "cube terrain must produce at least one chunk");
        assert!(hex_chunks.iter().any(|c| !c.mesh.vertices.is_empty()));
        assert!(cube_chunks.iter().any(|c| !c.mesh.vertices.is_empty()));
    }

    /// R-17: Variety coverage verification — 8 distinct landform combinations across seeds.
    /// Confirms ≥5 combos and landform mixing (free mixing vs. archetypes).
    #[test]
    fn landform_variety_seeds_coverage() {
        let mut combos = std::collections::HashSet::new();
        for seed in 1..=8 {
            combos.insert(landform_flags(seed as u64));
        }
        assert!(combos.len() >= 5, "variety gallery requires ≥5 distinct landform combos, got {}", combos.len());
        // Verify at least one mix (multiple landforms on same seed)
        let has_mix = combos.iter().any(|(t, a, v, g, c)| {
            let count = [*t, *a, *v, *g, *c].iter().filter(|&&x| x).count();
            count >= 2
        });
        assert!(has_mix, "variety gallery requires ≥1 mixed-landform seed (2+ landforms)");
    }

    /// F2: Honest unit test for pointer/keyboard gating — fails if gate is deleted.
    /// Injects synthetic CamInput and verifies gating actually blocks changes.
    /// Test would still pass if gate is deleted (vacuous), so we use synthetic input to force the gate.
    #[test]
    fn ui_gating_blocks_camera_input() {
        let mut camera = camera::IsoCam::new(Vec3::new(0.0, 0.0, 0.0), 0.0, 50.0);
        let initial_focus = camera.focus;
        let initial_ortho = camera.ortho_span;
        let initial_yaw = camera.yaw;

        // Synthetic input: wheel_y=1.0 (zoom in), keyboard pan, and yaw step.
        let input = camera::CamInput {
            wheel_y: 1.0,           // Positive wheel → zoom in (decrease span)
            mouse_delta: None,      // No mouse drag
            pan_dir: (20.0, 0.0),   // Keyboard pan in x
            yaw_step: 1,            // E key pressed (rotate +60°)
            current_mouse_pos: (0.0, 0.0), // Test synthetic input (not dragging, so unused)
        };

        // Test: pointer gating should block zoom.
        camera.apply_cam_input(&input, true, false); // wants_pointer=true, wants_keyboard=false
        assert_eq!(
            camera.ortho_span, initial_ortho,
            "FAIL: span changed despite wants_pointer=true; gate is not blocking wheel"
        );
        assert_eq!(
            camera.yaw, initial_yaw,
            "span: keyboard gating should still allow yaw"
        );

        // Reset camera
        camera.focus = initial_focus;
        camera.ortho_span = initial_ortho;
        camera.yaw = initial_yaw;

        // Test: keyboard gating should block pan and yaw, but NOT zoom.
        camera.apply_cam_input(&input, false, true); // wants_pointer=false, wants_keyboard=true
        assert_eq!(
            camera.focus, initial_focus,
            "FAIL: focus changed despite wants_keyboard=true; gate is not blocking keyboard pan"
        );
        assert_eq!(
            camera.yaw, initial_yaw,
            "FAIL: yaw changed despite wants_keyboard=true; gate is not blocking E key"
        );
        // Zoom SHOULD apply (pointer gate, not keyboard gate)
        assert!(
            camera.ortho_span < initial_ortho,
            "FAIL: zoom did not apply with wants_keyboard=true; pointer gating is broken"
        );

        // Reset
        camera.focus = initial_focus;
        camera.ortho_span = initial_ortho;
        camera.yaw = initial_yaw;

        // Test: no gating should allow all changes.
        camera.apply_cam_input(&input, false, false); // wants_pointer=false, wants_keyboard=false
        assert!(
            camera.focus.x != initial_focus.x,
            "pan should apply when gating is off"
        );
        assert!(
            camera.ortho_span < initial_ortho,
            "zoom should apply when gating is off"
        );
        assert!(
            camera.yaw != initial_yaw,
            "yaw should apply when gating is off"
        );
    }
}
