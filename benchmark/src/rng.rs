//! A small, deterministic PRNG — not a dependency on the `rand` crate.
//! `docs/BENCHMARK.md`'s whole point is a *reproducible* curve ("any
//! reviewer can regenerate it"); a fixed, self-contained generator with
//! an explicit seed is a better fit for that than pulling in an external
//! RNG crate's own default-algorithm churn across versions.
//!
//! Algorithm: SplitMix64 (Vigna & Steele) for the uniform stream, plus a
//! standard Box-Muller transform for approximately-normal noise. Neither
//! needs to be cryptographically strong — this only ever generates
//! synthetic benchmark data, never anything security-relevant.

pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        // Avoid the degenerate seed=0 fixed point some splitmix
        // implementations have; any nonzero odd-ish constant works.
        Self {
            state: seed ^ 0x9E3779B97F4A7C15,
        }
    }

    /// Next raw 64-bit value from the SplitMix64 stream.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    /// Uniform float in `[0, 1)`.
    pub fn next_f64(&mut self) -> f64 {
        // Top 53 bits give a uniform double in [0, 1), matching the
        // standard technique used by most PRNG libraries.
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Approximately-normal sample with the given mean and standard
    /// deviation, via Box-Muller. Good enough for synthetic benchmark
    /// noise; not intended for anything requiring exact normality.
    pub fn next_gaussian(&mut self, mean: f64, std_dev: f64) -> f64 {
        // Avoid u1 == 0.0, which would make ln(u1) undefined.
        let u1 = (self.next_f64() + f64::EPSILON).min(1.0);
        let u2 = self.next_f64();
        let z0 = (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos();
        mean + std_dev * z0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_produces_the_same_stream() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_produce_different_streams() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        let sample_a: Vec<u64> = (0..20).map(|_| a.next_u64()).collect();
        let sample_b: Vec<u64> = (0..20).map(|_| b.next_u64()).collect();
        assert_ne!(sample_a, sample_b);
    }

    #[test]
    fn next_f64_stays_within_zero_one() {
        let mut rng = Rng::new(7);
        for _ in 0..10_000 {
            let v = rng.next_f64();
            assert!((0.0..1.0).contains(&v), "value {v} out of range");
        }
    }

    #[test]
    fn gaussian_sample_mean_and_stddev_converge_over_many_samples() {
        let mut rng = Rng::new(99);
        let n = 50_000;
        let samples: Vec<f64> = (0..n).map(|_| rng.next_gaussian(1.0, 0.1)).collect();
        let mean = samples.iter().sum::<f64>() / n as f64;
        let variance = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n as f64;
        let std_dev = variance.sqrt();

        assert!(
            (mean - 1.0).abs() < 0.01,
            "sample mean {mean} too far from 1.0"
        );
        assert!(
            (std_dev - 0.1).abs() < 0.01,
            "sample std_dev {std_dev} too far from 0.1"
        );
    }
}
