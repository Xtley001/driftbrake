# driftbrake

A chain-agnostic simulation and drift-based halt guard for algorithmic trading strategies.

[![CI](https://github.com/Xtley001/driftbrake/actions/workflows/ci.yml/badge.svg)](https://github.com/Xtley001/driftbrake/actions)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)
[![crates.io](https://img.shields.io/crates/v/driftbrake)](https://crates.io/crates/driftbrake)
[![PyPI](https://img.shields.io/pypi/v/driftbrake)](https://pypi.org/project/driftbrake/)
[![docs.rs](https://img.shields.io/docsrs/driftbrake)](https://docs.rs/driftbrake)
[![book](https://img.shields.io/badge/book-design%20docs-3ddc84)](https://xtley001.github.io/driftbrake/)

`driftbrake` pre-flight simulates transactions against local chain state, tracks simulated versus realized profit upon on-chain confirmation, and halts execution the moment realized profit drifts from prediction. It prevents simulation drift, revert bursts, and silent capital bleed across high-frequency EVM strategies. For the complete mechanism specification and formal invariant proofs, see the [whitepaper](./docs/whitepaper.md).

## Installation

### Rust
```bash
cargo add driftbrake
# Optional concrete REVM simulation backend:
cargo add driftbrake-revm-backend
```

### Python
```bash
pip install driftbrake
```

## Quickstart

### Rust
```rust
use driftbrake::{
    HaltDecision, PredictedProfit, RealizedProfit, ReconcileHistory, ReconcilePolicy,
};

let mut policy = ReconcilePolicy::institutional_default();
let mut history = ReconcileHistory::new();

// In your execution loop: record pre-flight simulated profit vs actual confirmed profit
history.append(PredictedProfit(1_000_000), RealizedProfit(950_000));

match policy.evaluate(&history) {
    HaltDecision::Continue => {
        // Safe to proceed with trading strategy
    }
    HaltDecision::Halt(reason) => {
        // Immediately halt execution before capital bleed compounds
        eprintln!("Strategy halted: {reason:?}");
        std::process::exit(1);
    }
}
```

For the full end-to-end simulation loop wired to REVM, receipt polling, and local transaction broadcast, see [`examples/toy-arbitrage`](./examples/toy-arbitrage).

### Python
```python
from driftbrake import ReconcileHistory, ReconcilePolicy, StrategyHaltedError

policy = ReconcilePolicy(
    fast_window=3,
    fast_threshold=0.50,
    slow_window=20,
    slow_threshold=0.70,
    max_consecutive_reverts=3,
)
history = ReconcileHistory()

# In your execution loop:
history.append(predicted_profit=1_000_000, realized_profit=950_000)

decision = policy.evaluate(history)
if decision.is_halted:
    raise StrategyHaltedError(f"Strategy halted: {decision.reason}")
```

## Workspace Packages

| Crate / Package | Description | Target |
|---|---|---|
| `driftbrake` | Facade crate re-exporting core, reconcile, receipt-poller, and journal | crates.io |
| `driftbrake-core` | Chain-agnostic trait boundaries (`ProfitDecoder`, `HaltPolicy`, `SimEngine`) | crates.io |
| `driftbrake-reconcile` | Multi-guard engine (Fast/Slow drift, Revert-Burst, VWAR, Net Drawdown) | crates.io |
| `driftbrake-journal` | Crash-resilient binary Write-Ahead Log (WAL) with CRC32 integrity checks | crates.io |
| `driftbrake-receipt-poller` | Block-time-aware receipt polling and profit realization gating | crates.io |
| `driftbrake-revm-backend` | Concrete REVM simulation backend with bounded RPC concurrency | crates.io |
| `driftbrake-py` | Compiled CPython Stable ABI native module and backtest sweep engine | PyPI |
| `driftbrake-benchmark` | Synthetic drift generator and false-halt vs. missed-catch sweep tool | CLI / crate |

## Architecture

```
driftbrake/
├── driftbrake/            # facade crate: re-exports core + reconcile + poller + journal
│   ├── core/              # chain-agnostic traits, zero REVM/alloy dependency
│   ├── reconcile/         # multi-guard engine (Fast/Slow, Revert-Burst, VWAR, Drawdown)
│   ├── journal/           # crash-resilient binary Write-Ahead Log (WAL) with CRC32
│   └── receipt-poller/    # confirms receipts, decodes realized profit
├── driftbrake-py/         # official Python native extension (PyO3, ABI3 Stable, pip)
├── revm-backend/          # separate, opt-in: concrete SimEngine (fork-and-simulate)
├── benchmark/             # parameter sweep & false-halt/missed-catch harness
└── examples/
    └── toy-arbitrage/     # minimal 2-pool arb wired end-to-end against a testnet fork
```

For the module breakdown and why the guards are independently necessary, see [`docs/ARCHITECTURE.md`](./docs/ARCHITECTURE.md).

## Testing

```bash
# Run Rust workspace test suite
cargo test --workspace

# Run Python integration test suite
python tests/python/test_driftbrake.py
```

## Security

Report vulnerabilities per our [security policy](./SECURITY.md). This code has not been independently audited — review the `reconcile` and `HaltPolicy` logic yourself before running against real capital.

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md) for dev environment setup and PR guidelines.

## License

Released under the [MIT License](./LICENSE).
