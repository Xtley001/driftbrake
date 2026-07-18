//! The parameter sweep itself: for each `(T_f, k_f, T_s, k_s)`
//! combination, run the resulting `HaltPolicy` against every
//! healthy-regime sequence (recording false halts) and every
//! drifted-regime sequence (recording correct catches and how long they
//! took), per `docs/BENCHMARK.md`'s "Parameter sweep" section.

use driftbrake_core::{
    HaltDecision, HaltPolicy, PredictedProfit, RealizedProfit, ReconcileHistory,
};
use driftbrake_reconcile::{FastGuardConfig, ReconcilePolicy, SlowGuardConfig};

pub type Sequence = Vec<(PredictedProfit, RealizedProfit)>;

/// Aggregated result for one point on the sweep grid.
#[derive(Debug, Clone, Copy)]
pub struct SweepResult {
    pub fast: FastGuardConfig,
    pub slow: SlowGuardConfig,
    /// Fraction of healthy-regime sequences that halted at all — per
    /// `docs/BENCHMARK.md`'s definition, a false halt.
    pub false_halt_rate: f64,
    /// Fraction of drifted-regime sequences that *never* halted — a
    /// missed catch.
    pub missed_catch_rate: f64,
    /// Mean number of transactions elapsed before a halt, over only the
    /// drifted sequences that *were* caught. `None` if none were caught
    /// (i.e. `missed_catch_rate == 1.0`).
    pub mean_time_to_catch: Option<f64>,
}

/// Run one `HaltPolicy` against one sequence, appending pairs one at a
/// time (mirroring how a real strategy accumulates history) and checking
/// for a halt after each append. Returns the 1-based transaction count at
/// which it first halted, or `None` if it never did.
pub fn run_to_first_halt(policy: &mut ReconcilePolicy, sequence: &Sequence) -> Option<usize> {
    let mut history = ReconcileHistory::new();
    for (i, (predicted, realized)) in sequence.iter().enumerate() {
        history.append(*predicted, *realized);
        if let HaltDecision::Halt(_) = policy.evaluate(&history) {
            return Some(i + 1);
        }
    }
    None
}

/// Run the full sweep: one [`SweepResult`] per `(fast, slow)` combination
/// in `grid`, evaluated against `healthy_batch` and `drifted_batch`.
pub fn sweep(
    healthy_batch: &[Sequence],
    drifted_batch: &[Sequence],
    grid: &[(FastGuardConfig, SlowGuardConfig)],
) -> Vec<SweepResult> {
    grid.iter()
        .map(|&(fast, slow)| {
            let false_halts = healthy_batch
                .iter()
                .filter(|seq| {
                    let mut policy = ReconcilePolicy::new(fast, slow);
                    run_to_first_halt(&mut policy, seq).is_some()
                })
                .count();
            let false_halt_rate = false_halts as f64 / healthy_batch.len() as f64;

            let catch_times: Vec<usize> = drifted_batch
                .iter()
                .filter_map(|seq| {
                    let mut policy = ReconcilePolicy::new(fast, slow);
                    run_to_first_halt(&mut policy, seq)
                })
                .collect();
            let missed = drifted_batch.len() - catch_times.len();
            let missed_catch_rate = missed as f64 / drifted_batch.len() as f64;
            let mean_time_to_catch = (!catch_times.is_empty())
                .then(|| catch_times.iter().sum::<usize>() as f64 / catch_times.len() as f64);

            SweepResult {
                fast,
                slow,
                false_halt_rate,
                missed_catch_rate,
                mean_time_to_catch,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;
    use crate::synthetic::{generate_batch, RegimeParams};

    fn healthy_batch(seed: u64, count: usize, length: usize) -> Vec<Sequence> {
        let mut rng = Rng::new(seed);
        generate_batch(
            &mut rng,
            RegimeParams::healthy(0.05),
            count,
            length,
            1_000_000,
        )
    }

    fn drifted_batch(seed: u64, count: usize, length: usize, mu_drift: f64) -> Vec<Sequence> {
        let mut rng = Rng::new(seed);
        generate_batch(
            &mut rng,
            RegimeParams::drifted(0.05, mu_drift),
            count,
            length,
            1_000_000,
        )
    }

    #[test]
    fn loose_thresholds_rarely_false_halt_and_usually_miss_drift() {
        let grid = [(
            FastGuardConfig {
                threshold: 0.01,
                window: 3,
            },
            SlowGuardConfig {
                threshold: 0.02,
                window: 20,
            },
        )];
        let healthy = healthy_batch(1, 200, 30);
        let drifted = drifted_batch(2, 200, 30, 0.6);

        let result = sweep(&healthy, &drifted, &grid)[0];
        assert!(
            result.false_halt_rate < 0.05,
            "false_halt_rate {} should be near zero",
            result.false_halt_rate
        );
        assert!(
            result.missed_catch_rate > 0.9,
            "missed_catch_rate {} should be near one for a threshold this loose",
            result.missed_catch_rate
        );
    }

    #[test]
    fn tight_thresholds_catch_drift_immediately_but_false_halt_a_lot() {
        let grid = [(
            FastGuardConfig {
                threshold: 0.99,
                window: 1,
            },
            SlowGuardConfig {
                threshold: 0.995,
                window: 1,
            },
        )];
        let healthy = healthy_batch(3, 200, 30);
        let drifted = drifted_batch(4, 200, 30, 0.6);

        let result = sweep(&healthy, &drifted, &grid)[0];
        assert!(
            result.false_halt_rate > 0.9,
            "false_halt_rate {} should be near one for a threshold this tight",
            result.false_halt_rate
        );
        assert!(
            result.missed_catch_rate < 0.05,
            "missed_catch_rate {} should be near zero",
            result.missed_catch_rate
        );
        // Window of 1 means the very first transaction can trip it.
        assert_eq!(result.mean_time_to_catch, Some(1.0));
    }

    #[test]
    fn default_dual_guard_catches_severe_drift_while_rarely_false_halting_on_modest_noise() {
        let default = ReconcilePolicy::default_dual_guard();
        let grid = [(default.fast_guard_config(), default.slow_guard_config())];

        // Modest, realistic simulation-to-realization noise (5%) and a
        // severe, sustained drift (mu_drift = 0.3, well under both
        // T_f = 0.50 and T_s = 0.70).
        let healthy = healthy_batch(5, 500, 40);
        let drifted = drifted_batch(6, 500, 40, 0.3);

        let result = sweep(&healthy, &drifted, &grid)[0];
        assert!(
            result.false_halt_rate < 0.1,
            "default config false_halt_rate {} too high for 5% noise",
            result.false_halt_rate
        );
        assert!(
            result.missed_catch_rate < 0.1,
            "default config missed_catch_rate {} too high for severe drift",
            result.missed_catch_rate
        );
    }

    #[test]
    fn tightening_a_threshold_never_increases_missed_catch_rate() {
        // Whitepaper Property 3 (fast-guard monotonicity) implies the
        // sweep curve is well-behaved: moving a threshold in the
        // "tighter" direction should never make the guard *worse* at
        // catching real drift.
        let loose = (
            FastGuardConfig {
                threshold: 0.3,
                window: 3,
            },
            SlowGuardConfig {
                threshold: 0.5,
                window: 20,
            },
        );
        let tight = (
            FastGuardConfig {
                threshold: 0.6,
                window: 3,
            },
            SlowGuardConfig {
                threshold: 0.8,
                window: 20,
            },
        );
        let drifted = drifted_batch(7, 300, 30, 0.4);
        let healthy = healthy_batch(8, 300, 30); // unused for this assertion, but sweep() needs it

        let results = sweep(&healthy, &drifted, &[loose, tight]);
        assert!(
            results[1].missed_catch_rate <= results[0].missed_catch_rate,
            "tightening thresholds should not increase missed_catch_rate: loose={}, tight={}",
            results[0].missed_catch_rate,
            results[1].missed_catch_rate
        );
    }
}
