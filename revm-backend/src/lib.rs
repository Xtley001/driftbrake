//! `driftbrake-revm-backend`: the only crate in the workspace allowed to
//! depend on REVM directly (see `CONTRIBUTING.md`'s workspace layout).
//!
//! Provides [`RevmBackend`], a concrete [`driftbrake_core::SimEngine`]
//! implementation, plus the [`CandidateTx`] / [`BlockContext`] types it
//! takes as input.

mod backend;
mod tx;

pub use backend::{DbFactory, RevmBackend};
pub use tx::{BlockContext, CandidateTx};

/// Keccak-256, exposed as a plain `[u8; 32]` so downstream crates (e.g.
/// `examples/toy-arbitrage`, for ABI selectors and event topics) don't
/// need to depend on `revm` directly just to hash something.
pub fn keccak256(data: impl AsRef<[u8]>) -> [u8; 32] {
    revm::primitives::keccak256(data).0
}
