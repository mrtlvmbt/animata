//! R-16 pasteled material color palette (single source of truth).
//! Indexed by MaterialId 0..=10 (Air..SoilWet).
//! Shared between render crate (3D visualization) and map_dump (headless PPM preview).

/// Material ID → RGB triple. Single canonical palette source.
/// Values: R-16 pasteled (brightened, softer hues toward toy-diorama look).
pub const MATERIAL_COLORS: [[u8; 3]; 11] = [
    [200, 200, 210], // 0: Air (above-surface empty) — pale grey (brightened)
    [235, 220, 150], // 1: Sand (aeolian dune) — warm tan (lighter, softer)
    [220, 240, 248], // 2: Permafrost — ice grey (lightened)
    [140, 160, 110], // 3: Soil — softer green
    [150, 150, 156], // 4: Bedrock — cool grey (lightened)
    [110, 100, 115], // 5: Basalt (volcanic) — dark slate (lifted from near-black)
    [200, 175, 155], // 6: Tuff (volcanic) — light brown (brightened)
    [205, 215, 230], // 7: Till (glacial) — pale grey-blue (lightened)
    [120, 160, 200], // 8: Water (coastal/ocean) — lighter softer blue
    [210, 195, 135], // 9: SoilDry (W-10 presentation) — pale ochre (lighter)
    [150, 135, 90],  // 10: SoilWet (W-10 presentation) — softer mid-brown
];

/// Material ID → Russian name. Indexed same as MATERIAL_COLORS.
pub const MATERIAL_NAMES: [&str; 11] = [
    "воздух",           // 0: Air
    "песок",            // 1: Sand
    "вечная мерзлота",  // 2: Permafrost
    "почва",            // 3: Soil
    "коренная порода",  // 4: Bedrock
    "базальт",          // 5: Basalt
    "туф",              // 6: Tuff
    "морена",           // 7: Till
    "вода",             // 8: Water
    "сухая почва",      // 9: SoilDry
    "влажная почва",    // 10: SoilWet
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_names_colors_parity() {
        assert_eq!(MATERIAL_NAMES.len(), MATERIAL_COLORS.len(), "MATERIAL_NAMES and MATERIAL_COLORS must have the same length");
    }
}
