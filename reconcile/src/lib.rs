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

/// Revert-burst configuration: halts if `limit` consecutive transactions revert on-chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevertBurstConfig {
    pub limit: usize,
}

impl Default for RevertBurstConfig {
    fn default() -> Self {
        Self { limit: 3 }
    }
}

/// Revert gas-budget configuration: halts if cumulative gas fees burned on reverts
/// within the last `window` timeline entries exceeds `budget_limit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevertGasConfig {
    pub budget_limit: u128,
    pub window: usize,
}

/// Volume-weighted average ratio configuration: halts if VWAR of the last `window`
/// confirmed pairs drops below `threshold`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolumeWeightedConfig {
    pub threshold: f64,
    pub window: usize,
}

impl Default for VolumeWeightedConfig {
    fn default() -> Self {
        Self {
            threshold: 0.75,
            window: 20,
        }
    }
}

/// Absolute net capital drawdown configuration: halts if cumulative prediction deficit
/// (sum(predicted - realized)) across the last `window` pairs exceeds `max_drawdown_limit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrawdownConfig {
    pub max_drawdown_limit: i128,
    pub window: usize,
}

/// The default `HaltPolicy`: the phantom-guard dual guard, optionally extended
/// with revert and capital-weighted risk guards.
///
/// Construct via [`ReconcilePolicy::default_dual_guard`] for the
/// whitepaper's default thresholds, [`ReconcilePolicy::institutional_default`]
/// for revert-burst and volume-weighted protections, or [`ReconcilePolicy::new`]
/// for custom parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReconcilePolicy {
    fast: FastGuardConfig,
    slow: SlowGuardConfig,
    revert_burst: Option<RevertBurstConfig>,
    revert_gas: Option<RevertGasConfig>,
    volume_weighted: Option<VolumeWeightedConfig>,
    drawdown: Option<DrawdownConfig>,
}

impl ReconcilePolicy {
    /// Build a policy with explicit guard configuration.
    ///
    /// Per whitepaper Property 2, `slow.threshold` should be greater than
    /// `fast.threshold` for the two guards to be non-redundant as stated.
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
        Self {
            fast,
            slow,
            revert_burst: None,
            revert_gas: None,
            volume_weighted: None,
            drawdown: None,
        }
    }

    /// Enable consecutive revert burst detection.
    pub fn with_revert_burst(mut self, config: RevertBurstConfig) -> Self {
        assert!(config.limit > 0, "revert-burst limit must be > 0");
        self.revert_burst = Some(config);
        self
    }

    /// Enable cumulative revert gas budget monitoring.
    pub fn with_revert_gas(mut self, config: RevertGasConfig) -> Self {
        assert!(config.window > 0, "revert-gas window must be > 0");
        self.revert_gas = Some(config);
        self
    }

    /// Enable volume-weighted average ratio (VWAR) reconciliation.
    pub fn with_volume_weighted(mut self, config: VolumeWeightedConfig) -> Self {
        assert!(config.window > 0, "volume-weighted window must be > 0");
        self.volume_weighted = Some(config);
        self
    }

    /// Enable net capital slippage drawdown monitoring.
    pub fn with_drawdown(mut self, config: DrawdownConfig) -> Self {
        assert!(config.window > 0, "drawdown window must be > 0");
        self.drawdown = Some(config);
        self
    }

    /// The whitepaper's default configuration: `T_f = 0.50`, `k_f = 3`,
    /// `T_s = 0.70`, `k_s = 20` (Section 7).
    pub fn default_dual_guard() -> Self {
        Self::new(FastGuardConfig::default(), SlowGuardConfig::default())
    }

    /// Institutional multi-guard policy enabling revert-burst ($k_{rev}=3$)
    /// and volume-weighted ratio ($T_{vw}=0.75, k_{vw}=20$) protections.
    pub fn institutional_default() -> Self {
        Self::default_dual_guard()
            .with_revert_burst(RevertBurstConfig::default())
            .with_volume_weighted(VolumeWeightedConfig::default())
    }

    pub fn fast_guard_config(&self) -> FastGuardConfig {
        self.fast
    }

    pub fn slow_guard_config(&self) -> SlowGuardConfig {
        self.slow
    }

    pub fn revert_burst_config(&self) -> Option<RevertBurstConfig> {
        self.revert_burst
    }

    pub fn revert_gas_config(&self) -> Option<RevertGasConfig> {
        self.revert_gas
    }

    pub fn volume_weighted_config(&self) -> Option<VolumeWeightedConfig> {
        self.volume_weighted
    }

    pub fn drawdown_config(&self) -> Option<DrawdownConfig> {
        self.drawdown
    }
}

impl Default for ReconcilePolicy {
    fn default() -> Self {
        Self::default_dual_guard()
    }
}

impl HaltPolicy for ReconcilePolicy {
    fn evaluate(&mut self, history: &ReconcileHistory) -> HaltDecision {
        // 1. Revert burst check (immediate infrastructure failure)
        if let Some(burst) = self.revert_burst {
            let consecutive = history.consecutive_reverts();
            if consecutive >= burst.limit {
                return HaltDecision::Halt(HaltReason::RevertBurst {
                    consecutive_reverts: consecutive,
                    limit: burst.limit,
                });
            }
        }

        // 2. Revert gas budget check
        if let Some(gas_cfg) = self.revert_gas {
            let burned = history.recent_revert_gas_burned(gas_cfg.window);
            if burned > gas_cfg.budget_limit {
                return HaltDecision::Halt(HaltReason::RevertGasBudgetExceeded {
                    gas_cost_burned: burned,
                    budget_limit: gas_cfg.budget_limit,
                    window: gas_cfg.window,
                });
            }
        }

        // 3. Fast guard (whitepaper Section 4.2, Equation 2): halt on `k_f`
        // consecutive ratios all below `T_f`.
        let fast_window = history.recent_ratios(self.fast.window);
        if fast_window.len() == self.fast.window
            && fast_window.iter().all(|ratio| *ratio < self.fast.threshold)
        {
            return HaltDecision::Halt(HaltReason::FastGuard {
                window: fast_window,
                threshold: self.fast.threshold,
            });
        }

        // 4. Absolute net capital drawdown check
        if let Some(dd_cfg) = self.drawdown {
            let pairs_in_window = history
                .pairs
                .iter()
                .rev()
                .take(dd_cfg.window)
                .collect::<Vec<_>>();
            if pairs_in_window.len() == dd_cfg.window {
                let net_slippage_loss: i128 = pairs_in_window
                    .iter()
                    .map(|(p, r)| p.0 - r.0)
                    .sum();
                if net_slippage_loss > dd_cfg.max_drawdown_limit {
                    return HaltDecision::Halt(HaltReason::NetDrawdownExceeded {
                        net_slippage_loss,
                        max_drawdown_limit: dd_cfg.max_drawdown_limit,
                        window: dd_cfg.window,
                    });
                }
            }
        }

        // 5. Volume-weighted average ratio check
        if let Some(vw_cfg) = self.volume_weighted {
            let valid_pairs = history
                .pairs
                .iter()
                .rev()
                .filter(|(p, _)| p.0 > 0)
                .take(vw_cfg.window)
                .collect::<Vec<_>>();
            if valid_pairs.len() == vw_cfg.window {
                let total_predicted: i128 = valid_pairs.iter().map(|(p, _)| p.0).sum();
                let total_realized: i128 = valid_pairs.iter().map(|(_, r)| r.0).sum();
                if total_predicted > 0 {
                    let weighted_ratio = total_realized as f64 / total_predicted as f64;
                    if weighted_ratio < vw_cfg.threshold {
                        return HaltDecision::Halt(HaltReason::VolumeWeightedDrift {
                            weighted_ratio,
                            threshold: vw_cfg.threshold,
                            window: vw_cfg.window,
                        });
                    }
                }
            }
        }

        // 6. Slow guard (whitepaper Section 4.3, Equation 3): halt if the
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

    // -----------------------------------------------------------------
    // New Institutional Multi-Guard Tests (v0.2.0)
    // -----------------------------------------------------------------
    #[test]
    fn revert_burst_trips_on_consecutive_reverts() {
        let mut policy = ReconcilePolicy::default_dual_guard()
            .with_revert_burst(RevertBurstConfig { limit: 3 });
        let mut history = ReconcileHistory::new();

        // 10 healthy pairs
        for _ in 0..10 {
            history.append(PredictedProfit(100), RealizedProfit(100));
        }
        assert_eq!(policy.evaluate(&history), HaltDecision::Continue);

        let rev = driftbrake_core::RevertEvent {
            tx_hash: [0u8; 32],
            block_number: 1,
            reason: None,
            gas_used: 21_000,
            effective_gas_price: 30_000_000_000,
        };

        history.record_revert(rev.clone());
        assert_eq!(policy.evaluate(&history), HaltDecision::Continue);
        history.record_revert(rev.clone());
        assert_eq!(policy.evaluate(&history), HaltDecision::Continue);
        history.record_revert(rev);

        assert_eq!(
            policy.evaluate(&history),
            HaltDecision::Halt(HaltReason::RevertBurst {
                consecutive_reverts: 3,
                limit: 3,
            })
        );
    }

    #[test]
    fn confirmed_pair_resets_revert_burst_counter() {
        let mut policy = ReconcilePolicy::default_dual_guard()
            .with_revert_burst(RevertBurstConfig { limit: 3 });
        let mut history = ReconcileHistory::new();

        let rev = driftbrake_core::RevertEvent {
            tx_hash: [0u8; 32],
            block_number: 1,
            reason: None,
            gas_used: 21_000,
            effective_gas_price: 30_000_000_000,
        };

        // 2 reverts, then 1 confirmed trade, then 2 reverts: never reaches limit of 3
        history.record_revert(rev.clone());
        history.record_revert(rev.clone());
        assert_eq!(policy.evaluate(&history), HaltDecision::Continue);

        history.append(PredictedProfit(100), RealizedProfit(100));
        assert_eq!(policy.evaluate(&history), HaltDecision::Continue);

        history.record_revert(rev.clone());
        history.record_revert(rev);
        assert_eq!(policy.evaluate(&history), HaltDecision::Continue);
    }

    #[test]
    fn revert_gas_budget_trips_when_burned_gas_exceeds_limit() {
        let mut policy = ReconcilePolicy::default_dual_guard()
            .with_revert_gas(RevertGasConfig {
                budget_limit: 100_000_000,
                window: 5,
            });
        let mut history = ReconcileHistory::new();

        let rev = driftbrake_core::RevertEvent {
            tx_hash: [0u8; 32],
            block_number: 1,
            reason: None,
            gas_used: 50_000,
            effective_gas_price: 1_000, // 50,000,000 cost
        };

        history.record_revert(rev.clone());
        assert_eq!(policy.evaluate(&history), HaltDecision::Continue);

        // Second revert burns another 50,000,000 => total 100,000,000 (not exceeded yet)
        history.record_revert(rev.clone());
        assert_eq!(policy.evaluate(&history), HaltDecision::Continue);

        // Third revert pushes total to 150,000,000 > 100,000,000 limit
        history.record_revert(rev);
        assert!(matches!(
            policy.evaluate(&history),
            HaltDecision::Halt(HaltReason::RevertGasBudgetExceeded {
                gas_cost_burned: 150_000_000,
                budget_limit: 100_000_000,
                window: 5,
            })
        ));
    }

    #[test]
    fn volume_weighted_guard_trips_on_whale_collapse_even_if_small_trades_win() {
        // Asymmetric capital vulnerability: 19 small wins + 1 giant loss
        let mut policy = ReconcilePolicy::default_dual_guard()
            .with_volume_weighted(VolumeWeightedConfig {
                threshold: 0.75,
                window: 20,
            });
        let mut history = ReconcileHistory::new();

        // 19 small trades: predicted $10, realized $12 (ratio 1.20)
        for _ in 0..19 {
            history.append(PredictedProfit(10), RealizedProfit(12));
        }

        // 1 large whale trade: predicted $100,000, realized $40,000 (ratio 0.40)
        history.append(PredictedProfit(100_000), RealizedProfit(40_000));

        // Unweighted mean ratio = (19 * 1.2 + 0.4) / 20 = 1.16 => Slow guard thinks it's great!
        // But volume-weighted ratio = (19*12 + 40,000) / (19*10 + 100,000) = 40,228 / 100,190 ~ 0.4015
        match policy.evaluate(&history) {
            HaltDecision::Halt(HaltReason::VolumeWeightedDrift {
                weighted_ratio,
                threshold,
                window,
            }) => {
                assert!(weighted_ratio < 0.41);
                assert_eq!(threshold, 0.75);
                assert_eq!(window, 20);
            }
            other => panic!("expected VolumeWeightedDrift halt, got {other:?}"),
        }
    }

    #[test]
    fn drawdown_guard_trips_on_absolute_capital_deficit() {
        let mut policy = ReconcilePolicy::default_dual_guard()
            .with_drawdown(DrawdownConfig {
                max_drawdown_limit: 50_000,
                window: 3,
            });
        let mut history = ReconcileHistory::new();

        // 3 trades where slippage gap is 20,000 each => total 60,000 > 50,000 limit
        history.append(PredictedProfit(100_000), RealizedProfit(80_000));
        history.append(PredictedProfit(100_000), RealizedProfit(80_000));
        assert_eq!(policy.evaluate(&history), HaltDecision::Continue);

        history.append(PredictedProfit(100_000), RealizedProfit(80_000));
        assert_eq!(
            policy.evaluate(&history),
            HaltDecision::Halt(HaltReason::NetDrawdownExceeded {
                net_slippage_loss: 60_000,
                max_drawdown_limit: 50_000,
                window: 3,
            })
        );
    }

    #[test]
    fn institutional_default_enables_revert_and_volume_weighted_guards() {
        let policy = ReconcilePolicy::institutional_default();
        assert!(policy.revert_burst_config().is_some());
        assert!(policy.volume_weighted_config().is_some());
        assert_eq!(policy.revert_burst_config().unwrap().limit, 3);
        assert_eq!(policy.volume_weighted_config().unwrap().threshold, 0.75);
    }
}
