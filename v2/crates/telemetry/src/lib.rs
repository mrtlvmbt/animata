//! `telemetry` — read-only evolution statistics derived from the `sim-core` Observe samples (R25).
//!
//! This crate is an OBSERVER: it depends on `sim-core` but `sim-core` never depends on it, so the
//! statistics here can NEVER influence the tick (doc 12 §1). It is therefore free to use float — the
//! Price covariance is reported, never fed back into the deterministic state/hash.

use sim_core::TraitSample;

pub const TRAIT_NAMES: [&str; 8] = [
    "metabolism_eff", "move_speed", "sense_range", "size", "repro_threshold", "mutation_rate",
    "uptake_layer", "excrete_layer", // B-2 slots 6–7
];

/// Metabolic guild of an organism. Derived from the `uptake_layer` trait (slot 6 of `TraitSample`).
///
/// The two active guilds at Phase-1 L=2 are `Producer` (eats abiotic substrate, `uptake_layer=0`)
/// and `Consumer` (eats organics/excreta, `uptake_layer≥1`). `Reducer` and `Phototroph` have no
/// organisms yet (they arrive with C′/D′) but exist as variants NOW so the schema never shifts later.
///
/// Fixed `#[repr(u8)]` so `guild as usize` is a stable array index and the serialisation schema is
/// invariant to future variant additions (append-only; never reorder).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Guild {
    Producer = 0,
    Consumer = 1,
    Reducer = 2,
    Phototroph = 3,
}

impl Guild {
    /// All guild variants in definition order. Used to generate CSV headers AND data rows from a
    /// single source of truth — iterating this array for both keeps column count invariant to which
    /// guilds are currently populated.
    pub const ALL: [Guild; 4] =
        [Guild::Producer, Guild::Consumer, Guild::Reducer, Guild::Phototroph];

    /// Classify an organism by its metabolic guild. Uses `uptake_layer` (trait slot 6) as the
    /// primary discriminant. `detritus_layer` enables the Reducer class (C′-2): an organism
    /// with `uptake_layer == detritus_layer` is a Reducer regardless of the layer value. Pass
    /// `None` for configs without a detritus layer — preserves the original Producer/Consumer
    /// split byte-identically (`default_config`, `l3_config`).
    pub const fn classify(s: &TraitSample, detritus_layer: Option<usize>) -> Guild {
        if let Some(dl) = detritus_layer {
            if s.traits[6] as usize == dl {
                return Guild::Reducer;
            }
        }
        if s.traits[6] == 0 { Guild::Producer } else { Guild::Consumer }
    }

    /// Lowercase ASCII name, used in CSV column headers.
    pub const fn name(self) -> &'static str {
        match self {
            Guild::Producer => "producer",
            Guild::Consumer => "consumer",
            Guild::Reducer => "reducer",
            Guild::Phototroph => "phototroph",
        }
    }
}

/// One tick's evolution snapshot.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub population: usize,
    /// Mean of each of the 8 traits (6 Ф0 + 2 B-2 layer traits).
    pub means: [f64; 8],
    /// **Price covariance** cov(trait, offspring) per trait — the per-tick strength & direction of
    /// selection (≠ 0 ⇒ directional selection on that trait).
    pub price_cov: [f64; 8],
    /// Diversity proxy: total trait variance across the population (labelled `trait_var_diversity`
    /// in CSV output to distinguish it from the index-based Shannon/Simpson fields below).
    pub diversity: f64,
    /// Per-guild live population count, indexed by `Guild as usize`. Absent guild → 0 (never a
    /// missing slot). Counts sum exactly to `population`.
    pub guild_pop: [usize; 4],
    /// Per-guild trait means (8 traits × 4 guilds), indexed `[Guild as usize][trait_slot]`. Zero
    /// for guilds with no organisms.
    pub guild_means: [[f64; 8]; 4],
    /// Shannon entropy H = −Σ p_i·ln(p_i) computed from per-species live counts. 0 for a single
    /// species; rises with evenness. Sourced from `Telemetry.species_census`.
    pub shannon: f64,
    /// Simpson diversity index 1 − Σ p_i² ∈ [0, 1]. 0 for a monopoly; approaches 1 as diversity
    /// rises. Sourced from the same `species_census` as `shannon`.
    pub simpson: f64,
}

/// Compute the per-tick [`Report`] from Observe samples **and** the per-species census.
///
/// `species_census` is `Telemetry.species_census` from `sim-core` (`Vec<(species_id, count)>`).
/// Pass an empty slice when no census is available — Shannon/Simpson will then be 0.0.
/// `detritus_layer` enables Reducer classification (C′-2): pass `econ.detritus_layer` from the
/// running sim. Pass `None` for configs without a detritus layer — preserves existing behaviour.
pub fn compute_with_census(
    samples: &[TraitSample],
    species_census: &[(u32, u32)],
    detritus_layer: Option<usize>,
) -> Report {
    let n = samples.len();
    let nf = n as f64;

    // ── Trait means, Price covariance, trait-variance diversity ─────────────────────────────────
    let (means, price_cov, diversity) = if n == 0 {
        ([0.0f64; 8], [0.0f64; 8], 0.0f64)
    } else {
        let mut means = [0.0f64; 8];
        let mut off_mean = 0.0f64;
        for s in samples {
            for (t, m) in means.iter_mut().enumerate() {
                *m += s.traits[t] as f64;
            }
            off_mean += s.offspring as f64;
        }
        for m in &mut means {
            *m /= nf;
        }
        off_mean /= nf;

        let mut price_cov = [0.0f64; 8];
        let mut diversity = 0.0f64;
        for s in samples {
            for (t, c) in price_cov.iter_mut().enumerate() {
                let dt = s.traits[t] as f64 - means[t];
                *c += dt * (s.offspring as f64 - off_mean);
                diversity += dt * dt;
            }
        }
        for c in &mut price_cov {
            *c /= nf;
        }
        diversity /= nf;
        (means, price_cov, diversity)
    };

    // ── Guild census + per-guild trait means ─────────────────────────────────────────────────────
    let mut guild_pop = [0usize; 4];
    let mut guild_trait_sum = [[0f64; 8]; 4];
    for s in samples {
        let g = Guild::classify(s, detritus_layer) as usize;
        guild_pop[g] += 1;
        for (t, v) in guild_trait_sum[g].iter_mut().enumerate() {
            *v += s.traits[t] as f64;
        }
    }
    let mut guild_means = [[0.0f64; 8]; 4];
    for g in 0..4 {
        if guild_pop[g] > 0 {
            let gn = guild_pop[g] as f64;
            for t in 0..8 {
                guild_means[g][t] = guild_trait_sum[g][t] / gn;
            }
        }
    }

    // ── Shannon entropy + Simpson index from species_census ───────────────────────────────────────
    let (shannon, simpson) = diversity_indices(species_census);

    Report { population: n, means, price_cov, diversity, guild_pop, guild_means, shannon, simpson }
}

/// Backward-compatible shim: compute without a species census. Shannon/Simpson will be 0.0.
/// Passes `detritus_layer=None` — correct for all non-cprime configs (`default_config`, etc.).
pub fn compute(samples: &[TraitSample]) -> Report {
    compute_with_census(samples, &[], None)
}

/// Shannon H and Simpson (1 − Σ p_i²) from per-species counts.
/// Returns `(0.0, 0.0)` for an empty or all-zero census — defined, never NaN/panic.
fn diversity_indices(census: &[(u32, u32)]) -> (f64, f64) {
    let total: u64 = census.iter().map(|(_, c)| *c as u64).sum();
    if total == 0 {
        return (0.0, 0.0);
    }
    let n = total as f64;
    let mut shannon = 0.0f64;
    let mut sum_sq = 0.0f64;
    for &(_, count) in census {
        if count == 0 {
            continue;
        }
        let p = count as f64 / n;
        shannon -= p * p.ln();
        sum_sq += p * p;
    }
    (shannon, 1.0 - sum_sq)
}

/// CSV column header fragment for the guild census, generated from `Guild::ALL`.
/// Use alongside `guild_csv_row` to keep the header/row schema in lockstep.
pub fn guild_csv_header() -> String {
    Guild::ALL.iter().map(|g| format!("{}_pop", g.name())).collect::<Vec<_>>().join(",")
}

/// CSV data row fragment for the guild census, indexed from `Guild::ALL`.
/// Column order exactly matches `guild_csv_header()`.
pub fn guild_csv_row(rep: &Report) -> String {
    Guild::ALL.iter().map(|g| rep.guild_pop[*g as usize].to_string()).collect::<Vec<_>>().join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(traits: [i32; 8], offspring: u32) -> TraitSample {
        TraitSample { traits, offspring }
    }

    // ── Guild classifier ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn classifier_layer_0_is_producer() {
        let sample = s([200, 1, 1, 2, 1500, 32, 0, 1], 0);
        assert_eq!(Guild::classify(&sample, None), Guild::Producer);
    }

    #[test]
    fn classifier_layer_1_is_consumer() {
        let sample = s([200, 1, 1, 2, 1500, 32, 1, 0], 0);
        assert_eq!(Guild::classify(&sample, None), Guild::Consumer);
    }

    #[test]
    fn classifier_higher_layer_is_consumer() {
        let sample = s([200, 1, 1, 2, 1500, 32, 3, 1], 0);
        assert_eq!(Guild::classify(&sample, None), Guild::Consumer);
    }

    #[test]
    fn classifier_reducer_when_detritus_layer_matches() {
        // C′-2: uptake_layer == detritus_layer → Reducer (regardless of layer value).
        let reducer = s([200, 1, 1, 2, 1500, 32, 2, 0], 0); // uptake_layer=2 = detritus_layer
        assert_eq!(Guild::classify(&reducer, Some(2)), Guild::Reducer);
        // Other layers stay Producer/Consumer.
        let producer = s([200, 1, 1, 2, 1500, 32, 0, 1], 0);
        assert_eq!(Guild::classify(&producer, Some(2)), Guild::Producer);
        let consumer = s([200, 1, 1, 2, 1500, 32, 1, 0], 0);
        assert_eq!(Guild::classify(&consumer, Some(2)), Guild::Consumer);
        // detritus_layer=None: layer 2 is Consumer (old behaviour preserved).
        assert_eq!(Guild::classify(&reducer, None), Guild::Consumer);
    }

    // ── Guild census ─────────────────────────────────────────────────────────────────────────────

    #[test]
    fn guild_census_sums_to_population() {
        let samples = vec![
            s([200, 1, 1, 2, 1500, 32, 0, 1], 0), // producer
            s([200, 1, 1, 2, 1500, 32, 0, 1], 1), // producer
            s([200, 1, 1, 2, 1500, 32, 1, 0], 0), // consumer
        ];
        let rep = compute_with_census(&samples, &[], None);
        let sum: usize = rep.guild_pop.iter().sum();
        assert_eq!(sum, rep.population, "guild_pop must sum to population");
        assert_eq!(rep.guild_pop[Guild::Producer as usize], 2);
        assert_eq!(rep.guild_pop[Guild::Consumer as usize], 1);
        assert_eq!(rep.guild_pop[Guild::Reducer as usize], 0);
        assert_eq!(rep.guild_pop[Guild::Phototroph as usize], 0);
    }

    #[test]
    fn guild_census_empty_pop_is_defined() {
        let rep = compute_with_census(&[], &[], None);
        assert_eq!(rep.guild_pop, [0; 4]);
        assert_eq!(rep.guild_means, [[0.0; 8]; 4]);
        assert_eq!(rep.population, 0);
    }

    // ── Shannon / Simpson ────────────────────────────────────────────────────────────────────────

    #[test]
    fn shannon_single_species_is_zero() {
        let census = [(1u32, 10u32)];
        let (h, _) = diversity_indices(&census);
        assert!(h.abs() < 1e-12, "Shannon must be 0 for a single species, got {h}");
    }

    #[test]
    fn shannon_rises_with_evenness() {
        let unequal = [(1u32, 9u32), (2u32, 1u32)];
        let equal = [(1u32, 5u32), (2u32, 5u32)];
        let (h_unequal, _) = diversity_indices(&unequal);
        let (h_equal, _) = diversity_indices(&equal);
        assert!(
            h_equal > h_unequal,
            "equal split must have higher Shannon than skewed: {h_equal} vs {h_unequal}"
        );
    }

    #[test]
    fn simpson_in_unit_interval() {
        let census = [(1u32, 3u32), (2u32, 7u32)];
        let (_, d) = diversity_indices(&census);
        assert!(d >= 0.0 && d <= 1.0, "Simpson must be in [0,1], got {d}");
    }

    #[test]
    fn simpson_single_species_is_zero() {
        let census = [(1u32, 100u32)];
        let (_, d) = diversity_indices(&census);
        assert!(d.abs() < 1e-12, "Simpson monopoly must be 0, got {d}");
    }

    #[test]
    fn empty_census_is_zero_not_nan() {
        let (h, d) = diversity_indices(&[]);
        assert_eq!(h, 0.0);
        assert_eq!(d, 0.0);
        assert!(!h.is_nan() && !d.is_nan());
    }

    #[test]
    fn zero_count_census_is_defined() {
        let census = [(1u32, 0u32), (2u32, 0u32)];
        let (h, d) = diversity_indices(&census);
        assert_eq!(h, 0.0);
        assert_eq!(d, 0.0);
    }

    // ── Existing tests ────────────────────────────────────────────────────────────────────────────

    #[test]
    fn price_covariance_detects_directional_selection() {
        let samples = vec![
            s([200, 1, 1, 2, 1500, 32, 0, 1], 0),
            s([200, 1, 1, 4, 1500, 32, 0, 1], 0),
            s([200, 1, 1, 8, 1500, 32, 0, 1], 1),
            s([200, 1, 1, 10, 1500, 32, 0, 1], 1),
        ];
        let r = compute(&samples);
        assert!(r.price_cov[3] > 0.0, "size↔repro covariance must be positive: {}", r.price_cov[3]);
        assert!(r.price_cov[0].abs() < 1e-9);
        assert_eq!(r.population, 4);
    }

    #[test]
    fn empty_is_zero() {
        let r = compute(&[]);
        assert_eq!(r.population, 0);
        assert_eq!(r.price_cov, [0.0; 8]);
        assert_eq!(r.shannon, 0.0);
        assert_eq!(r.simpson, 0.0);
    }

    // ── guild_csv_header / guild_csv_row lockstep ────────────────────────────────────────────────

    #[test]
    fn guild_csv_header_and_row_same_column_count() {
        let rep = compute(&[]);
        let hcols: usize = guild_csv_header().split(',').count();
        let rcols: usize = guild_csv_row(&rep).split(',').count();
        assert_eq!(hcols, rcols, "guild_csv_header and guild_csv_row must have equal column count");
        assert_eq!(hcols, Guild::ALL.len());
    }
}
