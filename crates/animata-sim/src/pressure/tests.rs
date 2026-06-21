use super::*;
use crate::genome::Genome;
use crate::rng::Rng;
use crate::sim::Stratum;
use crate::sim_config::Params;

fn sample_with<'a>(layer: Stratum, temperature: f32, light: f32, genome: &'a Genome, pheno: &'a Phenotype) -> Sample<'a> {
    Sample { pheno, genome, layer, temperature, light, toxicity: 0.0, oxygen: 0.0, season_phase: 0.0, autotroph_shading: 1.0 }
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
    assert!(ids.contains(&"toxicity"));
    // Seasonality is default-OFF (an opt-in mode), so it must NOT be in the default registry.
    assert!(!ids.contains(&"seasonality"));
}

/// Seasonality swings `food_mult` around 1 with the phase: richer in summer (+1), leaner in winter
/// (−1), neutral at the equinox (0). Touches only the food channel.
#[test]
fn seasonality_swings_food_with_the_phase() {
    let mut rng = Rng::new(5);
    let genome = Genome::founder(&mut rng);
    let pheno = genome.develop();
    let season = seasonality::Seasonality { amplitude: 0.3 };
    let at = |phase: f32| {
        let mut s = sample_with(Stratum::Surface, 0.0, 1.0, &genome, &pheno);
        s.season_phase = phase;
        season.eval(&s)
    };
    assert_eq!(at(0.0).food_mult, 1.0, "equinox ⇒ neutral");
    assert!(at(1.0).food_mult > 1.0, "summer ⇒ richer");
    assert!(at(-1.0).food_mult < 1.0, "winter ⇒ leaner");
    assert!(at(1.0).energy_add == 0.0 && at(1.0).metab_mult == 1.0, "seasonality only touches food");
}

/// Toxicity writes `mortality_add` only when ground toxicity exceeds the creature's resistance.
#[test]
fn toxicity_hazard_scales_with_unresisted_excess() {
    let mut rng = Rng::new(3);
    let genome = Genome::founder(&mut rng); // toxin_resistance ∈ [0,1]
    let pheno = genome.develop();
    let tox = toxicity::Toxicity { lethality: Params::default().toxin_lethality };

    // Ground cleaner than the creature's resistance ⇒ no hazard, all channels identity.
    let mut clean = sample_with(Stratum::Surface, 0.0, 1.0, &genome, &pheno);
    clean.toxicity = (genome.toxin_resistance - 0.2).max(0.0);
    assert_eq!(tox.eval(&clean).mortality_add, 0.0);

    // Ground more toxic than resistance ⇒ a positive hazard, growing with the excess.
    let mut dirty = sample_with(Stratum::Surface, 0.0, 1.0, &genome, &pheno);
    dirty.toxicity = (genome.toxin_resistance + 0.5).min(1.0);
    let e = tox.eval(&dirty);
    assert!(e.mortality_add > 0.0, "toxic ground must add a death hazard");
    assert!(e.food_mult == 1.0 && e.energy_add == 0.0, "toxicity only touches mortality");
}
