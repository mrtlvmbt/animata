//! `WorldView::biome(&self, pos) -> u8` → display color (issue #223 acceptance: "the palette MUST
//! cover the FULL biome range of BOTH world impls").
//!
//! The trait carries a bare `u8` with no tag for which impl produced it, so ONE table must serve
//! both: `NoiseWorld`'s `{0, 1, 2}` (lowland/upland/rock — today, `world/src/lib.rs`) and, once W-6
//! lands, `ProcgenWorld`'s `world::gen::caps::FinalBiome` (the 8 zonal `BiomeId` variants + 5 azonal
//! overrides, ids `0..=12`, `world/src/gen/caps.rs`). This table is keyed on the WIDER (`FinalBiome`)
//! semantics, so it needs no re-touch when W-6 replaces `NoiseWorld` (the stated goal) — the
//! documented tradeoff is that `NoiseWorld`'s `0/1/2` render as Tundra/BorealForest/
//! TemperateGrassland colors TODAY rather than a literal lowland/upland/rock palette. That is an
//! accepted placeholder look, not a bug: `NoiseWorld` is explicitly "crude terrain" per #223's own
//! framing, replaced wholesale (mesh included) once W-6 merges.

use macroquad::color::Color;
use macroquad::prelude::Vec3;

/// A loud, unmistakable "unmapped biome id" marker (never a plausible terrain hue) — visible at a
/// glance if `WorldView::biome` ever returns an id beyond the documented `0..=12` range, rather than
/// silently drawing a wrong-but-plausible terrain color.
const UNKNOWN: Color = Color::new(1.0, 0.0, 1.0, 1.0);

/// Fixed light direction (normalized): sidelit and from above for volumetric clarity.
/// This is a unit vector pointing toward the light source.
const LIGHT_DIR: Vec3 = Vec3::new(0.577, 0.577, 0.577); // (1,1,1) normalized ≈ 60° elevation, sidelit

/// Ambient light contribution (always present, no shadow).
const AMBIENT: f32 = 0.3;

/// Diffuse light strength (modulated by normal·light_dir).
const DIFFUSE: f32 = 0.7;

/// `id` → top-face color. `_ => UNKNOWN` is the "no panic" fallback the acceptance criterion asks for.
pub fn biome_color(id: u8) -> Color {
    match id {
        0 => Color::from_rgba(198, 201, 189, 255),  // Tundra — pale lichen (NoiseWorld: lowland)
        1 => Color::from_rgba(38, 82, 55, 255),     // BorealForest — dark conifer (NoiseWorld: upland)
        2 => Color::from_rgba(163, 186, 92, 255),   // TemperateGrassland — yellow-green (NoiseWorld: rock)
        3 => Color::from_rgba(66, 128, 66, 255),    // TemperateForest — mid green
        4 => Color::from_rgba(24, 97, 58, 255),     // TemperateRainforest — deep green
        5 => Color::from_rgba(214, 186, 130, 255),  // Desert — sand tan
        6 => Color::from_rgba(196, 175, 96, 255),   // Savanna — golden tan-green
        7 => Color::from_rgba(14, 107, 46, 255),    // TropicalRainforest — jungle green
        8 => Color::from_rgba(72, 112, 104, 255),   // Wetland — marsh teal
        9 => Color::from_rgba(115, 122, 74, 255),   // Floodplain — muddy green-brown
        10 => Color::from_rgba(126, 126, 126, 255), // Rock
        11 => Color::from_rgba(112, 84, 54, 255),   // Fertile — rich soil brown
        12 => Color::from_rgba(232, 214, 156, 255), // Dune — pale sand
        _ => UNKNOWN,
    }
}

/// `MaterialId` byte (`world::gen::material::MaterialId`, `0..=8`) → top-face color. This is the
/// PRIMARY terrain palette: the renderer colours by physical surface material, not biome, because
/// biome misses the landform substrates the diverse-relief terragen produces — Ocean water, aeolian
/// sand, glacial till, volcanic basalt/tuff (biome=13 Ocean alone already fell off `biome_color`'s
/// `0..=12` table → magenta sea). Mirrors `world/src/bin/map_dump.rs`'s palette so the interactive
/// 3D view and the headless PPM preview read identically. `_ => UNKNOWN` keeps the no-panic contract.
pub fn material_color(m: u8) -> Color {
    match m {
        0 => Color::from_rgba(180, 180, 190, 255), // Air (above-surface empty) — pale grey
        1 => Color::from_rgba(222, 200, 120, 255), // Sand (aeolian dune) — tan
        2 => Color::from_rgba(205, 232, 240, 255), // Permafrost — pale ice
        3 => Color::from_rgba(96, 132, 66, 255),   // Soil — green
        4 => Color::from_rgba(128, 128, 132, 255), // Bedrock — grey
        5 => Color::from_rgba(58, 52, 62, 255),    // Basalt (volcanic) — near-black
        6 => Color::from_rgba(172, 150, 138, 255), // Tuff (volcanic) — light brown
        7 => Color::from_rgba(184, 194, 206, 255), // Till (glacial) — grey-blue
        8 => Color::from_rgba(40, 70, 130, 255),   // Water (coastal/ocean) — blue
        _ => UNKNOWN,
    }
}

/// Terrain top-face coloring mode, runtime-toggleable ('C' key). `Material` = physical substrate
/// palette ([`material_color`]); `Height` = hypsometric elevation ramp ([`height_color`]) so relief
/// reads by height (a ceiling plateau shows as a uniform snow-white cap, troughs as green lowland,
/// moraine ridges as brown bumps — the direct visual for the glacial relief fix).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorMode {
    Material,
    Height,
}

/// Hypsometric elevation ramp: raw `height` → a classic low-green→brown→snow-white gradient. Makes
/// pure RELIEF legible independent of material — the tool for eyeballing plateau/needle/trough shape.
///
/// Normalized by the map's *observed* relief band `[h_lo, h_hi]` (a per-map percentile stretch, not
/// the fixed `[0, hmax]` datum). Real terrain is bottom-heavy (half the cells sit near sea level, the
/// relief lives in the top decile), so stretching against `hmax=200` crams every landform into the
/// ramp's low green third — one uniform hue. Stretching against `[p2, p98]` spreads the full
/// green→brown→snow band over the relief that actually exists; the sparse >p98 peaks clamp to the
/// snow-white top rather than wasting the upper 40% of the ramp on cells that never occur.
pub fn height_color(height: i64, h_lo: i64, h_hi: i64) -> Color {
    let span = (h_hi - h_lo).max(1) as f32;
    let t = ((height - h_lo) as f32 / span).clamp(0.0, 1.0);
    // (stop_t, r, g, b) — ascending; classic hypsometric tint band.
    const STOPS: [(f32, f32, f32, f32); 7] = [
        (0.00, 34.0, 74.0, 44.0),    // lowland — dark green
        (0.25, 82.0, 138.0, 60.0),   // green
        (0.45, 158.0, 168.0, 82.0),  // yellow-green
        (0.60, 178.0, 146.0, 86.0),  // tan
        (0.78, 138.0, 100.0, 64.0),  // brown (moraine/upland)
        (0.90, 156.0, 156.0, 162.0), // bare rock — grey
        (1.00, 246.0, 246.0, 250.0), // peaks — snow white
    ];
    let mut lo = &STOPS[0];
    let mut hi = &STOPS[STOPS.len() - 1];
    for w in STOPS.windows(2) {
        if t >= w[0].0 && t <= w[1].0 {
            lo = &w[0];
            hi = &w[1];
            break;
        }
    }
    let span = (hi.0 - lo.0).max(1e-6);
    let f = ((t - lo.0) / span).clamp(0.0, 1.0);
    let lerp = |a: f32, b: f32| (a + (b - a) * f) / 255.0;
    Color::new(lerp(lo.1, hi.1), lerp(lo.2, hi.2), lerp(lo.3, hi.3), 1.0)
}

/// Dispatch a cell's top-face color by the active [`ColorMode`]. `material`/`height` are read from the
/// `WorldView`; `[h_lo, h_hi]` is the map's observed relief band that scales the [`height_color`] ramp.
pub fn surface_color(mode: ColorMode, material: u8, height: i64, h_lo: i64, h_hi: i64) -> Color {
    match mode {
        ColorMode::Material => material_color(material),
        ColorMode::Height => height_color(height, h_lo, h_hi),
    }
}

/// Cliff (side-quad) shade: a fixed darkening of the top-face color — a cheap "AO-ish" cue (RnD
/// `rendering/02` §3's baked-shading idea, without an actual light/AO bake) so columns read as
/// stepped 3D prisms rather than flat-shaded slabs of one hue.
pub fn cliff_shade(c: Color) -> Color {
    Color::new(c.r * 0.6, c.g * 0.6, c.b * 0.6, c.a)
}

/// Apply directional shading to a biome color based on a face normal.
/// Shading = base_color × clamp(ambient + diffuse·max(0, dot(normal, light_dir)))
/// The normal MUST be normalized.
pub fn apply_directional_shading(c: Color, normal: Vec3) -> Color {
    let dot_nl = (normal.x * LIGHT_DIR.x + normal.y * LIGHT_DIR.y + normal.z * LIGHT_DIR.z).max(0.0);
    let shade = (AMBIENT + DIFFUSE * dot_nl).min(1.0);
    Color::new(c.r * shade, c.g * shade, c.b * shade, c.a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_range_has_no_unknown_collisions() {
        for id in 0u8..=12 {
            assert_ne!(biome_color(id), UNKNOWN, "id {id} is in the documented 0..=12 range");
        }
    }

    #[test]
    fn out_of_range_falls_back_to_unknown_without_panic() {
        assert_eq!(biome_color(13), UNKNOWN);
        assert_eq!(biome_color(255), UNKNOWN);
    }
}
