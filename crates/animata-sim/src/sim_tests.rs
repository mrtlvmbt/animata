use super::*;

fn world() -> VoxelTerrain {
    VoxelTerrain::new(1)
}

#[test]
fn column_index_clamps_out_of_world() {
    assert_eq!(column_index(vec2(-100.0, -100.0)), (0, 0));
    assert_eq!(column_index(vec2(1e9, 1e9)), (COLS - 1, ROWS - 1));
}

/// The genome's brain-weight count must match this module's brain topology.
#[test]
fn brain_weight_count_matches_topology() {
    assert_eq!(crate::genome::BRAIN_WEIGHTS, N_INPUTS * N_HIDDEN + N_HIDDEN * N_OUTPUTS);
}

/// A run is reproducible from the world seed: two sims stepped the same number of fixed
/// ticks have an identical population and identical leading creatures.
#[test]
fn deterministic_replay() {
    let (mut t1, mut t2) = (world(), world());
    let (mut a, mut b) = (Sim::new(42, &t1), Sim::new(42, &t2));
    for tick in 0..300 {
        a.step(&mut t1, tick);
        b.step(&mut t2, tick);
    }
    assert_eq!(a.population(), b.population());
    assert_eq!(a.births, b.births);
    assert_eq!(a.deaths, b.deaths);
    for (x, y) in a.creatures.iter().zip(b.creatures.iter()).take(50) {
        assert_eq!(x.id, y.id);
        assert_eq!(x.pos, y.pos);
        assert_eq!(x.energy, y.energy);
    }
}

/// The bit-exact determinism lock (F1): a full state-hash replays identically, and is caught
/// the instant any refactor perturbs the trajectory — far stronger than counts alone. Pins a
/// golden `u64` so divergence is detectable at the PR that introduces it.
#[test]
fn state_checksum_replays_to_golden() {
    let run = || {
        let mut t = world();
        let mut s = Sim::new(42, &t);
        for tick in 0..300 {
            s.step(&mut t, tick);
        }
        state_checksum(&s, &t)
    };
    let a = run();
    let b = run();
    assert_eq!(a, b, "state_checksum is not deterministic across runs");
    assert_eq!(a, GOLDEN_CHECKSUM_SEED42_300, "state diverged from golden (some change shifted the trajectory)");
}

/// The lock metric: over a headless run the herbivore population neither dies out nor pins
/// the cap — a living, self-limiting ecosystem on the new world. (Tuning target for C0.)
#[test]
fn population_stays_in_a_living_corridor() {
    for &seed in &[1u64, 2, 3] {
        let mut t = world();
        let mut s = Sim::new(seed, &t);
        for tick in 0..4000 {
            s.step(&mut t, tick);
        }
        let pop = s.population();
        eprintln!("seed {seed}: pop {pop}, avg_energy {:.1}, births {}, deaths {}", s.avg_energy(), s.births, s.deaths);
        assert!(pop > 100, "population collapsed for seed {seed}: {pop}");
        assert!(pop < SIM_POP_CAP, "population pinned the cap for seed {seed}: {pop}");
    }
}

/// C1 acceptance: under the size→longevity gradient, multicellularity EMERGES from the
/// empty-GRN founders (biomass climbs above 1, a real fraction of the population becomes
/// multicellular) — the developmental mechanism is exercised live, not just in unit tests —
/// while the population stays alive and below the cap. Single seed ⇒ deterministic, not flaky.
#[test]
fn multicellularity_emerges_under_selection() {
    let mut t = world();
    let mut s = Sim::new(1, &t);
    assert_eq!(s.avg_biomass(), 1.0, "founders must start unicellular (C0 continuity)");
    for tick in 0..5000 {
        s.step(&mut t, tick);
    }
    let (multi, _) = s.complexity_mix();
    let bm = s.avg_biomass();
    eprintln!("after 5000 ticks: pop {} avg_biomass {bm:.3} multi {:.1}%", s.population(), multi * 100.0);
    assert!(bm > 1.1, "multicellularity did not emerge (avg_biomass {bm:.3})");
    assert!(multi > 0.05, "too few multicellular creatures emerged ({:.1}%)", multi * 100.0);
    assert!(s.population() > 100 && s.population() < SIM_POP_CAP, "population unhealthy: {}", s.population());
}

/// C2 acceptance: a predatory second trophic level EMERGES — some creatures evolve predator
/// cells, hunt and kill prey — and predators stay RARER than prey (a trophic pyramid, the
/// ~10% rule), with the population staying alive. Single seed ⇒ deterministic.
#[test]
fn predation_emerges_as_a_trophic_level() {
    let mut t = world();
    let mut s = Sim::new(1, &t);
    for tick in 0..8000 {
        s.step(&mut t, tick);
    }
    let carn = s.frac_carnivore();
    eprintln!("after 8000 ticks: pop {} kills {} carnivore {:.1}%", s.population(), s.kills, carn * 100.0);
    assert!(s.kills > 1000, "no predation happened (kills {})", s.kills);
    assert!(carn > 0.003, "no predator niche persisted ({:.2}%)", carn * 100.0);
    assert!(carn < 0.5, "predators outnumber prey — inverted pyramid ({:.0}%)", carn * 100.0);
    assert!(s.population() > 100 && s.population() < SIM_POP_CAP, "population unhealthy: {}", s.population());
}

/// C3-habitats acceptance: lineages sort into the climate band they're adapted to —
/// the thermal-preference↔local-temperature correlation rises well above 0 (allopatry /
/// habitats), starting from ~0 (random founders). Single seed ⇒ deterministic.
#[test]
fn habitats_emerge_by_climate_adaptation() {
    let mut t = world();
    let mut s = Sim::new(1, &t);
    let start = s.thermal_correlation(&t);
    for tick in 0..6000 {
        s.step(&mut t, tick);
    }
    let end = s.thermal_correlation(&t);
    eprintln!("thermal correlation: start {start:.3} → end {end:.3}");
    assert!(start.abs() < 0.15, "founders should be climate-random (corr {start:.3})");
    assert!(end > 0.3, "no habitat sorting emerged (thermal corr {end:.3})");
}

/// C3-strata acceptance: the vertical niches get colonised — burrowers, fliers AND swimmers
/// each appear as a persistent minority alongside the surface majority (their morphology
/// evolves the flight/burrow/fin cells that grant access). Single seed ⇒ deterministic.
#[test]
fn vertical_strata_get_colonised() {
    let mut t = world();
    let mut s = Sim::new(1, &t);
    for tick in 0..7000 {
        s.step(&mut t, tick);
    }
    let m = s.stratum_mix(&t);
    eprintln!("strata: underground {:.1}% surface {:.1}% air {:.1}% water {:.1}%", m[0] * 100.0, m[1] * 100.0, m[2] * 100.0, m[3] * 100.0);
    assert!(m[0] > 0.01, "underground unoccupied ({:.2}%)", m[0] * 100.0);
    assert!(m[2] > 0.01, "air unoccupied ({:.2}%)", m[2] * 100.0);
    assert!(m[3] > 0.01, "water unoccupied ({:.2}%)", m[3] * 100.0);
    assert!(m[1] > 0.5, "surface should stay the majority ({:.1}%)", m[1] * 100.0);
}

/// C3-autotrophs acceptance: a photosynthetic producer tier emerges INSIDE the creature
/// substrate (a real fraction evolve photo cells and persist), without taking over — the
/// self-shading keeps it a niche. Single seed ⇒ deterministic.
#[test]
fn autotrophs_emerge_as_a_producer_niche() {
    let mut t = world();
    let mut s = Sim::new(1, &t);
    assert_eq!(s.frac_autotroph(), 0.0, "founders must be heterotrophs (no photo cells)");
    for tick in 0..7000 {
        s.step(&mut t, tick);
    }
    let auto = s.frac_autotroph();
    eprintln!("after 7000 ticks: autotrophs {:.1}% pop {}", auto * 100.0, s.population());
    assert!(auto > 0.05, "no autotroph niche emerged ({:.1}%)", auto * 100.0);
    assert!(auto < 0.9, "autotrophs took over — shading too weak ({:.0}%)", auto * 100.0);
    assert!(s.population() > 100 && s.population() < SIM_POP_CAP, "population unhealthy: {}", s.population());
}

/// C3-nutrient-cycle acceptance: the mineral pool stays BOUNDED and self-sustaining — it
/// neither drains to zero (grazing without return) nor pins the ceiling (death return without
/// loss). Inhabited ground is drawn DOWN from its baseline by grazing (the drain works), the
/// death-return + weathering keep it from collapsing, and the population stays healthy.
#[test]
fn nutrient_cycle_is_bounded_and_self_sustaining() {
    let mut t = world();
    let start = t.nutrient_at(COLS / 2, ROWS / 2, 0); // a baseline sample before any grazing
    let mut s = Sim::new(1, &t);
    for tick in 0..6000 {
        s.step(&mut t, tick);
    }
    let n = s.avg_nutrient(&t, 6000);
    eprintln!("nutrient: baseline≈{start:.2} → inhabited {n:.2}, pop {}", s.population());
    assert!(n > 0.05, "nutrient pool collapsed to zero ({n:.3}) — death return too weak");
    assert!(n < 0.95, "nutrient pinned the ceiling ({n:.3}) — drain too weak");
    assert!(n < start, "grazing did not draw inhabited nutrient below baseline ({n:.2} vs {start:.2})");
    assert!(s.population() > 100 && s.population() < SIM_POP_CAP, "population unhealthy: {}", s.population());
}

/// C3-camouflage acceptance: prey evolve coloration MATCHING their local ground (crypsis) —
/// the appearance↔background correlation rises well above 0, driven by the predator detection
/// channel. Founders are colour-random. Single seed ⇒ deterministic.
#[test]
fn camouflage_emerges_against_background() {
    let mut t = world();
    let mut s = Sim::new(1, &t);
    let start = s.crypsis_correlation(&t);
    for tick in 0..8000 {
        s.step(&mut t, tick);
    }
    let end = s.crypsis_correlation(&t);
    eprintln!("crypsis correlation: start {start:.3} → end {end:.3}");
    assert!(start.abs() < 0.1, "founders should be colour-random (corr {start:.3})");
    // Crypsis is bounded by predation INTENSITY (predators are a ~2% mortality source — a
    // correct trophic pyramid), so the global signal is modest but clearly positive: prey
    // coloration tracks the local ground where predation actually presses.
    assert!(end > 0.1, "no crypsis emerged — coloration didn't track background ({end:.3})");
}

/// C3-speciation acceptance: the population RADIATES — founders are one species (identical
/// founder body/genome class), and over time the leader-clustering resolves MANY species and
/// a broad niche coverage (multiple strata × diets × climates × complexity tiers occupied),
/// not a monoculture. Single seed ⇒ deterministic.
#[test]
fn population_radiates_into_many_species_and_niches() {
    let mut t = world();
    let mut s = Sim::new(1, &t);
    let s0 = s.species_count();
    for tick in 0..8000 {
        s.step(&mut t, tick);
    }
    let (sp, nc) = (s.species_count(), s.niche_coverage(&t));
    eprintln!("founders {s0} species → {sp} species, {nc} niches occupied");
    assert!(s0 <= 3, "founders should cluster into ~one species, got {s0}");
    assert!(sp > 20, "no radiation — too few species emerged ({sp})");
    assert!(nc > 6, "niche space barely occupied ({nc} niches)");
}

/// PR4: feature toggles actually bite, and a fixed config replays deterministically.
#[test]
fn feature_toggles_bite_and_replay() {
    use crate::sim_config::SimConfig;

    // Predation off ⇒ nothing is ever hunted, deterministically, and the population survives.
    let mut cfg = SimConfig::default();
    cfg.features.predation = false;
    let mut t = world();
    let mut s = Sim::with_config(7, &t, cfg);
    for tick in 0..2000 {
        s.step(&mut t, tick);
    }
    assert_eq!(s.kills, 0, "predation off but {} kills happened", s.kills);
    assert!(s.population() > 0, "population died out with predation off");

    // A toggle changes the trajectory: climate acts on every creature's food each tick, so turning
    // it off diverges from the golden run by tick 300 — yet each config is itself deterministic.
    let run = |climate: bool| {
        let mut cfg = SimConfig::default();
        cfg.features.climate = climate;
        let mut t = world();
        let mut s = Sim::with_config(42, &t, cfg);
        for tick in 0..300 {
            s.step(&mut t, tick);
        }
        state_checksum(&s, &t)
    };
    assert_eq!(run(true), GOLDEN_CHECKSUM_SEED42_300, "default-climate run must equal the golden");
    assert_ne!(run(false), GOLDEN_CHECKSUM_SEED42_300, "climate off must change the trajectory");
    assert_eq!(run(false), run(false), "a fixed config must replay deterministically");
}

/// Tuning aid (ignored): print the population trajectory for one seed so the energy
/// constants can be balanced into a food-limited corridor below the cap.
#[test]
#[ignore]
fn tune_trajectory() {
    let mut t = world();
    let mut s = Sim::new(1, &t);
    for tick in 0..12000 {
        s.step(&mut t, tick);
        if tick % 1000 == 0 {
            let (multi, _) = s.complexity_mix();
            eprintln!(
                "tick {tick}: pop {} bm {:.2} multi {:.0}% carniv {:.1}% auto {:.1}% species {} niches {} allop {:.2} crypsis {:.2}",
                s.population(), s.avg_biomass(), multi * 100.0, s.frac_carnivore() * 100.0,
                s.frac_autotroph() * 100.0, s.species_count(), s.niche_coverage(&t), s.thermal_correlation(&t), s.crypsis_correlation(&t)
            );
        }
    }
}
