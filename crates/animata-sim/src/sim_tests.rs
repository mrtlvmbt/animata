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

/// Save/load is verified by the determinism lock itself: a full-state snapshot, round-tripped
/// through bytes and restored onto a regenerated world, must reproduce the exact `state_checksum`
/// — AND continue bit-identically. (Geometry is regenerated from the seed; only the overlay +
/// creatures + tick are carried.)
#[test]
fn snapshot_round_trips_bit_identical() {
    use crate::persist::Snapshot;
    // Run a world to a non-trivial state (creatures bred, vegetation grazed, nutrient moved).
    let mut t = world();
    let mut s = Sim::new(42, &t);
    for tick in 0..250 {
        s.step(&mut t, tick);
    }
    let csum_before = state_checksum(&s, &t);

    // Capture → serialise → deserialise → restore onto a freshly regenerated terrain.
    let snap = Snapshot::new(t.seed, 250, s.to_state(), t.clone_state());
    let mut bytes = Vec::new();
    snap.write(&mut bytes).expect("snapshot serialises");
    let restored = Snapshot::read(&bytes[..]).expect("snapshot deserialises");
    let mut t2 = VoxelTerrain::new(restored.terrain_seed);
    t2.set_state(restored.terrain).expect("overlay fits the regenerated terrain");
    let mut s2 = Sim::from_state(restored.sim);
    assert_eq!(state_checksum(&s2, &t2), csum_before, "restored state must equal the saved state");

    // And the resumed run must stay bit-identical to the original continuing past the save point.
    for tick in 250..300 {
        s.step(&mut t, tick);
        s2.step(&mut t2, tick);
    }
    assert_eq!(
        state_checksum(&s, &t),
        state_checksum(&s2, &t2),
        "a loaded world must continue bit-identically to the one it was saved from"
    );
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

/// Morphogenesis PR-C acceptance: ORGANS emerge. Founders (single cells) have none, but as bodies
/// evolve a real fraction develop a coherent organ — a connected same-type cluster ≥ `ORGAN_MIN` —
/// because a coherent organ out-performs the same cells scattered (`organ_power` drives speed/energy).
/// Population stays healthy. Single seed ⇒ deterministic.
#[test]
fn organs_emerge_under_selection() {
    let mut t = world();
    let mut s = Sim::new(1, &t);
    assert_eq!(s.frac_with_organ(), 0.0, "founders (single cells) must have no organs");
    for tick in 0..8000 {
        s.step(&mut t, tick);
    }
    let frac = s.frac_with_organ();
    eprintln!(
        "after 8000 ticks: {:.1}% carry a coherent organ, avg_biomass {:.2}, pop {}",
        frac * 100.0,
        s.avg_biomass(),
        s.population()
    );
    assert!(frac > 0.05, "no organs emerged ({:.1}%)", frac * 100.0);
    assert!(s.population() > 100 && s.population() < SIM_POP_CAP, "population unhealthy: {}", s.population());
}

/// Sensor cells now DO something (they were the one trait with no mechanical effect): a body's
/// sensing reach scales with its sensor ORGAN power. This pins the mechanism — floor at no-sensor
/// (no nerf), strictly monotone in sensor organ-power, a coherent organ beating scattered cells,
/// and capped so the grid query stays local. (Emergence of the cells themselves is the same
/// developmental machinery covered by `organs_emerge_under_selection`.)
#[test]
fn sensor_organ_extends_sensing_reach() {
    // A creature carrying a chosen phenotype (only `sense_mult` reads it here; sensor = type idx 2).
    let make = |sensor: u32, organ: u8| {
        let mut rng = Rng::new(1);
        let mut o = [0u8; 7];
        o[2] = organ;
        Creature {
            id: 0,
            founder: 0,
            pos: vec2(0.0, 0.0),
            heading: 0.0,
            energy: 0.0,
            age: 0,
            alive: true,
            genome: Genome::founder(&mut rng),
            pheno: Phenotype { n_cells: sensor.max(1), sensor, organ: o, ..Default::default() },
        }
    };
    // No sensor tissue ⇒ baseline reach, bit-identical to before this trait was wired.
    assert_eq!(make(0, 0).sense_mult(), SENSE_FLOOR);
    // More sensor organ-power ⇒ strictly farther reach, and a coherent organ beats scattered cells.
    assert!(make(4, 0).sense_mult() > make(0, 0).sense_mult(), "sensor cells must extend reach");
    assert!(
        make(4, 4).sense_mult() > make(4, 1).sense_mult(),
        "a coherent sensor organ must out-reach the same cells scattered"
    );
    // Capped so even an extreme body keeps the per-tick spatial-grid query local.
    assert_eq!(make(32, 32).sense_mult(), SENSE_CAP);
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
    // Crypsis is bounded by predation INTENSITY (predators are a ~2% mortality source — a correct
    // trophic pyramid), so the global signal is modest but clearly positive: prey coloration tracks
    // the local ground where predation presses. With the toxicity pressure now adding a competing
    // abiotic mortality source, predation is a smaller slice of total deaths, so the crypsis signal
    // is diluted further but stays clearly emergent (≈0.08 vs the ≈0.14 of the pre-toxicity world).
    assert!(end > 0.06, "no crypsis emerged — coloration didn't track background ({end:.3})");
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

/// Toxicity acceptance: under the new abiotic pressure, lineages on toxic ground evolve higher
/// `toxin_resistance` — the resistance↔local-toxicity correlation rises well above its ~0 founder
/// value (allopatric sorting on a non-thermal axis), and the population survives the filter.
#[test]
fn toxin_resistance_evolves_on_toxic_ground() {
    let mut t = world();
    let mut s = Sim::new(1, &t);
    let start = s.toxin_correlation(&t);
    for tick in 0..8000 {
        s.step(&mut t, tick);
    }
    let end = s.toxin_correlation(&t);
    eprintln!("toxin correlation: start {start:.3} → end {end:.3}, pop {}", s.population());
    assert!(start.abs() < 0.1, "founders should be toxin-random (corr {start:.3})");
    assert!(end > 0.1, "no toxic adaptation emerged — resistance didn't track toxicity ({end:.3})");
    assert!(s.population() > 100 && s.population() < SIM_POP_CAP, "population unhealthy: {}", s.population());
}

/// Seasonality acceptance: the seasonal food swing drives the ecosystem's ENERGY economy — average
/// creature energy rises in summer and falls in winter, year after year. Measured on `avg_energy`
/// (not population): energy tracks food income DIRECTLY, so the signal is robust to how the
/// population is regulated (near `SOFT_CAP` the birth gate, not food, sets the headcount — so a
/// population-amplitude test would be drowned by that gate). A time-domain pressure, vs the spatial
/// ones. Phase-agnostic: compares each year's PEAK vs TROUGH `avg_energy`, so the population's lag
/// behind the food cycle doesn't matter.
#[test]
fn seasonality_drives_the_energy_economy() {
    use crate::sim_config::SimConfig;
    let mut cfg = SimConfig::default();
    cfg.features.seasonality = true;
    cfg.params.season_len = 80.0; // a short year (800 ticks) ⇒ several cycles fit the run
    cfg.params.season_amplitude = 0.5; // a clear swing
    let mut t = world();
    let mut s = Sim::with_config(1, &t, cfg);
    // Collect (season angle, average energy) over the steady state, then measure the MAGNITUDE of
    // energy's seasonal component = √(corr_sin² + corr_cos²). avg_energy is a leaky integrator of
    // (food − metabolism), so its response is phase-SHIFTED from the food cycle (∫sin ≈ −cos);
    // correlating against sin alone would miss it. The sin+cos magnitude is phase-agnostic AND
    // amplitude-robust — it just asks "does energy carry the seasonal frequency, consistently?".
    // Off (aseasonal) there is no seasonal component ⇒ ≈0.
    let (mut sines, mut coses, mut energy) = (Vec::new(), Vec::new(), Vec::new());
    for tick in 0..5600u64 {
        s.step(&mut t, tick);
        if tick >= 1600 && tick % 5 == 0 {
            // skip the 2-year transient; subsample
            let angle = std::f32::consts::TAU * tick as f32 * crate::config::TICK_LEN / 80.0;
            sines.push(angle.sin() as f64);
            coses.push(angle.cos() as f64);
            energy.push(s.avg_energy() as f64);
        }
    }
    let pearson = |a: &[f64], b: &[f64]| -> f64 {
        let n = a.len() as f64;
        let (ma, mb) = (a.iter().sum::<f64>() / n, b.iter().sum::<f64>() / n);
        let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
        for (x, y) in a.iter().zip(b) {
            cov += (x - ma) * (y - mb);
            va += (x - ma).powi(2);
            vb += (y - mb).powi(2);
        }
        cov / (va.sqrt() * vb.sqrt())
    };
    let r = (pearson(&sines, &energy).powi(2) + pearson(&coses, &energy).powi(2)).sqrt();
    eprintln!("avg-energy seasonal component magnitude: {r:.3} ({} samples)", energy.len());
    assert!(
        r > 0.3,
        "average energy should carry the seasonal cycle — rich summers, lean winters (R {r:.3})"
    );
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
