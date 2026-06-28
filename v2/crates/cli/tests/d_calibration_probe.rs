//! D-slice pivot calibration probe (issue #169): measures equilibrium for uptake-layer switching.
//! Outputs: pop, R̄, lr-distribution, layer-dominance split, switching counts, control/plastic means.
//! DELETE before merge. Results inform: reg_setpoint value, corridor band, A/B margin.
use cli::{build_sim, default_config};
use sim_core::Genome;

#[test]
fn d_probe_layer_switch() {
    if cfg!(debug_assertions) { return; }

    let founder = Genome::founder(2);
    let setpoint = founder.reg_setpoint as i64;

    for &seed in &[0xA11A_2A11u64, 0x1234_5678u64] {
        // ── Plastic run (reg_gain_max=4, default) ─────────────────────────────────────────────
        let (plastic_mean, pop_end, r_bar, lr_min, lr_med, lr_max,
             fav_l0, fav_eq, fav_l1, cold_cnt, sw_cnt, g_lo, g_hi) = {
            let mut sim = build_sim(default_config(seed));
            let (mut sum, mut count) = (0u64, 0u64);
            for t in 0..4000u64 {
                sim.step();
                if t >= 2000 { sum += sim.population(); count += 1; }
            }
            let mean = if count > 0 { sum / count } else { 0 };
            let pop = sim.population();
            let field_total = sim.telemetry().field_total;
            let n_layers = sim.econ().n_layers as i64;
            let world_dim = sim.econ().world_dim;
            let rb = field_total / n_layers / (world_dim * world_dim);
            let (lr_min, lr_med, lr_max) = sim.local_resource_stats();
            let (fl0, feq, fl1) = sim.layer_dominance_at_occupied();
            let (cold, sw) = sim.switching_counts();
            let (glo, ghi) = sim.reg_gain_range();
            (mean, pop, rb, lr_min, lr_med, lr_max, fl0, feq, fl1, cold, sw, glo, ghi)
        };

        // ── Control run (reg_gain_max=0, regulation locked OFF) ───────────────────────────────
        let ctrl_mean = {
            let mut cfg = default_config(seed);
            cfg.econ.reg_gain_max = 0;
            let mut sim = build_sim(cfg);
            let (mut sum, mut count) = (0u64, 0u64);
            for t in 0..4000u64 {
                sim.step();
                if t >= 2000 { sum += sim.population(); count += 1; }
            }
            if count > 0 { sum / count } else { 0 }
        };

        let fav_total = fav_l0 + fav_l1;
        let pct_l0 = if fav_total > 0 { fav_l0 * 100 / fav_total } else { 0 };
        let pct_l1 = if fav_total > 0 { fav_l1 * 100 / fav_total } else { 0 };
        let sw_total = cold_cnt + sw_cnt;
        let pct_sw = if sw_total > 0 { sw_cnt * 100 / sw_total } else { 0 };
        let plastic_delta = if plastic_mean >= ctrl_mean {
            plastic_mean - ctrl_mean
        } else { 0 };

        eprintln!("PROBE seed=0x{seed:08X} sp={setpoint}");
        eprintln!("  pop_end={pop_end} R={r_bar} plastic[2k-4k]={plastic_mean} ctrl={ctrl_mean} delta={plastic_delta}");
        eprintln!("  lr=[{lr_min},{lr_med},{lr_max}] reg_gain=[{g_lo},{g_hi}]");
        eprintln!("  layer_dom: l0={fav_l0}({pct_l0}%) eq={fav_eq} l1={fav_l1}({pct_l1}%) of {fav_total} occupied");
        eprintln!("  switching: cold={cold_cnt} switched={sw_cnt}({pct_sw}%) of {sw_total} agents");
    }
}
