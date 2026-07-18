# Changelog

All notable changes to this project are documented in this file, in reverse-chronological order. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/); versioning follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- Design docs published as a browsable site at [xtley001.github.io/driftbrake](https://xtley001.github.io/driftbrake/), built from `docs/*.md` via mdBook and deployed automatically on push to `main` (`.github/workflows/docs.yml`).

### Planned
- `telemetry`: structured event emission (`HaltDecision`, and a `telemetry`-specific event shape for `core`'s `RawSimOutput`/`ReconcileHistory`) for the benchmark harness and other observability consumers.
- `benchmark`: a true 4-dimensional sweep over independent `(T_f, k_f, T_s, k_s)` combinations (the current tool sweeps a 1D slice with `T_s` derived from `T_f`), and a loader for real historical `(predicted, realized)` data in place of synthetic generation.

## [0.1.0] - 2026-07-16

### Added
- `core`: `ProfitDecoder`, `RealizedProfitDecoder`, `HaltPolicy`, `SimEngine` trait definitions, plus `PredictedProfit`, `RealizedProfit`, `RawSimOutput`, `TxReceipt`, `Log`, `ReconcileHistory`, `RevertEvent`, `HaltDecision`, `HaltReason`, `DecodeError`, `SimError`. Zero dependency on REVM or alloy.
- `revm-backend`: `RevmBackend`, a concrete `SimEngine` implementation — fork-and-simulate via a pluggable `DbFactory`, wrapped in `spawn_blocking`, with bounded concurrency and a block-time-relative timeout.
- `reconcile`: `ReconcilePolicy`, the default dual-guard `HaltPolicy` (fast guard + slow guard), with independently configurable thresholds and window sizes, plus the ratio-direction regression test.
- `receipt-poller`: `ReceiptPoller`, confirming receipts and booking realized profit only once both confirmed-status and a decodable profit event are present; confirmed-but-reverted transactions are recorded separately and never booked as zero-profit.
- `driftbrake`: a facade crate re-exporting `core` + `reconcile` + `receipt-poller` under a single dependency. Deliberately does not bundle `revm-backend`, so depending on `driftbrake` alone never forces a REVM dependency on a team using their own simulation stack.
- `benchmark`: the threshold-selection methodology from `docs/BENCHMARK.md` as a runnable CLI — synthetic healthy/drifted pair generation, a parameter sweep, and the resulting false-halt-rate / missed-catch-rate table.
- `examples/toy-arbitrage`: a minimal 2-pool arbitrage strategy wired end-to-end — pool-price reading, simulation, submission (local ECDSA signing + broadcast), receipt polling, and reconciliation — against a public testnet fork, plus an integration test proving the halt guard trips when drift is injected.

### Fixed
- `revm-backend`: EVM spec pinned to `SpecId::MERGE` — the default (Cancun-era) spec required EIP-4844 blob-gas fields that `BlockContext` doesn't model, which made every simulation fail validation before this was pinned.
- `examples/toy-arbitrage`: storage-slot and balance decoding in the JSON-RPC backend used a `u128` parse that silently failed on any value using bits above 128 — the common case for packed storage (e.g. a Uniswap-V2-shaped pair's `reserve0`/`reserve1`/`blockTimestampLast` all packed into one 256-bit slot). Replaced with a full-width 256-bit parse.
- `examples/toy-arbitrage`: transaction signing didn't normalize to low-S. Ethereum (EIP-2) rejects high-S signatures as invalid on broadcast, and `k256`'s recoverable signing does not normalize automatically — roughly half of all signed transactions would have silently failed to broadcast. Fixed by normalizing `s` and flipping the recovery ID's parity to match.
- `examples/toy-arbitrage`: the RLP encoder wrote a signature's `r`/`s` as fixed 32-byte strings instead of the minimal (leading-zero-trimmed) encoding RLP requires for integer fields, producing non-canonical output whenever either component had a leading zero byte (~1/256 of signatures per component). Fixed by trimming both like every other integer field.
- `reconcile`: a guard `window` of `0` was accepted silently and halted unconditionally on the very first `evaluate` call, including against an empty history (`recent_ratios(0)` is always empty, and "all ratios in an empty set are below threshold" is vacuously true). `ReconcilePolicy::new` now rejects `window == 0` outright.

### Changed
- `README.md`: Quickstart rewritten to match the real public API (it previously referenced a `driftbrake` crate that didn't exist yet, and several method/type names that didn't match the actual trait contracts). The example is now a doctest in the `driftbrake` crate, so it can't silently drift out of date again.
- `CONTRIBUTING.md`: workspace-layout table and the required regression-test command corrected to the actual crate names (`driftbrake-reconcile`, not `reconcile`).
