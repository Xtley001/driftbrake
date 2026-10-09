"""
driftbrake: Chain-agnostic pre-flight simulation and drift-based halt guard
for simulation-driven trading strategies and MEV searchers.
"""

from typing import List, Optional, Dict, Any

try:
    from driftbrake.driftbrake_rs import (
        ReconcileHistory,
        ReconcilePolicy,
        HaltDecision,
        SweepResult,
        StrategyHaltedError,
        run_sweep_raw,
    )
except ImportError:
    try:
        from driftbrake_rs import (
            ReconcileHistory,
            ReconcilePolicy,
            HaltDecision,
            SweepResult,
            StrategyHaltedError,
            run_sweep_raw,
        )
    except ImportError as e:
        raise ImportError(
            "Failed to load compiled driftbrake binary extension. "
            "Please build or install using maturin."
        ) from e

from driftbrake.backtest import run_sweep

__version__ = "0.2.0"
__all__ = [
    "ReconcileHistory",
    "ReconcilePolicy",
    "HaltDecision",
    "SweepResult",
    "StrategyHaltedError",
    "run_sweep",
    "run_sweep_raw",
]
