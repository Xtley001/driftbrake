//! `driftbrake`: chain-agnostic pre-flight simulation traits, plus a
//! drift-based halt guard that stops your strategy the moment realized
//! profit diverges from the simulated prediction — before drift compounds
//! into real capital loss.
//!
//! This is a **facade crate**: it re-exports [`driftbrake_core`],
//! [`driftbrake_reconcile`], and [`driftbrake_receipt_poller`] under one
//! dependency, so `cargo add driftbrake` gets you the chain-agnostic
//! traits, the default dual-guard halt policy, and receipt polling in one
//! shot.
//!
//! **The REVM simulation backend is intentionally not bundled here.**
//! `driftbrake-revm-backend` is a separate crate (`cargo add
//! driftbrake-revm-backend`) — see `docs/ARCHITECTURE.md`'s "swappable
//! simulation backend" design goal: a team with an existing simulation
//! stack implements [`SimEngine`] against their own tooling and should
//! never be forced to compile REVM just to depend on this crate for the
//! reconciliation and halt logic. If you *do* want the REVM backend
//! (most users will), add it alongside this crate — see the Quickstart
//! below.
//!
//! For the full mechanism specification and correctness argument, see
//! `docs/whitepaper.md`. For exact trait contracts, see `docs/API.md`.
//!
//! # Quickstart
//!
//! This example type-checks against the real API (see this crate's doc
//! tests) but is marked `no_run`, since it needs a live RPC endpoint to
//! actually execute — see `examples/toy-arbitrage` for a runnable
//! end-to-end version.
//!
//! ```no_run
//! use driftbrake::{
//!     DecodeError, HaltDecision, HaltPolicy, Log, PredictedProfit, ProfitDecoder,
//!     RawSimOutput, RealizedProfit, RealizedProfitDecoder, ReconcileHistory,
//!     ReconcilePolicy, SimEngine, TxReceipt, TxStatus,
//! };
//! // The REVM backend is a separate crate — `cargo add driftbrake-revm-backend`.
//! use driftbrake_revm_backend::{BlockContext, CandidateTx, DbFactory, RevmBackend};
//!
//! // 1. `SimEngine` needs a `DbFactory` — something that forks chain state.
//! //    Bring your own (a live RPC-backed one, or a test fixture); this
//! //    minimal stub is just enough to make the example self-contained.
//! # use revm::db::{CacheDB, EmptyDB};
//! #[derive(Clone)]
//! struct MyDbFactory;
//!
//! impl DbFactory for MyDbFactory {
//!     type Db = CacheDB<EmptyDB>;
//!     type Error = std::convert::Infallible;
//!     fn fresh_db(&self) -> Result<Self::Db, Self::Error> {
//!         Ok(CacheDB::new(EmptyDB::new()))
//!     }
//! }
//!
//! // 2. Implement `ProfitDecoder` / `RealizedProfitDecoder` for your
//! //    executor contract's specific ABI shape (see docs/API.md).
//! struct MyProfitDecoder;
//! impl ProfitDecoder for MyProfitDecoder {
//!     fn decode_predicted(&self, raw: &RawSimOutput) -> Result<PredictedProfit, DecodeError> {
//!         if raw.revert_reason.is_some() {
//!             return Err(DecodeError::Reverted);
//!         }
//!         Ok(PredictedProfit(0)) // decode your executor's real return value here
//!     }
//! }
//!
//! struct MyRealizedDecoder;
//! impl RealizedProfitDecoder for MyRealizedDecoder {
//!     fn decode_realized(&self, receipt: &TxReceipt, _logs: &[Log]) -> Result<RealizedProfit, DecodeError> {
//!         if receipt.status != TxStatus::Confirmed {
//!             return Err(DecodeError::Other("not confirmed".into()));
//!         }
//!         Ok(RealizedProfit(0)) // decode your Profit event and net gas cost here
//!     }
//! }
//!
//! # async fn submit_and_wait(_tx: &CandidateTx) -> anyhow::Result<TxReceipt> { unimplemented!() }
//! #
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let block = BlockContext { number: 0, timestamp: 0, gas_limit: 30_000_000, basefee: 0, coinbase: [0; 20] };
//!     let sim = RevmBackend::new(MyDbFactory, block, /* block_time_ms */ 12_000, /* timeout_fraction */ 0.5, /* concurrency */ 4);
//!     let predicted_decoder = MyProfitDecoder;
//!     let realized_decoder = MyRealizedDecoder;
//!     let mut guard = ReconcilePolicy::default_dual_guard();
//!     let mut history = ReconcileHistory::new();
//!
//!     let candidate_tx = CandidateTx { caller: [0; 20], to: [0; 20], value: 0, data: vec![], gas_limit: 100_000 };
//!     let raw = sim.simulate(&candidate_tx).await?;
//!     let predicted = match predicted_decoder.decode_predicted(&raw) {
//!         Ok(p) if p.0 > 0 => p,
//!         _ => return Ok(()), // not profitable in simulation (or reverted), skip submission
//!     };
//!
//!     let receipt = submit_and_wait(&candidate_tx).await?;
//!     let realized = realized_decoder.decode_realized(&receipt, &[])?;
//!     history.append(predicted, realized);
//!
//!     match guard.evaluate(&history) {
//!         HaltDecision::Continue => {}
//!         HaltDecision::Halt(reason) => {
//!             eprintln!("phantom-guard halted strategy: {reason:?}");
//!             std::process::exit(1);
//!         }
//!     }
//!     Ok(())
//! }
//! ```

pub use driftbrake_core::{
    DecodeError, HaltDecision, HaltPolicy, HaltReason, Log, PredictedProfit, ProfitDecoder,
    RawSimOutput, RealizedProfit, RealizedProfitDecoder, ReconcileHistory, RevertEvent, SimEngine,
    SimError, TxReceipt, TxStatus,
};

pub use driftbrake_reconcile::{FastGuardConfig, ReconcilePolicy, SlowGuardConfig};

pub use driftbrake_receipt_poller::{
    PollError, PollOutcome, PollerConfig, ReceiptPoller, ReceiptSource,
};
