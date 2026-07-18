use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

/// Profit predicted by simulation, in the smallest denominated unit of the
/// relevant token (e.g. wei for ETH-denominated strategies).
///
/// Signed: a strategy can simulate a loss. The guard math (whitepaper
/// Section 4.1) excludes non-positive `PredictedProfit` values from the
/// ratio computation rather than assuming both sides are always positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PredictedProfit(pub i128);

/// Profit actually realized on-chain, net of gas cost, in the same unit
/// and same token as the corresponding [`PredictedProfit`].
///
/// **Contract:** the implementer of [`crate::RealizedProfitDecoder`] is
/// responsible for netting `gas_used * effective_gas_price` before
/// constructing this value — `core` does not do it for you.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RealizedProfit(pub i128);

/// A single decoded event log from a confirmed transaction receipt.
///
/// Deliberately minimal and chain-agnostic: `core` does not depend on
/// `alloy` or `ethers` log types. Convert to/from those at the boundary in
/// your own `RealizedProfitDecoder` implementation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Log {
    /// Emitting contract address, as raw bytes (20 bytes for an EVM
    /// address). Left as bytes rather than a chain-specific address type
    /// so `core` doesn't need to depend on one.
    pub address: Vec<u8>,
    /// Indexed topics, raw 32-byte words.
    pub topics: Vec<[u8; 32]>,
    /// Non-indexed log data.
    pub data: Vec<u8>,
}

/// Confirmation status of a transaction, as reported by the chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TxStatus {
    Confirmed,
    Reverted,
}

/// Standard confirmed-transaction receipt.
///
/// **Contract:** [`crate::RealizedProfitDecoder::decode_realized`] must
/// only ever be called by `receipt-poller` once `status ==
/// TxStatus::Confirmed`; a defensive decoder implementation should still
/// treat an unexpected `Reverted` status as an error rather than silently
/// returning a zero profit (see `docs/API.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxReceipt {
    pub tx_hash: [u8; 32],
    pub block_number: u64,
    pub status: TxStatus,
    pub gas_used: u64,
    /// Effective gas price paid, in the chain's smallest fee-denominated
    /// unit (e.g. wei per gas unit on an EVM chain).
    pub effective_gas_price: u128,
}

/// Opaque wrapper around whatever a [`crate::SimEngine`] implementation
/// returns for a single candidate-transaction simulation.
///
/// Intentionally close to a raw EVM execution result rather than a
/// strategy-specific shape — the whole point of [`crate::ProfitDecoder`]
/// is to turn this generic shape into a [`PredictedProfit`] for your
/// specific executor contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawSimOutput {
    pub return_data: Vec<u8>,
    pub gas_used: u64,
    pub revert_reason: Option<String>,
    pub logs: Vec<Log>,
}

/// A confirmed-but-reverted transaction, tracked separately from
/// [`ReconcileHistory::pairs`] rather than folded in as a zero-profit
/// realization (whitepaper Section 4.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevertEvent {
    pub tx_hash: [u8; 32],
    pub block_number: u64,
    pub reason: Option<String>,
}

/// Append-only, confirmation-time-ordered history of `(predicted,
/// realized)` pairs.
///
/// **Contract (`docs/API.md`):** `pairs` must be ordered by confirmation
/// time, not submission time. A confirmed-but-reverted transaction is
/// never appended to `pairs`; it is recorded in `reverts` instead.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReconcileHistory {
    pub pairs: VecDeque<(PredictedProfit, RealizedProfit)>,
    pub reverts: Vec<RevertEvent>,
}

impl ReconcileHistory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a new `(predicted, realized)` pair.
    ///
    /// **Contract:** only call this for a transaction where `predicted.0 >
    /// 0` was the value used at simulation time. Pairs with
    /// `predicted.0 <= 0` are still stored (the history is a record of
    /// what happened), but [`Self::recent_ratios`] excludes them per
    /// whitepaper Property 4 (no silent zero-division).
    pub fn append(&mut self, predicted: PredictedProfit, realized: RealizedProfit) {
        self.pairs.push_back((predicted, realized));
    }

    /// Record a confirmed-but-reverted transaction. Never enters `pairs`
    /// or the ratio computation (whitepaper Section 4.5).
    pub fn record_revert(&mut self, event: RevertEvent) {
        self.reverts.push(event);
    }

    /// Ratios for the most recent `n` pairs, oldest first. Excludes any
    /// pair whose `PredictedProfit` was `<= 0` (whitepaper Section 4.1 /
    /// Property 4 — division by a non-positive predicted profit is
    /// undefined and out of scope for the guards).
    ///
    /// **Ratio direction is `realized / predicted`, never the inverse**
    /// — see whitepaper Property 1. Getting this backwards silently
    /// flips which case (underperformance vs. overperformance) the
    /// guards flag.
    pub fn recent_ratios(&self, n: usize) -> Vec<f64> {
        self.pairs
            .iter()
            .rev()
            .filter(|(predicted, _)| predicted.0 > 0)
            .take(n)
            .map(|(predicted, realized)| realized.0 as f64 / predicted.0 as f64)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }
}

/// Structured reason a [`HaltDecision::Halt`] was returned.
///
/// Deliberately structured rather than a bare string for the two built-in
/// guards — this is what lets `telemetry` emit a machine-readable event
/// that the benchmark harness can aggregate without string parsing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HaltReason {
    FastGuard {
        window: Vec<f64>,
        threshold: f64,
    },
    SlowGuard {
        mean_ratio: f64,
        window_size: usize,
        threshold: f64,
    },
    /// For non-default `HaltPolicy` implementations. Implementers of a
    /// reusable custom policy are encouraged to define their own
    /// structured variant instead of relying on this indefinitely.
    Custom(String),
}

/// Decision returned by [`crate::HaltPolicy::evaluate`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HaltDecision {
    Continue,
    Halt(HaltReason),
}
