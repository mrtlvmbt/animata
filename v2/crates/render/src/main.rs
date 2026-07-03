//! animata v2 renderer — R-ladder scaffold (R-1, issue #219): **proof-of-life ONLY**. A macroquad
//! window + egui HUD showing the LIVE `tick`/`population` from a worker-thread `Sim`, and each live
//! creature as a naive screen-space dot (a trivial linear world→screen map — NOT the real
//! iso-camera). This proves the render seam (worker-thread driver → read-only `RenderSnapshot` →
//! render loop) end to end. The real hex-voxel terrain mesh (R-2), iso-camera + culling (R-3),
//! creature LOD/morphology (R-4), cube-voxel toggle (R-5), and full HUD/inspector (R-6) are OUT of
//! scope here — see issue #219's R-ladder.
//!
//! Not part of the v2 CI workspace (`v2/Cargo.toml`'s `exclude`) — a leaf bin, verified LOCALLY:
//! `cargo build`/`cargo clippy` from this directory + a manual run (window opens, HUD counts advance).

mod driver;

use macroquad::prelude::*;

fn window_conf() -> Conf {
    Conf {
        window_title: "animata v2 — render scaffold (R-1 proof-of-life)".to_owned(),
        window_width: 1024,
        window_height: 768,
        high_dpi: true,
        ..Default::default()
    }
}

/// The v2 demo/test seed used across the cli/telemetry suites — an arbitrary but consistent choice,
/// not load-bearing (R-1 draws whatever the economy produces).
const SEED: u64 = 0xA11A_2A11;

#[macroquad::main(window_conf)]
async fn main() {
    let world_dim = cli::default_config(SEED).econ.world_dim as f32;
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

        // Naive world→screen: linear map of the world_dim×world_dim grid onto a square viewport
        // inset from the window edges. Proof that per-entity data flows end to end — NOT the real
        // iso-camera (R-3 replaces this projection entirely; hex terrain is R-2).
        let margin = 40.0;
        let view = screen_width().min(screen_height()) - 2.0 * margin;
        let scale = view / world_dim.max(1.0);

        if let Some(s) = snap.as_ref() {
            for c in &s.creatures {
                let x = margin + c.pos.0 as f32 * scale;
                let y = margin + c.pos.1 as f32 * scale;
                let color = match c.cell_type {
                    Some(sim_core::CellType::A) => YELLOW,
                    Some(sim_core::CellType::B) => SKYBLUE,
                    Some(sim_core::CellType::Mixed) => GREEN,
                    None => WHITE,
                };
                draw_circle(x, y, 2.0, color);
            }
        }

        egui_macroquad::ui(|ctx| {
            egui::Window::new("v2 render scaffold — R-1").show(ctx, |ui| {
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
