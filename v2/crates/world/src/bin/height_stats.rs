//! THROWAWAY measurement bin — height distribution + lone-peak (base-needle) census for the
//! render height-ramp normalization (A) and the despike investigation (B). Delete after use.
//! Usage: height_stats <dim> [seed]

use world::gen::caps::classify_and_caps;

const HMAX: i64 = 200;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dim: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(512);
    let seed: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);

    let f = classify_and_caps(seed, HMAX, dim, false, true, true, true, true, true);
    let h = &f.height;
    assert_eq!(h.len(), dim * dim);

    let mut sorted = h.clone();
    sorted.sort_unstable();
    let n = sorted.len();
    let pct = |p: f64| sorted[((p / 100.0 * (n as f64 - 1.0)).round() as usize).min(n - 1)];
    println!("dim={dim} seed={seed} n={n}");
    println!(
        "min={} p1={} p2={} p5={} p50={} p90={} p95={} p98={} p99={} p99.9={} max={}",
        sorted[0],
        pct(1.0),
        pct(2.0),
        pct(5.0),
        pct(50.0),
        pct(90.0),
        pct(95.0),
        pct(98.0),
        pct(99.0),
        pct(99.9),
        sorted[n - 1]
    );

    // Lone-peak (base-needle) census: cell strictly exceeds max of its 8 grid neighbours by > MARGIN.
    // Report at several margins on the CURRENT (production de-needle=40) field to pick a tighter knob.
    let at = |x: i64, z: i64| -> Option<i64> {
        if x >= 0 && z >= 0 && (x as usize) < dim && (z as usize) < dim {
            Some(h[z as usize * dim + x as usize])
        } else {
            None
        }
    };
    let mut peaks: Vec<(i64, i64, i64, i64)> = Vec::new(); // (height, excess, x, z)
    for z in 0..dim as i64 {
        for x in 0..dim as i64 {
            let hv = h[z as usize * dim + x as usize];
            let mut nmax = i64::MIN;
            for dz in -1..=1 {
                for dx in -1..=1 {
                    if dx == 0 && dz == 0 {
                        continue;
                    }
                    if let Some(nh) = at(x + dx, z + dz) {
                        nmax = nmax.max(nh);
                    }
                }
            }
            if nmax != i64::MIN {
                peaks.push((hv, hv - nmax, x, z));
            }
        }
    }
    peaks.sort_unstable_by(|a, b| b.1.cmp(&a.1));
    for m in [15, 20, 25, 30, 35, 40] {
        let c = peaks.iter().filter(|p| p.1 > m).count();
        println!("lone_peaks(margin>{m})={c}");
    }
    println!("top residual columns (height, excess over nmax, x, z):");
    for (hv, exc, x, z) in peaks.iter().take(15) {
        println!("  h={hv} excess={exc} at ({x},{z})");
    }
}
