# Contributing to driftbrake

Thanks for considering a contribution. This project is intentionally focused in scope (see [Non-goals](./docs/ARCHITECTURE.md#non-goals)) — working within that scope is the fastest path to getting a pull request merged.

## Development Setup

```bash
git clone https://github.com/Xtley001/driftbrake.git
cd driftbrake
rustup show                # verify toolchain matches rust-toolchain.toml
cargo build --workspace
cargo test --workspace
```

### Requirements

- **Rust toolchain**, pinned per `rust-toolchain.toml` in the repository root.
- **Python 3.10+** (if building or testing `driftbrake-py`).
- **Archive RPC endpoint** (if working on `revm-backend` or running `examples/toy-arbitrage` against a live fork).

## Workspace Layout

The repository is structured as a Cargo workspace with strict crate boundaries:

```
driftbrake/          # Facade crate: re-exports core, reconcile, receipt-poller, journal
core/                # Trait definitions only — zero REVM, alloy, or RPC dependencies
reconcile/           # Multi-guard HaltPolicy engine (fast/slow, revert-burst, VWAR, drawdown)
journal/             # Crash-resilient binary Write-Ahead Log (WAL) with CRC32
receipt-poller/      # Receipt confirmation and profit-realization gating
revm-backend/        # Opt-in concrete SimEngine implementation (REVM fork-and-simulate)
driftbrake-py/       # PyO3 CPython Stable ABI native extension and backtesting sweep engine
benchmark/           # Parameter sweep tool and false-halt vs. missed-catch curve generator
examples/            # End-to-end strategy demonstrations (publish = false)
```

Adding dependencies to `core` is prohibited. `core`'s zero-dependency guarantee is what makes `revm-backend` swappable across different execution backends.

## Contribution Boundaries

| Modifying | Target Crate | Review Focus |
|---|---|---|
| Trait signatures (`ProfitDecoder`, `HaltPolicy`, `SimEngine`) | `core` | Public contract impact on all downstream crates |
| Guard math, threshold evaluation, drawdown logic | `reconcile` | Invariant proofs, ratio direction, property tests |
| Write-Ahead Log framing, CRC32, crash recovery | `journal` | Torn-write resilience, cross-platform locking |
| Receipt polling, status gating, gas decoding | `receipt-poller` | Asynchronous timing, block-time relative timeouts |
| REVM forking, `spawn_blocking`, RPC concurrency | `revm-backend` | Worker-thread non-starvation, bounded concurrency |
| Python bindings, PyO3 wrappers, backtest engine | `driftbrake-py` | ABI3 compatibility, GIL release, zero-copy safety |
| Benchmark generation, synthetic noise models | `benchmark` | Mathematical reproducibility, parameter grid |

## Pre-PR Checklist

- **Run workspace tests**: `cargo test --workspace`.
- **Run Python tests**: `python tests/python/test_driftbrake.py`.
- **Verify ratio-direction invariant**: `cargo test -p driftbrake-reconcile ratio_direction`.
- **Run linter and formatter**: `cargo fmt --check` and `cargo clippy --workspace --all-targets`.
- **Update documentation**: ensure matching updates in `docs/` and `CHANGELOG.md` under `[Unreleased]`.

## Non-Goals

Pull requests adding the following items will be declined per project scope:

- **Alerting channels**: Telegram, Discord, Slack, PagerDuty, or webhook integrations. Alerting belongs in external infrastructure monitoring events.
- **Wallet management**: EOA nonces, private key custody, or transaction broadcasting orchestration.
- **Price oracles**: Fiat or USD conversions inside `core` or `revm-backend`.
- **Inventory unwinding**: Position unwind algorithms are deferred to external portfolio engines.

## Publishing a Release

Maintainer release workflow requires publishing crates in strict dependency order:

```bash
cargo publish -p driftbrake-core
cargo publish -p driftbrake-reconcile
cargo publish -p driftbrake-journal
cargo publish -p driftbrake-receipt-poller
cargo publish -p driftbrake-revm-backend
cargo publish -p driftbrake-benchmark
cargo publish -p driftbrake
```

For the Python distribution:

```bash
cd driftbrake-py
maturin build --release
twine upload target/wheels/*
```

## Security Issues

Do not open public GitHub issues for security vulnerabilities. See [`SECURITY.md`](./SECURITY.md) for private disclosure instructions.
