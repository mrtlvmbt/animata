    use super::*;

    // Std-only image dump (binary PPM / P6) for the `#[ignore]` diagnostics — keeps the sim crate
    // macroquad-free. View with any image tool (Preview, GIMP, `pnmtopng`).
    #[allow(dead_code)]
    fn dump_ppm(path: &str, w: usize, h: usize, px: impl Fn(usize, usize) -> (u8, u8, u8)) {
        use std::io::Write;
        let mut buf = Vec::with_capacity(w * h * 3 + 32);
        write!(buf, "P6\n{w} {h}\n255\n").unwrap();
        for y in 0..h {
            for x in 0..w {
                let (r, g, b) = px(x, y);
                buf.extend_from_slice(&[r, g, b]);
            }
        }
        let _ = std::fs::write(path, buf);
    }
    /// Quantise a `[0,1]` channel to `u8` for the PPM dumps.
    #[allow(dead_code)]
    fn u8c(v: f32) -> u8 {
        (v.clamp(0.0, 1.0) * 255.0).round() as u8
    }

    /// Guard the "mountains are LOCAL" invariant: rock + snow must stay a minority of
    /// the land, so added worldgen complexity (ridged noise now; tectonics/erosion
    /// later) can't quietly turn the map into one mountain mess. Prints the fraction.
    #[test]
    fn mountains_are_a_minority() {
        for seed in 1..4 {
            let t = VoxelTerrain::new(seed);
            let (mut land, mut high) = (0u64, 0u64);
            for y in 0..ROWS {
                for x in 0..COLS {
                    if t.is_water(x, y) {
                        continue;
                    }
                    land += 1;
                    if matches!(t.biome_at(x, y), BiomeKind::Mountain | BiomeKind::Snow) {
                        high += 1;
                    }
                }
            }
            let frac = high as f64 / land.max(1) as f64;
            eprintln!("seed {seed}: mountain+snow = {:.1}% of land", frac * 100.0);
            assert!(frac < 0.35, "mountains dominate the land for seed {seed}: {:.1}%", frac * 100.0);
        }
    }

    /// Tectonic sanity: mountains should form a few large connected BELTS (chains), not
    /// scattered specks, and the land/water balance must stay reasonable per seed (the
    /// oceanic-plate layout shouldn't drown or fill the whole map). Prints both.
    #[test]
    fn tectonic_chains_and_balance() {
        for seed in 1..4 {
            let t = VoxelTerrain::new(seed);
            let n = COLS * ROWS;
            let mut high = vec![false; n];
            let (mut water, mut mtn) = (0u64, 0u64);
            for y in 0..ROWS {
                for x in 0..COLS {
                    let i = y * COLS + x;
                    if t.is_water(x, y) {
                        water += 1;
                    }
                    if matches!(t.biome_at(x, y), BiomeKind::Mountain | BiomeKind::Snow) {
                        high[i] = true;
                        mtn += 1;
                    }
                }
            }
            // Largest connected mountain component (4-connectivity, iterative flood fill).
            let mut seen = vec![false; n];
            let mut largest = 0u64;
            let mut stack = Vec::new();
            for start in 0..n {
                if !high[start] || seen[start] {
                    continue;
                }
                let mut size = 0u64;
                stack.push(start);
                seen[start] = true;
                while let Some(i) = stack.pop() {
                    size += 1;
                    let (x, y) = ((i % COLS) as i32, (i / COLS) as i32);
                    for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                        if nx < 0 || ny < 0 || nx >= COLS as i32 || ny >= ROWS as i32 {
                            continue;
                        }
                        let j = ny as usize * COLS + nx as usize;
                        if high[j] && !seen[j] {
                            seen[j] = true;
                            stack.push(j);
                        }
                    }
                }
                largest = largest.max(size);
            }
            let water_pct = water as f64 / n as f64 * 100.0;
            let chain = if mtn > 0 { largest as f64 / mtn as f64 } else { 0.0 };
            eprintln!(
                "seed {seed}: water {water_pct:.0}%, mountains in chains {:.0}% (largest/total)",
                chain * 100.0
            );
            assert!((8.0..92.0).contains(&water_pct), "extreme water balance for seed {seed}: {water_pct:.0}%");
        }
    }

    /// Debug dump (run with `--ignored`): writes grayscale PNGs of the generation fields
    /// to /tmp so the straight-cliff artifact can be located visually and traced to the
    /// field that produces it. Not a gate.
    #[test]
    #[ignore]
    fn dump_debug_fields() {
        let seed = 1u64;
        let t = VoxelTerrain::new(seed);
        let tect = TectonicField::generate(seed);
        let dump = |path: &str, f: &dyn Fn(usize, usize) -> f32| {
            dump_ppm(path, COLS, ROWS, |x, y| {
                let v = u8c(f(x, y));
                (v, v, v)
            });
        };
        dump("/tmp/dbg_macro.ppm", &|x, y| tect.macro_field()[y * COLS + x]);
        dump("/tmp/dbg_mtn.ppm", &|x, y| tect.mountain_field()[y * COLS + x]);
        dump("/tmp/dbg_height.ppm", &|x, y| t.height_at(x, y) as f32 / MAX_H as f32);
        // Biome map: a distinct flat colour per biome id, so the climate distribution is
        // visible (poles cold, equator hot; dry↔wet bands).
        let pal: [(f32, f32, f32); 12] = [
            (0.13, 0.32, 0.55), (0.84, 0.78, 0.54), (0.42, 0.62, 0.30), (0.20, 0.46, 0.24),
            (0.80, 0.70, 0.44), (0.48, 0.46, 0.45), (0.93, 0.95, 0.98), (0.17, 0.38, 0.29),
            (0.62, 0.64, 0.56), (0.70, 0.66, 0.34), (0.31, 0.40, 0.25), (0.12, 0.43, 0.17),
        ];
        dump_ppm("/tmp/dbg_biome.ppm", COLS, ROWS, |x, y| {
            let (r, g, b) = pal[t.biome_at(x, y).id() as usize];
            (u8c(r), u8c(g), u8c(b))
        });
        // Hillshade of the actual terrain — reveals erosion channels/ridges far better
        // than raw height (slope-lit, sun from the NW).
        dump("/tmp/dbg_shade.ppm", &|x, y| {
            let xi = x as i32;
            let yi = y as i32;
            let gx = (t.height(xi + 1, yi) as f32 - t.height(xi - 1, yi) as f32) * 0.5;
            let gy = (t.height(xi, yi + 1) as f32 - t.height(xi, yi - 1) as f32) * 0.5;
            let inv = 1.0 / (gx * gx + gy * gy + 1.0).sqrt();
            // light dir (0.5, 0.5, 0.7) normalised ≈ (0.49,0.49,0.69), dot with normal
            let shade = (-gx * 0.49 - gy * 0.49 + 0.69) * inv;
            0.15 + 0.85 * shade.clamp(0.0, 1.0)
        });
        // Cliff map: the largest DOWNWARD step from a column to any 4-neighbour, in
        // levels, scaled so a ~10-level drop is white. This isolates where the knife
        // cliffs actually are, independent of biome colour.
        dump("/tmp/dbg_cliff.ppm", &|x, y| {
            let h = t.height(x as i32, y as i32) as i32;
            let mut drop = 0i32;
            for (nx, ny) in [(x as i32 + 1, y as i32), (x as i32 - 1, y as i32), (x as i32, y as i32 + 1), (x as i32, y as i32 - 1)] {
                drop = drop.max(h - t.height(nx, ny) as i32);
            }
            drop as f32 / 10.0
        });
        eprintln!("dumped /tmp/dbg_macro.ppm dbg_mtn.ppm dbg_height.ppm dbg_cliff.ppm");
    }

    /// Guard against KNIFE CLIFFS — the artifact where the macro field stepped a full
    /// relief in one column (root cause: taking the single NEAREST plate boundary's
    /// convergence, which flips across the medial axis between two boundaries; fixed by
    /// using a distance-weighted average convergence instead). The worst LAND-to-LAND
    /// downward step must stay a slope, not a wall. Prints the worst per seed.
    #[test]
    fn land_has_no_knife_cliffs() {
        for seed in 1..4 {
            let t = VoxelTerrain::new(seed);
            let mut worst = 0i32;
            for y in 0..ROWS as i32 {
                for x in 0..COLS as i32 {
                    let h = t.height(x, y) as i32;
                    for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                        // In-world land neighbours only (the map-edge slab to air and the
                        // shoreline drop to the sea floor are legitimate, not artifacts).
                        if nx < 0 || ny < 0 || nx >= COLS as i32 || ny >= ROWS as i32 {
                            continue;
                        }
                        let nh = t.height(nx, ny) as i32;
                        if nh == 0 || t.is_water(nx as usize, ny as usize) {
                            continue;
                        }
                        worst = worst.max(h - nh);
                    }
                }
            }
            eprintln!("seed {seed}: worst land cliff = {worst} levels (of {SURFACE_RANGE})");
            assert!(worst < 16, "knife cliff for seed {seed}: {worst}-level step in one column");
        }
    }

    /// Report the erosion preprocess cost (the heavy one-time pass). Run with `--release`
    /// for a representative number; informational, not a gate.
    #[test]
    #[ignore]
    fn report_erosion_cost() {
        let tect = TectonicField::generate(1);
        let mut elev = vec![0.0f32; COLS * ROWS];
        for y in 0..ROWS {
            for x in 0..COLS {
                elev[y * COLS + x] =
                    elevation(1, x as f32, y as f32, tect.macro_at(x, y), tect.mountain_at(x, y));
            }
        }
        let t0 = std::time::Instant::now();
        crate::erosion::erode(1, &mut elev, &|_| {});
        eprintln!(
            "erosion: {} cols, {:.0} ms (MAP_SCALE={MAP_SCALE})",
            COLS * ROWS,
            t0.elapsed().as_secs_f64() * 1000.0
        );
    }

    /// Climate must give the giant map real biome DIVERSITY: several lowland biomes
    /// present (temperature × moisture bands), none absurdly dominant. Prints the mix.
    #[test]
    fn biome_diversity() {
        let t = VoxelTerrain::new(1);
        let mut counts = [0u64; 12];
        let mut land = 0u64;
        for y in 0..ROWS {
            for x in 0..COLS {
                if t.is_water(x, y) {
                    continue;
                }
                land += 1;
                counts[t.biome_at(x, y).id() as usize] += 1;
            }
        }
        for id in 1..12u8 {
            let pct = counts[id as usize] as f64 / land as f64 * 100.0;
            if pct > 0.1 {
                eprintln!("  {:?}: {:.1}%", BiomeKind::from_id(id), pct);
            }
        }
        let present = counts.iter().filter(|&&c| c as f64 / land as f64 > 0.01).count();
        let maxf = *counts.iter().max().unwrap() as f64 / land as f64;
        eprintln!("distinct biomes (>1%): {present}, largest share {:.0}%", maxf * 100.0);
        assert!(present >= 6, "too few biomes present: {present}");
        assert!(maxf < 0.6, "one biome dominates the land: {:.0}%", maxf * 100.0);
    }

    /// Guard that hydrology actually produces both rivers and lakes (a regression in the
    /// flood routing once silently gave 0 rivers). Rebuilds the eroded field + hydrology.
    #[test]
    fn hydrology_makes_rivers_and_lakes() {
        let seed = 2u64;
        let tect = TectonicField::generate(seed);
        let mut elev = vec![0.0f32; COLS * ROWS];
        for y in 0..ROWS {
            for x in 0..COLS {
                elev[y * COLS + x] =
                    elevation(seed, x as f32, y as f32, tect.macro_at(x, y), tect.mountain_at(x, y));
            }
        }
        crate::erosion::erode(seed, &mut elev, &|_| {});
        let hydro = crate::hydrology::compute(&elev, SEA_FRACTION);
        let rivers = hydro.river.iter().filter(|&&r| r).count();
        let lakes = hydro.lake.iter().filter(|&&l| l).count();
        eprintln!("seed {seed}: {rivers} river cells, {lakes} lake cells");
        assert!(rivers > 200, "no river network: {rivers} cells");
        assert!(lakes > 50, "no lakes: {lakes} cells");
    }

    /// Guard the water model: water is never rendered below its own terrain (`misset`),
    /// and there are no swarms of 1-cell water specks (the lake-size filter). Both were
    /// artifacts reported on the 3D view; this locks the data side of the fixes.
    #[test]
    fn water_model_is_clean() {
        let t = VoxelTerrain::new(1);
        let nb = |x: i32, y: i32| [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)];
        let (mut misset, mut isolated, mut water_cols) = (0u64, 0u64, 0u64);
        for y in 0..ROWS as i32 {
            for x in 0..COLS as i32 {
                let (h, wl) = (t.height(x, y), t.water_level(x, y));
                if wl == 0 {
                    continue;
                }
                water_cols += 1;
                if wl <= h {
                    misset += 1;
                }
                if nb(x, y).iter().all(|&(a, b)| t.water_level(a, b) == 0) {
                    isolated += 1;
                }
            }
        }
        // Relative to the water area (scale-independent: ×16 has 4× the columns).
        let frac = isolated as f64 / water_cols.max(1) as f64;
        eprintln!("misset_water={misset}, isolated_water={isolated} ({:.3}% of water)", frac * 100.0);
        assert_eq!(misset, 0, "water rendered below terrain in {misset} columns");
        assert!(frac < 0.005, "too many 1-cell water specks: {:.3}% of water", frac * 100.0);
    }

    /// Diagnose the reported water/tree artifacts numerically on the FINAL world model:
    /// mis-set water (rendered where it shouldn't), terrain poking into water (dry holes →
    /// internal walls), isolated 1-cell water (specks), and land trees overhanging water.
    #[test]
    #[ignore]
    fn diagnose_water_artifacts() {
        let t = VoxelTerrain::new(1);
        let nb = |x: i32, y: i32| [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)];
        let (mut misset, mut dry_holes, mut isolated, mut trees_over_water) = (0u64, 0u64, 0u64, 0u64);
        let mut mountain_with_soil = 0u64;
        for y in 0..ROWS as i32 {
            for x in 0..COLS as i32 {
                let h = t.height(x, y);
                let wl = t.water_level(x, y);
                let watn = nb(x, y).iter().filter(|&&(a, b)| t.water_level(a, b) > 0).count();
                if wl > 0 && wl <= h {
                    misset += 1;
                }
                if wl > 0 && watn == 0 {
                    isolated += 1;
                }
                // Dry land cell mostly ringed by water (pokes up inside a water body).
                if wl == 0 && h > 0 && watn >= 3 {
                    dry_holes += 1;
                }
                // A tree-growing land column next to water → canopy overhangs the water.
                if wl == 0 && h > 0 {
                    let biome = t.biome_at(x as usize, y as usize);
                    if matches!(biome, BiomeKind::Mountain | BiomeKind::Snow) && h >= GROUND_MIN + 3 {
                        mountain_with_soil += 1; // would show a brown topsoil strata band
                    }
                }
            }
        }
        // Trees overhanging water (approximate: tree columns with a water neighbour).
        for y in 0..ROWS {
            for x in 0..COLS {
                if t.water_level(x as i32, y as i32) > 0 {
                    continue;
                }
                let b = t.biome_at(x, y);
                let grows = matches!(b, BiomeKind::Forest | BiomeKind::Jungle | BiomeKind::Taiga | BiomeKind::Plains | BiomeKind::Savanna | BiomeKind::Swamp);
                let near_water = nb(x as i32, y as i32).iter().any(|&(a, c)| t.water_level(a, c) > 0);
                if grows && near_water {
                    trees_over_water += 1;
                }
            }
        }
        eprintln!("misset_water={misset} dry_holes={dry_holes} isolated_water={isolated} trees_near_water={trees_over_water} mountain_soil_bands={mountain_with_soil}");
    }

    /// Report river/lake coverage and dump a water map (ocean / lake / river distinct).
    /// Rebuilds the eroded field + hydrology directly. Run with `--release`.
    #[test]
    #[ignore]
    fn dump_water() {
        let seed = 1u64;
        let tect = TectonicField::generate(seed);
        let n = COLS * ROWS;
        let mut elev = vec![0.0f32; n];
        for y in 0..ROWS {
            for x in 0..COLS {
                elev[y * COLS + x] =
                    elevation(seed, x as f32, y as f32, tect.macro_at(x, y), tect.mountain_at(x, y));
            }
        }
        crate::erosion::erode(seed, &mut elev, &|_| {});
        let hydro = crate::hydrology::compute(&elev, SEA_FRACTION);
        let (mut land, mut river, mut lake) = (0u64, 0u64, 0u64);
        let mut cols: Vec<(u8, u8, u8)> = vec![(0, 0, 0); n];
        for i in 0..n {
            let sea = elev[i] < SEA_FRACTION;
            cols[i] = if sea {
                (u8c(0.10), u8c(0.22), u8c(0.42)) // ocean
            } else if hydro.lake[i] {
                lake += 1;
                (u8c(0.30), u8c(0.65), u8c(0.85)) // lake
            } else if hydro.river[i] {
                river += 1;
                land += 1;
                (u8c(0.55), u8c(0.80), u8c(1.0)) // river
            } else {
                land += 1;
                let v = 0.25 + 0.5 * ((elev[i] - SEA_FRACTION) / (1.0 - SEA_FRACTION)).clamp(0.0, 1.0);
                (u8c(v), u8c(v * 0.95), u8c(v * 0.8))
            };
        }
        dump_ppm("/tmp/dbg_water.ppm", COLS, ROWS, |x, y| cols[y * COLS + x]);
        eprintln!(
            "rivers {:.2}% of land, lakes {} cells; dumped /tmp/dbg_water.ppm",
            river as f64 / land.max(1) as f64 * 100.0,
            lake
        );
    }

    /// Find the tallest water-to-lower-water step (= the height of a `push_water_side`
    /// wall). A big value explains the "vertical walls in the water" — a water cell whose
    /// neighbour's water surface is many levels lower.
    #[test]
    #[ignore]
    fn diagnose_water_walls() {
        let t = VoxelTerrain::new(1);
        let nb = |x: i32, y: i32| [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)];
        let (mut worst, mut wx, mut wy, mut wwl, mut wnwl) = (0u8, 0i32, 0i32, 0u8, 0u8);
        let mut count_tall = 0u64;
        for y in 0..ROWS as i32 {
            for x in 0..COLS as i32 {
                let wl = t.water_level(x, y);
                if wl == 0 {
                    continue;
                }
                for (nx, ny) in nb(x, y) {
                    let nwl = t.water_level(nx, ny);
                    if nwl > 0 && nwl < wl {
                        let d = wl - nwl;
                        if d >= 3 {
                            count_tall += 1;
                        }
                        if d > worst {
                            worst = d;
                            (wx, wy, wwl, wnwl) = (x, y, wl, nwl);
                        }
                    }
                }
            }
        }
        eprintln!("tallest water wall = {worst} levels at ({wx},{wy}) wl={wwl} nwl={wnwl}; cells with >=3-tall walls: {count_tall}");
    }

    /// Dump every connected WATER body coloured by its surface height (voxel level): each
    /// 4-connected component of `water_level > 0` is flood-filled and painted with a height
    /// ramp (blue=low → red=high). A flat body reads as ONE solid colour; any gradient inside
    /// a single blob is a stepped body. Land is dark grey. Writes /tmp/dbg_water_height.png.
    #[test]
    #[ignore]
    fn dump_water_height() {
        let t = VoxelTerrain::new(1);
        let n = COLS * ROWS;
        // Range of water surface levels present (for normalising the colour ramp).
        let (mut lo, mut hi) = (u8::MAX, 0u8);
        for i in 0..n {
            let wl = t.geo.water[i];
            if wl > 0 {
                lo = lo.min(wl);
                hi = hi.max(wl);
            }
        }
        let span = (hi - lo).max(1) as f32;
        // Blue(low) → cyan → green → yellow → red(high) ramp.
        let ramp = |u: f32| -> (u8, u8, u8) {
            let stops = [
                (0.00, (0.10, 0.20, 0.70)),
                (0.25, (0.10, 0.75, 0.85)),
                (0.50, (0.20, 0.80, 0.30)),
                (0.75, (0.95, 0.85, 0.15)),
                (1.00, (0.85, 0.15, 0.10)),
            ];
            for w in stops.windows(2) {
                let (a, ca) = w[0];
                let (b, cb) = w[1];
                if u <= b {
                    let f = ((u - a) / (b - a)).clamp(0.0, 1.0);
                    return (
                        u8c(ca.0 + (cb.0 - ca.0) * f),
                        u8c(ca.1 + (cb.1 - ca.1) * f),
                        u8c(ca.2 + (cb.2 - ca.2) * f),
                    );
                }
            }
            (u8c(0.85), u8c(0.15), u8c(0.10))
        };
        dump_ppm("/tmp/dbg_water_height.ppm", COLS, ROWS, |x, y| {
            let wl = t.geo.water[y * COLS + x];
            if wl == 0 {
                (u8c(0.12), u8c(0.12), u8c(0.13)) // dry land
            } else {
                ramp((wl - lo) as f32 / span)
            }
        });
        let (mut bodies, mut water_cells) = (0u64, 0u64);
        let mut seen = vec![false; n];
        let mut stack: Vec<usize> = Vec::new();
        // Count connected water bodies (4-connected over water_level > 0).
        for start in 0..n {
            if t.geo.water[start] == 0 || seen[start] {
                continue;
            }
            bodies += 1;
            stack.push(start);
            seen[start] = true;
            while let Some(i) = stack.pop() {
                water_cells += 1;
                let (x, y) = ((i % COLS) as i32, (i / COLS) as i32);
                for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                    if nx < 0 || ny < 0 || nx >= COLS as i32 || ny >= ROWS as i32 {
                        continue;
                    }
                    let j = (ny * COLS as i32 + nx) as usize;
                    if t.geo.water[j] > 0 && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        eprintln!(
            "water levels {lo}..{hi}; connected bodies={bodies}, water cells={water_cells}; dumped /tmp/dbg_water_height.ppm"
        );
    }

    /// LOCK: no "lake inside a lake". The bug pinned inland sub-sea pits to the global sea
    /// level (`SEA_ABS`) by absolute elevation, so a deep pit rendered as an ocean-level pool
    /// embedded in a higher lake. Invariant after the fix: an ocean-level water column
    /// (`wl == SEA_ABS`) only exists in a body CONNECTED TO THE MAP BORDER — i.e. the real sea.
    /// Any landlocked `SEA_ABS` cell is the bug. Must be EXACTLY 0 (it's impossible by
    /// construction once ocean is defined by border-connectivity, not by `e < SEA_FRACTION`).
    /// RED until the classify/ocean fix lands.
    #[test]
    fn no_landlocked_ocean() {
        let t = VoxelTerrain::new(1);
        let n = COLS * ROWS;
        let mut seen = vec![false; n];
        let mut stack: Vec<usize> = Vec::new();
        let mut landlocked_ocean_cells = 0u64;
        for start in 0..n {
            if t.geo.water[start] == 0 || seen[start] {
                continue;
            }
            stack.push(start);
            seen[start] = true;
            // Count SEA_ABS cells in this body; flag if the body ever touches the map edge.
            let (mut ocean_cells, mut touches_border) = (0u64, false);
            while let Some(i) = stack.pop() {
                if t.geo.water[i] == SEA_ABS {
                    ocean_cells += 1;
                }
                let (x, y) = ((i % COLS) as i32, (i / COLS) as i32);
                if x == 0 || y == 0 || x == COLS as i32 - 1 || y == ROWS as i32 - 1 {
                    touches_border = true;
                }
                for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                    if nx < 0 || ny < 0 || nx >= COLS as i32 || ny >= ROWS as i32 {
                        continue;
                    }
                    let j = (ny * COLS as i32 + nx) as usize;
                    if t.geo.water[j] > 0 && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
            // Ocean-level water in a body that never reaches the map edge = inland pit pinned
            // to SEA_ABS (the bug). The real sea always touches the border.
            if !touches_border {
                landlocked_ocean_cells += ocean_cells;
            }
        }
        eprintln!("landlocked_ocean_cells={landlocked_ocean_cells}");
        assert_eq!(
            landlocked_ocean_cells, 0,
            "{landlocked_ocean_cells} water cells sit at SEA_ABS in a landlocked body (lake-in-lake bug)"
        );
    }

    /// "Lake inside a lake": flood-fill connected WATER bodies (final model, `water > 0`) and
    /// report bodies that span >1 surface level, separating OCEAN-classified cells (`wl ==
    /// SEA_ABS`) from lake/river. Also counts LANDLOCKED ocean (a `wl == SEA_ABS` body that
    /// never touches the map border) — an inland below-sea-level pit pinned to the global sea.
    #[test]
    #[ignore]
    fn diagnose_lake_in_lake() {
        let t = VoxelTerrain::new(1);
        let n = COLS * ROWS;
        let mut seen = vec![false; n];
        let mut stack: Vec<usize> = Vec::new();
        let mut comp: Vec<usize> = Vec::new();
        let (mut bodies, mut mixed, mut worst_span) = (0u64, 0u64, 0i32);
        let (mut landlocked_ocean_bodies, mut landlocked_ocean_cells) = (0u64, 0u64);
        let mut example = (0usize, 0usize, 0i32, 0i32);
        for start in 0..n {
            if t.geo.water[start] == 0 || seen[start] {
                continue;
            }
            comp.clear();
            stack.push(start);
            seen[start] = true;
            let (mut lo, mut hi) = (i32::MAX, i32::MIN);
            let (mut has_ocean, mut has_other, mut touches_border) = (false, false, false);
            while let Some(i) = stack.pop() {
                comp.push(i);
                let wl = t.geo.water[i] as i32;
                lo = lo.min(wl);
                hi = hi.max(wl);
                if t.geo.water[i] == SEA_ABS {
                    has_ocean = true;
                } else {
                    has_other = true;
                }
                let (x, y) = ((i % COLS) as i32, (i / COLS) as i32);
                if x == 0 || y == 0 || x == COLS as i32 - 1 || y == ROWS as i32 - 1 {
                    touches_border = true;
                }
                for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                    if nx < 0 || ny < 0 || nx >= COLS as i32 || ny >= ROWS as i32 {
                        continue;
                    }
                    let j = (ny * COLS as i32 + nx) as usize;
                    if t.geo.water[j] > 0 && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
            bodies += 1;
            let span = hi - lo;
            if span > 0 {
                mixed += 1;
                if span > worst_span {
                    worst_span = span;
                    example = (start % COLS, start / COLS, lo, hi);
                }
            }
            // A wholly-ocean body that never reaches the border = inland "sea" at SEA_ABS.
            if has_ocean && !has_other && !touches_border {
                landlocked_ocean_bodies += 1;
                landlocked_ocean_cells += comp.len() as u64;
            }
        }
        eprintln!(
            "water bodies={bodies}, MIXED-level bodies={mixed}, worst span={worst_span} levels (lo={} hi={} near col={} row={}); landlocked-ocean bodies={landlocked_ocean_bodies} cells={landlocked_ocean_cells}",
            example.2, example.3, example.0, example.1
        );
    }

    /// Lake flatness: flood-fill each connected LAKE body (`hydro.lake`) and report how many
    /// distinct rendered water levels (`elev_to_level(filled).round()`) it spans. A correct
    /// lake is ONE flat mirror → span 0. Any body with span > 0 is a stepped lake (the bug).
    #[test]
    #[ignore]
    fn diagnose_lake_steps() {
        let seed = 1u64;
        let tect = TectonicField::generate(seed);
        let n = COLS * ROWS;
        let mut elev = vec![0.0f32; n];
        for y in 0..ROWS {
            for x in 0..COLS {
                elev[y * COLS + x] =
                    elevation(seed, x as f32, y as f32, tect.macro_at(x, y), tect.mountain_at(x, y));
            }
        }
        crate::erosion::erode(seed, &mut elev, &|_| {});
        let hydro = crate::hydrology::compute(&elev, SEA_FRACTION);
        // Rendered water level per lake column (same formula the world model uses).
        let lvl = |i: usize| elev_to_level(hydro.filled[i]).round() as i32;
        let mut seen = vec![false; n];
        let (mut bodies, mut stepped_bodies, mut stepped_cells, mut worst_span) = (0u64, 0u64, 0u64, 0i32);
        let mut stack: Vec<usize> = Vec::new();
        let mut comp: Vec<usize> = Vec::new();
        for start in 0..n {
            if !hydro.lake[start] || seen[start] {
                continue;
            }
            comp.clear();
            stack.push(start);
            seen[start] = true;
            let (mut lo, mut hi) = (i32::MAX, i32::MIN);
            while let Some(i) = stack.pop() {
                comp.push(i);
                let l = lvl(i);
                lo = lo.min(l);
                hi = hi.max(l);
                let (x, y) = ((i % COLS) as i32, (i / COLS) as i32);
                for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                    if nx < 0 || ny < 0 || nx >= COLS as i32 || ny >= ROWS as i32 {
                        continue;
                    }
                    let j = (ny * COLS as i32 + nx) as usize;
                    if hydro.lake[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
            bodies += 1;
            let span = hi - lo;
            if span > 0 {
                stepped_bodies += 1;
                stepped_cells += comp.len() as u64;
                worst_span = worst_span.max(span);
            }
        }
        eprintln!(
            "lake bodies={bodies}, STEPPED bodies={stepped_bodies} ({:.0}%), cells in stepped bodies={stepped_cells}, worst intra-lake span={worst_span} levels",
            stepped_bodies as f64 / bodies.max(1) as f64 * 100.0
        );
    }

    /// Shore HOLES: dry cells whose top sits BELOW an adjacent water cell's surface — the
    /// gap between bank and water the user reported. Counts them and attributes each to WHY
    /// it stayed dry (ocean/lake/river flag vs none), so we can tell a hydrology
    /// classification gap from a pure discretisation artefact. Run: `cargo test
    /// diagnose_shore_holes -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn diagnose_shore_holes() {
        for seed in [1u64, 7, 42] {
            let t = VoxelTerrain::new(seed);
            // Recompute hydrology to attribute each hole (same inputs as `VoxelTerrain::new`).
            let tect = TectonicField::generate(seed);
            let n = COLS * ROWS;
            let mut elev = vec![0.0f32; n];
            for y in 0..ROWS {
                for x in 0..COLS {
                    elev[y * COLS + x] =
                        elevation(seed, x as f32, y as f32, tect.macro_at(x, y), tect.mountain_at(x, y));
                }
            }
            crate::erosion::erode(seed, &mut elev, &|_| {});
            let hydro = crate::hydrology::compute(&elev, SEA_FRACTION);

            let (mut holes, mut as_lake, mut as_river, mut as_ocean, mut as_none) =
                (0u64, 0u64, 0u64, 0u64, 0u64);
            let mut holes_by_lake = 0u64; // holes whose deepest adjacent water is a LAKE
            let mut max_drop = 0u8;
            let mut sample = None;
            for y in 0..ROWS {
                for x in 0..COLS {
                    let i = y * COLS + x;
                    let (xi, yi) = (x as i32, y as i32);
                    if t.water_level(xi, yi) != 0 {
                        continue; // this cell is water — not a hole
                    }
                    let h = t.height(xi, yi);
                    // Deepest adjacent water surface standing ABOVE this dry cell's top, and
                    // whether the deepest such neighbour is a LAKE (vs a river channel wall).
                    let mut below_wl = 0u8;
                    let mut adj_is_lake = false;
                    for (nx, ny) in [(xi + 1, yi), (xi - 1, yi), (xi, yi + 1), (xi, yi - 1)] {
                        let nwl = t.water_level(nx, ny);
                        if nwl > h && nwl > below_wl {
                            below_wl = nwl;
                            let j = (ny as usize) * COLS + nx as usize;
                            adj_is_lake = (0..COLS as i32).contains(&nx)
                                && (0..ROWS as i32).contains(&ny)
                                && hydro.lake[j];
                        }
                    }
                    if below_wl == 0 {
                        continue; // sits above all neighbouring water — a normal bank, not a hole
                    }
                    holes += 1;
                    if adj_is_lake {
                        holes_by_lake += 1;
                    }
                    max_drop = max_drop.max(below_wl - h);
                    if hydro.ocean[i] {
                        as_ocean += 1;
                    } else if hydro.lake[i] {
                        as_lake += 1;
                    } else if hydro.river[i] {
                        as_river += 1;
                    } else {
                        as_none += 1;
                    }
                    if sample.is_none() {
                        sample = Some((x, y, h, below_wl, hydro.filled[i] - elev[i]));
                    }
                }
            }

            // SPIKES: dry cells ringed by water on >=3 sides that poke ABOVE every adjacent
            // water surface — a lone pillar standing in the water. Also flag how many were
            // pushed up by the shore-lift pass (height now exceeds the raw classify height),
            // to tell a manufactured spike from a natural island.
            let (mut spikes, mut spikes_lifted) = (0u64, 0u64);
            let mut spike_sample = None;
            for y in 0..ROWS {
                for x in 0..COLS {
                    let (xi, yi) = (x as i32, y as i32);
                    if t.water_level(xi, yi) != 0 {
                        continue;
                    }
                    let h = t.height(xi, yi);
                    let (mut water_nb, mut above_all) = (0u8, true);
                    for (nx, ny) in [(xi + 1, yi), (xi - 1, yi), (xi, yi + 1), (xi, yi - 1)] {
                        let nwl = t.water_level(nx, ny);
                        if nwl > 0 {
                            water_nb += 1;
                            if nwl >= h {
                                above_all = false; // some neighbour water reaches this cell's top
                            }
                        }
                    }
                    if water_nb >= 3 && above_all {
                        spikes += 1;
                        // Raw classify height (pre shore-lift) for this column.
                        let raw = elev_to_level(elev[y * COLS + x]).round() as u8;
                        if h > raw {
                            spikes_lifted += 1;
                        }
                        if spike_sample.is_none() {
                            spike_sample = Some((x, y, h, raw, water_nb));
                        }
                    }
                }
            }

            eprintln!(
                "seed {seed}: shore holes={holes} (dry but below adj water), of which adjacent-to-LAKE={holes_by_lake}, max drop={max_drop} lvl; \
                 hole-cell class: ocean={as_ocean} lake={as_lake} river={as_river} none={as_none}; \
                 sample (x,y,h,adjwl,filled-elev)={sample:?}"
            );
            eprintln!(
                "seed {seed}: SPIKES={spikes} (dry pillar, >=3 water nbrs, above all), of which shore-lifted={spikes_lifted}; \
                 sample (x,y,h,raw,water_nbrs)={spike_sample:?}"
            );
        }
    }

    #[test]
    fn bit_pack_roundtrips() {
        for &h in &[1u8, 4, 7, 10, 200] {
            for b in [BiomeKind::Ocean, BiomeKind::Forest, BiomeKind::Snow] {
                for &f in &[0u8, FLAG_WATER, 0xF] {
                    let c = pack_cell(h, b, f);
                    assert_eq!(cell_height(c), h);
                    assert_eq!(cell_biome(c), b);
                    assert_eq!(cell_flags(c), f & 0xF);
                }
            }
        }
    }

    #[test]
    fn generation_is_deterministic() {
        let a = VoxelTerrain::new(42);
        let b = VoxelTerrain::new(42);
        for y in (0..ROWS).step_by(7) {
            for x in (0..COLS).step_by(7) {
                assert_eq!(a.height_at(x, y), b.height_at(x, y));
                assert_eq!(a.biome_at(x, y), b.biome_at(x, y));
            }
        }
    }

    #[test]
    fn different_seeds_differ() {
        let a = VoxelTerrain::new(1);
        let b = VoxelTerrain::new(2);
        let mut diff = 0;
        for y in 0..ROWS {
            for x in 0..COLS {
                if a.height_at(x, y) != b.height_at(x, y) {
                    diff += 1;
                }
            }
        }
        assert!(diff > (COLS * ROWS) / 10, "seeds barely differ: {diff}");
    }

    #[test]
    fn heights_in_range_and_mixed_water_land() {
        let t = VoxelTerrain::new(7);
        let mut water = 0;
        let total = COLS * ROWS;
        for y in 0..ROWS {
            for x in 0..COLS {
                let h = t.height_at(x, y);
                assert!((1..=MAX_H).contains(&h), "height {h} out of range");
                if t.is_water(x, y) {
                    water += 1;
                }
            }
        }
        assert!(water > 0 && water < total, "expected mix of water/land, got {water}/{total}");
    }

    #[test]
    fn out_of_world_is_air_and_sampling_is_consistent() {
        let t = VoxelTerrain::new(3);
        // Out of the world reads as air (height 0) on every side — that's the slab edge.
        assert_eq!(t.height(-1, 0), 0);
        assert_eq!(t.height(0, -1), 0);
        assert_eq!(t.height(COLS as i32, 0), 0);
        assert_eq!(t.height(0, ROWS as i32), 0);
        // The signed `height`/`cell` and the unsigned `height_at` agree in-world, and
        // a column read straight across a chunk seam (x = CHUNK-1 vs CHUNK) is the same
        // value whether reached as "self" or as a neighbour — the seam is just one array.
        for &(x, y) in &[(0usize, 0usize), (CHUNK - 1, 1), (CHUNK, 1), (COLS - 1, ROWS - 1)] {
            assert_eq!(t.height(x as i32, y as i32), t.height_at(x, y));
            assert_eq!(cell_height(t.cell(x as i32, y as i32)), t.height_at(x, y));
        }
    }

    /// `quant_unit` ↔ `/255.0` round-trips within one quantisation step (≤ 1/255).
    #[test]
    fn quant_roundtrip_within_one_step() {
        for k in 0..=1000u32 {
            let v = k as f32 / 1000.0;
            let back = quant_unit(v) as f32 / 255.0;
            assert!((back - v).abs() <= 1.0 / 255.0 + 1e-6, "quant({v}) round-trips to {back}");
        }
        // Saturates, doesn't wrap.
        assert_eq!(quant_unit(-0.5), 0);
        assert_eq!(quant_unit(1.5), 255);
    }

    /// S1: temperature/moisture are now populated for EVERY column (not just lowland), and
    /// temperature is a REAL latitude field everywhere — the equator row is warmer on average
    /// than the poles. (If climate were left unset on beach/mountain/snow columns, those rows
    /// would read a flat 0 and the gradient would vanish.) Also bounds every value to `[0,1]`.
    #[test]
    fn env_fields_populated_and_latitude_gradient() {
        let t = VoxelTerrain::new(1);
        let row_mean_temp = |y: usize| {
            let s: f32 = (0..COLS).map(|x| t.temperature_at(x, y)).sum();
            s / COLS as f32
        };
        let equator = row_mean_temp(ROWS / 2);
        let pole = (row_mean_temp(0) + row_mean_temp(ROWS - 1)) * 0.5;
        eprintln!("mean temp: equator {equator:.3}, poles {pole:.3}");
        assert!(equator > pole + 0.1, "no latitude temp gradient: equator {equator:.3} ≤ poles {pole:.3}");
        // Every sampled column (across all biome bands) has in-range, defined climate.
        let mut moist_seen_low = false;
        let mut moist_seen_high = false;
        for y in (0..ROWS).step_by(ROWS / 20) {
            for x in (0..COLS).step_by(COLS / 20) {
                let (te, mo) = (t.temperature_at(x, y), t.moisture_at(x, y));
                assert!((0.0..=1.0).contains(&te) && (0.0..=1.0).contains(&mo));
                moist_seen_low |= mo < 0.35;
                moist_seen_high |= mo > 0.65;
            }
        }
        assert!(moist_seen_low && moist_seen_high, "moisture field has no variety");
    }

    /// S1: distance-to-water is 0 exactly on water, grows off the shore, and is a valid BFS
    /// (every non-source column under the cap has a 4-neighbour one step closer). The far
    /// inland plateau reaching the 255 floor is expected, not a bug.
    #[test]
    fn water_dist_is_a_valid_bfs() {
        let t = VoxelTerrain::new(1);
        let mut max_d = 0u8;
        for y in 0..ROWS {
            for x in 0..COLS {
                let d = t.water_dist_at(x, y);
                // Source set agrees with the water flag (they're set together in `generate`).
                assert_eq!(d == 0, t.is_water(x, y), "water_dist 0 vs is_water disagree at ({x},{y})");
                max_d = max_d.max(d);
                // BFS invariant: an interior column with finite distance has a strictly closer
                // neighbour (skip the saturated 255 rim, where the true distance is clipped).
                if (1..255).contains(&d) && x > 0 && y > 0 && x + 1 < COLS && y + 1 < ROWS {
                    let closer = [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]
                        .iter()
                        .any(|&(a, b)| t.water_dist_at(a, b) == d - 1);
                    assert!(closer, "water_dist not a BFS at ({x},{y}): d={d}, no neighbour at d-1");
                }
            }
        }
        eprintln!("max water_dist = {max_d}");
        assert!(max_d > 1, "distance-to-water never grows (no inland?)");
    }

    /// S1: slope is in `[0,1]`, zero on flat ground (open-sea floor is flat), positive where
    /// the terrain steps, and shows NO false cliff at the map edge (out-of-world neighbours
    /// are treated as level, so the corner column isn't read as a wall).
    #[test]
    fn slope_bounds_and_no_edge_cliff() {
        let t = VoxelTerrain::new(1);
        let mut max_s = 0.0f32;
        for y in (0..ROWS).step_by(7) {
            for x in (0..COLS).step_by(7) {
                let s = t.slope_at(x, y);
                assert!((0.0..=1.0).contains(&s), "slope out of range at ({x},{y}): {s}");
                max_s = max_s.max(s);
            }
        }
        assert!(max_s > 0.0, "slope is zero everywhere (no relief?)");
        // Corners: bounded, and not a spurious full-relief cliff from the world edge.
        for &(x, y) in &[(0, 0), (COLS - 1, 0), (0, ROWS - 1), (COLS - 1, ROWS - 1)] {
            let s = t.slope_at(x, y);
            assert!((0.0..1.0).contains(&s), "edge column ({x},{y}) reads a false cliff: {s}");
        }
    }

    /// S1: the new fields are deterministic per seed (the sim must replay).
    #[test]
    fn env_fields_are_deterministic() {
        let (a, b) = (VoxelTerrain::new(7), VoxelTerrain::new(7));
        for &(x, y) in &[(0, 0), (COLS / 3, ROWS / 2), (COLS - 1, ROWS - 1), (CHUNK, CHUNK)] {
            assert_eq!(a.temperature_at(x, y), b.temperature_at(x, y));
            assert_eq!(a.moisture_at(x, y), b.moisture_at(x, y));
            assert_eq!(a.water_dist_at(x, y), b.water_dist_at(x, y));
            assert_eq!(a.slope_at(x, y), b.slope_at(x, y));
        }
    }

    // ---- S3: vegetation (pure-law tests, no world generation needed) ----

    /// The regrow law recovers from 0 (no fixed point there — the bug a logistic law would
    /// have had), is monotonic, and never overshoots capacity.
    #[test]
    fn regrow_recovers_from_zero_and_saturates() {
        let cap = 0.8;
        assert_eq!(regrow(0.0, cap, 0.0), 0.0); // no time → no growth
        let a = regrow(0.0, cap, 50.0);
        let b = regrow(0.0, cap, 100.0);
        assert!(0.0 < a && a < b && b < cap, "not monotonic toward cap: {a} {b} {cap}");
        assert!(regrow(0.0, cap, 1e6) <= cap + 1e-6, "overshot cap");
        assert!((regrow(0.0, cap, 1e6) - cap).abs() < 1e-3, "did not saturate to cap");
        // From a non-zero start it still only ever climbs to cap.
        assert!(regrow(0.5, cap, 1e6) <= cap + 1e-6);
    }

    /// Closed-form ⇒ the lazy (skip-the-untouched-ticks) update equals the stepwise one:
    /// regrowing once over `t1+t2` matches regrowing over `t1` then `t2`. This is what makes
    /// amortised regen exact, so a column the sim ignores for a million ticks is still correct.
    #[test]
    fn regrow_is_semigroup() {
        let cap = 0.9;
        let (t1, t2) = (37.0, 121.0);
        let lazy = regrow(0.1, cap, t1 + t2);
        let stepwise = regrow(regrow(0.1, cap, t1), cap, t2);
        assert!((lazy - stepwise).abs() < 1e-6, "lazy {lazy} ≠ stepwise {stepwise}");
    }

    /// Capacity: water carries nothing, wetter biomes carry more, and moisture nudges it up.
    #[test]
    fn carrying_capacity_ordering() {
        assert_eq!(carrying_capacity(BiomeKind::Ocean, 1.0), 0.0);
        let jungle = carrying_capacity(BiomeKind::Jungle, 0.5);
        let plains = carrying_capacity(BiomeKind::Plains, 0.5);
        let desert = carrying_capacity(BiomeKind::Desert, 0.5);
        assert!(jungle > plains && plains > desert, "{jungle} {plains} {desert}");
        assert!(
            carrying_capacity(BiomeKind::Forest, 0.9) > carrying_capacity(BiomeKind::Forest, 0.1),
            "moisture should raise capacity"
        );
    }

    /// S3 on the real world model (one generation): vegetation starts mature, water is bare,
    /// grazing removes ≤ what's present, a cleared column regrows from 0, and a long chain of
    /// graze→requant doesn't drift (the F5 quantisation-noise guard).
    #[test]
    fn vegetation_field_grazing_and_regrowth() {
        let mut t = VoxelTerrain::new(1);
        // Find a high-capacity land column and a water column.
        let (mut land, mut wet) = (None, None);
        'scan: for y in (0..ROWS).step_by(13) {
            for x in (0..COLS).step_by(13) {
                if land.is_none() && !t.is_water(x, y) {
                    let cap = carrying_capacity(t.biome_at(x, y), t.moisture_at(x, y));
                    if cap > 0.3 {
                        land = Some((x, y, cap));
                    }
                }
                if wet.is_none() && t.is_water(x, y) {
                    wet = Some((x, y));
                }
                if land.is_some() && wet.is_some() {
                    break 'scan;
                }
            }
        }
        let (lx, ly, base_cap) = land.expect("no high-capacity land column found");
        let (wx, wy) = wet.expect("no water column found");

        // Effective capacity is the carrying capacity SCALED by the column's nutrient (C3 Liebig
        // limit). Plants start mature at THAT, not the un-limited carrying capacity.
        let eff_cap = base_cap * t.nutrient_at(lx, ly, 0);
        let b0 = t.biomass_at(lx, ly, 0);
        assert!((b0 - eff_cap).abs() <= 1.0 / 255.0 + 1e-6, "veg not mature at gen: b0={b0} eff_cap={eff_cap}");
        // Water is bare at any tick.
        assert_eq!(t.biomass_at(wx, wy, 0), 0.0);
        assert_eq!(t.biomass_at(wx, wy, 100_000), 0.0);
        assert_eq!(t.graze(wx, wy, 1.0, 5), 0.0, "grazed biomass off water");

        // Clear-cut the land column: takes ≈ the mature biomass, leaves ~0.
        let taken = t.graze(lx, ly, 1.0, 0);
        assert!((taken - eff_cap).abs() <= 2.0 / 255.0, "clear-cut took {taken}, expected ≈{eff_cap}");
        assert!(t.biomass_at(lx, ly, 0) <= 1.0 / 255.0, "column not bare right after clear-cut");
        // Over-graze immediately: nothing left to take.
        assert!(t.graze(lx, ly, 1.0, 0) <= 1.0 / 255.0, "over-graze produced food from nothing");

        // Regrows from 0 upward, monotonically (no downward drift across 200 graze→requantise
        // cycles — the F5 quant-noise guard), never above the un-limited carrying capacity, and
        // recovers a real fraction of capacity as nutrient weathers back toward its baseline.
        let mut prev = 0.0f32;
        for k in 1..=200u64 {
            let tick = k * 50;
            t.graze(lx, ly, 0.0, tick); // take nothing, but re-quantise current at this tick
            let b = t.biomass_at(lx, ly, tick);
            assert!(b >= prev - 2.0 / 255.0, "biomass drifted DOWN at step {k}: {b} < {prev}");
            assert!(b <= base_cap + 1.0 / 255.0, "biomass exceeded carrying capacity at step {k}: {b} > {base_cap}");
            prev = b;
        }
        assert!(prev > 0.4 * eff_cap, "did not regrow a meaningful fraction of cap: {prev} vs eff {eff_cap}");
    }

    /// Biomass replays deterministically: same graze sequence ⇒ same readings.
    #[test]
    fn biomass_is_deterministic() {
        let (mut a, mut b) = (VoxelTerrain::new(5), VoxelTerrain::new(5));
        let col = (COLS / 2 + 7, ROWS / 3 + 3);
        for k in 0..10u64 {
            a.graze(col.0, col.1, 0.05, k * 20);
            b.graze(col.0, col.1, 0.05, k * 20);
        }
        assert_eq!(a.biomass_at(col.0, col.1, 500), b.biomass_at(col.0, col.1, 500));
    }
