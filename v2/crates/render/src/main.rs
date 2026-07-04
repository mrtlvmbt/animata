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
mod hex;
mod terrain;
mod terrain_cube;

use camera::IsoCam;
use macroquad::prelude::*;
use sim_core::WorldView;

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

/// R-8 (#261): parsed CLI flags. `--standalone`/`--no-sim` are aliases for the same no-sim mode.
struct CliArgs {
    standalone: bool,
    seed: u64,
    /// Only honoured in standalone mode — see the module doc comment for why.
    dim_override: Option<i64>,
}

fn parse_args() -> CliArgs {
    let mut standalone = false;
    let mut seed = SEED;
    let mut dim_override = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--standalone" | "--no-sim" => standalone = true,
            "--seed" => {
                let v = args.next().expect("--seed requires a value");
                seed = v.parse().unwrap_or_else(|_| panic!("--seed expects a u64, got {v:?}"));
            }
            "--dim" => {
                let v = args.next().expect("--dim requires a value");
                dim_override = Some(v.parse().unwrap_or_else(|_| panic!("--dim expects an integer, got {v:?}")));
            }
            other => eprintln!("render: ignoring unknown arg {other:?}"),
        }
    }
    CliArgs { standalone, seed, dim_override }
}

// ── Pinned-param contract (W-6 WIRE: ProcgenWorld; critic F3, issue #223 acceptance; R-6) ──────────
//
// The render's `WorldView` MUST resolve to the SAME terrain the sim worker runs on. `ProcgenWorld` is
// a pure function of `(world_dim, hmax, resource_base, seed)` — `cli::build_sim` constructs it as
// `ProcgenWorld::new(econ.world_dim, HMAX, econ.resource_base, config.seed ^ WORLD_SALT)` (`cli/src/lib.rs`).
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
    // `terrain.rs`). Raised well above that worst case — a one-time ~10 MB CPU/GPU buffer
    // allocation, not a per-frame cost.
    gl_set_drawcall_buffer_capacity(200_000, 400_000);

    let cli_args = parse_args();
    let config = cli::default_config(cli_args.seed);
    // R-8: `--dim` only applies in standalone — in sim mode the worker thread builds its OWN world
    // from `config.econ.world_dim` (via `cli::build_sim`), so overriding it here alone would desync
    // the render's terrain from the sim's, breaking the pinned-param contract below.
    let world_dim = if cli_args.standalone {
        cli_args.dim_override.unwrap_or(config.econ.world_dim)
    } else {
        config.econ.world_dim
    };

    // The render's OWN WorldView, built ONCE from the SAME (dim, hmax, resource_base, seed) tuple the
    // sim worker uses — the single source of provenance the pinned-param contract above requires.
    // W-6 WIRE: use `cli::HMAX`, `cli::RESOURCE_BASE`, `cli::WORLD_SALT` directly (now pub). The
    // ProcgenWorld pipeline (integer relief + erosion + biome/edaphic + caps) runs once here and caches
    // all per-cell fields (height, biome, resource, etc.) — never re-run per frame or per query.
    let world = world::ProcgenWorld::new(world_dim, cli::HMAX, cli::RESOURCE_BASE, config.seed ^ cli::WORLD_SALT);
    let hex_terrain_chunks = terrain::build_hex_terrain(world_dim, &world);
    let cube_terrain_chunks = terrain_cube::build_cube_terrain(world_dim, &world);

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
                    // ─── MID tier: cell-type-colored sphere (R-3 behavior) ─────────────────────────────
                    // Standard rendering: a sphere scaled by `size` and energy. This is the workhorse tier.
                    // Size is [1..32], so scale by (size / 16.0) for a base of ~0.1 world units at size=16.
                    let size_scale = c.size as f32 / 16.0;
                    let energy_factor = (c.energy as f32).max(0.0).sqrt() / 100.0; // Rough energy visual cue.
                    let radius = (size_scale * energy_factor).max(0.08); // Clamp to visible minimum.
                    draw_sphere(creature_pos, radius, None, color);
                } else {
                    // ─── NEAR tier: minimal cell-type morphology + uptake_layer base color ───────────────
                    // Each cell_type has a small distinctive form, sized by creature's `size`.
                    // Base color is uptake_layer (feeding guild); morphology reflects cell_type (if available).
                    let size_scale = c.size as f32 / 16.0;
                    let base_size = 0.15 * size_scale;

                    match c.cell_type {
                        Some(sim_core::CellType::A) => {
                            // Type A: main body + upper accent sphere (a small top ball).
                            draw_sphere(creature_pos, base_size, None, color);
                            // Accent in a brighter shade of the uptake_layer color
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
                        None => {
                            // Neutral: single sphere (for non-morphogen configs).
                            draw_sphere(creature_pos, base_size, None, color);
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
        let world = world::ProcgenWorld::new(dim, cli::HMAX, cli::RESOURCE_BASE, SEED ^ cli::WORLD_SALT);
        let hex_chunks = terrain::build_hex_terrain(dim, &world);
        let cube_chunks = terrain_cube::build_cube_terrain(dim, &world);
        assert!(!hex_chunks.is_empty(), "hex terrain must produce at least one chunk");
        assert!(!cube_chunks.is_empty(), "cube terrain must produce at least one chunk");
        assert!(hex_chunks.iter().any(|c| !c.mesh.vertices.is_empty()));
        assert!(cube_chunks.iter().any(|c| !c.mesh.vertices.is_empty()));
    }
}
