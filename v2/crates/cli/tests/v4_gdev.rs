//! V-4 (#276): evolvable developmental grid (body-size axis) — the config-level gate that keeps
//! every production config's golden byte-identical. `Genome::mutate` only draws from `SALT_GDEV`
//! when `evolve_body_size==true` (see `genome.rs` `mutate()` unit teeth for the draw-level proof).
//! This file proves the OTHER half: every production config except `driver_config` actually leaves
//! `evolve_body_size` at its `false` default, so `v2_golden_drift` (`golden.rs`) and the five
//! `v2_golden_conserved*` pins (`golden_conserved.rs`) never see a g_dev draw and stay untouched by
//! V-4. Only `driver_config` opts in (and re-pins `v2_golden_conserved_driver` accordingly).

use cli::{cprime_config, default_config, differentiation_config, dprime_config, driver_config, l3_config, phase2_config};

const SEED: u64 = 0xA11A_2A11;

/// Teeth (6): `evolve_body_size` is `false` on every pinned/production config except
/// `driver_config` — the gate that protects `phase2`/`differentiation`/the five conserved goldens
/// from ever drawing a `g_dev` mutation, so their trajectories stay byte-identical to pre-V-4.
#[test]
fn v4_gate_off_goldens_byte_identical() {
    assert!(!default_config(SEED).econ.evolve_body_size, "default_config must keep evolve_body_size=false");
    assert!(!l3_config(SEED).econ.evolve_body_size, "l3_config must keep evolve_body_size=false");
    assert!(!cprime_config(SEED).econ.evolve_body_size, "cprime_config must keep evolve_body_size=false");
    assert!(!dprime_config(SEED).econ.evolve_body_size, "dprime_config must keep evolve_body_size=false");
    assert!(!phase2_config(SEED).econ.evolve_body_size, "phase2_config must keep evolve_body_size=false");
    assert!(!differentiation_config(SEED).econ.evolve_body_size, "differentiation_config must keep evolve_body_size=false");

    assert!(driver_config(SEED).econ.evolve_body_size, "driver_config must opt IN to evolve_body_size=true");
    assert_eq!(driver_config(SEED).econ.morphogen.expect("driver_config must carry a morphogen spec").g_dev, 1,
        "driver_config's founder must start unicellular (g_dev=1) so multicellularity evolves");
}
