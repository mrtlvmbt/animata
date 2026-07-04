//! M7-c golden-NEUTRAL guard (#249): every shipped production config keeps `germ_threshold: None`
//! on its `MorphogenSpec` (or carries no morphogen spec at all). The germ/soma labeling pass added
//! to `CellGraph::from_gradient` (sim-core `genome.rs`) is gated by that field — `None` leaves
//! `module_is_germ` all-`false`, byte-identical to M7-b, so the 6 production goldens (`golden.rs` +
//! the 5 `golden_conserved.rs` configs) stay byte-identical, un-re-pinned. The real byte-identity
//! proof is those existing golden tests staying green; this is the structural belt-and-braces
//! guard that the gate is never accidentally armed in a shipped config. Mirrors
//! `m7b_apoptosis.rs`'s `m7b_prod_inert_all_goldens` but asserts the NEW `germ_threshold` field —
//! does not rely on the M7-b test, which only checks `apoptosis_threshold`.

use cli::{
    cprime_config, default_config, differentiation_config, dprime_config, l3_config, phase2_config,
};
use sim_core::SimConfig;

#[test]
fn m7c_prod_inert_all_goldens() {
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
                mspec.germ_threshold, None,
                "production config '{name}' must ship germ_threshold=None (M7-c golden-NEUTRAL gate)"
            );
        }
    }
}
