//! animata v2 renderer — R-ladder scaffold. R-2 (issue #223): the first REAL view — a hex-voxel
//! terrain mesh (`WorldView` → flat-top hex columns + cliff quads, biome-colored) under a minimal
//! fixed 3D iso camera, with R-1's creatures now projected into that same view as dots. R-1 (#219,
//! merged #220) built the seam: the worker-thread `Sim` driver, the read-only `RenderSnapshot`
//! double buffer, and a proof-of-life naive 2D projection — R-2 replaces that projection with the
//! real 3D hex view; the sim seam itself (driver.rs) is untouched.
//!
//! OUT of scope here (later R-slices): interactive pan/zoom/rotate + box-frustum culling (R-3),
//! creature LOD/morphology (R-4), the cube-voxel toggle (R-5), full HUD/inspector/minimap (R-6).
//!
//! Not part of the v2 CI workspace (`v2/Cargo.toml`'s `exclude`) — a leaf bin, verified LOCALLY:
//! `cargo build`/`cargo clippy` from this directory + a manual run (window opens, hex terrain +
//! creature dots visible, HUD counts advance).

mod biome_palette;
mod driver;
mod hex;
mod terrain;

use macroquad::prelude::*;
use sim_core::WorldView;

fn window_conf() -> Conf {
    Conf {
        window_title: "animata v2 — render scaffold (R-2 hex terrain)".to_owned(),
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

// ── Pinned-param contract (critic F3, issue #223 acceptance) ────────────────────────────────────────
//
// The render's `WorldView` MUST resolve to the SAME terrain the sim worker runs on. `NoiseWorld` is a
// pure function of `(world_dim, hmax, seed)` — `cli::build_sim` constructs it as
// `NoiseWorld::new(econ.world_dim, HMAX, RESOURCE_BASE, config.seed ^ WORLD_SALT)` (`cli/src/lib.rs`).
// `HMAX`/`WORLD_SALT` are private to `cli` (not `pub`) and `cli` is a CI'd crate R-1/R-2 do not touch
// beyond the `render_firewall.rs` test — so this file mirrors those two literals directly, rather
// than exposing them from `cli`. `RESOURCE_BASE` is NOT mirrored: `NoiseWorld::height`/`biome`/
// `is_solid` (the only methods R-2 reads) do not depend on it (only `resource()` does, which the
// render never calls) — see `world/src/lib.rs`. If `cli`'s `HMAX`/`WORLD_SALT` ever change, this
// constant must change with them or the rendered terrain silently diverges from the sim's.
const WORLD_HMAX: i64 = 16;
const WORLD_SALT: u64 = 0x5743_4C44; // "WCLD" — cli/src/lib.rs::WORLD_SALT

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
    let world = world::NoiseWorld::new(world_dim, WORLD_HMAX, 0, config.seed ^ WORLD_SALT);
    let terrain_meshes = terrain::build_hex_terrain(world_dim, &world);

    // A minimal FIXED iso-ish camera — just enough to see the mesh (R-3 replaces this with a real
    // interactive IsoCam: pan/zoom/rotate + box-frustum culling). Orthographic → true isometric (no
    // perspective foreshortening, RnD `rendering/02` §1).
    let (span_x, _) = hex::hex_center(world_dim, 0);
    let (_, span_z) = hex::hex_center(0, world_dim);
    let world_span = span_x.max(span_z).max(1.0);
    let center = Vec3::new(span_x * 0.5, hex::HEIGHT_SCALE * WORLD_HMAX as f32 * 0.5, span_z * 0.5);
    let camera = Camera3D {
        position: center + vec3(1.0, 1.0, 1.0).normalize() * world_span * 1.4,
        target: center,
        up: vec3(0.0, 1.0, 0.0),
        projection: Projection::Orthographics,
        fovy: world_span * 1.5,
        ..Default::default()
    };

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

        set_camera(&camera);
        for mesh in &terrain_meshes {
            draw_mesh(mesh);
        }
        // Creatures as dots, projected into the SAME hex view (a rough mapping is fine — R-4 does
        // real creature rendering): world (x, z) → the hex center of its cell, floating just above
        // that cell's terrain height so the dot doesn't z-fight the top face.
        if let Some(s) = snap.as_ref() {
            for c in &s.creatures {
                let (cx, cz) = hex::hex_center(c.pos.0, c.pos.1);
                let h = world.height(c.pos.0, c.pos.1) as f32 * hex::HEIGHT_SCALE;
                let color = match c.cell_type {
                    Some(sim_core::CellType::A) => YELLOW,
                    Some(sim_core::CellType::B) => SKYBLUE,
                    Some(sim_core::CellType::Mixed) => GREEN,
                    None => WHITE,
                };
                draw_sphere(vec3(cx, h + 0.15, cz), 0.12, None, color);
            }
        }
        set_default_camera();

        egui_macroquad::ui(|ctx| {
            egui::Window::new("v2 render scaffold — R-2").show(ctx, |ui| {
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
                ui.label(format!("terrain: {world_dim}×{world_dim} hexes, {} mesh chunks", terrain_meshes.len()));
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
