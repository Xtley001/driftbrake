use thiserror::Error;

/// Shared error vocabulary for [`crate::ProfitDecoder`] and
/// [`crate::RealizedProfitDecoder`].
///
/// Both traits are fundamentally "turn raw EVM output into a profit figure"
/// operations and benefit from a consistent error vocabulary across the
/// pipeline's telemetry (see `docs/API.md`).
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// The simulation (or, for a realized decode, the receipt) reverted.
    #[error("reverted")]
    Reverted,
    /// `return_data` / log data did not match the shape the decoder expected.
    #[error("malformed data: {0}")]
    MalformedData(String),
    /// A `RealizedProfitDecoder` implementation expected a profit event in
    /// the receipt's logs and did not find one.
    #[error("missing profit event")]
    MissingProfitEvent,
    /// Catch-all for implementer-specific decode failures.
    #[error("{0}")]
    Other(String),
}

/// Error type returned by a [`crate::SimEngine`] implementation.
///
/// `core` does not know anything about *why* a simulation backend might
/// fail (RPC error, timeout, node desync, ...) — that's backend-specific —
/// so this is deliberately a thin, opaque wrapper rather than an attempt to
/// enumerate every possible backend failure mode.
#[derive(Debug, Error)]
pub enum SimError {
    /// The simulation did not complete within its configured timeout
    /// (see `docs/ARCHITECTURE.md` — timeout is a fraction of block time).
    #[error("simulation timed out after {0:?}")]
    Timeout(std::time::Duration),
    /// Backend-specific failure (RPC error, fork setup failure, etc.),
    /// carried as an opaque boxed error so `core` never needs to know the
    /// concrete backend error type.
    #[error("simulation backend error: {0}")]
    Backend(#[from] Box<dyn std::error::Error + Send + Sync>),
}
