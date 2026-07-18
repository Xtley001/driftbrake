//! `driftbrake-core`: chain-agnostic trait boundaries and data types.
//!
//! This crate has **zero dependency on REVM or `alloy`** — see
//! `docs/ARCHITECTURE.md`'s "Design goals" and `CONTRIBUTING.md`'s
//! workspace-layout rules. That property is load-bearing: it's what makes
//! `revm-backend` swappable rather than load-bearing. If a change to this
//! crate would require pulling in REVM or an RPC client type, that's a
//! signal to reconsider the change, not to add a feature flag around it.
//!
//! The three trait boundaries — [`ProfitDecoder`], [`RealizedProfitDecoder`],
//! and [`HaltPolicy`] — are the places where strategy-specific and
//! chain-specific knowledge actually enters the pipeline. [`SimEngine`] is
//! the fourth trait, covering the simulation backend itself; `core` stays
//! REVM-free by leaving the candidate-transaction type as an associated
//! type that a concrete backend (e.g. `revm-backend`) fixes.

mod error;
mod traits;
mod types;

pub use error::{DecodeError, SimError};
pub use traits::{HaltPolicy, ProfitDecoder, RealizedProfitDecoder, SimEngine};
pub use types::{
    HaltDecision, HaltReason, Log, PredictedProfit, RawSimOutput, RealizedProfit, ReconcileHistory,
    RevertEvent, TxReceipt, TxStatus,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(predicted: i128, realized: i128) -> (PredictedProfit, RealizedProfit) {
        (PredictedProfit(predicted), RealizedProfit(realized))
    }

    #[test]
    fn recent_ratios_computes_realized_over_predicted_not_the_inverse() {
        // Property 1 (ratio-direction correctness), whitepaper Section 5:
        // rho_i = r_i / p_hat_i, never the inverse. This regression test
        // lives here too (not just in `reconcile`) because
        // `recent_ratios` is where the division actually happens.
        let mut history = ReconcileHistory::new();
        let (p, r) = pair(100, 42);
        history.append(p, r);

        let ratios = history.recent_ratios(1);
        assert_eq!(ratios, vec![0.42]);
        // The inverted (wrong) computation would give ~2.38, which is
        // *greater* than 1 for a case that is actually underperformance.
        assert!(ratios[0] < 1.0);
    }

    #[test]
    fn recent_ratios_excludes_non_positive_predicted_profit() {
        // Whitepaper Property 4 (no silent zero-division): a pair whose
        // PredictedProfit was <= 0 at recording time must never enter the
        // ratio window.
        let mut history = ReconcileHistory::new();
        history.append(PredictedProfit(100), RealizedProfit(50));
        history.append(PredictedProfit(0), RealizedProfit(999)); // excluded
        history.append(PredictedProfit(-10), RealizedProfit(5)); // excluded
        history.append(PredictedProfit(200), RealizedProfit(180));

        let ratios = history.recent_ratios(10);
        assert_eq!(ratios, vec![0.5, 0.9]);
    }

    #[test]
    fn recent_ratios_returns_oldest_first_within_the_window() {
        let mut history = ReconcileHistory::new();
        for (p, r) in [(100, 90), (100, 80), (100, 70), (100, 60)] {
            history.append(PredictedProfit(p), RealizedProfit(r));
        }

        // Most recent 2, oldest first: the 3rd and 4th appended pairs.
        let ratios = history.recent_ratios(2);
        assert_eq!(ratios, vec![0.7, 0.6]);
    }

    #[test]
    fn recent_ratios_handles_short_history_gracefully() {
        // HaltPolicy::evaluate must not panic on an empty or short
        // history; recent_ratios backs that by simply returning fewer
        // ratios than requested rather than panicking or padding.
        let history = ReconcileHistory::new();
        assert_eq!(history.recent_ratios(20), Vec::<f64>::new());

        let mut history = ReconcileHistory::new();
        history.append(PredictedProfit(100), RealizedProfit(50));
        assert_eq!(history.recent_ratios(20), vec![0.5]);
    }

    #[test]
    fn revert_events_never_enter_pairs_or_ratios() {
        // Whitepaper Section 4.5: receipt-gated realization. A
        // confirmed-but-reverted transaction is recorded separately and
        // must never be silently treated as a zero-profit realization.
        let mut history = ReconcileHistory::new();
        history.append(PredictedProfit(100), RealizedProfit(80));
        history.record_revert(RevertEvent {
            tx_hash: [0u8; 32],
            block_number: 1,
            reason: Some("out of gas".to_string()),
        });

        assert_eq!(history.pairs.len(), 1);
        assert_eq!(history.reverts.len(), 1);
        assert_eq!(history.recent_ratios(10), vec![0.8]);
    }
}
