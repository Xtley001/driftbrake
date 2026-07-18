//! `driftbrake-reconcile`: the default [`driftbrake_core::HaltPolicy`]
//! implementation — the dual-guard reconciliation mechanism.
//!
//! This module carries the name **phantom-guard** internally and in
//! documentation (see `docs/ARCHITECTURE.md`'s "Naming" section):
//! "phantom profit" is the existing term of art for a bot believing it
//! made money it did not actually make, and `phantom-guard` is the name
//! for the halt mechanism itself, distinct from the umbrella `driftbrake`
//! crate name.
//!
//! The mechanism is specified formally in `docs/whitepaper.md`. This
//! crate is the implementation of that specification: two independent
//! guards, evaluated on every new `(predicted, realized)` pair, that halt
//! a strategy when the relationship between prediction and outcome
//! degrades beyond a configurable threshold.
//!
//! - **Fast guard** (Section 4.2): halts on `k_f` consecutive ratios all
//!   below `T_f`. Catches a sudden, severe break.
//! - **Slow guard** (Section 4.3): halts when the mean of the last `k_s`
//!   ratios drops below `T_s`. Catches a slow, individually-forgivable
//!   bleed that never trips the fast guard.
//!
//! Both are disjunctive (Section 4.4: `Halt = FastHalt OR SlowHalt`) —
//! neither guard substitutes for the other (Property 2, "guard
//! independence"), and removing either one strictly reduces detection
//! coverage.

use driftbrake_core::{HaltDecision, HaltPolicy, HaltReason, ReconcileHistory};

/// Fast-guard configuration: halts on `window` consecutive ratios all
/// below `threshold`.
///
/// Defaults (`T_f = 0.50`, `k_f = 3`) are not asserted constants — they're
/// derived from the benchmark sweep in `docs/BENCHMARK.md` and should be
/// re-tuned per chain rather than assumed to transfer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FastGuardConfig {
    pub threshold: f64,
    pub window: usize,
}

impl Default for FastGuardConfig {
    fn default() -> Self {
        Self {
            threshold: 0.50,
            window: 3,
        }
    }
}

/// Slow-guard configuration: halts when the mean of the last `window`
/// ratios drops below `threshold`.
///
/// Defaults (`T_s = 0.70`, `k_s = 20`) — see [`FastGuardConfig`]'s note on
/// re-tuning.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlowGuardConfig {
    pub threshold: f64,
    pub window: usize,
}

impl Default for SlowGuardConfig {
    fn default() -> Self {
        Self {
            threshold: 0.70,
            window: 20,
        }
    }
}

/// The default `HaltPolicy`: the phantom-guard dual guard.
///
/// Construct via [`ReconcilePolicy::default_dual_guard`] for the
/// whitepaper's default thresholds, or [`ReconcilePolicy::new`] for
/// custom ones (see `docs/BENCHMARK.md` for the re-derivation
/// methodology).
///
/// **Statelessness note:** this policy keeps no fields beyond its static
/// configuration — every `evaluate` call recomputes ratios fresh from
/// `history`. This isn't just an implementation choice: `core`'s
/// `HaltPolicy` contract requires that any internal state be derivable
/// from `history` alone, precisely so a policy can be replayed against
/// historical data for the benchmark sweep. Keeping no derived state at
/// all trivially satisfies that.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReconcilePolicy {
    fast: FastGuardConfig,
    slow: SlowGuardConfig,
}

impl ReconcilePolicy {
    /// Build a policy with explicit guard configuration.
    ///
    /// Per whitepaper Property 2, `slow.threshold` should be greater than
    /// `fast.threshold` for the two guards to be non-redundant as stated;
    /// this is checked with a `debug_assert` (a warning to a developer
    /// running tests, not a hard runtime requirement — a team may have a
    /// deliberate reason to deviate, at their own risk).
    ///
    /// A `window` of `0` is rejected outright (not just via
    /// `debug_assert`): `recent_ratios(0)` always returns an empty `Vec`,
    /// and "all ratios in an empty window are below threshold" is
    /// vacuously true — a `window: 0` guard halts unconditionally on the
    /// very first `evaluate` call, even against a completely empty
    /// history, which is never the intended behavior for a threshold
    /// window.
    ///
    /// # Panics
    /// Panics if `fast.window == 0` or `slow.window == 0`.
    pub fn new(fast: FastGuardConfig, slow: SlowGuardConfig) -> Self {
        assert!(
            fast.window > 0,
            "fast-guard window must be > 0 (see docs/whitepaper.md Section 4.2)"
        );
        assert!(
            slow.window > 0,
            "slow-guard window must be > 0 (see docs/whitepaper.md Section 4.3)"
        );
        debug_assert!(
            slow.threshold > fast.threshold,
            "slow-guard threshold ({}) should exceed fast-guard threshold ({}) \
             for Property 2 (guard independence) to hold as stated — see docs/whitepaper.md",
            slow.threshold,
            fast.threshold
        );
        Self { fast, slow }
    }

    /// The whitepaper's default configuration: `T_f = 0.50`, `k_f = 3`,
    /// `T_s = 0.70`, `k_s = 20` (Section 7).
    pub fn default_dual_guard() -> Self {
        Self::new(FastGuardConfig::default(), SlowGuardConfig::default())
    }

    pub fn fast_guard_config(&self) -> FastGuardConfig {
        self.fast
    }

    pub fn slow_guard_config(&self) -> SlowGuardConfig {
        self.slow
    }
}

impl Default for ReconcilePolicy {
    fn default() -> Self {
        Self::default_dual_guard()
    }
}

impl HaltPolicy for ReconcilePolicy {
    fn evaluate(&mut self, history: &ReconcileHistory) -> HaltDecision {
        // Fast guard (whitepaper Section 4.2, Equation 2): halt on `k_f`
        // consecutive ratios all below `T_f`. Requires a *full* window —
        // per `core`'s HaltPolicy contract, a short history must not
        // panic or be treated as if it satisfied the guard.
        let fast_window = history.recent_ratios(self.fast.window);
        if fast_window.len() == self.fast.window
            && fast_window.iter().all(|ratio| *ratio < self.fast.threshold)
        {
            return HaltDecision::Halt(HaltReason::FastGuard {
                window: fast_window,
                threshold: self.fast.threshold,
            });
        }

        // Slow guard (whitepaper Section 4.3, Equation 3): halt if the
        // mean of the last `k_s` ratios drops below `T_s`.
        let slow_window = history.recent_ratios(self.slow.window);
        if slow_window.len() == self.slow.window {
            let mean_ratio = slow_window.iter().sum::<f64>() / slow_window.len() as f64;
            if mean_ratio < self.slow.threshold {
                return HaltDecision::Halt(HaltReason::SlowGuard {
                    mean_ratio,
                    window_size: self.slow.window,
                    threshold: self.slow.threshold,
                });
            }
        }

        HaltDecision::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use driftbrake_core::{PredictedProfit, RealizedProfit};

    /// Push a sequence of realized/predicted ratios (as `(predicted,
    /// realized)` pairs with `predicted` fixed at 100 for readability)
    /// into a fresh history.
    fn history_from_ratios(ratios: &[f64]) -> ReconcileHistory {
        let mut history = ReconcileHistory::new();
        for &ratio in ratios {
            let predicted = 100i128;
            let realized = (predicted as f64 * ratio).round() as i128;
            history.append(PredictedProfit(predicted), RealizedProfit(realized));
        }
        history
    }

    // -----------------------------------------------------------------
    // Property 1 (ratio-direction correctness) — the single most
    // important test in this suite (see CONTRIBUTING.md: "run the
    // ratio-direction regression test explicitly ... a passing test
    // suite that happens to skip this one is not sufficient"). Kept
    // indefinitely as insurance against a silent inversion regression,
    // per whitepaper Section 5 / Section 6's security table.
    // -----------------------------------------------------------------
    #[test]
    fn ratio_direction_is_realized_over_predicted_and_flags_underperformance_not_overperformance() {
        let mut policy = ReconcilePolicy::new(
            FastGuardConfig {
                threshold: 0.50,
                window: 1,
            },
            SlowGuardConfig {
                threshold: 0.99, // neutered via an unreachable window below, not via threshold ordering
                window: usize::MAX,
            },
        );

        // Whitepaper Section 4.1 worked example: predicted 100, realized
        // 42 => rho = 0.42, which is underperformance and must halt.
        let underperforming = history_from_ratios(&[0.42]);
        assert!(matches!(
            policy.evaluate(&underperforming),
            HaltDecision::Halt(HaltReason::FastGuard { .. })
        ));

        // The mirror case: realized *exceeds* predicted (rho = 2.38,
        // i.e. predicted 100 / realized 238). Under the correct
        // definition (realized / predicted) this is >= 1 and must NOT
        // halt. Under the inverted (wrong) definition this would look
        // like underperformance and incorrectly halt.
        let overperforming = history_from_ratios(&[2.38]);
        assert_eq!(policy.evaluate(&overperforming), HaltDecision::Continue);
    }

    #[test]
    fn ratio_direction_worked_example_from_whitepaper_section_4_1() {
        // p_hat = 100, r = 42 => rho = 0.42, below T_f = 0.50.
        let mut history = ReconcileHistory::new();
        history.append(PredictedProfit(100), RealizedProfit(42));
        assert_eq!(history.recent_ratios(1), vec![0.42]);
    }

    // -----------------------------------------------------------------
    // Fast guard (Section 4.2)
    // -----------------------------------------------------------------
    #[test]
    fn fast_guard_trips_on_three_consecutive_bad_ratios_after_healthy_history() {
        // Whitepaper Section 4.2 worked example: 0.9, 0.9, 0.3, 0.2, 0.1
        // — the last three are all < 0.50, so FastHalt is true, even
        // though the two before them were healthy (memoryless by design
        // w.r.t. anything outside its window).
        let mut policy = ReconcilePolicy::default_dual_guard();
        let history = history_from_ratios(&[0.9, 0.9, 0.3, 0.2, 0.1]);

        match policy.evaluate(&history) {
            HaltDecision::Halt(HaltReason::FastGuard { window, threshold }) => {
                assert_eq!(window, vec![0.3, 0.2, 0.1]);
                assert_eq!(threshold, 0.50);
            }
            other => panic!("expected FastGuard halt, got {other:?}"),
        }
    }

    #[test]
    fn fast_guard_does_not_trip_on_a_single_bad_ratio() {
        let mut policy = ReconcilePolicy::default_dual_guard();
        let history = history_from_ratios(&[0.9, 0.9, 0.1]); // only 1 of 3 bad
        assert_eq!(policy.evaluate(&history), HaltDecision::Continue);
    }

    #[test]
    fn fast_guard_requires_the_bad_run_to_be_the_most_recent_and_unbroken() {
        let mut policy = ReconcilePolicy::default_dual_guard();
        // Bad, good, bad, bad: the most recent 3 are [good, bad, bad] —
        // not all below threshold, so no halt.
        let history = history_from_ratios(&[0.1, 0.9, 0.1, 0.2]);
        assert_eq!(policy.evaluate(&history), HaltDecision::Continue);
    }

    #[test]
    fn fast_guard_is_configurable_not_hardcoded() {
        let mut policy = ReconcilePolicy::new(
            FastGuardConfig {
                threshold: 0.80, // stricter than the 0.50 default
                window: 2,
            },
            SlowGuardConfig {
                threshold: 0.99,
                window: usize::MAX,
            },
        );
        // 0.75 would pass the *default* 0.50 threshold but must trip
        // this custom, stricter 0.80 threshold over a window of 2.
        let history = history_from_ratios(&[0.75, 0.75]);
        assert!(matches!(
            policy.evaluate(&history),
            HaltDecision::Halt(HaltReason::FastGuard { .. })
        ));
    }

    // -----------------------------------------------------------------
    // Slow guard (Section 4.3)
    // -----------------------------------------------------------------
    #[test]
    fn slow_guard_trips_on_a_slow_bleed_the_fast_guard_misses() {
        // Whitepaper Section 4.3 worked example: 20 ratios averaging
        // 0.68, individually ranging 0.65-0.72 (well above T_f = 0.50,
        // so the fast guard never trips), but the mean is below
        // T_s = 0.70.
        let mut policy = ReconcilePolicy::default_dual_guard();
        let ratios: Vec<f64> = (0..20)
            .map(|i| 0.65 + (i % 8) as f64 * 0.01) // stays within 0.65-0.72
            .collect();
        let mean = ratios.iter().sum::<f64>() / ratios.len() as f64;
        assert!(mean < 0.70, "test fixture's mean ({mean}) must be < 0.70");
        assert!(
            ratios.iter().all(|r| *r >= 0.50),
            "test fixture must never trip the fast guard on its own"
        );

        let history = history_from_ratios(&ratios);
        match policy.evaluate(&history) {
            HaltDecision::Halt(HaltReason::SlowGuard {
                window_size,
                threshold,
                ..
            }) => {
                assert_eq!(window_size, 20);
                assert_eq!(threshold, 0.70);
            }
            other => panic!("expected SlowGuard halt, got {other:?}"),
        }
    }

    #[test]
    fn slow_guard_does_not_trip_with_fewer_than_the_full_window() {
        let mut policy = ReconcilePolicy::default_dual_guard();
        // Only 19 ratios, all bad enough to fail the mean test if it
        // were (wrongly) computed over a partial window.
        let history = history_from_ratios(&[0.6; 19]);
        assert_eq!(policy.evaluate(&history), HaltDecision::Continue);
    }

    // -----------------------------------------------------------------
    // Property 2 (guard independence / non-redundancy)
    // -----------------------------------------------------------------
    #[test]
    fn property_2_slow_guard_can_trip_when_fast_guard_would_not() {
        let mut policy = ReconcilePolicy::default_dual_guard();
        let ratios: Vec<f64> = (0..20).map(|i| 0.65 + (i % 8) as f64 * 0.01).collect();
        let history = history_from_ratios(&ratios);
        assert!(matches!(
            policy.evaluate(&history),
            HaltDecision::Halt(HaltReason::SlowGuard { .. })
        ));
    }

    #[test]
    fn property_2_fast_guard_can_trip_when_slow_guard_would_not() {
        let mut policy = ReconcilePolicy::default_dual_guard();
        // 17 perfect trades, then 3 catastrophic ones: the rolling mean
        // over 20 stays comfortably above 0.70, but the last 3 trip the
        // fast guard.
        let mut ratios = vec![1.0; 17];
        ratios.extend([0.1, 0.1, 0.1]);
        let history = history_from_ratios(&ratios);

        match policy.evaluate(&history) {
            HaltDecision::Halt(HaltReason::FastGuard { .. }) => {}
            other => panic!("expected FastGuard halt, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // Construction-time guards: a window of 0 is a footgun, not a valid
    // configuration.
    // -----------------------------------------------------------------
    #[test]
    #[should_panic(expected = "fast-guard window must be > 0")]
    fn rejects_a_zero_fast_guard_window() {
        // Regression test: window=0 used to vacuously trip the guard on
        // every call (an empty window trivially satisfies "all ratios
        // below threshold"), including on a completely empty history.
        // That's a footgun, not a valid configuration, and must be
        // rejected at construction rather than silently misbehaving.
        ReconcilePolicy::new(
            FastGuardConfig {
                threshold: 0.5,
                window: 0,
            },
            SlowGuardConfig::default(),
        );
    }

    #[test]
    #[should_panic(expected = "slow-guard window must be > 0")]
    fn rejects_a_zero_slow_guard_window() {
        ReconcilePolicy::new(
            FastGuardConfig::default(),
            SlowGuardConfig {
                threshold: 0.9,
                window: 0,
            },
        );
    }

    // -----------------------------------------------------------------
    // HaltPolicy contract: must not panic on short/empty history.
    // -----------------------------------------------------------------
    #[test]
    fn continues_on_empty_history() {
        let mut policy = ReconcilePolicy::default_dual_guard();
        let history = ReconcileHistory::new();
        assert_eq!(policy.evaluate(&history), HaltDecision::Continue);
    }

    #[test]
    fn continues_when_history_is_shorter_than_either_window() {
        let mut policy = ReconcilePolicy::default_dual_guard();
        let history = history_from_ratios(&[0.1, 0.1]); // shorter than k_f = 3
        assert_eq!(policy.evaluate(&history), HaltDecision::Continue);
    }

    // -----------------------------------------------------------------
    // Property 4 (no silent zero-division), exercised through the
    // policy rather than ReconcileHistory directly (core already tests
    // ReconcileHistory::recent_ratios in isolation).
    // -----------------------------------------------------------------
    #[test]
    fn non_positive_predicted_profit_is_excluded_from_both_guards() {
        let mut policy = ReconcilePolicy::default_dual_guard();
        let mut history = ReconcileHistory::new();
        // Three genuinely bad ratios...
        history.append(PredictedProfit(100), RealizedProfit(10));
        history.append(PredictedProfit(100), RealizedProfit(10));
        history.append(PredictedProfit(100), RealizedProfit(10));
        // ...but the strategy should never have submitted this one, and
        // it must not dilute or dodge the fast guard's window either way.
        history.append(PredictedProfit(-5), RealizedProfit(999_999));

        assert!(matches!(
            policy.evaluate(&history),
            HaltDecision::Halt(HaltReason::FastGuard { .. })
        ));
    }
}
