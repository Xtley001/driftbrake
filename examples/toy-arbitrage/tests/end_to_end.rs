//! End-to-end integration test: `SimEngine` -> `ProfitDecoder` ->
//! (fixture submission) -> `RealizedProfitDecoder` (via
//! `ReceiptPoller`) -> `HaltPolicy`, exactly the chain `README.md`
//! describes this example as exercising — using in-memory fixtures for
//! the network-facing pieces (`rpc.rs`'s live implementations are
//! covered by their own unit tests; see `README.md`'s "Known
//! limitations" for why this doesn't run against a real testnet fork).
//!
//! The scenario: simulate several profitable trades that realize as
//! predicted (guards stay quiet), then inject drift and confirm the
//! default dual guard actually halts the loop — proving the wiring
//! between crates, not just each crate in isolation.

use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;

use driftbrake_core::{
    DecodeError, HaltDecision, HaltPolicy, Log, ProfitDecoder, RawSimOutput, ReconcileHistory,
    SimEngine, SimError, TxReceipt, TxStatus,
};
use driftbrake_receipt_poller::{PollOutcome, PollerConfig, ReceiptPoller, ReceiptSource};
use driftbrake_reconcile::ReconcilePolicy;
use driftbrake_revm_backend::CandidateTx;

use toy_arbitrage::abi::event_topic0;
use toy_arbitrage::decoder::{
    DriftInjectingDecoder, ToyArbProfitDecoder, ToyArbRealizedDecoder, PROFIT_EVENT_SIGNATURE,
};

/// Always simulates a fixed-profit `executeArb` call: `return_data` is a
/// single `uint256` word equal to `profit_per_trade`.
struct FixedProfitSimEngine {
    profit_per_trade: u128,
}

#[async_trait]
impl SimEngine for FixedProfitSimEngine {
    type Tx = CandidateTx;

    async fn simulate(&self, _tx: &Self::Tx) -> Result<RawSimOutput, SimError> {
        let mut return_data = vec![0u8; 32];
        return_data[16..].copy_from_slice(&self.profit_per_trade.to_be_bytes());
        Ok(RawSimOutput {
            return_data,
            gas_used: 150_000,
            revert_reason: None,
            logs: vec![],
        })
    }
}

/// Immediately "confirms" every transaction with a `Profit` event equal
/// to whatever `RealizedProfitDecoder` (wrapped with drift injection, in
/// this test) is asked to produce — realistic realized profit comes from
/// the drift wrapper dampening it, not from this fixture varying its
/// output.
struct InstantConfirmSource {
    gross_profit_per_trade: u128,
    calls: AtomicUsize,
}

#[async_trait]
impl ReceiptSource for InstantConfirmSource {
    type Error = std::convert::Infallible;

    async fn get_receipt(
        &self,
        tx_hash: [u8; 32],
    ) -> Result<Option<(TxReceipt, Vec<Log>)>, Self::Error> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let receipt = TxReceipt {
            tx_hash,
            block_number: 100 + n as u64,
            status: TxStatus::Confirmed,
            gas_used: 0, // zero gas cost isolates this test to the drift-injection effect
            effective_gas_price: 0,
        };
        let mut data = vec![0u8; 32];
        data[16..].copy_from_slice(&self.gross_profit_per_trade.to_be_bytes());
        let log = Log {
            address: vec![0xEE; 20],
            topics: vec![event_topic0(PROFIT_EVENT_SIGNATURE)],
            data,
        };
        Ok(Some((receipt, vec![log])))
    }
}

fn candidate_tx(nonce: u8) -> CandidateTx {
    CandidateTx {
        caller: [0xCAu8; 20],
        to: [0x99u8; 20],
        value: 0,
        data: vec![nonce],
        gas_limit: 500_000,
    }
}

/// Run one simulate -> decode -> poll -> reconcile -> halt-check cycle,
/// mirroring `main.rs`'s loop body closely enough to exercise the same
/// wiring, without needing a live network.
async fn run_one_trade(
    engine: &FixedProfitSimEngine,
    poller: &ReceiptPoller<InstantConfirmSource, DriftInjectingDecoder<ToyArbRealizedDecoder>>,
    history: &mut ReconcileHistory,
    halt_policy: &mut ReconcilePolicy,
    trade_index: u8,
) -> HaltDecision {
    let raw = engine.simulate(&candidate_tx(trade_index)).await.unwrap();
    let predicted = ToyArbProfitDecoder.decode_predicted(&raw).unwrap();
    assert!(
        predicted.0 > 0,
        "fixture only ever simulates profitable trades"
    );

    let tx_hash = [trade_index; 32];
    let outcome = poller
        .poll_until_settled(tx_hash, predicted, history)
        .await
        .unwrap();
    assert!(
        matches!(outcome, PollOutcome::Realized(_)),
        "fixture always confirms with a decodable profit event, got {outcome:?}"
    );

    halt_policy.evaluate(history)
}

#[tokio::test]
async fn healthy_trades_never_halt_but_injected_drift_trips_the_fast_guard() {
    let gross_profit = 1_000_000_000_000_000u128; // 0.001 ETH-equivalent, arbitrary unit
    let engine = FixedProfitSimEngine {
        profit_per_trade: gross_profit,
    };
    let source = InstantConfirmSource {
        gross_profit_per_trade: gross_profit,
        calls: AtomicUsize::new(0),
    };
    // drift_after = 5: the first 5 trades realize exactly as predicted
    // (ratio 1.0); from the 6th trade onward, realized profit is
    // dampened to 10% of what it should be (ratio ~0.1, well under the
    // fast guard's default 0.50 threshold).
    let realized_decoder = DriftInjectingDecoder::new(ToyArbRealizedDecoder, 5, 0.10);
    let poller = ReceiptPoller::new(source, realized_decoder, PollerConfig::new(12_000, 1.0, 1));

    let mut history = ReconcileHistory::new();
    let mut halt_policy = ReconcilePolicy::default_dual_guard();

    // Trades 0-4: healthy, ratio 1.0, must never halt.
    for i in 0..5u8 {
        let decision = run_one_trade(&engine, &poller, &mut history, &mut halt_policy, i).await;
        assert_eq!(
            decision,
            HaltDecision::Continue,
            "trade {i} should not halt"
        );
    }
    assert_eq!(history.pairs.len(), 5);
    assert!(
        history
            .recent_ratios(5)
            .iter()
            .all(|r| (*r - 1.0).abs() < 1e-9),
        "pre-drift ratios should all be ~1.0"
    );

    // Trades 5, 6, 7: drift injected, ratio ~0.1 each. The fast guard
    // (k_f = 3 consecutive ratios below T_f = 0.50) should trip by the
    // third drifted trade at the latest.
    let mut halted = false;
    for i in 5..8u8 {
        let decision = run_one_trade(&engine, &poller, &mut history, &mut halt_policy, i).await;
        if let HaltDecision::Halt(reason) = decision {
            match reason {
                driftbrake_core::HaltReason::FastGuard { window, threshold } => {
                    assert_eq!(threshold, 0.50);
                    assert!(window.iter().all(|r| *r < 0.50));
                }
                other => panic!("expected FastGuard, got {other:?}"),
            }
            halted = true;
            break;
        }
    }
    assert!(
        halted,
        "the fast guard should have tripped within 3 drifted trades"
    );
}

#[tokio::test]
async fn a_decode_failure_never_gets_silently_booked_as_a_pair() {
    // Sanity check on the wiring itself: ToyArbProfitDecoder correctly
    // rejects a reverted simulation before anything reaches history.
    let raw = RawSimOutput {
        return_data: vec![],
        gas_used: 21_000,
        revert_reason: Some("reverted".into()),
        logs: vec![],
    };
    assert_eq!(
        ToyArbProfitDecoder.decode_predicted(&raw),
        Err(DecodeError::Reverted)
    );
}
