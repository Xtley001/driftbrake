//! `driftbrake-receipt-poller`: confirms receipts and books realized
//! profit, gated on both confirmed status *and* a decodable profit event
//! (`docs/ARCHITECTURE.md`'s "receipt-poller" section).
//!
//! Requiring both conditions, rather than either alone, avoids two
//! distinct failure modes:
//! - A confirmed-but-**reverted** transaction being miscounted as a
//!   zero-profit realization (whitepaper Section 4.5) — this crate
//!   records it as a [`driftbrake_core::RevertEvent`] instead, and never
//!   appends it to [`driftbrake_core::ReconcileHistory::pairs`].
//! - A profit-shaped log appearing without the transaction actually
//!   having confirmed (e.g. from a stale or unrelated context) — gated by
//!   requiring `TxStatus::Confirmed` before the decoder is even called.

mod source;

pub use source::{PollerConfig, ReceiptSource};

use thiserror::Error;

use driftbrake_core::{
    DecodeError, PredictedProfit, RealizedProfit, RealizedProfitDecoder, ReconcileHistory,
    RevertEvent, TxStatus,
};

/// Outcome of polling a single transaction to settlement (or giving up).
#[derive(Debug, Clone, PartialEq)]
pub enum PollOutcome {
    /// Confirmed, and a profit event was successfully decoded. Already
    /// appended to the caller's [`ReconcileHistory`] by the time this is
    /// returned.
    Realized(RealizedProfit),
    /// Confirmed but reverted. Recorded in
    /// [`ReconcileHistory::reverts`], never in `pairs`.
    Reverted(RevertEvent),
    /// Confirmed, but `RealizedProfitDecoder` could not extract a profit
    /// event (e.g. `DecodeError::MissingProfitEvent`). Deliberately
    /// **not** booked as a realization of any kind — a decode failure on
    /// a confirmed, non-reverted receipt is a data-quality problem with
    /// your decoder or executor contract, not a signal about the
    /// strategy's performance, and silently treating it as `0` would
    /// corrupt the guard's input the same way miscounting a revert
    /// would.
    DecodeFailed(DecodeError),
    /// No confirmed receipt appeared within `confirmation_timeout_blocks`
    /// block-times. The transaction may still confirm later; this crate
    /// does not resubmit or track it further — that's a submission-layer
    /// concern, out of scope here (see `docs/ARCHITECTURE.md`'s
    /// non-goals).
    TimedOut,
}

/// Error from the receipt source itself (RPC failure, connection drop,
/// etc.) — distinct from [`PollOutcome::DecodeFailed`], which is a
/// successful poll that then failed to decode.
#[derive(Debug, Error)]
pub enum PollError<E> {
    #[error("receipt source error: {0}")]
    Source(E),
}

/// Polls a [`ReceiptSource`] for a transaction's receipt and books its
/// realized profit via a [`RealizedProfitDecoder`], once both the
/// confirmed-status and decodable-profit-event conditions are met.
pub struct ReceiptPoller<S, D> {
    source: S,
    decoder: D,
    config: PollerConfig,
}

impl<S, D> ReceiptPoller<S, D>
where
    S: ReceiptSource,
    D: RealizedProfitDecoder,
{
    pub fn new(source: S, decoder: D, config: PollerConfig) -> Self {
        Self {
            source,
            decoder,
            config,
        }
    }

    /// Poll until `tx_hash` reaches a confirmed receipt, is confirmed as
    /// finally reverted, or the configured timeout elapses.
    ///
    /// On [`PollOutcome::Realized`], the `(predicted, realized)` pair has
    /// already been appended to `history`. On
    /// [`PollOutcome::Reverted`], the revert has already been recorded in
    /// `history.reverts`. Every other outcome leaves `history` untouched
    /// — see the whitepaper's Section 4.5 (receipt-gated realization):
    /// nothing enters the guard's input except a genuinely confirmed,
    /// decodable outcome.
    pub async fn poll_until_settled(
        &self,
        tx_hash: [u8; 32],
        predicted: PredictedProfit,
        history: &mut ReconcileHistory,
    ) -> Result<PollOutcome, PollError<S::Error>> {
        let deadline = tokio::time::Instant::now() + self.config.timeout();

        loop {
            match self
                .source
                .get_receipt(tx_hash)
                .await
                .map_err(PollError::Source)?
            {
                Some((receipt, logs)) => {
                    return Ok(match receipt.status {
                        TxStatus::Reverted => {
                            let event = RevertEvent {
                                tx_hash,
                                block_number: receipt.block_number,
                                reason: None,
                            };
                            history.record_revert(event.clone());
                            PollOutcome::Reverted(event)
                        }
                        TxStatus::Confirmed => {
                            match self.decoder.decode_realized(&receipt, &logs) {
                                Ok(realized) => {
                                    history.append(predicted, realized);
                                    PollOutcome::Realized(realized)
                                }
                                Err(decode_err) => PollOutcome::DecodeFailed(decode_err),
                            }
                        }
                    });
                }
                None => {
                    if tokio::time::Instant::now() >= deadline {
                        return Ok(PollOutcome::TimedOut);
                    }
                    tokio::time::sleep(self.config.poll_interval()).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use driftbrake_core::{Log, TxReceipt};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Returns `None` for the first `confirms_after_polls` polls, then a
    /// fixed `(TxReceipt, Vec<Log>)` forever after — a deterministic
    /// stand-in for "the tx confirms after N polling round-trips."
    struct ScriptedSource {
        confirms_after_polls: usize,
        polls_so_far: AtomicUsize,
        result: Option<(TxReceipt, Vec<Log>)>,
    }

    #[derive(Debug, thiserror::Error)]
    #[error("scripted source never fails")]
    struct Never;

    #[async_trait::async_trait]
    impl ReceiptSource for ScriptedSource {
        type Error = Never;

        async fn get_receipt(
            &self,
            _tx_hash: [u8; 32],
        ) -> Result<Option<(TxReceipt, Vec<Log>)>, Self::Error> {
            let n = self.polls_so_far.fetch_add(1, Ordering::SeqCst);
            if n >= self.confirms_after_polls {
                Ok(self.result.clone())
            } else {
                Ok(None)
            }
        }
    }

    fn confirmed_receipt(block_number: u64) -> TxReceipt {
        TxReceipt {
            tx_hash: [1u8; 32],
            block_number,
            status: TxStatus::Confirmed,
            gas_used: 100_000,
            effective_gas_price: 10_000_000_000,
        }
    }

    fn reverted_receipt(block_number: u64) -> TxReceipt {
        TxReceipt {
            status: TxStatus::Reverted,
            ..confirmed_receipt(block_number)
        }
    }

    /// Decodes a fixed `RealizedProfit` from any log whose first byte is
    /// `1`, fails with `MissingProfitEvent` for an empty log set, and
    /// panics if ever called on a receipt whose status isn't
    /// `Confirmed` — enforcing the "must not be called on a non-confirmed
    /// receipt" contract from `docs/API.md` from the test side too.
    struct FixedProfitDecoder;

    impl RealizedProfitDecoder for FixedProfitDecoder {
        fn decode_realized(
            &self,
            receipt: &TxReceipt,
            logs: &[Log],
        ) -> Result<RealizedProfit, DecodeError> {
            assert_eq!(
                receipt.status,
                TxStatus::Confirmed,
                "decode_realized must never be called on a non-confirmed receipt"
            );
            logs.iter()
                .find(|log| log.data.first() == Some(&1))
                .map(|log| RealizedProfit(log.data[1] as i128))
                .ok_or(DecodeError::MissingProfitEvent)
        }
    }

    fn profit_log(amount: u8) -> Log {
        Log {
            address: vec![0xAA; 20],
            topics: vec![],
            data: vec![1, amount],
        }
    }

    fn poller_config() -> PollerConfig {
        PollerConfig::new(
            /* block_time_ms */ 100, /* poll_interval_fraction */ 0.1, // 10ms
            /* confirmation_timeout_blocks */ 5, // 500ms
        )
    }

    #[tokio::test]
    async fn confirmed_with_profit_event_is_realized_and_appended_to_history() {
        let source = ScriptedSource {
            confirms_after_polls: 2,
            polls_so_far: AtomicUsize::new(0),
            result: Some((confirmed_receipt(100), vec![profit_log(42)])),
        };
        let poller = ReceiptPoller::new(source, FixedProfitDecoder, poller_config());
        let mut history = ReconcileHistory::new();

        let outcome = poller
            .poll_until_settled([1u8; 32], PredictedProfit(100), &mut history)
            .await
            .unwrap();

        assert_eq!(outcome, PollOutcome::Realized(RealizedProfit(42)));
        assert_eq!(history.pairs.len(), 1);
        assert_eq!(history.pairs[0], (PredictedProfit(100), RealizedProfit(42)));
        assert!(history.reverts.is_empty());
    }

    #[tokio::test]
    async fn reverted_receipt_is_recorded_as_a_revert_event_not_a_zero_profit_pair() {
        let source = ScriptedSource {
            confirms_after_polls: 0,
            polls_so_far: AtomicUsize::new(0),
            result: Some((reverted_receipt(101), vec![])),
        };
        let poller = ReceiptPoller::new(source, FixedProfitDecoder, poller_config());
        let mut history = ReconcileHistory::new();

        let outcome = poller
            .poll_until_settled([2u8; 32], PredictedProfit(100), &mut history)
            .await
            .unwrap();

        match outcome {
            PollOutcome::Reverted(event) => {
                assert_eq!(event.block_number, 101);
                assert_eq!(event.tx_hash, [2u8; 32]);
            }
            other => panic!("expected Reverted, got {other:?}"),
        }
        // Whitepaper Section 4.5: never silently treated as r_i = 0.
        assert!(history.pairs.is_empty());
        assert_eq!(history.reverts.len(), 1);
    }

    #[tokio::test]
    async fn confirmed_but_no_profit_event_is_decode_failed_not_booked_as_zero() {
        let source = ScriptedSource {
            confirms_after_polls: 0,
            polls_so_far: AtomicUsize::new(0),
            result: Some((confirmed_receipt(102), vec![])), // no matching log
        };
        let poller = ReceiptPoller::new(source, FixedProfitDecoder, poller_config());
        let mut history = ReconcileHistory::new();

        let outcome = poller
            .poll_until_settled([3u8; 32], PredictedProfit(100), &mut history)
            .await
            .unwrap();

        assert_eq!(
            outcome,
            PollOutcome::DecodeFailed(DecodeError::MissingProfitEvent)
        );
        assert!(history.pairs.is_empty());
        assert!(history.reverts.is_empty());
    }

    #[tokio::test]
    async fn gives_up_and_returns_timed_out_if_never_confirmed() {
        let source = ScriptedSource {
            confirms_after_polls: usize::MAX, // never confirms
            polls_so_far: AtomicUsize::new(0),
            result: None,
        };
        // Short timeout so the test itself stays fast.
        let config = PollerConfig::new(10, 1.0, 2); // 10ms poll, 20ms timeout
        let poller = ReceiptPoller::new(source, FixedProfitDecoder, config);
        let mut history = ReconcileHistory::new();

        let outcome = poller
            .poll_until_settled([4u8; 32], PredictedProfit(100), &mut history)
            .await
            .unwrap();

        assert_eq!(outcome, PollOutcome::TimedOut);
        assert!(history.pairs.is_empty());
        assert!(history.reverts.is_empty());
    }

    #[tokio::test]
    async fn polls_repeatedly_at_the_configured_interval_until_confirmed() {
        // confirms_after_polls = 3: the source must be polled multiple
        // times, proving the loop actually re-polls rather than giving
        // up or succeeding on the first attempt.
        let source = ScriptedSource {
            confirms_after_polls: 3,
            polls_so_far: AtomicUsize::new(0),
            result: Some((confirmed_receipt(200), vec![profit_log(7)])),
        };
        let poller = ReceiptPoller::new(source, FixedProfitDecoder, poller_config());
        let mut history = ReconcileHistory::new();

        let outcome = poller
            .poll_until_settled([5u8; 32], PredictedProfit(50), &mut history)
            .await
            .unwrap();

        assert_eq!(outcome, PollOutcome::Realized(RealizedProfit(7)));
        assert!(poller.source.polls_so_far.load(Ordering::SeqCst) >= 3);
    }

    #[test]
    fn poller_config_computes_interval_and_timeout_from_block_time() {
        let config = PollerConfig::new(12_000, 0.5, 3);
        assert_eq!(
            config.poll_interval(),
            std::time::Duration::from_millis(6_000)
        );
        assert_eq!(config.timeout(), std::time::Duration::from_millis(36_000));
    }

    #[test]
    #[should_panic(expected = "poll_interval_fraction must be in (0.0, 1.0]")]
    fn poller_config_rejects_out_of_range_fraction() {
        PollerConfig::new(12_000, 1.5, 3);
    }
}
