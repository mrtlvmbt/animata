// TEMP CALIBRATION PROBE — DELETE AFTER READING x86 CI VALUES
// Measures actual default_config(S) pop and R̄ at t=4000 (the B-4 corridor horizon).

use cli::{build_sim, default_config};

const S: u64 = 0xA11A_2A11;

#[test]
fn b4_calibration_probe_4000() {
    if cfg!(debug_assertions) { return; }
    let mut sim = build_sim(default_config(S));
    for _ in 0..4_000 { sim.step(); }
    let pop = sim.population();
    let tel = sim.telemetry();
    let field_total = tel.field_total;
    let n_layers = sim.econ().n_layers as i64;
    let world_dim = sim.econ().world_dim;
    let n_cells = world_dim * world_dim;
    let r_bar = field_total / n_layers / n_cells;
    panic!("CALIBRATION t=4000: pop={pop} field={field_total} n_layers={n_layers} world_dim={world_dim} n_cells={n_cells} R_bar={r_bar}");
}

#[test]
fn b4_calibration_probe_16000() {
    if cfg!(debug_assertions) { return; }
    let mut sim = build_sim(default_config(S));
    for _ in 0..16_000 { sim.step(); }
    let pop = sim.population();
    let tel = sim.telemetry();
    let field_total = tel.field_total;
    let n_layers = sim.econ().n_layers as i64;
    let world_dim = sim.econ().world_dim;
    let n_cells = world_dim * world_dim;
    let r_bar = field_total / n_layers / n_cells;
    panic!("CALIBRATION t=16000: pop={pop} field={field_total} n_layers={n_layers} world_dim={world_dim} n_cells={n_cells} R_bar={r_bar}");
}
