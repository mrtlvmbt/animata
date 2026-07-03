//! animata v2 renderer — R-ladder scaffold. R-2 (issue #223): the first REAL view — a hex-voxel
//! terrain mesh (`WorldView` → flat-top hex columns + cliff quads, biome-colored) under a minimal
//! fixed 3D iso camera, with R-1's creatures now projected into that same view as dots. R-1 (#219,
//! merged #220) built the seam: the worker-thread `Sim` driver, the read-only `RenderSnapshot`
//! double buffer, and a proof-of-life naive 2D projection — R-2 replaces that projection with the
//! real 3D hex view; the sim seam itself (driver.rs) is untouched.
//!
//! R-3 (this slice): interactive pan/zoom/rotate IsoCam + box-frustum culling (terrain chunks +
//! creatures), minimal zoom-LOD, and the R-2 HMAX-literal footgun fix (cli consts now pub).
//!
//! OUT of scope here (later R-slices): creature LOD tiers/morphology (R-4), cube-voxel toggle (R-5),
//! full HUD/inspector/minimap (R-6).
//!
//! Not part of the v2 CI workspace (`v2/Cargo.toml`'s `exclude`) — a leaf bin, verified LOCALLY:
//! `cargo build`/`cargo clippy` from this directory + a manual run (window opens, hex terrain +
//! creature dots visible, HUD counts advance).

mod biome_palette;
mod camera;
mod driver;
mod hex;
mod terrain;

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
        window_title: "animata v2 — render scaffold (R-3 isocam + cull)".to_owned(),
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

// ── Pinned-param contract (critic F3, issue #223 acceptance; R-3 footgun fix) ────────────────────────
//
// The render's `WorldView` MUST resolve to the SAME terrain the sim worker runs on. `NoiseWorld` is a
// pure function of `(world_dim, hmax, seed)` — `cli::build_sim` constructs it as
// `NoiseWorld::new(econ.world_dim, HMAX, RESOURCE_BASE, config.seed ^ WORLD_SALT)` (`cli/src/lib.rs`).
// `HMAX`/`WORLD_SALT` are now `pub` in `cli`, so this file IMPORTS them directly from `cli` rather
// than duplicating literals. This removes the R-2 footgun where changing `cli`'s consts would silently
// diverge the rendered terrain from the sim's. `RESOURCE_BASE` is NOT mirrored: `NoiseWorld::height`/
// `biome`/`is_solid` (the only methods render reads) do not depend on it (only `resource()` does, which
// the render never calls) — see `world/src/lib.rs`. The values come from `cli::HMAX` and
// `cli::WORLD_SALT` (visible below as imports).

#[macroquad::main(window_conf)]
async fn main() {
    // macroquad's default per-draw-call buffer (10 000 verts / 5 000 indices) silently CLAMPS (drops
    // trailing geometry, logging "exceeded max drawcall size" every frame) a terrain chunk's worst
    // case (`ROWS_PER_CHUNK` rows × `world_dim` cols × ≤30 verts/≤48 indices per hex column,
    // `terrain.rs`). Raised well above that worst case — a one-time ~10 MB CPU/GPU buffer
    // allocation, not a per-frame cost.
    gl_set_drawcall_buffer_capacity(200_000, 400_000);

    let config = cli::default_config(SEED);
    let world_dim = config.econ.world_dim;

    // The render's OWN WorldView, built ONCE from the SAME (dim, hmax, seed) triple the sim worker
    // uses — the single source of provenance the pinned-param contract above requires. `resource_base`
    // is unused by the methods read below, so `0` is a documented don't-care (see the contract note).
    // R-3 footgun fix: use `cli::HMAX` and `cli::WORLD_SALT` directly (now pub).
    let world = world::NoiseWorld::new(world_dim, cli::HMAX, 0, config.seed ^ cli::WORLD_SALT);
    let terrain_chunks = terrain::build_hex_terrain(world_dim, &world);

    // R-3: Interactive isometric camera — pan (WASD/arrows + mouse drag), zoom (scroll),
    // rotate (yaw: Q/E or comma/period). Starts centered on the world at a standard iso view.
    let (span_x, _) = hex::hex_center(world_dim, 0);
    let (_, span_z) = hex::hex_center(0, world_dim);
    let world_span = span_x.max(span_z).max(1.0);
    let center = Vec3::new(span_x * 0.5, hex::HEIGHT_SCALE * cli::HMAX as f32 * 0.5, span_z * 0.5);
    let mut camera = IsoCam::new(center, 0.0, world_span * 1.5);

    let handle = driver::spawn(SEED);

    loop {
        if is_key_pressed(KeyCode::Space) {
            handle.toggle_pause();
        }
        if is_key_pressed(KeyCode::Right) || is_key_pressed(KeyCode::N) {
            handle.step_once();
        }

        clear_background(Color::from_rgba(18, 18, 22, 255));

        let snap = handle.latest();

        // R-3: Update camera input and build frustum.
        camera.update();
        let cam3d = camera.to_camera3d();
        let frustum_planes = camera.frustum_planes();

        set_camera(&cam3d);

        // R-3: Frustum-cull terrain chunks — only draw chunks whose AABB intersects the frustum.
        let mut chunks_drawn = 0;
        for chunk in &terrain_chunks {
            let (min, max) = chunk.bounds;
            if frustum_planes.iter().all(|plane| plane.aabb_intersects(min, max)) {
                draw_mesh(&chunk.mesh);
                chunks_drawn += 1;
            }
        }

        // R-4: Creatures rendered by px_per_m LOD tier (point → sphere → morphology).
        // Projected into the SAME hex view: world (x, z) → hex center of cell, floating above terrain.
        // Tier selection is a pure function of camera zoom ONLY (RnD R21), never per-creature distance.
        if let Some(s) = snap.as_ref() {
            let px_per_m = camera.px_per_m(); // Pure fn of ortho_span + viewport; whole frame shares one tier.

            for c in &s.creatures {
                let (cx, cz) = hex::hex_center(c.pos.0, c.pos.1);
                let h = world.height(c.pos.0, c.pos.1) as f32 * hex::HEIGHT_SCALE;
                let creature_pos = vec3(cx, h + 0.15, cz);

                // R-3 frustum cull: skip creatures outside the view frustum.
                if !camera.point_in_frustum(creature_pos) {
                    continue;
                }

                // Base color by cell_type (used across all tiers).
                let color = match c.cell_type {
                    Some(sim_core::CellType::A) => YELLOW,
                    Some(sim_core::CellType::B) => SKYBLUE,
                    Some(sim_core::CellType::Mixed) => GREEN,
                    None => WHITE,
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
                    // ─── NEAR tier: minimal cell-type morphology (shape differentiation) ───────────────
                    // Each cell_type has a small distinctive form, sized by creature's `size`.
                    let size_scale = c.size as f32 / 16.0;
                    let base_size = 0.15 * size_scale;

                    match c.cell_type {
                        Some(sim_core::CellType::A) => {
                            // Type A: main body + upper accent sphere (a small top ball).
                            draw_sphere(creature_pos, base_size, None, color);
                            draw_sphere(creature_pos + vec3(0.0, base_size * 1.2, 0.0), base_size * 0.5, None, YELLOW);
                        }
                        Some(sim_core::CellType::B) => {
                            // Type B: main body + side accent sphere (a small offset ball).
                            draw_sphere(creature_pos, base_size, None, color);
                            draw_sphere(creature_pos + vec3(base_size * 1.2, 0.0, 0.0), base_size * 0.5, None, SKYBLUE);
                        }
                        Some(sim_core::CellType::Mixed) => {
                            // Type Mixed: main body + front accent sphere (a small forward ball).
                            draw_sphere(creature_pos, base_size, None, color);
                            draw_sphere(creature_pos + vec3(0.0, 0.0, base_size * 1.2), base_size * 0.5, None, GREEN);
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
            egui::Window::new("v2 render scaffold — R-3").show(ctx, |ui| {
                match snap.as_ref() {
                    Some(s) => {
                        ui.label(format!("tick: {}", s.tick));
                        ui.label(format!("population: {}", s.population));
                        ui.label(format!("species: {}", s.species_count));
                        ui.label(format!("creatures drawn: {}", s.creatures.len()));
                    }
                    None => {
                        ui.label("waiting for the sim worker's first tick…");
                    }
                }
                ui.label(format!("terrain: {world_dim}×{world_dim} hexes, {} mesh chunks", terrain_chunks.len()));
                ui.label(format!("chunks drawn: {}/{}", chunks_drawn, terrain_chunks.len()));
                ui.separator();
                ui.label("Pan: WASD / arrows / middle-drag");
                ui.label("Zoom: mouse wheel (clamped 5..200)");
                ui.label("Rotate: Q/E or ,/. (60° steps)");
                ui.separator();
                ui.label(if handle.is_paused() {
                    "PAUSED — Space to resume"
                } else {
                    "running — Space to pause"
                });
                ui.label("Right / N: step once while paused");
            });
        });
        egui_macroquad::draw();

        next_frame().await;
    }
}
