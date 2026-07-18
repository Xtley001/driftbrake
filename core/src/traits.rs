use async_trait::async_trait;

use crate::error::{DecodeError, SimError};
use crate::types::{
    HaltDecision, Log, PredictedProfit, RawSimOutput, RealizedProfit, ReconcileHistory, TxReceipt,
};

/// Decodes a raw simulation result into a predicted profit.
///
/// Replaces what was originally a hardcoded `simulate()` call returning a
/// single `uint256`. Any executor contract shape — single-call, multicall,
/// flash-loan-wrapped — implements this trait once and plugs into the
/// rest of the pipeline unchanged.
///
/// **Must not:**
/// - Perform any network I/O. `decode_predicted` is called synchronously
///   from within the simulation pipeline and is expected to be a pure
///   decode step. If you need a live price to convert between tokens,
///   fetch it before calling this method and pass it in via your
///   implementation's own state.
/// - Assume `raw.revert_reason.is_none()`. A `SimEngine` may hand you a
///   `RawSimOutput` for a reverted simulation; return
///   `Err(DecodeError::Reverted)` rather than decoding `return_data` that
///   doesn't exist for a revert.
pub trait ProfitDecoder: Send + Sync {
    /// Decode a raw REVM execution trace/return value into a predicted
    /// profit, in the smallest denominated unit (e.g. wei).
    fn decode_predicted(&self, raw: &RawSimOutput) -> Result<PredictedProfit, DecodeError>;
}

/// Decodes a confirmed receipt + its logs into a realized profit, net of
/// gas cost.
///
/// Replaces a hardcoded `Profit` event ABI. `receipt-poller` calls this
/// once a receipt reaches confirmed status.
///
/// **Must not:**
/// - Be called on a receipt with a non-confirmed status. `receipt-poller`
///   is responsible for gating this, but a defensive implementation
///   should still treat an unexpected reverted-status receipt as an
///   error rather than silently returning `RealizedProfit(0)`.
/// - Assume exactly one matching profit event per transaction. If your
///   executor contract can emit zero or multiple profit events in one
///   transaction, your implementation is responsible for defining and
///   documenting the aggregation rule.
///
/// **Implementer obligation:** net gas cost yourself
/// (`gas_used * effective_gas_price`) — the `RealizedProfit` you return
/// is taken as already net.
pub trait RealizedProfitDecoder: Send + Sync {
    /// Decode a confirmed receipt + its logs into a realized profit,
    /// net of gas cost, in the same unit as `PredictedProfit`.
    fn decode_realized(
        &self,
        receipt: &TxReceipt,
        logs: &[Log],
    ) -> Result<RealizedProfit, DecodeError>;
}

/// Given the full `(sim, realized)` history so far, decides whether to
/// halt.
///
/// Replaces hardcoded fast/slow guard constants. The default
/// implementation (fast guard + slow guard, "phantom-guard") ships in the
/// `reconcile` crate, but this is a default, not the only option — a team
/// can implement their own `HaltPolicy` entirely (e.g. a Bayesian
/// change-point detector) and keep everything else in the pipeline.
///
/// **Must not:**
/// - Panic on an empty or short history. `evaluate` is called starting
///   from the very first confirmed transaction, when `history.pairs.len()`
///   may be smaller than the policy's window size — return
///   `HaltDecision::Continue` in that case.
/// - Retain state that isn't reflected in `history`. `evaluate` takes
///   `&mut self` so a policy *can* keep internal state (e.g. a running
///   sum for efficiency), but that state must always be derivable from
///   `history` alone — never store information `history` doesn't also
///   contain, since a policy that can't be reconstructed from `history`
///   can't be replayed against historical data for the benchmark sweep.
pub trait HaltPolicy: Send + Sync {
    /// Given the full `(sim, realized)` history so far, decide whether to
    /// halt.
    fn evaluate(&mut self, history: &ReconcileHistory) -> HaltDecision;
}

/// Forks chain state and runs a candidate transaction through a
/// simulation backend, returning a [`RawSimOutput`] within a hard latency
/// budget.
///
/// This trait lives in `core` (rather than `revm-backend`) so that a team
/// with an existing simulation stack can implement it against their own
/// tooling and still get `reconcile` and `receipt-poller` for free — see
/// `docs/ARCHITECTURE.md`, "Swappable simulation backend". `core` itself
/// stays REVM/alloy-free: `Tx` and `Error` are associated types supplied
/// by the implementer (`revm-backend`'s `RevmBackend` fixes `Tx` to its
/// own candidate-transaction type and `Error` to [`SimError`]).
///
/// A conforming implementation is expected to uphold the three
/// load-bearing properties documented in `docs/ARCHITECTURE.md`:
/// 1. REVM/RPC work is off-loaded via `tokio::task::spawn_blocking`
///    rather than run directly on an async worker thread.
/// 2. Concurrent simulations are bounded (e.g. `buffer_unordered(N)`)
///    rather than fanned out unbounded against the RPC provider.
/// 3. The simulation timeout is derived as a configurable fraction of
///    block time, not a fixed constant.
#[async_trait]
pub trait SimEngine: Send + Sync {
    /// Backend-specific candidate-transaction type (e.g. an ABI-encoded
    /// call plus target address and value).
    type Tx: Send + Sync;

    /// Run `tx` against forked chain state and return the raw simulation
    /// output.
    async fn simulate(&self, tx: &Self::Tx) -> Result<RawSimOutput, SimError>;
}
