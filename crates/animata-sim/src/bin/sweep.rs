//! Parameter sweep — vary ONE `Params` field across a value grid (× seeds), run the sim per cell, and
//! emit a CSV of emergent stats. OBSERVATIONAL, off the golden path: every cell builds a sim from a
//! NON-default `SimConfig` (the golden trajectory is `SimConfig::default()`), so this never asserts or
//! perturbs the determinism golden. The `checksum_hex` column is a per-cell signature compared only
//! between cells of the same run on the same machine — never against the arch-bound golden.
//!
//! Correctness (the reason this is not `headless` in a loop): each `(seed, value)` cell needs a FRESH
//! PRISTINE terrain overlay. `VoxelTerrain` holds a sim-mutated overlay (`state`, vegetation+nutrients)
//! that `step` writes every tick; reusing a stepped terrain would start the next value from a depleted
//! world. So we `clone_state` the initial overlay ONCE per seed and `set_state` it back before each
//! value — every cell reproduces a standalone `VoxelTerrain::new(seed)` start, WITHOUT re-running
//! worldgen per cell (worldgen is paid once per seed, like the `multiseed` sim-run scenario).
//!
//! Usage: `cargo run -p animata-sim --bin sweep --release -- \
//!   --param toxin_lethality --values 0.0,0.5,1.0 --seeds 1,2 --ticks 8000 [--force] [--out sweep.csv]`

use animata_sim::sim::{state_checksum, Sim};
use animata_sim::sim_config::SimConfig;
use animata_sim::terrain::VoxelTerrain;

/// Max cells (`|values| × |seeds|`) without `--force`. Per-cell cost is `ticks × (per-tick cost ∝ live
/// population)` and the swept param itself moves population, so this caps the cell COUNT, not wallclock
/// — the sim's hard population cap + the CI job timeout are the real backstops.
const CELL_CAP: usize = 64;

struct Row {
    value: f32,
    seed: u64,
    population: usize,
    multi: f32,
    complex: f32,
    carnivore: f32,
    autotroph: f32,
    species: usize,
    niches: usize,
    checksum: u64,
}

/// Run one `(seed, value)` cell to completion and collect its row. `terrain` MUST already hold the
/// pristine overlay for `seed` (the caller `set_state`s it back before each call); `param` MUST be a
/// valid `Params` name (validated in `main`).
fn run_cell(seed: u64, terrain: &mut VoxelTerrain, param: &str, value: f32, ticks: u64) -> Row {
    let mut cfg = SimConfig::default();
    cfg.params.set(param, value);
    let mut sim = Sim::with_config(seed, terrain, cfg);
    for t in 0..ticks {
        sim.step(terrain, t);
    }
    let (multi, complex) = sim.complexity_mix();
    Row {
        value,
        seed,
        population: sim.population(),
        multi,
        complex,
        carnivore: sim.frac_carnivore(),
        autotroph: sim.frac_autotroph(),
        species: sim.species_count(),
        niches: sim.niche_coverage(terrain),
        checksum: state_checksum(&sim, terrain),
    }
}

fn csv_row(param: &str, ticks: u64, r: &Row) -> String {
    format!(
        "{param},{},{},{ticks},{},{:.1},{:.1},{:.1},{:.1},{},{},{:016x}",
        r.value,
        r.seed,
        r.population,
        r.multi * 100.0,
        r.complex * 100.0,
        r.carnivore * 100.0,
        r.autotroph * 100.0,
        r.species,
        r.niches,
        r.checksum,
    )
}

fn die(msg: &str) -> ! {
    eprintln!("sweep: {msg}");
    std::process::exit(2);
}

fn parse_list<T: std::str::FromStr>(s: &str, what: &str) -> Vec<T> {
    s.split(',')
        .map(|x| x.trim().parse().unwrap_or_else(|_| die(&format!("bad {what} value: '{}'", x.trim()))))
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opt = |name: &str| -> Option<String> {
        args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
    };
    let force = args.iter().any(|a| a == "--force");
    let param = opt("--param").unwrap_or_else(|| die("missing --param NAME (one of Params)"));
    let values_s = opt("--values").unwrap_or_else(|| die("missing --values v1,v2,.."));
    let seeds_s = opt("--seeds").unwrap_or_else(|| "1".to_string());
    let ticks: u64 = opt("--ticks").and_then(|s| s.parse().ok()).unwrap_or(4000);
    let out = opt("--out");

    // Validate the param name against the introspection surface (false ⇒ unknown).
    if !SimConfig::default().params.set(&param, 1.0) {
        let names: Vec<&str> = SimConfig::default().params.pairs().iter().map(|(n, _)| *n).collect();
        die(&format!("unknown --param '{param}'. valid: {}", names.join(", ")));
    }
    let values: Vec<f32> = parse_list(&values_s, "--values");
    let seeds: Vec<u64> = parse_list(&seeds_s, "--seeds");

    let cells = values.len() * seeds.len();
    // Cost-line + cap → STDERR so stdout stays pure CSV.
    eprintln!("sweep: {cells} cells × {ticks} ticks (param={param}) — cost ∝ population, not a wallclock ETA");
    if cells > CELL_CAP && !force {
        die(&format!("{cells} cells > cap {CELL_CAP}; pass --force to override"));
    }

    let header = "param,value,seed,ticks,population,multi_pct,complex_pct,carnivore_pct,autotroph_pct,species,niches,checksum_hex";
    let mut lines = vec![header.to_string()];
    for &seed in &seeds {
        let mut terrain = VoxelTerrain::new(seed); // worldgen ONCE per seed
        let pristine = terrain.clone_state(); // the initial overlay, captured before any step
        for &value in &values {
            terrain.set_state(pristine.clone()).expect("overlay reset (set_state) failed");
            let row = run_cell(seed, &mut terrain, &param, value, ticks);
            lines.push(csv_row(&param, ticks, &row));
        }
    }
    let csv = lines.join("\n");
    println!("{csv}"); // stdout = pure CSV (cost-line went to stderr)
    if let Some(path) = out {
        match std::fs::write(&path, format!("{csv}\n")) {
            Ok(()) => eprintln!("sweep: wrote {path}"),
            Err(e) => eprintln!("sweep: write {path} failed: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Both tests are OBSERVATIONAL and arch-independent: they compare two checksums produced in the
    // SAME process (no golden constant is pinned), so they run in the x86 CI job and never touch the
    // arch-bound `golden-arm64` job. Cheap (300 ticks).

    /// Step-1 guard: run a value, RESET the overlay via `set_state`, run the SAME value again →
    /// IDENTICAL checksum. A leaked overlay (a broken reset) would make the second run diverge.
    #[test]
    fn sweep_overlay_resets_between_values() {
        let seed = 1;
        let mut terrain = VoxelTerrain::new(seed);
        let pristine = terrain.clone_state();
        let a = run_cell(seed, &mut terrain, "photo_rate", 0.5, 300).checksum;
        terrain.set_state(pristine.clone()).unwrap();
        let b = run_cell(seed, &mut terrain, "photo_rate", 0.5, 300).checksum;
        assert_eq!(a, b, "overlay reset between values is broken (set_state leak)");
    }

    /// Step-2 guard (valid only because step-1 proves the reset): two DIFFERENT `photo_rate` values
    /// must diverge — else the swept param never reaches `step`/the registry and every sweep column
    /// would be silently flat (e.g. a registry refactor that stops consuming the param).
    #[test]
    fn sweep_param_reaches_step() {
        let seed = 1;
        let mut terrain = VoxelTerrain::new(seed);
        let pristine = terrain.clone_state();
        let a = run_cell(seed, &mut terrain, "photo_rate", 0.5, 300).checksum;
        terrain.set_state(pristine.clone()).unwrap();
        let b = run_cell(seed, &mut terrain, "photo_rate", 2.0, 300).checksum;
        assert_ne!(a, b, "swept param photo_rate does not reach step (flat sweep column)");
    }
}
