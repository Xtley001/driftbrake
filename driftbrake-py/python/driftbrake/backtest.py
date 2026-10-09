"""
Vectorized batch simulation and threshold calibration helpers for quants.
"""

from typing import Any, List, Optional
from driftbrake.driftbrake_rs import run_sweep_raw, SweepResult


def run_sweep(
    trades: Any,
    fast_thresholds: Optional[List[float]] = None,
    slow_thresholds: Optional[List[float]] = None,
    revert_limits: Optional[List[int]] = None,
) -> List[SweepResult]:
    """
    Executes a high-speed parameter grid sweep over historical trade records in Rust.

    Args:
        trades: A pandas.DataFrame, polars.DataFrame, or dict containing:
            - 'predicted_profit' (or 'predicted'): int
            - 'realized_profit' (or 'realized'): int
            - 'is_revert' (or 'reverted'): bool (optional, defaults to False)
            - 'gas_used': int (optional, defaults to 21000)
            - 'effective_gas_price': int (optional, defaults to 0)
        fast_thresholds: List of fast guard thresholds (default: [0.3, 0.5, 0.7])
        slow_thresholds: List of slow guard thresholds (default: [0.65, 0.70, 0.80])
        revert_limits: List of consecutive revert limits (default: [2, 3, 5])

    Returns:
        List of SweepResult objects detailing halts, latency, and drawdown prevented.
    """
    if fast_thresholds is None:
        fast_thresholds = [0.30, 0.50, 0.70]
    if slow_thresholds is None:
        slow_thresholds = [0.65, 0.70, 0.80]
    if revert_limits is None:
        revert_limits = [2, 3, 5]

    # Support dictionary, Pandas, and Polars
    if isinstance(trades, dict):
        predicted = list(trades.get("predicted_profit") or trades["predicted"])
        realized = list(trades.get("realized_profit") or trades["realized"])
        is_revert = list(trades.get("is_revert") or trades.get("reverted") or [False] * len(predicted))
        gas_used = list(trades.get("gas_used") or [21_000] * len(predicted))
        gas_price = list(trades.get("effective_gas_price") or [0] * len(predicted))
    else:
        # DataFrame-like interface
        pred_col = "predicted_profit" if "predicted_profit" in trades.columns else "predicted"
        real_col = "realized_profit" if "realized_profit" in trades.columns else "realized"
        predicted = [int(x) for x in trades[pred_col]]
        realized = [int(x) for x in trades[real_col]]

        rev_col = "is_revert" if "is_revert" in trades.columns else ("reverted" if "reverted" in trades.columns else None)
        if rev_col:
            is_revert = [bool(x) for x in trades[rev_col]]
        else:
            is_revert = [False] * len(predicted)

        if "gas_used" in trades.columns:
            gas_used = [int(x) for x in trades["gas_used"]]
        else:
            gas_used = [21_000] * len(predicted)

        if "effective_gas_price" in trades.columns:
            gas_price = [int(x) for x in trades["effective_gas_price"]]
        else:
            gas_price = [0] * len(predicted)

    return run_sweep_raw(
        predicted=predicted,
        realized=realized,
        is_revert=is_revert,
        gas_used=gas_used,
        gas_price=gas_price,
        fast_thresholds=fast_thresholds,
        slow_thresholds=slow_thresholds,
        revert_limits=revert_limits,
    )
