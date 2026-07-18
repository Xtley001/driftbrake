//! Synthetic `(predicted, realized)` pair generation under the two
//! regimes described in `docs/BENCHMARK.md`'s "Methodology" section.

use driftbrake_core::{PredictedProfit, RealizedProfit};

use crate::rng::Rng;

/// One regime's noise/drift parameters.
#[derive(Debug, Clone, Copy)]
pub struct RegimeParams {
    /// Standard deviation of the zero-mean noise term `epsilon_i`,
    /// representing normal simulation-to-realization variance (venue
    /// slippage, minor timing effects) that isn't indicative of a
    /// strategy problem.
    pub noise_std_dev: f64,
    /// `None` for the healthy regime (`r_i = p_hat_i * (1 + epsilon_i)`).
    /// `Some(mu_drift)` for the drifted regime
    /// (`r_i = p_hat_i * (mu_drift + epsilon_i)`), where `mu_drift < 1`
    /// represents genuine, sustained underperformance.
    pub drift_mean: Option<f64>,
}

impl RegimeParams {
    pub fn healthy(noise_std_dev: f64) -> Self {
        Self {
            noise_std_dev,
            drift_mean: None,
        }
    }

    pub fn drifted(noise_std_dev: f64, drift_mean: f64) -> Self {
        Self {
            noise_std_dev,
            drift_mean: Some(drift_mean),
        }
    }

    fn center(&self) -> f64 {
        self.drift_mean.unwrap_or(1.0)
    }
}

/// A single synthetic `(predicted, realized)` pair sequence, generated at
/// a fixed `predicted` magnitude (the ratio is what matters for the
/// guards, not the absolute scale — see `docs/whitepaper.md` Section
/// 4.1).
pub fn generate_sequence(
    rng: &mut Rng,
    regime: RegimeParams,
    length: usize,
    predicted_magnitude: i128,
) -> Vec<(PredictedProfit, RealizedProfit)> {
    (0..length)
        .map(|_| {
            let ratio = rng.next_gaussian(regime.center(), regime.noise_std_dev);
            let predicted = PredictedProfit(predicted_magnitude);
            let realized = RealizedProfit((predicted_magnitude as f64 * ratio).round() as i128);
            (predicted, realized)
        })
        .collect()
}

/// Generate `count` independent sequences of `length` pairs each, under
/// the given regime. Each sequence gets its own slice of the RNG stream
/// (not independently seeded), so the whole batch is still fully
/// reproducible from one top-level seed.
pub fn generate_batch(
    rng: &mut Rng,
    regime: RegimeParams,
    count: usize,
    length: usize,
    predicted_magnitude: i128,
) -> Vec<Vec<(PredictedProfit, RealizedProfit)>> {
    (0..count)
        .map(|_| generate_sequence(rng, regime, length, predicted_magnitude))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healthy_regime_centers_ratios_near_one() {
        let mut rng = Rng::new(1);
        let regime = RegimeParams::healthy(0.05);
        let sequence = generate_sequence(&mut rng, regime, 5_000, 1_000_000);
        let mean_ratio: f64 = sequence
            .iter()
            .map(|(p, r)| r.0 as f64 / p.0 as f64)
            .sum::<f64>()
            / sequence.len() as f64;
        assert!(
            (mean_ratio - 1.0).abs() < 0.01,
            "mean ratio {mean_ratio} should be near 1.0"
        );
    }

    #[test]
    fn drifted_regime_centers_ratios_near_the_drift_mean() {
        let mut rng = Rng::new(2);
        let regime = RegimeParams::drifted(0.05, 0.6);
        let sequence = generate_sequence(&mut rng, regime, 5_000, 1_000_000);
        let mean_ratio: f64 = sequence
            .iter()
            .map(|(p, r)| r.0 as f64 / p.0 as f64)
            .sum::<f64>()
            / sequence.len() as f64;
        assert!(
            (mean_ratio - 0.6).abs() < 0.01,
            "mean ratio {mean_ratio} should be near 0.6"
        );
    }

    #[test]
    fn generate_batch_is_reproducible_from_the_same_seed() {
        let regime = RegimeParams::healthy(0.05);
        let mut rng_a = Rng::new(123);
        let batch_a = generate_batch(&mut rng_a, regime, 10, 20, 1_000_000);
        let mut rng_b = Rng::new(123);
        let batch_b = generate_batch(&mut rng_b, regime, 10, 20, 1_000_000);

        for (seq_a, seq_b) in batch_a.iter().zip(batch_b.iter()) {
            assert_eq!(seq_a, seq_b);
        }
    }

    #[test]
    fn all_predicted_profits_are_positive_so_no_pair_is_excluded_from_ratio_math() {
        // A synthetic pair whose PredictedProfit could be <= 0 would
        // silently be excluded from recent_ratios (Property 4), which
        // would corrupt the benchmark's own accounting of how many
        // pairs it generated vs how many the guard actually saw. Fixed
        // positive predicted_magnitude sidesteps this entirely, but this
        // test pins that assumption down explicitly.
        let mut rng = Rng::new(3);
        let regime = RegimeParams::drifted(0.2, 0.3);
        let sequence = generate_sequence(&mut rng, regime, 1_000, 1_000_000);
        assert!(sequence.iter().all(|(p, _)| p.0 > 0));
    }
}
