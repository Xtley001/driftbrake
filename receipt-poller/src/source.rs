use std::time::Duration;

use async_trait::async_trait;

use driftbrake_core::{Log, TxReceipt};

/// Where `receipt-poller` gets receipts and logs from.
///
/// Deliberately a trait rather than a hardcoded RPC client, for the same
/// reason `driftbrake_core::SimEngine` is generic over its database
/// factory: this crate's job is the polling *loop* and the confirmed
/// status / decodable-profit-event gating logic (`docs/ARCHITECTURE.md`'s
/// "receipt-poller" section), not a specific JSON-RPC client integration.
/// A production implementation wraps a live provider (`alloy`, `ethers`,
/// a raw JSON-RPC client); this crate's tests use an in-memory fixture.
#[async_trait]
pub trait ReceiptSource: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Look up the current receipt + logs for `tx_hash`, if the chain
    /// knows about it yet.
    ///
    /// Returns `Ok(None)` for "not yet mined" — this is a normal,
    /// expected outcome during polling, not an error.
    async fn get_receipt(
        &self,
        tx_hash: [u8; 32],
    ) -> Result<Option<(TxReceipt, Vec<Log>)>, Self::Error>;
}

/// Polling cadence and timeout, both expressed as a function of block
/// time rather than a fixed millisecond constant — the same reasoning as
/// `driftbrake-revm-backend`'s `SimEngine` timeout: a value tuned for one
/// chain's block time doesn't transfer to a chain with a meaningfully
/// different one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PollerConfig {
    block_time_ms: u64,
    /// Poll interval, as a fraction of block time. `0.5` on a 12s-block
    /// chain polls every 6s.
    poll_interval_fraction: f64,
    /// How many block-times to wait for a confirmed receipt before
    /// giving up on this transaction.
    confirmation_timeout_blocks: u64,
}

impl PollerConfig {
    /// # Panics
    /// Panics if `poll_interval_fraction` is not in `(0.0, 1.0]`, or if
    /// `confirmation_timeout_blocks` or `block_time_ms` is `0` — all are
    /// programmer errors, not runtime conditions.
    pub fn new(
        block_time_ms: u64,
        poll_interval_fraction: f64,
        confirmation_timeout_blocks: u64,
    ) -> Self {
        assert!(block_time_ms > 0, "block_time_ms must be > 0");
        assert!(
            poll_interval_fraction > 0.0 && poll_interval_fraction <= 1.0,
            "poll_interval_fraction must be in (0.0, 1.0], got {poll_interval_fraction}"
        );
        assert!(
            confirmation_timeout_blocks > 0,
            "confirmation_timeout_blocks must be > 0"
        );
        Self {
            block_time_ms,
            poll_interval_fraction,
            confirmation_timeout_blocks,
        }
    }

    pub fn poll_interval(&self) -> Duration {
        Duration::from_millis((self.block_time_ms as f64 * self.poll_interval_fraction) as u64)
    }

    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.block_time_ms * self.confirmation_timeout_blocks)
    }
}
