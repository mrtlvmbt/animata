//! Headless top-down map preview → binary PPM (P6). GOLDEN-NEUTRAL: it only READS the
//! world via `gen::caps::classify_and_caps` and never touches the sim path, so it cannot
//! move any determinism golden. No macroquad, no window, no GPU — pure CPU generation +
//! a byte dump. Each surface cell is coloured by its primary `MaterialId`.
//!
//! Usage:  map_dump <dim> [seed] [out.ppm]
//!   dim      map edge in cells (required), e.g. 256 or 512
//!   seed     u64, decimal or 0x-hex (default 1)
//!   out.ppm  output path (default `map_<dim>_<seed>.ppm`)
//!
//! All four landform stages (tectonics / aeolian / volcanic / glacial) are turned ON so the
//! preview shows the full diverse-relief material palette; patchiness is OFF.

use std::io::Write;
use world::gen::caps::classify_and_caps;
use world::gen::material::MaterialId;

/// Matches the production world height ceiling (`cli::HMAX`), so erosion / glacial ELA / all
/// height-relative thresholds fire exactly as the real generator sees them.
const HMAX: i64 = 200;

/// Primary-material → RGB palette (top-down surface colour).
fn colour(m: u8) -> [u8; 3] {
    match m {
        x if x == MaterialId::Air as u8 => [40, 70, 130], // water / air — blue
        x if x == MaterialId::Sand as u8 => [222, 200, 120], // aeolian sand — tan
        x if x == MaterialId::Permafrost as u8 => [205, 232, 240], // permafrost — pale ice
        x if x == MaterialId::Soil as u8 => [96, 132, 66], // soil — green
        x if x == MaterialId::Bedrock as u8 => [128, 128, 132], // bedrock — grey
        x if x == MaterialId::Basalt as u8 => [58, 52, 62], // volcanic basalt — near-black
        x if x == MaterialId::Tuff as u8 => [172, 150, 138], // volcanic tuff — light brown
        x if x == MaterialId::Till as u8 => [184, 194, 206], // glacial till — grey-blue
        _ => [255, 0, 255],                                  // unknown — magenta
    }
}

fn parse_seed(s: &str) -> u64 {
    s.strip_prefix("0x").map_or_else(|| s.parse().unwrap_or(1), |h| u64::from_str_radix(h, 16).unwrap_or(1))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dim: usize = match args.get(1).and_then(|s| s.parse().ok()) {
        Some(d) if d > 0 => d,
        _ => {
            eprintln!("usage: map_dump <dim> [seed] [out.ppm]   (dim = map edge in cells, e.g. 256)");
            std::process::exit(2);
        }
    };
    let seed: u64 = args.get(2).map_or(1, |s| parse_seed(s));
    let out = args.get(3).cloned().unwrap_or_else(|| format!("map_{dim}_{seed:#x}.ppm"));

    // patchiness=false, then all four landforms ON.
    let fields = classify_and_caps(seed, HMAX, dim, false, true, true, true, true);
    assert_eq!(fields.surface_material.len(), dim * dim, "surface_material must be dim*dim");

    // P6 binary PPM: header then RGB triples, row-major (idx = z*dim + x).
    let mut buf = Vec::with_capacity(dim * dim * 3 + 32);
    buf.extend_from_slice(format!("P6\n{dim} {dim}\n255\n").as_bytes());
    for &m in &fields.surface_material {
        buf.extend_from_slice(&colour(m));
    }
    std::fs::File::create(&out).and_then(|mut f| f.write_all(&buf)).expect("write ppm");

    // Material histogram to stderr — a quick sanity read without opening the image.
    let mut hist = [0u32; 8];
    for &m in &fields.surface_material {
        if (m as usize) < 8 {
            hist[m as usize] += 1;
        }
    }
    let names = ["Air", "Sand", "Permafrost", "Soil", "Bedrock", "Basalt", "Tuff", "Till"];
    eprintln!("wrote {out}  ({dim}x{dim}, seed={seed:#x}, all landforms ON)");
    for (i, n) in names.iter().enumerate() {
        if hist[i] > 0 {
            eprintln!("  {n:<10} {:>8}  ({:.1}%)", hist[i], 100.0 * hist[i] as f64 / (dim * dim) as f64);
        }
    }
}
