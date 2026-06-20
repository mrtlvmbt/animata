use super::*;
use crate::genome::Genome;
use crate::rng::Rng;
use crate::sim::Stratum;
use crate::sim_config::Params;

fn sample_with<'a>(layer: Stratum, temperature: f32, light: f32, genome: &'a Genome, pheno: &'a Phenotype) -> Sample<'a> {
    Sample { pheno, genome, layer, temperature, light, autotroph_shading: 1.0 }
}

fn climate() -> climate::Climate {
    climate::Climate { thermal_penalty: Params::default().thermal_penalty }
}
fn autotrophy() -> autotrophy::Autotrophy {
    autotrophy::Autotrophy { photo_rate: Params::default().photo_rate }
}
fn metabolism() -> metabolism::Metabolism {
    let p = Params::default();
    metabolism::Metabolism { air: p.air_metab_mult, underground: p.underground_metab_mult }
}

/// A pressure writes ONLY its own channel; everything else stays identity.
#[test]
fn each_pressure_touches_one_channel() {
    let mut rng = Rng::new(1);
    let genome = Genome::founder(&mut rng); // no photo cells, thermal_pref set
    let pheno = genome.develop();

    // Climate → food_mult only (≠1 when temp far from pref), energy_add/metab identity.
    let s = sample_with(Stratum::Surface, 0.0, 1.0, &genome, &pheno);
    let e = climate().eval(&s);
    assert!(e.energy_add == 0.0 && e.metab_mult == 1.0);

    // Metabolism → metab_mult only; Air is dearer than Surface.
    let air = metabolism().eval(&sample_with(Stratum::Air, 0.0, 1.0, &genome, &pheno));
    let surf = metabolism().eval(&sample_with(Stratum::Surface, 0.0, 1.0, &genome, &pheno));
    assert!(air.metab_mult > surf.metab_mult);
    assert!(air.food_mult == 1.0 && air.energy_add == 0.0);

    // Autotrophy → energy_add only; identity for a heterotroph (no photo cells).
    let a = autotrophy().eval(&s);
    assert_eq!(a.energy_add, 0.0);
    assert!(a.food_mult == 1.0 && a.metab_mult == 1.0);
}

/// Composition with identities reproduces a single contributor bit-for-bit (the F4 invariant).
#[test]
fn compose_is_identity_preserving_bitexact() {
    let mut rng = Rng::new(7);
    let genome = Genome::founder(&mut rng);
    let pheno = genome.develop();
    let s = sample_with(Stratum::Air, 0.0, 1.0, &genome, &pheno);

    let reg = PressureRegistry::default();
    let composed = reg.eval_all(&s);
    // The composed channels must equal the individual contributors exactly.
    assert_eq!(composed.food_mult, climate().eval(&s).food_mult);
    assert_eq!(composed.energy_add, autotrophy().eval(&s).energy_add);
    assert_eq!(composed.metab_mult, metabolism().eval(&s).metab_mult);
}

/// The registry exposes its membership (extensibility surface).
#[test]
fn default_registry_lists_pressures() {
    let ids: Vec<_> = PressureRegistry::default().ids().collect();
    assert!(ids.contains(&"climate"));
    assert!(ids.contains(&"autotrophy"));
    assert!(ids.contains(&"metabolism"));
}
