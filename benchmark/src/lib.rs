//! `driftbrake-benchmark`: the threshold-selection methodology from
//! `docs/BENCHMARK.md`, as a runnable tool — synthetic pair generation,
//! a parameter sweep, and the resulting false-halt-rate /
//! missed-catch-rate curve.
//!
//! See `docs/BENCHMARK.md` for the methodology this implements, and this
//! crate's `src/main.rs` for the CLI that runs it.

pub mod rng;
pub mod sweep;
pub mod synthetic;
