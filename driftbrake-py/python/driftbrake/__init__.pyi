from typing import List, Optional, Dict, Any, Union

class StrategyHaltedError(Exception): ...

class HaltDecision:
    @property
    def should_halt(self) -> bool: ...
    @property
    def reason_name(self) -> Optional[str]: ...
    @property
    def details(self) -> Dict[str, Any]: ...
    def unwrap_or_raise(self) -> None: ...

class ReconcileHistory:
    def __init__(self) -> None: ...
    def append(self, predicted_profit: int, realized_profit: int) -> None: ...
    def record_revert(
        self,
        tx_hash: bytes,
        block_number: int,
        gas_used: int,
        effective_gas_price: int,
        reason: Optional[str] = ...,
    ) -> None: ...
    def recent_ratios(self, n: int) -> List[float]: ...
    def total_reverts(self) -> int: ...
    def consecutive_reverts(self) -> int: ...
    def recent_revert_gas_burned(self, window: int) -> int: ...
    def __len__(self) -> int: ...

class ReconcilePolicy:
    @staticmethod
    def default_dual_guard() -> ReconcilePolicy: ...
    @staticmethod
    def institutional(
        fast_threshold: float = 0.50,
        fast_window: int = 3,
        slow_threshold: float = 0.70,
        slow_window: int = 20,
        revert_burst_limit: Optional[int] = 3,
        volume_weighted_threshold: Optional[float] = 0.75,
        volume_weighted_window: int = 20,
        revert_gas_budget: Optional[int] = None,
        revert_gas_window: int = 10,
        drawdown_limit: Optional[int] = None,
        drawdown_window: int = 20,
    ) -> ReconcilePolicy: ...
    def evaluate(self, history: ReconcileHistory) -> HaltDecision: ...

class SweepResult:
    fast_threshold: float
    fast_window: int
    slow_threshold: float
    slow_window: int
    revert_limit: int
    total_halts: int
    first_halt_index: Optional[int]
    total_drawdown_prevented: int

def run_sweep(
    trades: Any,
    fast_thresholds: Optional[List[float]] = ...,
    slow_thresholds: Optional[List[float]] = ...,
    revert_limits: Optional[List[int]] = ...,
) -> List[SweepResult]: ...
