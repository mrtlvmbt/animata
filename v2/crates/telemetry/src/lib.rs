//! `telemetry` — read-only evolution statistics derived from the `sim-core` Observe samples (R25).
//!
//! This crate is an OBSERVER: it depends on `sim-core` but `sim-core` never depends on it, so the
//! statistics here can NEVER influence the tick (doc 12 §1). It is therefore free to use float — the
//! Price covariance is reported, never fed back into the deterministic state/hash.

use sim_core::TraitSample;

pub const TRAIT_NAMES: [&str; 6] =
    ["metabolism_eff", "move_speed", "sense_range", "size", "repro_threshold", "mutation_rate"];

/// One tick's evolution snapshot.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub population: usize,
    /// Mean of each of the 6 traits.
    pub means: [f64; 6],
    /// **Price covariance** cov(trait, offspring) per trait — the per-tick strength & direction of
    /// selection (≠ 0 ⇒ directional selection on that trait).
    pub price_cov: [f64; 6],
    /// Diversity proxy: total trait variance across the population.
    pub diversity: f64,
}

/// Compute the per-tick [`Report`] from the Observe samples.
pub fn compute(samples: &[TraitSample]) -> Report {
    let n = samples.len();
    if n == 0 {
        return Report::default();
    }
    let nf = n as f64;

    let mut means = [0.0f64; 6];
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

    let mut price_cov = [0.0f64; 6];
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

    Report { population: n, means, price_cov, diversity }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(traits: [i32; 6], offspring: u32) -> TraitSample {
        TraitSample { traits, offspring }
    }

    #[test]
    fn price_covariance_detects_directional_selection() {
        // Larger `size` (index 3) reproduces; smaller does not → positive Price covariance on size.
        let samples = vec![
            s([200, 1, 1, 2, 1500, 32], 0),
            s([200, 1, 1, 4, 1500, 32], 0),
            s([200, 1, 1, 8, 1500, 32], 1),
            s([200, 1, 1, 10, 1500, 32], 1),
        ];
        let r = compute(&samples);
        assert!(r.price_cov[3] > 0.0, "size↔repro covariance must be positive: {}", r.price_cov[3]);
        // A trait that does not co-vary with reproduction has ~zero covariance.
        assert!(r.price_cov[0].abs() < 1e-9);
        assert_eq!(r.population, 4);
    }

    #[test]
    fn empty_is_zero() {
        let r = compute(&[]);
        assert_eq!(r.population, 0);
        assert_eq!(r.price_cov, [0.0; 6]);
    }
}
