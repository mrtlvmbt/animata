//! W-17 gen-perf timing harness — measures worldgen performance per stage.
//! Uses U-11 progress callback to record stage boundaries.
//!
//! Usage: `cargo run --release --example w17_timing`
//! Builds worlds at:
//! - dim=512, ALL-ON (all landforms enabled)
//! - dim=512, DEFAULT (base+erosion only)
//! - dim=64, DEFAULT (sim-lane build)
//!
//! Each reports per-stage durations (via callback boundary timestamps).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;
use world::ProcgenWorld;

const SEED: u64 = 0xA11A_2A11; // Same seed as tests
const HMAX: i64 = 200;
const RESOURCE_BASE: i64 = 120;

/// Stage names corresponding to U-11 ordinals (0-11).
const STAGE_NAMES: &[&str] = &[
    "0: GenerateHeightfield",
    "1: ApplyTectonics",
    "2: ApplyErosion",
    "3: ApplyRidges",
    "4: ApplyAeolian",
    "5: ApplyVolcanic",
    "6: ApplyGlacial",
    "7: ApplyCoastal",
    "8: ApplyBeaches",
    "9: TalusStepFinal",
    "10: DeNeedlePass",
    "11: ClassifyAndCaps",
];

fn run(dim: i64, config: &str, all_on: bool) {
    println!(
        "\n=== W-17 Timing: dim={}, config={} {}===",
        dim,
        config,
        if all_on { "(ALL-ON)" } else { "" }
    );

    let events: Rc<RefCell<Vec<(u8, Instant)>>> = Rc::new(RefCell::new(Vec::new()));
    let events_clone = events.clone();

    let start = Instant::now();

    let _world = ProcgenWorld::new_with_callback(
        dim,
        HMAX,
        RESOURCE_BASE,
        SEED,
        None,
        true,  // enable_base
        all_on,  // enable_tectonics
        all_on,  // enable_aeolian
        all_on,  // enable_volcanic
        all_on,  // enable_glacial
        all_on,  // enable_coastal
        true,  // enable_erosion
        all_on,  // enable_ridges
        all_on,  // enable_beaches
        100,   // erosion_strength
        100,   // glacial_strength
        Some(Box::new(move |stage: u8| {
            events_clone.borrow_mut().push((stage, Instant::now()));
        })),
    );

    let total = start.elapsed();

    // Print stage boundaries and durations
    let events_vec = events.borrow();
    if !events_vec.is_empty() {
        for i in 0..events_vec.len() {
            let (stage, time) = events_vec[i];
            let duration = if i + 1 < events_vec.len() {
                events_vec[i + 1].1.duration_since(time)
            } else {
                total.saturating_sub(time.elapsed())
            };
            let stage_name = if (stage as usize) < STAGE_NAMES.len() {
                STAGE_NAMES[stage as usize].to_string()
            } else {
                format!("{}: Unknown", stage)
            };
            if duration.as_millis() > 0 {
                println!("  {} — {:.1} ms", stage_name, duration.as_millis() as f64);
            }
        }
    }

    println!("  TOTAL: {:.1} ms", total.as_secs_f64() * 1000.0);
}

fn main() {
    println!("W-17 gen-perf timing probe");
    println!("Warmup (dim=128, to initialize rayon)...");
    let _ = ProcgenWorld::new(
        128,
        HMAX,
        RESOURCE_BASE,
        SEED,
        None,
        true,
        false,
        false,
        false,
        false,
        false,
        true,
        false,
        false,
        100,
        100,
    );

    // dim=512 ALL-ON
    run(512, "ALL-ON", true);

    // dim=512 DEFAULT (base+erosion)
    run(512, "DEFAULT", false);

    // dim=64 DEFAULT (sim-lane build)
    run(64, "DEFAULT", false);

    println!("\n(Note: timing includes callback overhead; divide by 5-run median to establish baseline.)");
}
