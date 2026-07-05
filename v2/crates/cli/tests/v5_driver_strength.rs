//! V-5 (#278): driver strength — stronger, sweepable predation bite.
//!
//! Round-2 emergence returned NULL because `bite_shift=3` (1/8 of prey energy) was a minor,
//! recoverable tax — the D-1 size-refuge had no fitness gradient to exploit. V-5 lowers
//! `driver_config`'s default to `bite_shift=1` (half the prey's energy) and makes `bite_shift`
//! sweepable via `--set`. `predation_config` (P-2a, non-driver) keeps its original `bite_shift=3`
//! fixture value — untouched. Arch-independent integer invariants — run on BOTH CI jobs.

use cli::{
    apply_overrides, cprime_config, default_config, differentiation_config, driver_config,
    dprime_config, l3_config, phase2_config, predation_config, run, run_conserved_hashes,
};
use sim_core::{resolve_encounter, Genome, SimConfig};

const SEED: u64 = 0xBE_EF_5EED;

/// `v5_stronger_bite`: lowering `bite_shift` strictly increases the bite as a fraction of prey
/// energy (monotone), and R15 (conservation) holds at every point on the sweep — exercised over
/// the exact V-5 sweep values {3, 2, 1, 0} through `driver_config`'s own spec (refuge isolated out
/// so the effect measured is purely the bite strength, mirrors `d1_refuge_monotone`'s isolation
/// style in `predation.rs`).
#[test]
fn v5_stronger_bite() {
    let predator = Genome::founder(1);
    let prey_energy = 10_000i64;

    let mut spec = driver_config(SEED).econ.predation.expect("driver_config must configure predation");
    spec.size_refuge = None; // isolate the bite-strength effect from the refuge

    let losses: Vec<i64> = [3u32, 2, 1, 0]
        .iter()
        .map(|&bs| {
            spec.bite_shift = bs;
            let outcome = resolve_encounter(&predator, prey_energy, 1, &spec);
            assert_eq!(
                outcome.predator_gain + outcome.dissipated,
                outcome.prey_loss,
                "R15 broken at bite_shift={bs}: gain={} + dissipated={} != loss={}",
                outcome.predator_gain, outcome.dissipated, outcome.prey_loss
            );
            outcome.prey_loss
        })
        .collect();

    for w in losses.windows(2) {
        assert!(
            w[0] < w[1],
            "lowering bite_shift must strictly increase the bite: losses={:?} (sweep=[3,2,1,0])",
            losses
        );
    }
}

/// `v5_set_bite_shift`: `--set bite_shift=<v>` applies to `econ.predation.bite_shift`, range-guards
/// to `[0, 10]` (predation.rs's documented range), rejects when no `predation` is configured
/// (structural — mirrors `refuge_k`'s relationship to `size_refuge`), and leaves the trajectory
/// byte-identical when no override is passed.
#[test]
fn v5_set_bite_shift() {
    // Apply: valid values (including both documented bounds) update econ.predation.bite_shift.
    let mut econ = driver_config(SEED).econ;
    apply_overrides(&mut econ, &[("bite_shift".to_string(), "4".to_string())])
        .expect("bite_shift=4 must be accepted (in [0,10])");
    assert_eq!(econ.predation.unwrap().bite_shift, 4);

    apply_overrides(&mut econ, &[("bite_shift".to_string(), "0".to_string())])
        .expect("bite_shift=0 (documented lower bound) must be accepted");
    assert_eq!(econ.predation.unwrap().bite_shift, 0);

    apply_overrides(&mut econ, &[("bite_shift".to_string(), "10".to_string())])
        .expect("bite_shift=10 (documented upper bound) must be accepted");
    assert_eq!(econ.predation.unwrap().bite_shift, 10);

    // Range-guard: > 10 rejected.
    let mut econ_oob = driver_config(SEED).econ;
    let r = apply_overrides(&mut econ_oob, &[("bite_shift".to_string(), "11".to_string())]);
    assert!(r.is_err(), "bite_shift=11 must return Err (out of [0,10])");
    assert!(r.unwrap_err().starts_with("error:"));

    // Negative values are rejected by the u32 parse itself (no negative u32 representation).
    let r_neg = apply_overrides(&mut econ_oob, &[("bite_shift".to_string(), "-1".to_string())]);
    assert!(r_neg.is_err(), "bite_shift=-1 must return Err (u32 parse failure)");
    assert!(r_neg.unwrap_err().starts_with("error:"));

    // Rejected when no predation is configured (structural — mirrors refuge_k's guard).
    let mut econ_plain = default_config(SEED).econ;
    assert!(econ_plain.predation.is_none(), "default_config must not configure predation");
    let r_no_pred = apply_overrides(&mut econ_plain, &[("bite_shift".to_string(), "2".to_string())]);
    assert!(r_no_pred.is_err(), "bite_shift must be rejected when predation is None");
    assert!(r_no_pred.unwrap_err().starts_with("error:"));

    // No-flag byte-identical: empty override set leaves driver_config's own trajectory untouched.
    if !cfg!(debug_assertions) {
        const TICKS: u64 = 64;
        let baseline = run(driver_config(SEED), TICKS);
        let mut econ_empty = driver_config(SEED).econ;
        apply_overrides(&mut econ_empty, &[]).expect("empty override set is always Ok");
        let overridden = run(SimConfig { econ: econ_empty, ..driver_config(SEED) }, TICKS);
        assert_eq!(
            baseline, overridden,
            "empty --set must be byte-identical to driver_config's own trajectory"
        );
    }
}

/// `v5_non_driver_byte_identical` (#278): the non-driver production configs never configure
/// predation (default/l3/cprime/dprime/phase2/differentiation) or keep their own fixture value
/// (`predation_config`'s `bite_shift=3`) — V-5 only lowers `driver_config`'s own default and adds
/// an opt-in `--set` key, so none of these can shift. Proves it at the config level (the spec
/// values themselves) AND the trajectory level (a no-op `--set` reproduces the direct build, over
/// the arch-independent conserved-field hash — works on both CI jobs).
#[test]
fn v5_non_driver_byte_identical() {
    assert_eq!(
        predation_config(SEED).econ.predation.unwrap().bite_shift, 3,
        "predation_config (P-2a, non-driver) must keep its original bite_shift fixture value"
    );
    for (name, predation) in [
        ("default", default_config(SEED).econ.predation),
        ("l3", l3_config(SEED).econ.predation),
        ("cprime", cprime_config(SEED).econ.predation),
        ("dprime", dprime_config(SEED).econ.predation),
        ("phase2", phase2_config(SEED).econ.predation),
        ("differentiation", differentiation_config(SEED).econ.predation),
    ] {
        assert!(predation.is_none(), "{name}_config must not configure predation");
    }

    if cfg!(debug_assertions) {
        return;
    }
    const TICKS: u64 = 64;
    let configs: [(&str, SimConfig); 7] = [
        ("default", default_config(SEED)),
        ("l3", l3_config(SEED)),
        ("cprime", cprime_config(SEED)),
        ("dprime", dprime_config(SEED)),
        ("phase2", phase2_config(SEED)),
        ("differentiation", differentiation_config(SEED)),
        ("predation", predation_config(SEED)),
    ];
    for (name, cfg) in configs {
        let mut econ_overridden = cfg.econ.clone();
        apply_overrides(&mut econ_overridden, &[]).expect("empty override set is always Ok");
        let baseline = run_conserved_hashes(cfg.clone(), TICKS);
        let overridden = run_conserved_hashes(SimConfig { econ: econ_overridden, ..cfg }, TICKS);
        assert_eq!(
            baseline, overridden,
            "{name}_config's conserved trajectory shifted after a no-op --set apply — \
             the bite_shift whitelist addition leaked into a non-driver config"
        );
    }
}

/// `v5_predation_bites_harder_in_driver` (#278): `driver_config`'s new default (`bite_shift=1`)
/// drains strictly more prey energy per predation event than the OLD default (`bite_shift=3`,
/// still `predation_config`'s fixture value) — the V-5 fix is a measurable stronger pressure, not
/// just a smaller shift value.
#[test]
fn v5_predation_bites_harder_in_driver() {
    let predator = Genome::founder(1);
    let prey_energy = 10_000i64;

    let new_spec = driver_config(SEED).econ.predation.expect("driver_config must configure predation");
    assert_eq!(new_spec.bite_shift, 1, "driver_config must ship the V-5 bite_shift=1 default");

    let mut old_spec = new_spec;
    old_spec.bite_shift = 3; // the pre-V-5 default (still predation_config's fixture value)

    let new_loss = resolve_encounter(&predator, prey_energy, 1, &new_spec).prey_loss;
    let old_loss = resolve_encounter(&predator, prey_energy, 1, &old_spec).prey_loss;

    assert!(
        new_loss > old_loss,
        "V-5's bite_shift=1 must drain strictly more prey energy per encounter than the old \
         bite_shift=3: new_loss={new_loss} <= old_loss={old_loss}"
    );
}

/// `v5_determinism` (#278): `driver_config`'s new bite_shift=1 default replays bit-identically
/// under thread-count variation (1-vs-N), on the arch-independent conserved-field hash (works on
/// both CI jobs) — mirrors `r14.rs`'s generic sweep, specifically for `driver_config` (which
/// `r14.rs` itself doesn't cover).
#[test]
fn v5_determinism() {
    const TICKS: u64 = 160;
    const N: usize = 4;
    let one = run_conserved_hashes(SimConfig { sim_threads: 1, ..driver_config(SEED) }, TICKS);
    let many = run_conserved_hashes(SimConfig { sim_threads: N, ..driver_config(SEED) }, TICKS);
    for t in 0..TICKS as usize {
        assert_eq!(
            one[t], many[t],
            "driver_config conserved hash differs 1-vs-{N} at tick {t} (R14 broken after \
             V-5's bite_shift change)"
        );
    }
}
