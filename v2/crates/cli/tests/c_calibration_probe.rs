use cli::{build_sim, default_config};
const S: u64 = 0xA11A_2A11;

#[test]
fn c_probe_t4000() {
    if cfg!(debug_assertions) { return; }
    let mut sim = build_sim(default_config(S));
    for _ in 0..4_000 { sim.step(); }
    let pop = sim.population();
    let tel = sim.telemetry();
    let n_layers = sim.econ().n_layers as i64;
    let world_dim = sim.econ().world_dim;
    let n_cells = world_dim * world_dim;
    let r_bar = tel.field_total / n_layers / n_cells;
    panic!("C-PROBE t=4000: pop={pop} R_bar={r_bar} field_total={}", tel.field_total);
}
