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
mod loader_state;
mod raw_chunk;
mod terrain;
mod terrain_cube;
mod tuning;
mod ui;
mod world_builder;
mod world_spec;

use camera::IsoCam;
use macroquad::prelude::*;
use sim_core::WorldView;
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::time::Instant;
use world_spec::{landform_flags, WorldSpec, WorldSource, Stage};
use loader_state::{LoadState, AppPhase};
use raw_chunk::{RawChunk, BuiltWorld};
use terrain::TerrainChunk;

// ── R-4 LOD tier thresholds (px_per_m-driven) ──────────────────────────────────────────────────────
/// FAR tier: creatures are sub-pixel or nearly invisible (point/billboard). Triggers when px_per_m < 5.
/// At default 768px tall, this happens when ortho_span > ~154 world units (very far zoom).
const PX_PER_M_FAR_THRESHOLD: f32 = 5.0;

/// MID tier: creatures are cell-type-colored spheres (R-3 behavior). Active when 5 <= px_per_m < 20.
/// At default viewport, this is ortho_span in [38, 154] — a standard play range.
const PX_PER_M_MID_THRESHOLD: f32 = 20.0;

/// NEAR tier: creatures are minimal cell-type morphology (differentiated small shapes).
/// Triggers when px_per_m >= 20 (ortho_span <= ~38, zoomed in close).

// ── U-2: World building and assembly helpers ─────────────────────────────────────────────────────────

/// Convert a RawChunk (worker-thread-safe buffers) to a TerrainChunk (GPU-side Mesh).
/// Called on the main thread after build_world completes.
fn raw_chunk_to_terrain_chunk(raw: RawChunk) -> TerrainChunk {
    let mesh = Mesh {
        vertices: raw.vertices,
        indices: raw.indices,
        texture: None,
    };
    TerrainChunk {
        mesh,
        bounds: (raw.lo, raw.hi),
    }
}

/// Convert all raw chunks to terrain chunks (GPU assembly).
fn convert_raw_chunks(raw_chunks: Vec<RawChunk>) -> Vec<TerrainChunk> {
    raw_chunks.into_iter().map(raw_chunk_to_terrain_chunk).collect()
}

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
    /// U-2: `--slow-load`: inject ~600ms delay per stage (for loader screenshot capture).
    slow_load: bool,
    /// U-2: `--screenshot-loader <path>`: capture loader modal mid-build, save PNG, exit.
    screenshot_loader: Option<String>,
    /// U-7: `--regen-to <seed>`: after startup, immediately regenerate the world to this seed (dev harness).
    /// When combined with `--slow-load`, shows the full-screen loader modal mid-regen.
    regen_to: Option<u64>,
    /// U-5: `--jump-to <x>,<z>`: after startup, jump camera to world coords (x, z) for viewport-quad + click-to-jump verification.
    jump_to: Option<(f32, f32)>,
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
    let mut slow_load = false;
    let mut screenshot_loader = None;
    let mut regen_to = None;
    let mut jump_to = None;

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
            "--slow-load" => {
                slow_load = true;
            }
            "--screenshot-loader" => {
                screenshot_loader = Some(args.next().expect("--screenshot-loader requires a path"));
                standalone = true;  // loader implies standalone
            }
            "--regen-to" => {
                let v = args.next().expect("--regen-to requires a u64 seed value");
                regen_to = Some(v.parse().unwrap_or_else(|_| panic!("--regen-to expects a u64, got {v:?}")));
                standalone = true; // reseed harness is standalone
            }
            "--jump-to" => {
                let coords_str = args.next().expect("--jump-to requires <x>,<z>");
                let parts: Vec<&str> = coords_str.split(',').collect();
                let x = parts.get(0).unwrap_or(&"0").parse::<f32>().unwrap_or_else(|_| panic!("--jump-to x must be f32, got {:?}", parts.get(0)));
                let z = parts.get(1).unwrap_or(&"0").parse::<f32>().unwrap_or_else(|_| panic!("--jump-to z must be f32, got {:?}", parts.get(1)));
                jump_to = Some((x, z));
                standalone = true; // jump harness is standalone
            }
            other => eprintln!("render: ignoring unknown arg {other:?}"),
        }
    }
    CliArgs { standalone, seed, dim_override, v1_dump, screenshot, screenshot_warmup, bench, cam_preset, retained, bare_mode, height_scale_override, slow_load, screenshot_loader, regen_to, jump_to }
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

    // U-7: Load tuning config (feel + key mapping) once at startup
    let tuning = tuning::Tuning::load();

    // U-2: Create WorldSpec (single source of truth for world building; D5: all inputs here)
    let mut spec = WorldSpec {
        seed: cli_args.seed,
        standalone: cli_args.standalone,
        bare_mode: cli_args.bare_mode,
        source: if let Some(path) = &cli_args.v1_dump {
            WorldSource::Dump(std::path::PathBuf::from(path))
        } else {
            // D5 dim rule: dim_request lives in Procgen, honored only if spec.standalone
            WorldSource::Procgen { dim_request: cli_args.dim_override }
        },
    };

    // U-2: For harnesses (screenshot/bench), build world inline before their loops
    // For app path, we'll spawn a worker thread and initialize from recv
    let is_harness = cli_args.screenshot.is_some() || cli_args.bench;

    let (mut hex_terrain_chunks, mut cube_terrain_chunks, mut world_dim, mut world): (Vec<TerrainChunk>, Vec<TerrainChunk>, i64, Box<dyn WorldView>) = if is_harness {
        // Harnesses: build_world inline (synchronous, no loader)
        let mut on_stage = |_stage: Stage| true;  // No-op callback for harnesses
        let built = world_builder::build_world(&spec, on_stage).expect("build_world failed");
        let world_dim = built.dim;
        let world = built.world;
        let hex_chunks = convert_raw_chunks(built.hex);
        let cube_chunks = convert_raw_chunks(built.cube);
        (hex_chunks, cube_chunks, world_dim, world)
    } else {
        // App path: world will be built on worker thread and received in Loading phase
        // Initialize with minimal dummy values (camera will reinit from built.dim after recv)
        let (tect, aeol, volc, glac, coast) = landform_flags(spec.seed, spec.standalone);
        let temp_world: Box<dyn WorldView> = Box::new(world::ProcgenWorld::new(
            config.econ.world_dim, cli::HMAX, cli::RESOURCE_BASE, spec.seed ^ cli::WORLD_SALT, None,
            tect, aeol, volc, glac, coast  // Use spec.seed (F4), landforms always match eventually
        ));
        (Vec::new(), Vec::new(), config.econ.world_dim, temp_world)
    };

    // R-15a: Retained-buffer GPU terrain initialization (if --retained).
    let (mut gpu_hex_chunks, mut gpu_cube_chunks, mut gpu_pipeline) = if cli_args.retained {
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

    // U-1: Initialize UI root with DebugPanel and MinimapPanel.
    let mut ui_root = ui::UiRoot::new();
    ui_root.push(Box::new(ui::DebugPanel));
    // U-5: Add MinimapPanel
    ui_root.push(Box::new(ui::MinimapPanel));

    // R-13: Apply camera preset to ensure deterministic view.
    let world_span = span_x.max(span_z).max(1.0);
    if cli_args.screenshot.is_some() || cli_args.bench {
        cli_args.cam_preset.apply_to_camera(&mut camera, center, world_span);
    }

    // U-2: App path: spawn world builder on worker thread
    let (mut app_phase, rx_built_world): (AppPhase, mpsc::Receiver<BuiltWorld>) = if !is_harness {
        let load_state = LoadState::new(cli_args.seed);
        let spec_worker = spec.clone();
        let slow_load_flag = cli_args.slow_load;

        let (tx, rx) = mpsc::channel();

        let load_clone = load_state.clone();

        let _ = std::thread::spawn(move || {
            let mut on_stage = |stage: Stage| {
                load_clone.set_stage(stage);
                if slow_load_flag {
                    std::thread::sleep(std::time::Duration::from_millis(600));
                }
                true
            };
            if let Ok(built) = world_builder::build_world(&spec_worker, on_stage) {
                load_clone.mark_done();
                let _ = tx.send(built);
            }
        });

        (AppPhase::Loading(load_state), rx)
    } else {
        // Harnesses don't use AppPhase
        (AppPhase::Running, mpsc::channel().1)
    };

    // U-3/F14: Regen state for harness mode (if --regen-to is set)
    // In screenshot mode, we may need to rebuild the world to a target seed.
    // This rx_regen_built will receive the BuiltWorld when ready.
    let mut harness_regen_load_state: Option<LoadState> = None;
    let rx_regen_built: Option<mpsc::Receiver<BuiltWorld>> = if let Some(target_seed) = cli_args.regen_to {
        if is_harness {
            // Harness mode: spawn async regen on a worker thread, wait for result before capture
            let regen_spec = WorldSpec {
                seed: target_seed,
                standalone: spec.standalone,
                bare_mode: spec.bare_mode,
                source: spec.source.clone(),
            };
            let load_state = LoadState::new(target_seed);
            harness_regen_load_state = Some(load_state.clone());
            let (tx, rx) = mpsc::channel();
            let slow_load_flag = cli_args.slow_load;  // Capture for thread
            let _ = std::thread::spawn(move || {
                let load_clone = load_state.clone();
                let mut on_stage = |stage: Stage| {
                    load_clone.set_stage(stage);
                    // U-7: Wire progress permille based on stage (matches interactive RegenSeed worker)
                    let progress = match stage {
                        Stage::GenerateWorld => 0,
                        Stage::BuildMeshes => 400,
                        Stage::Done => 1000,
                    };
                    load_clone.set_progress(progress);
                    // Honor --slow-load flag to stretch build stages (allow mid-build captures)
                    if slow_load_flag {
                        std::thread::sleep(std::time::Duration::from_millis(600));
                    }
                    true
                };
                if let Ok(built) = world_builder::build_world(&regen_spec, on_stage) {
                    let _ = tx.send(built);
                }
            });
            Some(rx)
        } else {
            None  // Will be handled in the main loop
        }
    } else {
        None
    };

    // R-13: Screenshot mode — render warmup frames, then capture on the final frame.
    if let Some(screenshot_path) = &cli_args.screenshot {
        // U-3/F14: Regen handling — different strategies for chip capture vs determinism gate.
        // --slow-load + --regen-to: render Running frames (old world + chip) while regen builds (chip visible).
        // Otherwise: block immediately on regen before rendering (determinism contract).
        let mut rx_regen_blocking = rx_regen_built;
        let render_with_regen = cli_args.slow_load && rx_regen_blocking.is_some();

        if !render_with_regen {
            // Determinism gate path (no --slow-load): block on regen before rendering
            if let Some(ref rx_regen) = rx_regen_blocking {
                if let Ok(built) = rx_regen.recv() {
                    hex_terrain_chunks = convert_raw_chunks(built.hex);
                    cube_terrain_chunks = convert_raw_chunks(built.cube);
                    world_dim = built.dim;
                    world = built.world;
                    if cli_args.retained {
                        use macroquad::prelude::get_internal_gl;
                        let chunk_vert = load_shader("chunk_v2.vert");
                        let chunk_frag = load_shader("chunk_v2.frag");
                        let mut gl = unsafe { get_internal_gl() };
                        let ctx = gl.quad_context;
                        let pipeline = gpu_terrain::chunk_pipeline(ctx, &chunk_vert, &chunk_frag);
                        gpu_hex_chunks = gpu_terrain::upload_chunks(ctx, &hex_terrain_chunks);
                        gpu_cube_chunks = gpu_terrain::upload_chunks(ctx, &cube_terrain_chunks);
                        gpu_pipeline = Some(pipeline);
                    }
                }
            }
            rx_regen_blocking = None;  // Already consumed
        }

        let mut jump_to_fired = false;
        for frame_num in 0..=cli_args.screenshot_warmup {
            // U-3: When --slow-load + regen in flight, render Running phase (old world + chip).
            // LoadState progress indicates if regen is still building (< 1000 permille = < 100%).
            let regen_still_building = render_with_regen && harness_regen_load_state.as_ref()
                .map(|ls| ls.get_progress() < 1000)
                .unwrap_or(false);

            if regen_still_building {
                // Modal loader path: render the full-screen loader modal (v1 parity), NOT panels.
                // CRITICAL: when modal is up, skip ui_root entirely (modal XOR panels, never both).
                let snap = handle.as_ref().and_then(|h| h.latest());
                let terrain_chunks = if use_cube_terrain { &cube_terrain_chunks } else { &hex_terrain_chunks };
                let mut ui_actions: Vec<ui::UiAction> = Vec::new();

                // U-5: Fire --jump-to action in harness on frame 0 (before any rendering)
                if frame_num == 0 && !jump_to_fired {
                    if let Some((x, z)) = cli_args.jump_to {
                        ui_actions.push(ui::UiAction::JumpCamera(glam::vec2(x, z)));
                        jump_to_fired = true;
                    }
                }

                egui_macroquad::ui(|ctx| {
                    // U-6: Set DPI scale for Retina/HiDPI displays (high_dpi=true in window_conf)
                    ctx.set_pixels_per_point(macroquad::miniquad::window::dpi_scale());
                    // Draw the loader modal using the same LoadState the regen worker bumps
                    if let Some(ref load_state) = harness_regen_load_state {
                        ui::loader::draw(ctx, load_state);
                    }
                });

                // Apply UI actions (includes --jump-to)
                for action in ui_actions {
                    match action {
                        ui::UiAction::JumpCamera(world_pos) => {
                            camera.focus = glam::vec3(world_pos.x, camera.focus.y, world_pos.y);
                        }
                        _ => {}  // Ignore other actions in harness
                    }
                }

                clear_background(Color::from_rgba(18, 18, 22, 255));
                camera.update(&tuning);
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
                ui::draw();  // Render the chip UI

                // Capture at final frame while regen is still in progress (chip guaranteed visible under --slow-load)
                if frame_num == cli_args.screenshot_warmup {
                    let img = get_screen_data();
                    img.export_png(screenshot_path);
                    println!("[screenshot] captured chip mid-regen to {}", screenshot_path);
                    std::process::exit(0);
                }
            } else if render_with_regen && frame_num == 0 {
                // Regen just completed: swap worlds now before rendering
                if let Some(ref rx) = rx_regen_blocking {
                    if let Ok(built) = rx.recv() {
                        hex_terrain_chunks = convert_raw_chunks(built.hex);
                        cube_terrain_chunks = convert_raw_chunks(built.cube);
                        world_dim = built.dim;
                        world = built.world;
                        spec.seed = built.seed;
                        if cli_args.retained {
                            use macroquad::prelude::get_internal_gl;
                            let chunk_vert = load_shader("chunk_v2.vert");
                            let chunk_frag = load_shader("chunk_v2.frag");
                            let mut gl = unsafe { get_internal_gl() };
                            let ctx = gl.quad_context;
                            let pipeline = gpu_terrain::chunk_pipeline(ctx, &chunk_vert, &chunk_frag);
                            gpu_hex_chunks = gpu_terrain::upload_chunks(ctx, &hex_terrain_chunks);
                            gpu_cube_chunks = gpu_terrain::upload_chunks(ctx, &cube_terrain_chunks);
                            gpu_pipeline = Some(pipeline);
                        }
                        harness_regen_load_state = None;
                        rx_regen_blocking = None;
                    }
                }
                // Fall through to normal render
            }

            // Normal screenshot render (after regen complete or no regen)
            if !regen_still_building {
                let no_ui = ui::UiOut::default();
                for ev in input::collect(&no_ui) {
                    match ev {
                        input::InputEvent::TogglePause => {
                            if let Some(h) = &handle { h.toggle_pause(); }
                        }
                        input::InputEvent::StepOnce => {
                            if let Some(h) = &handle { h.step_once(); }
                        }
                        input::InputEvent::ToggleTerrainKind => {}
                        input::InputEvent::RegenSeed => {}
                    }
                }

                clear_background(Color::from_rgba(18, 18, 22, 255));
                let snap = handle.as_ref().and_then(|h| h.latest());

                // U-5: Fire --jump-to action in harness on frame 0 (normal screenshot path)
                if frame_num == 0 && !jump_to_fired {
                    if let Some((x, z)) = cli_args.jump_to {
                        camera.focus = glam::vec3(x, camera.focus.y, z);
                        jump_to_fired = true;
                    }
                }

                camera.update(&tuning);
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

                // R-13 F-B5: Capture on final frame
                if frame_num == cli_args.screenshot_warmup {
                    let img = get_screen_data();
                    img.export_png(screenshot_path);
                    println!("[screenshot] captured to {}", screenshot_path);
                    std::process::exit(0);
                }
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
                    input::InputEvent::RegenSeed => {
                        // Not used in benchmark mode
                    }
                }
            }

            clear_background(Color::from_rgba(18, 18, 22, 255));
            let snap = handle.as_ref().and_then(|h| h.latest());

            camera.update(&tuning);
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
                    input::InputEvent::RegenSeed => {
                        // Not used in benchmark mode
                    }
                }
            }

            clear_background(Color::from_rgba(18, 18, 22, 255));
            let snap = handle.as_ref().and_then(|h| h.latest());

            let cpu_start = Instant::now();

            camera.update(&tuning);
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

    // U-2: Frame counter for --screenshot-loader (capture loader mid-build)
    let mut loading_frame_count = 0u32;

    // U-3: Reseed state (in-progress world rebuild on worker thread)
    let mut rx_regen_in_flight: Option<mpsc::Receiver<BuiltWorld>> = None;
    let mut regen_load_state: Option<LoadState> = None;
    // U-5: Track whether we've fired the --jump-to action (fire once per run)
    let mut jump_to_fired = false;

    loop {
        // U-2/D4: AppPhase state machine — Loading renders only loader, Running renders world
        match &mut app_phase {
            AppPhase::Loading(ref load_state) => {
                // Loading phase: render loader modal, poll worker for completion
                clear_background(Color::from_rgba(18, 18, 22, 255));

                egui_macroquad::ui(|ctx| {
                    // U-6: Set DPI scale for Retina/HiDPI displays (high_dpi=true in window_conf)
                    ctx.set_pixels_per_point(macroquad::miniquad::window::dpi_scale());
                    ui::loader::draw(ctx, load_state);
                });

                // Flush egui to framebuffer (critical: must happen before capture or get_screen_data)
                egui_macroquad::draw();

                // U-2: Capture loader screenshot at frame ~20 (stable mid-load state)
                if let Some(ref screenshot_path) = cli_args.screenshot_loader {
                    if loading_frame_count == 20 {
                        let img = get_screen_data();
                        img.export_png(screenshot_path);
                        println!("[screenshot-loader] captured loader to {}", screenshot_path);
                        std::process::exit(0);
                    }
                    loading_frame_count += 1;
                }

                // U-2/F1: Try to receive BuiltWorld from worker thread
                if let Ok(built) = rx_built_world.try_recv() {
                    // Mesh assembly: convert RawChunks to TerrainChunks (GPU-side)
                    hex_terrain_chunks = convert_raw_chunks(built.hex);
                    cube_terrain_chunks = convert_raw_chunks(built.cube);
                    world_dim = built.dim;
                    world = built.world;

                    // U-2/D5: Camera init from built.dim (output, not input)
                    let (span_x, _) = hex::hex_center(world_dim, 0);
                    let (_, span_z) = hex::hex_center(0, world_dim);
                    let world_span = span_x.max(span_z).max(1.0);
                    let center = Vec3::new(span_x * 0.5, hex::HEIGHT_SCALE * cli::HMAX as f32 * 0.5, span_z * 0.5);
                    camera = IsoCam::new(center, 0.0, world_span * 1.5);

                    // U-2/D5: GPU upload if --retained (GL-thread-only work)
                    if cli_args.retained {
                        use macroquad::prelude::get_internal_gl;
                        let chunk_vert = load_shader("chunk_v2.vert");
                        let chunk_frag = load_shader("chunk_v2.frag");

                        let mut gl = unsafe { get_internal_gl() };
                        let ctx = gl.quad_context;
                        let pipeline = gpu_terrain::chunk_pipeline(ctx, &chunk_vert, &chunk_frag);
                        gpu_hex_chunks = gpu_terrain::upload_chunks(ctx, &hex_terrain_chunks);
                        gpu_cube_chunks = gpu_terrain::upload_chunks(ctx, &cube_terrain_chunks);
                        gpu_pipeline = Some(pipeline);
                    }

                    // Transition to Running
                    app_phase = AppPhase::Running;
                }

                next_frame().await;
            }
            AppPhase::Running => {
                // Running phase: render world as normal (original main loop body)
                let snap = handle.as_ref().and_then(|h| h.latest());
                let terrain_chunks = if use_cube_terrain { &cube_terrain_chunks } else { &hex_terrain_chunks };

                // F1: Collect all commands (UiActions + InputEvents) and apply them in one place
                let mut ui_actions: Vec<ui::UiAction> = Vec::new();

                // U-5: Fire --jump-to action once at start of Running phase
                if !jump_to_fired {
                    if let Some((x, z)) = cli_args.jump_to {
                        ui_actions.push(ui::UiAction::JumpCamera(glam::vec2(x, z)));
                        jump_to_fired = true;
                    }
                }

                egui_macroquad::ui(|ctx| {
                    // U-6: Set DPI scale for Retina/HiDPI displays (high_dpi=true in window_conf)
                    ctx.set_pixels_per_point(macroquad::miniquad::window::dpi_scale());

                    // U-7: If regen is in flight, draw the full-screen loader modal (v1 parity).
                    // CRITICAL: when modal is up, skip ui_root entirely (modal XOR panels, never both).
                    let mut wants_pointer_regen = false;
                    let mut wants_keyboard_regen = false;
                    let ui_out = if let Some(ref load_state) = regen_load_state {
                        ui::loader::draw(ctx, load_state);
                        wants_pointer_regen = true;
                        wants_keyboard_regen = true;
                        // Return a default UiOut with pointer/keyboard gating set; no panels drawn
                        ui::UiOut {
                            wants_pointer: true,
                            wants_keyboard: true,
                        }
                    } else {
                        // Plan D3: UI draws BEFORE world input to read previous-frame egui state.
                        // This causes a 1-frame lag (acceptable, identical to v1 behavior).
                        let mut ui_ctx = ui::UiCtx {
                            world_dim,
                            seed: spec.seed,  // F4: use spec.seed (spec-driven design)
                            fps: get_fps(),
                            chunks_drawn: 0, // Will be updated after drawing
                            verts: 0,
                            snap: snap.as_ref().map(|v| &**v),
                            standalone_mode: cli_args.standalone,
                            terrain_chunks_total: terrain_chunks.len(),
                            actions: &mut ui_actions,
                            // U-3/F12: gate "New world" button visibility
                            is_procgen: matches!(spec.source, WorldSource::Procgen { .. }),
                            // U-3: pass regen state (now used for modal display)
                            regen_load_state: regen_load_state.as_ref(),
                            // U-5: pass world reference for minimap rendering
                            world: Some(world.as_ref()),
                            bare_mode: spec.bare_mode,
                            cache: std::ptr::null_mut(),  // Will be set by ui_root.draw()
                            // U-5: pass camera state for minimap viewport quad
                            camera_focus: camera.focus,
                            camera_ortho_span: camera.ortho_span,
                            camera_yaw: camera.yaw,
                            screen_dims: (screen_width(), screen_height()),
                        };
                        ui_root.draw(ctx, &mut ui_ctx)
                    };

                    // Apply camera update with gating.
                    // U-7: Gate camera input when regen loader modal is showing
                    camera.update_gated(&tuning, ui_out.wants_pointer || wants_pointer_regen, ui_out.wants_keyboard || wants_keyboard_regen);

                    // Collect keyboard input events with gating.
                    // U-7: Skip input when regen loader modal is showing (gated by wants_keyboard_regen)
                    if !wants_keyboard_regen && !ui_out.wants_keyboard {
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
                                // U-3: N key triggers reseed (only valid in Procgen+standalone; gating here)
                                input::InputEvent::RegenSeed => {
                                    let can_reseed = matches!(spec.source, WorldSource::Procgen { .. }) && spec.standalone && rx_regen_in_flight.is_none();
                                    if can_reseed {
                                        // Trigger reseed with next seed (current+1)
                                        ui_actions.push(ui::UiAction::RegenSeed(spec.seed.wrapping_add(1)));
                                    }
                                }
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
                        // U-3: Reseed — spawn async world build on worker, keep rendering old world
                        ui::UiAction::RegenSeed(target_seed) => {
                            if rx_regen_in_flight.is_none() && matches!(spec.source, WorldSource::Procgen { .. }) && spec.standalone {
                                let regen_spec = WorldSpec {
                                    seed: target_seed,
                                    standalone: spec.standalone,
                                    bare_mode: spec.bare_mode,
                                    source: spec.source.clone(),
                                };
                                let load_state = LoadState::new(target_seed);
                                regen_load_state = Some(load_state.clone());
                                let (tx, rx) = mpsc::channel();
                                let slow_load_flag = cli_args.slow_load;  // Capture for thread
                                let _ = std::thread::spawn(move || {
                                    let load_clone = load_state.clone();
                                    let mut on_stage = |stage: Stage| {
                                        load_clone.set_stage(stage);
                                        // U-7: Wire progress permille based on stage (matches harness worker)
                                        let progress = match stage {
                                            Stage::GenerateWorld => 0,
                                            Stage::BuildMeshes => 400,
                                            Stage::Done => 1000,
                                        };
                                        load_clone.set_progress(progress);
                                        // Honor --slow-load flag to stretch build stages (matches harness worker)
                                        if slow_load_flag {
                                            std::thread::sleep(std::time::Duration::from_millis(600));
                                        }
                                        true
                                    };
                                    if let Ok(built) = world_builder::build_world(&regen_spec, on_stage) {
                                        let _ = tx.send(built);
                                    }
                                });
                                rx_regen_in_flight = Some(rx);
                            }
                        }
                        // U-5: Jump camera to a world position (from minimap click)
                        ui::UiAction::JumpCamera(world_pos) => {
                            camera.focus = glam::vec3(world_pos.x, camera.focus.y, world_pos.y);
                        }
                    }
                }

                // U-3: Poll for in-flight reseed completion and swap worlds if ready
                if let Some(ref mut rx) = rx_regen_in_flight {
                    if let Ok(built) = rx.try_recv() {
                        // Reseed complete: swap worlds and rebuild meshes
                        hex_terrain_chunks = convert_raw_chunks(built.hex);
                        cube_terrain_chunks = convert_raw_chunks(built.cube);
                        world_dim = built.dim;
                        world = built.world;

                        // Update spec.seed to match the new world
                        spec.seed = built.seed;

                        // U-2/D5: GPU upload if --retained
                        if cli_args.retained {
                            use macroquad::prelude::get_internal_gl;
                            let chunk_vert = load_shader("chunk_v2.vert");
                            let chunk_frag = load_shader("chunk_v2.frag");
                            let mut gl = unsafe { get_internal_gl() };
                            let ctx = gl.quad_context;
                            let pipeline = gpu_terrain::chunk_pipeline(ctx, &chunk_vert, &chunk_frag);
                            gpu_hex_chunks = gpu_terrain::upload_chunks(ctx, &hex_terrain_chunks);
                            gpu_cube_chunks = gpu_terrain::upload_chunks(ctx, &cube_terrain_chunks);
                            gpu_pipeline = Some(pipeline);
                        }

                        rx_regen_in_flight = None;
                        regen_load_state = None;
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
            combos.insert(landform_flags(seed as u64, true));  // standalone mode for variety
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
            mouse_pos: (400.0, 300.0), // Center of 800x600 screen
            screen_dims: (800.0, 600.0), // Standard test viewport
            left_button_down: false,
            left_button_pressed: false,
            mouse_delta: None,      // No mouse drag
            pan_dir: (20.0, 0.0),   // Keyboard pan in x
            yaw_step: 1,            // E key pressed (rotate +60°)
        };

        // Test: pointer gating should block zoom, but keyboard should still work.
        camera.apply_cam_input(&input, &tuning::Tuning::default(), true, false); // wants_pointer=true, wants_keyboard=false
        assert_eq!(
            camera.ortho_span, initial_ortho,
            "FAIL: zoom changed despite wants_pointer=true; pointer gate should block wheel"
        );
        assert_ne!(
            camera.yaw, initial_yaw,
            "FAIL: yaw did not change with wants_keyboard=false; keyboard should be free for Q/E yaw"
        );
        assert_ne!(
            camera.focus, initial_focus,
            "FAIL: focus did not change with wants_keyboard=false; keyboard should be free for WASD pan"
        );

        // Reset camera
        camera.focus = initial_focus;
        camera.ortho_span = initial_ortho;
        camera.yaw = initial_yaw;

        // Test (U-1): keyboard gating should block pan and yaw, but NOT zoom.
        // Sub-case (a): Pure keyboard gating — wheel_y=0, pan_dir!=0, yaw_step!=0.
        // Focus should remain exactly frozen (no zoom-to-cursor side effect).
        let input_keyboard_only = camera::CamInput {
            wheel_y: 0.0,            // NO wheel input
            mouse_pos: (400.0, 300.0),
            screen_dims: (800.0, 600.0),
            left_button_down: false,
            left_button_pressed: false,
            mouse_delta: None,
            pan_dir: (20.0, 0.0),   // Keyboard pan in x
            yaw_step: 1,            // E key pressed (rotate +60°)
        };
        camera.apply_cam_input(&input_keyboard_only, &tuning::Tuning::default(), false, true); // wants_pointer=false, wants_keyboard=true
        assert_eq!(
            camera.focus, initial_focus,
            "FAIL: focus changed under keyboard gate + wheel_y=0; gate is not blocking keyboard pan"
        );
        assert_eq!(
            camera.yaw, initial_yaw,
            "FAIL: yaw changed under keyboard gate; gate is not blocking E key"
        );
        assert_eq!(
            camera.ortho_span, initial_ortho,
            "FAIL: ortho_span changed with wheel_y=0; something is wrong"
        );

        // Reset camera
        camera.focus = initial_focus;
        camera.ortho_span = initial_ortho;
        camera.yaw = initial_yaw;

        // Sub-case (b): Keyboard gating with zoom-to-cursor (U-4 new semantics).
        // wheel_y!=0 is pointer-domain, correctly allowed under wants_keyboard=true.
        // Focus SHOULD shift to keep ground point under cursor (U-4 zoom-to-cursor feature).
        // Pan (keyboard) and yaw (keyboard) should still be blocked.
        let input_with_wheel = camera::CamInput {
            wheel_y: 1.0,            // Positive wheel → zoom in (decrease span)
            mouse_pos: (400.0, 300.0), // Center of screen → zoom-to-cursor is ~neutral
            screen_dims: (800.0, 600.0),
            left_button_down: false,
            left_button_pressed: false,
            mouse_delta: None,
            pan_dir: (20.0, 0.0),   // Keyboard pan in x (should be blocked)
            yaw_step: 1,            // E key (should be blocked)
        };
        camera.apply_cam_input(&input_with_wheel, &tuning::Tuning::default(), false, true); // wants_pointer=false,wants_keyboard=true
        assert_eq!(
            camera.yaw, initial_yaw,
            "FAIL: yaw changed despite wants_keyboard=true; keyboard gate broken"
        );
        // Zoom SHOULD apply (pointer gate, not keyboard gate)
        assert!(
            camera.ortho_span < initial_ortho,
            "FAIL: zoom did not apply with wants_keyboard=true; pointer gating is broken"
        );
        // Focus may shift slightly due to zoom-to-cursor, but keyboard pan should be blocked.
        // At screen center, zoom-to-cursor is nearly neutral (focus shift ~0), so allow epsilon.
        let focus_shift_x = (camera.focus.x - initial_focus.x).abs();
        let focus_shift_z = (camera.focus.z - initial_focus.z).abs();
        assert!(
            focus_shift_x < 0.01,
            "FAIL: focus X shifted too much under zoom-to-cursor at screen center; expected ~0, got {}",
            focus_shift_x
        );
        assert!(
            focus_shift_z < 0.01,
            "FAIL: focus Z shifted too much under zoom-to-cursor at screen center; expected ~0, got {}",
            focus_shift_z
        );

        // Reset
        camera.focus = initial_focus;
        camera.ortho_span = initial_ortho;
        camera.yaw = initial_yaw;

        // Test: no gating should allow all changes.
        camera.apply_cam_input(&input, &tuning::Tuning::default(), false, false); // wants_pointer=false, wants_keyboard=false
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
