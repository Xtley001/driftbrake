# driftbrake

A chain-agnostic simulation and drift-based halt guard for algorithmic trading strategies.

[![PyPI](https://img.shields.io/pypi/v/driftbrake)](https://pypi.org/project/driftbrake/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/Xtley001/driftbrake/blob/main/LICENSE)
[![GitHub](https://img.shields.io/badge/github-driftbrake-black)](https://github.com/Xtley001/driftbrake)
[![Documentation](https://img.shields.io/badge/docs-design%20docs-3ddc84)](https://xtley001.github.io/driftbrake/)

`driftbrake` pre-flight simulates transactions against local chain state, tracks simulated versus realized profit upon on-chain confirmation, and halts execution the moment realized profit drifts from prediction. It prevents simulation drift, revert bursts, and silent capital bleed across algorithmic trading strategies.

## Installation

```bash
pip install driftbrake
```

## Quickstart

```python
from driftbrake import ReconcileHistory, ReconcilePolicy, StrategyHaltedError

# Configure institutional multi-guard policy
policy = ReconcilePolicy(
    fast_window=3,
    fast_threshold=0.50,
    slow_window=20,
    slow_threshold=0.70,
    max_consecutive_reverts=3,
    max_revert_gas_budget=500_000,
    volume_weighted_window=15,
    volume_weighted_threshold=0.65,
)
history = ReconcileHistory()

# In your execution loop:
# Append predicted vs realized profit pairs (in wei or native units)
history.append(predicted_profit=1_000_000, realized_profit=950_000)

# Evaluate halt decision:
decision = policy.evaluate(history)
if decision.is_halted:
    raise StrategyHaltedError(f"Strategy halted: {decision.reason}")
```

## Vectorized Backtesting Sweep

Run parameter sweeps over millions of historical trade rows in microsecond compiled Rust time:

```python
from driftbrake import run_sweep_raw

# pairs: list of (predicted_profit, realized_profit)
results = run_sweep_raw(
    pairs,
    fast_windows=[2, 3, 4],
    fast_thresholds=[0.40, 0.50, 0.60],
    slow_windows=[15, 20, 25],
    slow_thresholds=[0.65, 0.70, 0.75],
)

for r in results:
    if not r.halted:
        print(f"Optimal parameters: fast={r.fast_window}/{r.fast_threshold}, slow={r.slow_window}/{r.slow_threshold}")
```

## Documentation & Whitepaper

- **Whitepaper & Mathematical Invariants**: [xtley001.github.io/driftbrake/whitepaper.html](https://xtley001.github.io/driftbrake/whitepaper.html)
- **Architecture & System Design**: [xtley001.github.io/driftbrake/ARCHITECTURE.html](https://xtley001.github.io/driftbrake/ARCHITECTURE.html)
- **GitHub Repository**: [github.com/Xtley001/driftbrake](https://github.com/Xtley001/driftbrake)

## License

Released under the [MIT License](https://github.com/Xtley001/driftbrake/blob/main/LICENSE).
