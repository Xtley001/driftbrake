//! Chain-agnostic-at-the-edges input types for [`crate::RevmBackend`].
//!
//! These types are REVM-adjacent (they get converted into `revm_primitives`
//! types internally) but are defined here, not in `driftbrake-core`, because
//! `core` must stay REVM/alloy-free (see `docs/ARCHITECTURE.md`).

/// A candidate transaction to simulate: a single call from `caller` to `to`
/// with `value` and calldata `data`.
///
/// Contract creation and multi-call bundles are out of scope for this
/// minimal extraction — see `docs/ARCHITECTURE.md`'s non-goals. A
/// multicall/flash-loan-wrapped executor is expected to encode itself as a
/// single call to its own entrypoint, which is exactly what
/// [`driftbrake_core::ProfitDecoder`] downstream of this exists to decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateTx {
    pub caller: [u8; 20],
    pub to: [u8; 20],
    pub value: u128,
    pub data: Vec<u8>,
    pub gas_limit: u64,
}

/// The forked-block context every simulation runs against.
///
/// `RevmBackend` forks state *once* (at construction / re-fork time) and
/// reuses this same block context for every candidate transaction
/// simulated against that fork — it does not re-fetch a new block per
/// simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockContext {
    pub number: u64,
    pub timestamp: u64,
    pub gas_limit: u64,
    /// Base fee per gas, in wei. `0` is a valid pre-London value.
    pub basefee: u128,
    pub coinbase: [u8; 20],
}
