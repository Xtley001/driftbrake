# Changelog

All notable changes to this project are documented in this file, in reverse-chronological order. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/); versioning follows [Semantic Versioning](https://semver.org/).

## [0.2.0] - 2026-10-09

### Added
- `core`: `HistoryEntry` enum to track chronological timeline interleaving confirmed profit pairs and revert events.
- `core`: `RevertEvent` extended with `gas_used` and `effective_gas_price` fields for cumulative revert gas accounting.
- `core`: `HaltReason` extended with `RevertBurst`, `RevertGasBudgetExceeded`, `VolumeWeightedDrift`, and `NetDrawdownExceeded`.
- `reconcile`: Institutional multi-guard hierarchy:
  - `RevertBurstConfig` tracking consecutive reverts.
  - `RevertGasConfig` tracking rolling window gas burned on reverts.
  - `VolumeWeightedConfig` (VWAR) weighting profit ratios by capital magnitude.
  - `DrawdownConfig` enforcing peak high-watermark drawdown caps.
  - `ReconcilePolicy::institutional_default()` constructor.
- `journal`: New crate `driftbrake-journal` implementing a crash-resilient binary Write-Ahead Log (WAL) with IEEE 802.3 CRC32 verification, torn-write truncation recovery, and exclusive process locks.
- `driftbrake-py`: New native Python extension (`pip install driftbrake`) built with PyO3 and CPython Stable ABI (`abi3-py310`), exposing `ReconcileHistory`, `ReconcilePolicy`, `HaltDecision`, and `run_sweep_raw` vectorized parameter backtest sweeps.
- `driftbrake`: Facade crate re-exports `driftbrake_journal::{JournalError, JournaledHistory}`.
- `docs`: Published `llms.txt` Generative Engine Optimization index for AI tools and web indexers.
- `docs`: Injected OpenGraph, Twitter cards, and Schema.org `SoftwareSourceCode` JSON-LD structured data into mdBook documentation.

### Changed
- `receipt-poller`: Updated to populate `gas_used` and `effective_gas_price` on recorded `RevertEvent` instances.
- `README.md`: Reformatted to DeFi-grade production documentation standard with progressive disclosure.
- All workspace crates: Added official `categories` metadata for crates.io discovery.

## [0.1.0] - 2026-07-16

### Added
- `core`: `ProfitDecoder`, `RealizedProfitDecoder`, `HaltPolicy`, `SimEngine` trait definitions, plus `PredictedProfit`, `RealizedProfit`, `RawSimOutput`, `TxReceipt`, `Log`, `ReconcileHistory`, `RevertEvent`, `HaltDecision`, `HaltReason`, `DecodeError`, `SimError`. Zero dependency on REVM or alloy.
- `revm-backend`: `RevmBackend`, a concrete `SimEngine` implementation — fork-and-simulate via a pluggable `DbFactory`, wrapped in `spawn_blocking`, with bounded concurrency and a block-time-relative timeout.
- `reconcile`: `ReconcilePolicy`, the default dual-guard `HaltPolicy` (fast guard + slow guard), with independently configurable thresholds and window sizes, plus the ratio-direction regression test.
- `receipt-poller`: `ReceiptPoller`, confirming receipts and booking realized profit only once both confirmed-status and a decodable profit event are present; confirmed-but-reverted transactions are recorded separately and never booked as zero-profit.
- `driftbrake`: A facade crate re-exporting `core` + `reconcile` + `receipt-poller` under a single dependency.
- `benchmark`: The threshold-selection methodology from `docs/BENCHMARK.md` as a runnable CLI.
- `examples/toy-arbitrage`: A minimal 2-pool arbitrage strategy wired end-to-end against a public testnet fork.

### Fixed
- `revm-backend`: EVM spec pinned to `SpecId::MERGE` to avoid unmodeled EIP-4844 blob-gas field validation failures.
- `examples/toy-arbitrage`: Fixed storage-slot and balance decoding to full 256-bit width to handle packed Uniswap v2 pair storage.
- `examples/toy-arbitrage`: Added low-S signature normalization and parity flipping for EIP-2 broadcast compliance.
- `examples/toy-arbitrage`: Canonicalized integer RLP encoding for signature components.
- `reconcile`: Guard window of `0` rejected at initialization to prevent vacuum-state halts.
