//! Library surface for `toy-arbitrage`, so `tests/` can exercise the
//! whole pipeline end-to-end (see README.md: "serve as the first
//! integration test of the whole pipeline ... rather than testing each
//! piece in isolation") without needing a live RPC endpoint, by wiring
//! `driftbrake`'s traits to in-memory fixtures instead of the `rpc`
//! module's live-network implementations.

pub mod abi;
pub mod decoder;
pub mod pools;
pub mod rlp;
pub mod rpc;
