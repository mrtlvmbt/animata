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

/// A loud, unmistakable "unmapped biome id" marker (never a plausible terrain hue) — visible at a
/// glance if `WorldView::biome` ever returns an id beyond the documented `0..=12` range, rather than
/// silently drawing a wrong-but-plausible terrain color.
const UNKNOWN: Color = Color::new(1.0, 0.0, 1.0, 1.0);

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

/// Cliff (side-quad) shade: a fixed darkening of the top-face color — a cheap "AO-ish" cue (RnD
/// `rendering/02` §3's baked-shading idea, without an actual light/AO bake) so columns read as
/// stepped 3D prisms rather than flat-shaded slabs of one hue.
pub fn cliff_shade(c: Color) -> Color {
    Color::new(c.r * 0.6, c.g * 0.6, c.b * 0.6, c.a)
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
