//! Concrete [`driftbrake_core::SimEngine`] implementation backed by REVM.
//!
//! This module exists to preserve, exactly, three fixes documented as
//! load-bearing in `docs/ARCHITECTURE.md`:
//!
//! 1. REVM execution runs inside `tokio::task::spawn_blocking`, so it never
//!    blocks an async worker thread (and therefore never starves the
//!    main block-processing loop or other concurrent simulations sharing
//!    that thread pool).
//! 2. Concurrent simulations are bounded by a semaphore sized to the RPC
//!    provider's actual rate limit, not fanned out unbounded.
//! 3. The simulation timeout is derived as a configurable fraction of
//!    block time, not a fixed millisecond constant, so the same backend
//!    behaves correctly on both a 400ms-block chain and a 12s-block chain.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use revm::primitives::{Address, BlockEnv, Bytes, ExecutionResult, TransactTo, TxEnv, U256};
use revm::{Database, EVM};
use tokio::sync::Semaphore;

use driftbrake_core::{Log, RawSimOutput, SimEngine, SimError};

use crate::tx::{BlockContext, CandidateTx};

/// Produces a fresh, independent database for a single simulation.
///
/// Each call to [`RevmBackend::simulate`] gets its own `DB` instance from
/// this factory — REVM execution mutates its database's journal in place,
/// and candidate-transaction simulations must never see each other's
/// speculative state changes. A production implementation of this trait
/// wraps a live RPC connection (fetching account/storage state lazily,
/// cached per block); the unit tests in this module use an in-memory
/// fixture instead, since exercising a real archive-node RPC fork is
/// `examples/toy-arbitrage`'s job, not this crate's unit-test suite.
pub trait DbFactory: Send + Sync {
    type Db: Database + Send + 'static;
    type Error: std::error::Error + Send + Sync + 'static;

    fn fresh_db(&self) -> Result<Self::Db, Self::Error>;
}

/// `SimEngine` backed by an in-process REVM instance.
///
/// Forks chain state (via `F::fresh_db`) and runs a [`CandidateTx`]
/// through REVM within a hard latency budget derived from `block_time_ms`.
pub struct RevmBackend<F: DbFactory> {
    db_factory: F,
    block: BlockContext,
    block_time_ms: u64,
    /// Fraction of `block_time_ms` allotted to a single simulation before
    /// it is timed out. Kept explicit and tunable rather than hardcoded
    /// (`docs/ARCHITECTURE.md`, "Timeout as a fraction of block time").
    timeout_fraction: f64,
    concurrency: Arc<Semaphore>,
}

impl<F: DbFactory> RevmBackend<F> {
    /// # Panics
    /// Panics if `timeout_fraction` is not in `(0.0, 1.0]` or if
    /// `concurrency_limit` is `0` — both are programmer errors, not
    /// runtime conditions, so a panic at construction time is preferable
    /// to a silently-misbehaving backend.
    pub fn new(
        db_factory: F,
        block: BlockContext,
        block_time_ms: u64,
        timeout_fraction: f64,
        concurrency_limit: usize,
    ) -> Self {
        assert!(
            timeout_fraction > 0.0 && timeout_fraction <= 1.0,
            "timeout_fraction must be in (0.0, 1.0], got {timeout_fraction}"
        );
        assert!(concurrency_limit > 0, "concurrency_limit must be > 0");
        Self {
            db_factory,
            block,
            block_time_ms,
            timeout_fraction,
            concurrency: Arc::new(Semaphore::new(concurrency_limit)),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_millis((self.block_time_ms as f64 * self.timeout_fraction) as u64)
    }

    fn block_env(&self) -> BlockEnv {
        BlockEnv {
            number: U256::from(self.block.number),
            coinbase: Address::from(self.block.coinbase),
            timestamp: U256::from(self.block.timestamp),
            gas_limit: U256::from(self.block.gas_limit),
            basefee: U256::from(self.block.basefee),
            difficulty: U256::ZERO,
            prevrandao: Some(Default::default()),
            blob_excess_gas_and_price: None,
        }
    }
}

fn tx_env(tx: &CandidateTx, basefee: u128) -> TxEnv {
    TxEnv {
        caller: Address::from(tx.caller),
        gas_limit: tx.gas_limit,
        gas_price: U256::from(basefee),
        transact_to: TransactTo::Call(Address::from(tx.to)),
        value: U256::from(tx.value),
        data: Bytes::from(tx.data.clone()),
        nonce: None,
        chain_id: None,
        access_list: Vec::new(),
        gas_priority_fee: None,
        blob_hashes: Vec::new(),
        max_fee_per_blob_gas: None,
    }
}

fn convert_result(result: ExecutionResult) -> RawSimOutput {
    match result {
        ExecutionResult::Success {
            gas_used,
            logs,
            output,
            ..
        } => RawSimOutput {
            return_data: output.into_data().to_vec(),
            gas_used,
            revert_reason: None,
            logs: logs.into_iter().map(convert_log).collect(),
        },
        ExecutionResult::Revert { gas_used, output } => RawSimOutput {
            return_data: output.to_vec(),
            gas_used,
            revert_reason: Some("reverted".to_string()),
            logs: Vec::new(),
        },
        ExecutionResult::Halt { reason, gas_used } => RawSimOutput {
            return_data: Vec::new(),
            gas_used,
            revert_reason: Some(format!("halted: {reason:?}")),
            logs: Vec::new(),
        },
    }
}

fn convert_log(log: revm::primitives::Log) -> Log {
    Log {
        address: log.address.0.to_vec(),
        topics: log.topics.into_iter().map(|t| t.0).collect(),
        data: log.data.to_vec(),
    }
}

#[async_trait]
impl<F> SimEngine for RevmBackend<F>
where
    F: DbFactory + Clone + Send + Sync + 'static,
    <F::Db as Database>::Error: std::error::Error + Send + Sync + 'static,
{
    type Tx = CandidateTx;

    async fn simulate(&self, tx: &Self::Tx) -> Result<RawSimOutput, SimError> {
        // Fix 2: bounded concurrency. A block can surface 50+ candidate
        // opportunities needing simulation; this permit caps how many run
        // concurrently against the RPC-backed database, rather than
        // firing all of them at once and saturating the provider's rate
        // limit (docs/ARCHITECTURE.md).
        let permit = self
            .concurrency
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| SimError::Backend(Box::new(e)))?;

        let tx_env = tx_env(tx, self.block.basefee);
        let block_env = self.block_env();
        let db_factory = self.db_factory.clone();

        // Fix 1: spawn_blocking. REVM execution (and, once `db_factory`
        // wraps a live RPC client, the synchronous provider calls it
        // makes) is CPU/IO-blocking work. Running it directly on an async
        // worker thread would starve every other task sharing that
        // thread pool, including the main block-processing loop
        // (docs/ARCHITECTURE.md).
        let blocking = tokio::task::spawn_blocking(move || {
            let _permit = permit; // held for the duration of the blocking work
            let db = db_factory
                .fresh_db()
                .map_err(|e| SimError::Backend(Box::new(e)))?;

            let mut evm: EVM<F::Db> = EVM::new();
            // Pinned to MERGE rather than the default (Cancun-era) spec:
            // `BlockContext` doesn't model EIP-4844 blob-gas fields that
            // Shanghai/Cancun validation requires, and the default spec
            // would reject every simulation on that basis. Revisit once
            // a strategy needs post-Merge fields.
            evm.env.cfg.spec_id = revm::primitives::SpecId::MERGE;
            evm.env.block = block_env;
            evm.env.tx = tx_env;
            evm.db = Some(db);

            let result_and_state = evm.transact().map_err(|e| SimError::Backend(Box::new(e)))?;

            Ok::<RawSimOutput, SimError>(convert_result(result_and_state.result))
        });

        // Fix 3: timeout as a fraction of block time, not a fixed
        // constant, so the same backend is correct on both a fast-block
        // and a slow-block chain (docs/ARCHITECTURE.md).
        match tokio::time::timeout(self.timeout(), blocking).await {
            Ok(Ok(inner)) => inner,
            Ok(Err(join_err)) => Err(SimError::Backend(Box::new(join_err))),
            Err(_elapsed) => Err(SimError::Timeout(self.timeout())),
        }
    }
}

/// Test-only fixture: an in-memory `DbFactory` seeded with a fixed set of
/// accounts, used so the three load-bearing fixes above can be exercised
/// deterministically without a live RPC connection.
#[cfg(test)]
pub(crate) mod test_fixture {
    use revm::db::{CacheDB, EmptyDB};
    use revm::primitives::{AccountInfo, Address, Bytecode, Bytes as RevmBytes, U256};

    use super::DbFactory;

    #[derive(Clone, Default)]
    pub struct InMemoryDbFactory {
        accounts: Vec<(Address, AccountInfo)>,
    }

    impl InMemoryDbFactory {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn with_account(mut self, address: [u8; 20], code: Vec<u8>, balance: u128) -> Self {
            let info = AccountInfo {
                balance: U256::from(balance),
                nonce: 0,
                code_hash: revm::primitives::keccak256(&code),
                code: Some(Bytecode::new_raw(RevmBytes::from(code))),
            };
            self.accounts.push((Address::from(address), info));
            self
        }
    }

    impl DbFactory for InMemoryDbFactory {
        type Db = CacheDB<EmptyDB>;
        type Error = std::convert::Infallible;

        fn fresh_db(&self) -> Result<Self::Db, Self::Error> {
            let mut db = CacheDB::new(EmptyDB::new());
            for (address, info) in &self.accounts {
                db.insert_account_info(*address, info.clone());
            }
            Ok(db)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_fixture::InMemoryDbFactory;
    use super::*;

    fn block() -> BlockContext {
        BlockContext {
            number: 19_000_000,
            timestamp: 1_700_000_000,
            gas_limit: 30_000_000,
            basefee: 10_000_000_000,
            coinbase: [0u8; 20],
        }
    }

    // The EVM "identity" precompile at address 0x...04 always returns
    // exactly the calldata it was given. It's a real, always-available
    // piece of EVM behavior (no bytecode needed), which makes it a
    // convenient deterministic fixture for exercising `simulate()`
    // end-to-end without depending on a specific test contract.
    fn identity_precompile_address() -> [u8; 20] {
        let mut addr = [0u8; 20];
        addr[19] = 4;
        addr
    }

    #[tokio::test]
    async fn simulate_runs_a_call_through_revm_and_returns_output() {
        let backend = RevmBackend::new(
            InMemoryDbFactory::new().with_account([0xAAu8; 20], Vec::new(), 10u128.pow(20)),
            block(),
            /* block_time_ms */ 12_000,
            /* timeout_fraction */ 0.5,
            /* concurrency_limit */ 4,
        );

        let tx = CandidateTx {
            caller: [0xAAu8; 20],
            to: identity_precompile_address(),
            value: 0,
            data: vec![1, 2, 3, 4],
            gas_limit: 100_000,
        };

        let out = backend.simulate(&tx).await.expect("simulation succeeds");
        assert!(out.revert_reason.is_none());
        assert_eq!(out.return_data, vec![1, 2, 3, 4]);
    }

    #[tokio::test]
    async fn simulate_respects_bounded_concurrency() {
        // Concurrency limit of 1: two simultaneous simulate() calls must
        // not both be "in flight" against the blocking pool at once. We
        // can't observe the semaphore directly from here, but we can
        // confirm both complete successfully when serialized behind a
        // limit of 1, which is what bounded (not unbounded) concurrency
        // guarantees — this is a smoke test for the wiring, not a timing
        // assertion.
        let backend = Arc::new(RevmBackend::new(
            InMemoryDbFactory::new().with_account([0xAAu8; 20], Vec::new(), 10u128.pow(20)),
            block(),
            12_000,
            0.5,
            1,
        ));

        let tx = CandidateTx {
            caller: [0xAAu8; 20],
            to: identity_precompile_address(),
            value: 0,
            data: vec![9, 9],
            gas_limit: 100_000,
        };

        let (a, b) = tokio::join!(backend.simulate(&tx), backend.simulate(&tx));
        assert_eq!(a.unwrap().return_data, vec![9, 9]);
        assert_eq!(b.unwrap().return_data, vec![9, 9]);
    }

    #[tokio::test]
    async fn simulate_times_out_when_backend_never_completes() {
        // A DbFactory whose fresh_db() call blocks forever must still be
        // bounded by the configured timeout (fraction of block time),
        // not hang the caller indefinitely.
        #[derive(Clone)]
        struct HangingDbFactory;

        impl DbFactory for HangingDbFactory {
            type Db = revm::db::CacheDB<revm::db::EmptyDB>;
            type Error = std::convert::Infallible;

            fn fresh_db(&self) -> Result<Self::Db, Self::Error> {
                std::thread::sleep(Duration::from_secs(5));
                Ok(revm::db::CacheDB::new(revm::db::EmptyDB::new()))
            }
        }

        let backend = RevmBackend::new(
            HangingDbFactory,
            block(),
            /* block_time_ms */ 100,
            /* timeout_fraction */ 0.5, // => 50ms timeout, well under the 5s hang
            1,
        );

        let tx = CandidateTx {
            caller: [0xAAu8; 20],
            to: identity_precompile_address(),
            value: 0,
            data: vec![],
            gas_limit: 100_000,
        };

        let err = backend.simulate(&tx).await.expect_err("must time out");
        assert!(matches!(err, SimError::Timeout(_)));
    }
}
