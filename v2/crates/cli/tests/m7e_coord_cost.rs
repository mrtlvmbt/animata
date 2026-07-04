//! M7-e-a golden-NEUTRAL guard (#251): every shipped production config keeps `EconParams.c_coord`
//! at `0`. The coordination-cost sink `c_coord * Σ module_cell_count` wired into `stage_metabolism`
//! (sim-core `stages.rs`) is gated by that coefficient — at `c_coord=0` the added term is `0` for
//! every entity (populated or empty `CellGraph` alike), byte-identical to the pre-M7-e trajectory,
//! so the 6 production goldens (`golden.rs` + the 5 `golden_conserved.rs` configs) stay
//! byte-identical, un-re-pinned. The real byte-identity proof is those existing golden tests
//! staying green; this is the structural belt-and-braces guard that the sink is never accidentally
//! armed in a shipped config. `c_coord > 0` (calibration + viability verification + re-pin) is
//! M7-e-b, not this slice.

use cli::{
    cprime_config, default_config, differentiation_config, dprime_config, l3_config, phase2_config,
};
use sim_core::SimConfig;

#[test]
fn m7e_prod_inert_all_goldens() {
    let seed = 0xA11A_2A11;
    let configs: [(&str, SimConfig); 6] = [
        ("default", default_config(seed)),
        ("cprime", cprime_config(seed)),
        ("dprime", dprime_config(seed)),
        ("l3", l3_config(seed)),
        ("phase2", phase2_config(seed)),
        ("differentiation", differentiation_config(seed)),
    ];

    for (name, cfg) in &configs {
        assert_eq!(
            cfg.econ.c_coord, 0,
            "production config '{name}' must ship c_coord=0 (M7-e golden-NEUTRAL gate)"
        );
    }
}
