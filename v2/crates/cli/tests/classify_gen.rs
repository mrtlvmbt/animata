//! classify-gen golden-NEUTRAL guard (#247): every shipped production config keeps
//! `GrnSpec::classify_nway: false` (or carries no `GrnSpec` at all). `classify`'s generalized
//! N-way path (sim-core `grn.rs`) is gated by that field — `false` reproduces the EXACT pre-#247
//! argmax-of-2 (A/B/Mixed), byte-identical, so the 6 production goldens (`golden.rs` + the 5
//! `golden_conserved.rs` configs) stay byte-identical, un-re-pinned. The real byte-identity proof
//! is those existing golden tests staying green; this is the structural belt-and-braces guard that
//! the gate is never accidentally armed in a shipped config. Mirrors `m7d_supply_gate.rs`'s
//! `m7d_prod_inert_all_goldens` but asserts the NEW `classify_nway` field.

use cli::{
    cprime_config, default_config, differentiation_config, dprime_config, l3_config, phase2_config,
};
use sim_core::SimConfig;

#[test]
fn classifygen_prod_inert_all_goldens() {
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
        if let Some(gspec) = &cfg.econ.grn {
            assert!(
                !gspec.classify_nway,
                "production config '{name}' must ship classify_nway=false (#247 golden-NEUTRAL gate)"
            );
        }
    }
}
