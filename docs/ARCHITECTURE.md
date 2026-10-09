# Architecture

This document describes the architectural structure of `driftbrake`: module boundaries, trait contracts, non-blocking asynchronous concurrency, state persistence, and Python bindings. For formal invariant proofs and mathematical derivations, see the [whitepaper](./whitepaper.md).

## Design Goals

1. **Chain-Agnostic**: Core trait boundaries impose zero assumptions regarding block time, consensus mechanisms, EVM version, or ABI encodings.
2. **Strategy-Agnostic**: Decouples profit decoding from specific contract layouts, decentralized exchange routers, or liquidation venues.
3. **Focused Scope**: Encapsulates simulation pre-flight, reconciliation, and halting. Alerting integrations, wallet management, and price oracles are deliberately excluded (see [Non-goals](#non-goals)).
4. **Swappable Backends**: SimEngine is a pluggable trait. Teams with proprietary simulation infrastructure can integrate their custom backends while utilizing Driftbrake's reconciliation engine.
5. **Crash Resilience**: State is persisted to a Write-Ahead Log (WAL) to ensure continuous risk enforcement across bot restarts.

## Module Map

```
driftbrake/
├── core/                  # Chain-agnostic traits, zero REVM/alloy dependencies
│   ├── ProfitDecoder / RealizedProfitDecoder
│   ├── HaltPolicy / HaltDecision / HaltReason
│   └── RevertEvent / HistoryEntry
├── reconcile/             # Multi-guard HaltPolicy implementation
│   ├── Fast guard (consecutive acute drops)
│   ├── Slow guard (rolling mean drift)
│   ├── Revert-Burst & Gas budget guards
│   ├── Volume-Weighted drift guard (VWAR)
│   └── Net capital drawdown guard
├── journal/               # Crash-resilient binary Write-Ahead Log (WAL) with CRC32
├── receipt-poller/        # Receipt polling and profit realization gating
├── revm-backend/          # Concrete SimEngine implementation (in-process REVM forks)
├── driftbrake-py/         # Python bindings (PyO3, Stable ABI3, vectorized parameter sweeps)
├── benchmark/             # Threshold calibration harness and CLI
└── examples/
    └── toy-arbitrage/     # Minimal 2-pool arbitrage strategy wired end-to-end
```

## Core Trait Boundaries

The generalization layer resides in `driftbrake-core`, which maintains zero dependencies on REVM or RPC clients.

### `ProfitDecoder`

Decodes raw simulation execution traces into a strategy-denominated predicted profit figure:

```rust
pub trait ProfitDecoder: Send + Sync {
    /// Decode a raw execution trace or return value into predicted profit (e.g. wei).
    fn decode_predicted(&self, raw: &RawSimOutput) -> Result<PredictedProfit, DecodeError>;
}
```

Implementations must be pure and synchronous. Network I/O during decoding is prohibited.

### `RealizedProfitDecoder`

Extracts realized profit from confirmed receipts and transaction logs, net of gas expenses:

```rust
pub trait RealizedProfitDecoder: Send + Sync {
    /// Decode a confirmed receipt and its logs into realized profit, net of gas cost.
    fn decode_realized(&self, receipt: &TxReceipt, logs: &[Log]) -> Result<RealizedProfit, DecodeError>;
}
```

Receipt status must be confirmed. Reverted transactions are handled separately and never passed to `decode_realized`.

### `HaltPolicy`

Evaluates accumulated history to determine whether strategy execution remains safe:

```rust
pub trait HaltPolicy: Send + Sync {
    /// Evaluate the full reconciliation history and return a halt decision.
    fn evaluate(&mut self, history: &ReconcileHistory) -> HaltDecision;
}
```

The default institutional implementation is provided by `driftbrake-reconcile`.

## Simulation Backend (`driftbrake-revm-backend`)

`SimEngine` executes candidate transactions against local state forks. Three implementation patterns are critical for production stability:

### 1. `spawn_blocking` Isolation
Simulating transactions in REVM is CPU-bound synchronous work. Executing simulations directly inside an async task starves Tokio worker threads, delaying mempool ingestion and block-processing routines. `RevmBackend` executes simulations within `tokio::task::spawn_blocking` thread pools, preserving async runtime responsiveness.

### 2. Bounded RPC Concurrency
Simulating multiple candidate transactions per block can saturate RPC rate limits if executed concurrently via unbounded joins. `RevmBackend` enforces a concurrency semaphore bounded by provider capacity (e.g. `buffer_unordered(N)`).

### 3. Block-Time Relative Timeouts
Fixed timeouts fail across diverse networks (e.g. 400ms on Arbitrum vs. 12s on Ethereum mainnet). `RevmBackend` calculates simulation timeouts dynamically as a configurable fraction of target block time.

## Multi-Guard Reconciliation (`driftbrake-reconcile`)

`driftbrake-reconcile` implements an institutional multi-guard hierarchy evaluated on every new history entry:

| Guard | Trigger Condition | Operational Role |
|---|---|---|
| **Revert-Burst** | $N$ consecutive on-chain reverts | Halts on competitive front-running or stale state |
| **Revert-Gas Budget** | Cumulative revert gas exceeds budget | Caps unproductive gas burn |
| **Fast Guard** | $k_f$ consecutive ratios $< T_f$ | Detects sudden market dislocations or broken price feeds |
| **Slow Guard** | Rolling mean ratio $< T_s$ | Detects subtle, compounding drift bleed |
| **Volume-Weighted** | VWAR across window $< T_v$ | Prevents large trade miscalculations masked by small trades |
| **Net Drawdown** | Realized profit drops $> D_{\text{max}}$ from peak | Protects accumulated portfolio capital |

### Ratio Direction Invariant
Ratios are computed strictly as $\text{realized} / \text{predicted}$. Computing the inverse $\text{predicted} / \text{realized}$ maps underperformance to values $> 1.0$, which causes guards to flag harmless overperformance while ignoring severe capital losses. This invariant is enforced through regression tests.

## State Persistence (`driftbrake-journal`)

In high-frequency environments, process crashes or server restarts must not erase risk state. `driftbrake-journal` provides:

- **Append-Only Binary Log**: Low-overhead disk logging for trade pairs and revert events.
- **Checksum Verification**: Every binary frame includes an IEEE 802.3 CRC32 checksum.
- **Torn-Write Recovery**: If a crash occurs mid-write, recovery truncates corrupted trailing bytes to the last verified frame.
- **Single-Writer Lock**: File locking prevents multiple concurrent bot processes from corrupting shared state.

## Python Integration (`driftbrake-py`)

To accommodate quantitative research and offline parameter optimization:

- **PyO3 & CPython Stable ABI**: Compiled against `abi3-py310`, ensuring forward compatibility across Python 3.10–3.14+.
- **Vectorized Sweep Engine**: `run_sweep_raw` evaluates parameter grids over historical datasets in compiled Rust with GIL release, testing millions of combinations per second.
- **Native Exceptions**: Tripped decisions map directly to `StrategyHaltedError`.

## Non-Goals

The following features are explicitly out of scope:

- **Alerting Integrations**: Telegram, Discord, and Slack webhooks belong in external monitoring stacks.
- **Private Key Custody**: Transaction signing and nonce tracking belong in execution wallets.
- **Fiat Price Feeds**: USD conversion logic requires external oracles and does not belong in core traits.
- **Inventory Unwind**: Position unwinding algorithms are specific to portfolio structures and are decoupled from circuit breaking.
