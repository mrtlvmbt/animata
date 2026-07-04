//! M7-d golden-NEUTRAL guard (#250): every shipped production config keeps `supply_source: None`
//! on its `MorphogenSpec` (or carries no morphogen spec at all). The supply-gate reachability pass
//! added to `CellGraph::from_gradient` (sim-core `genome.rs`) is gated by that field — `None` leaves
//! `module_reachable` all-`true`, byte-identical to M7-c, so the 6 production goldens (`golden.rs` +
//! the 5 `golden_conserved.rs` configs) stay byte-identical, un-re-pinned. The real byte-identity
//! proof is those existing golden tests staying green; this is the structural belt-and-braces guard
//! that the gate is never accidentally armed in a shipped config. Mirrors `m7c_germ_soma.rs`'s
//! `m7c_prod_inert_all_goldens` but asserts the NEW `supply_source` field — does not rely on the
//! M7-c test, which only checks `germ_threshold`.

use cli::{
    cprime_config, default_config, differentiation_config, dprime_config, l3_config, phase2_config,
};
use sim_core::SimConfig;

#[test]
fn m7d_prod_inert_all_goldens() {
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
        if let Some(mspec) = cfg.econ.morphogen {
            assert_eq!(
                mspec.supply_source, None,
                "production config '{name}' must ship supply_source=None (M7-d golden-NEUTRAL gate)"
            );
        }
    }
}
