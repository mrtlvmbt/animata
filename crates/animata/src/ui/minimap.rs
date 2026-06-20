//! World minimap image builder — a downscaled top-down preview of the terrain (172×118). In the
//! normal view it samples each column's biome surface colour (the exact mapping the chunk meshes
//! use); in a debug field view it samples that scalar field with the same ramps as the legend.

use animata_sim::config::{COLS, ROWS};
use animata_sim::terrain::VoxelTerrain;
use egui::Color32;

use crate::render::mesh::top_rgb;
use crate::DebugView;

pub const MW: usize = 172;
pub const MH: usize = 118;

fn c32(r: f32, g: f32, b: f32) -> Color32 {
    Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

/// Sample one column for the minimap. Mirrors `build_field_minimap`'s ramps for the field views;
/// normal/topo views show the biome surface colour (water → ocean blue).
fn sample(t: &VoxelTerrain, view: DebugView, x: usize, y: usize, tick: u64) -> Color32 {
    match view {
        DebugView::Temp => {
            let v = t.temperature_at(x, y);
            c32(v, 0.15, 1.0 - v)
        }
        DebugView::Moist => {
            let v = t.moisture_at(x, y);
            c32(0.65 * (1.0 - v) + 0.1, 0.35 + 0.45 * v, 0.25 + 0.5 * v)
        }
        DebugView::WaterDist => {
            let f = t.water_dist_at(x, y) as f32 / 255.0;
            if f == 0.0 {
                c32(0.2, 0.5, 1.0)
            } else {
                let b = 1.0 - 0.85 * f;
                c32(b, b, b)
            }
        }
        DebugView::Slope => {
            let v = t.slope_at(x, y);
            c32(v, v, 0.25 * v)
        }
        DebugView::Biomass => {
            if t.is_water(x, y) {
                c32(0.18, 0.32, 0.5)
            } else {
                let v = t.biomass_at(x, y, tick);
                c32(0.45 * (1.0 - v) + 0.1, 0.25 + 0.6 * v, 0.12)
            }
        }
        // None / Topo: biome surface colour.
        _ => {
            if t.is_water(x, y) {
                c32(0.13, 0.32, 0.55) // ocean
            } else {
                let (r, g, b) = top_rgb(t.biome_at(x, y));
                c32(r, g, b)
            }
        }
    }
}

/// Build the minimap as an egui image, sampling the whole map down to `MW×MH`.
pub fn build_image(t: &VoxelTerrain, view: DebugView, tick: u64) -> egui::ColorImage {
    let mut pixels = vec![Color32::BLACK; MW * MH];
    for py in 0..MH {
        let y = (py * ROWS / MH).min(ROWS - 1);
        for px in 0..MW {
            let x = (px * COLS / MW).min(COLS - 1);
            pixels[py * MW + px] = sample(t, view, x, y, tick);
        }
    }
    egui::ColorImage {
        size: [MW, MH],
        pixels,
    }
}
