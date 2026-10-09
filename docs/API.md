# API Reference

This document specifies the trait contracts, data structures, and persistent storage APIs in `driftbrake`.

---

## Data Types (`driftbrake-core`)

### `PredictedProfit` and `RealizedProfit`

```rust
pub struct PredictedProfit(pub i128);
pub struct RealizedProfit(pub i128);
```

Both values are signed 128-bit integers denominated in the strategy's native unit (e.g. wei).
- `PredictedProfit`: Estimated profit returned by pre-flight simulation. Values $\le 0$ are excluded from ratio calculations.
- `RealizedProfit`: Actual on-chain profit extracted from confirmed transaction receipts, net of all gas expenses.

### `RevertEvent` and `HistoryEntry`

```rust
pub struct RevertEvent {
    pub tx_hash: [u8; 32],
    pub block_number: u64,
    pub revert_reason: Option<String>,
    pub gas_used: u64,
    pub effective_gas_price: u128,
}

pub enum HistoryEntry {
    Pair(PredictedProfit, RealizedProfit),
    Revert(RevertEvent),
}
```

`HistoryEntry` maintains the chronological timeline of execution events. Reverts are tracked separately from the profit ratio pool to prevent skewing moving averages.

### `HaltDecision` and `HaltReason`

```rust
pub enum HaltDecision {
    Continue,
    Halt(HaltReason),
}

pub enum HaltReason {
    FastGuard { window: Vec<f64>, threshold: f64 },
    SlowGuard { mean_ratio: f64, window_size: usize, threshold: f64 },
    RevertBurst { consecutive_reverts: usize, threshold: usize },
    RevertGasBudgetExceeded { total_gas_burned: u64, budget: u64 },
    VolumeWeightedDrift { volume_weighted_ratio: f64, threshold: f64 },
    NetDrawdownExceeded { current_drawdown: i128, max_drawdown: i128 },
    Custom(String),
}
```

`HaltReason` provides structured metadata enabling automated incident handling and telemetry logging.

---

## Traits (`driftbrake-core`)

### `ProfitDecoder`

```rust
pub trait ProfitDecoder: Send + Sync {
    fn decode_predicted(&self, raw: &RawSimOutput) -> Result<PredictedProfit, DecodeError>;
}
```

- **Obligation**: Pure decode from `RawSimOutput` (EVM return data, gas, and revert status) into a `PredictedProfit`.
- **Constraint**: Must not perform asynchronous network I/O or database access.

### `RealizedProfitDecoder`

```rust
pub trait RealizedProfitDecoder: Send + Sync {
    fn decode_realized(&self, receipt: &TxReceipt, logs: &[Log]) -> Result<RealizedProfit, DecodeError>;
}
```

- **Obligation**: Extract strategy profits from confirmed transaction logs, subtracting gas costs (`receipt.gas_used * receipt.effective_gas_price`).
- **Constraint**: Must only be invoked on receipts with `TxStatus::Confirmed`.

### `HaltPolicy`

```rust
pub trait HaltPolicy: Send + Sync {
    fn evaluate(&mut self, history: &ReconcileHistory) -> HaltDecision;
}
```

- **Obligation**: Evaluate history and return a `HaltDecision`.
- **Constraint**: Must handle empty and short histories without panicking.

---

## State Persistence (`driftbrake-journal`)

### `JournaledHistory`

```rust
use driftbrake_journal::JournaledHistory;

let mut journal = JournaledHistory::open("path/to/reconcile.wal")?;

// Appends pair to in-memory history and flushes CRC32-verified frame to disk
journal.append(PredictedProfit(1_000_000), RealizedProfit(950_000))?;

// Appends revert event
journal.append_revert(revert_event)?;

// Access underlying history
let history: &ReconcileHistory = journal.history();
```

- **Torn-Write Protection**: Automatically truncates corrupted partial writes at EOF upon initialization.
- **Process Lock**: Acquires an exclusive file lock, preventing concurrent process writes.

---

## Python API (`driftbrake-py`)

Native CPython extension compiled with Stable ABI (`abi3-py310`).

```python
from driftbrake import ReconcileHistory, ReconcilePolicy, StrategyHaltedError

# Initialize policy
policy = ReconcilePolicy(
    fast_window=3,
    fast_threshold=0.50,
    slow_window=20,
    slow_threshold=0.70,
    max_consecutive_reverts=3,
    max_revert_gas_budget=500_000,
    volume_weighted_window=15,
    volume_weighted_threshold=0.65,
    max_drawdown=10_000_000,
)

history = ReconcileHistory()
history.append(predicted_profit=1_000_000, realized_profit=950_000)

decision = policy.evaluate(history)
if decision.is_halted:
    raise StrategyHaltedError(decision.reason)
```

### Vectorized Backtesting (`run_sweep_raw`)

```python
from driftbrake import run_sweep_raw

# pairs: list of (predicted_profit, realized_profit)
# fast_windows, fast_thresholds, slow_windows, slow_thresholds: lists of parameters
results = run_sweep_raw(
    pairs,
    fast_windows=[2, 3, 4],
    fast_thresholds=[0.40, 0.50, 0.60],
    slow_windows=[15, 20, 25],
    slow_thresholds=[0.65, 0.70, 0.75],
)
# Returns list of dicts: {'fast_window', 'fast_threshold', 'slow_window', 'slow_threshold', 'halted', 'halt_index', 'reason'}
```
