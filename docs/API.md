# API Reference: `core`

This document is the implementer's reference for `driftbrake`'s three trait boundaries and their associated data types. If [`docs/ARCHITECTURE.md`](./ARCHITECTURE.md) explains *why* the boundaries are drawn where they are, this document is the contract you implement against: exact signatures, what each method must and must not do, and the invariants the rest of the pipeline assumes hold once you've implemented it.

All types below live in the `core` crate, which has zero dependency on REVM or `alloy` — nothing here requires you to pull in a specific simulation backend or RPC client to implement against it.

---

## Data types

### `PredictedProfit` / `RealizedProfit`

```rust
/// Profit predicted by simulation, in the smallest denominated unit of the
/// relevant token (e.g. wei for ETH-denominated strategies).
pub struct PredictedProfit(pub i128);

/// Profit actually realized on-chain, net of gas cost, in the same unit
/// and same token as the corresponding PredictedProfit.
pub struct RealizedProfit(pub i128);
```

Both are signed — a strategy can simulate or realize a loss, and the guard math (Section 4.1 of the whitepaper) is defined to exclude non-positive `PredictedProfit` values from the ratio computation rather than assume both are always positive.

**Contract:** a `PredictedProfit` and its corresponding `RealizedProfit` must be denominated in the same token and same unit. `driftbrake` does not perform currency conversion; if your strategy trades across multiple denomination tokens, you are responsible for normalizing to a common unit before these values enter the pipeline.

### `RawSimOutput`

```rust
/// Opaque wrapper around whatever a SimEngine implementation returns —
/// shape is backend-specific (e.g. REVM execution trace + return data).
pub struct RawSimOutput {
    pub return_data: Vec<u8>,
    pub gas_used: u64,
    pub revert_reason: Option<String>,
    pub logs: Vec<Log>,
}
```

This is intentionally close to a raw EVM execution result rather than a strategy-specific shape — the whole point of `ProfitDecoder` is to turn this generic shape into a `PredictedProfit` for your specific executor contract.

### `TxReceipt` / `Log`

Standard confirmed-transaction receipt and log types (fields: transaction hash, block number, status, `gas_used`, `effective_gas_price`, log entries with topics/data). If you're integrating with `alloy` or `ethers`-style types in your own code, convert to/from these at the boundary — `core` itself does not depend on either.

### `ReconcileHistory`

```rust
/// Append-only, confirmation-time-ordered history of (predicted, realized) pairs.
pub struct ReconcileHistory {
    pub pairs: VecDeque<(PredictedProfit, RealizedProfit)>,
    pub reverts: Vec<RevertEvent>, // tracked separately, see below
}

impl ReconcileHistory {
    /// Ratios for the most recent `n` pairs, oldest first. Excludes any
    /// pair whose PredictedProfit was <= 0 at the time it was recorded.
    pub fn recent_ratios(&self, n: usize) -> Vec<f64>;
}
```

**Contract:** `pairs` must be ordered by confirmation time, not submission time — two transactions can confirm out of submission order, and the guards' notion of "recent" is defined relative to confirmation order (see Assumption (c) in the whitepaper's Section 5). A confirmed-but-reverted transaction is never appended to `pairs`; it is recorded in `reverts` instead (see `RealizedProfitDecoder` below).

### `HaltDecision`

```rust
pub enum HaltDecision {
    Continue,
    Halt(HaltReason),
}

pub enum HaltReason {
    FastGuard { window: Vec<f64>, threshold: f64 },
    SlowGuard { mean_ratio: f64, window_size: usize, threshold: f64 },
    Custom(String), // for non-default HaltPolicy implementations
}
```

`HaltReason` is deliberately structured, not a bare string, for the two built-in guards — this is what lets `telemetry` emit a machine-readable event that the benchmark harness (see `docs/BENCHMARK.md`) can aggregate without string parsing. Custom `HaltPolicy` implementations may use `Custom(String)` but are encouraged to define their own structured variant if the policy is intended for reuse beyond a single strategy.

---

## Traits

### `ProfitDecoder`

```rust
pub trait ProfitDecoder: Send + Sync {
    fn decode_predicted(&self, raw: &RawSimOutput) -> Result<PredictedProfit, DecodeError>;
}
```

**What it must do:** convert a raw simulation result into a single `PredictedProfit` figure, in your strategy's chosen denomination unit.

**What it must not do:**
- Perform any network I/O. `ProfitDecoder::decode_predicted` is called synchronously from within the simulation pipeline and is expected to be a pure decode step — if you need a live price to convert between tokens, fetch it before calling this method and pass it in via your implementation's own state, not by reaching out mid-decode.
- Assume `raw.revert_reason.is_none()`. A `SimEngine` may hand you a `RawSimOutput` for a reverted simulation; a correct implementation should return `Err(DecodeError::Reverted)` or similar rather than attempting to decode `return_data` that doesn't exist for a revert.

**Implementer obligation:** if your executor contract can return a `PredictedProfit` of exactly zero for a real "breakeven" simulation, decide deliberately whether that should be treated as `Ok(PredictedProfit(0))` (excluded from ratio computation per Section 4.1 of the whitepaper, since the guards require `PredictedProfit > 0`) or as an error. Both are defensible; being inconsistent between them is not — pick one and document it in your implementation.

**Example implementation sketch:**

```rust
struct MyExecutorDecoder;

impl ProfitDecoder for MyExecutorDecoder {
    fn decode_predicted(&self, raw: &RawSimOutput) -> Result<PredictedProfit, DecodeError> {
        if raw.revert_reason.is_some() {
            return Err(DecodeError::Reverted);
        }
        // return_data is abi.encoded(uint256 profit) for this executor
        let profit = decode_uint256(&raw.return_data)?;
        Ok(PredictedProfit(profit as i128))
    }
}
```

### `RealizedProfitDecoder`

```rust
pub trait RealizedProfitDecoder: Send + Sync {
    fn decode_realized(&self, receipt: &TxReceipt, logs: &[Log]) -> Result<RealizedProfit, DecodeError>;
}
```

**What it must do:** extract a realized profit figure from a *confirmed* receipt and its logs, net of gas cost (`gas_used × effective_gas_price`), in the same unit as the corresponding `PredictedProfit`.

**What it must not do:**
- Be called on a receipt with `status != confirmed`. `receipt-poller` is responsible for gating this — see [`docs/ARCHITECTURE.md`](./ARCHITECTURE.md#receipt-poller) — but a defensive implementation should still treat an unexpected reverted-status receipt as an error rather than silently returning `RealizedProfit(0)`, since a revert and a genuine zero-profit outcome are different events (see the whitepaper's Section 4.5) and conflating them corrupts the guard's input.
- Assume exactly one matching profit event per transaction. If your executor contract can emit zero or multiple profit events in one transaction (e.g. multi-hop strategies), your implementation is responsible for defining and documenting the aggregation rule (sum all matching events, take the last, etc.) — `core` does not impose one.

**Implementer obligation:** net gas cost yourself. `driftbrake` does not automatically subtract `gas_used × effective_gas_price` from whatever profit figure your decoder returns — the `RealizedProfit` you return is taken as already net.

### `HaltPolicy`

```rust
pub trait HaltPolicy: Send + Sync {
    fn evaluate(&mut self, history: &ReconcileHistory) -> HaltDecision;
}
```

**What it must do:** given the full history so far, return a `HaltDecision`. Called once per newly appended `(PredictedProfit, RealizedProfit)` pair.

**What it must not do:**
- Panic on an empty or short history. `evaluate` will be called starting from the very first confirmed transaction, when `history.pairs.len()` may be smaller than your policy's window size — return `HaltDecision::Continue` in that case rather than assuming a minimum history length.
- Retain state that isn't reflected in `history`. `evaluate` takes `&mut self` specifically so a policy *can* keep internal state (e.g. a running sum for efficiency, rather than recomputing a mean from scratch every call), but that internal state must always be derivable from `history` alone — never store information that `history` doesn't also contain, since a policy that can't be reconstructed from `history` is a policy that can't be replayed against historical data for the benchmark sweep in `docs/BENCHMARK.md`.

**The default implementation** (fast guard + slow guard, see the whitepaper's Section 4) ships in the `reconcile` crate and implements this trait directly — read its source alongside this reference if you're implementing a custom `HaltPolicy` and want a worked example of the pattern.

**Example implementation sketch (a simplified single-threshold policy, for illustration — not the shipped default):**

```rust
struct SimpleThresholdPolicy { threshold: f64, window: usize }

impl HaltPolicy for SimpleThresholdPolicy {
    fn evaluate(&mut self, history: &ReconcileHistory) -> HaltDecision {
        let ratios = history.recent_ratios(self.window);
        if ratios.len() < self.window {
            return HaltDecision::Continue; // not enough history yet
        }
        let mean = ratios.iter().sum::<f64>() / ratios.len() as f64;
        if mean < self.threshold {
            HaltDecision::Halt(HaltReason::Custom(format!(
                "mean ratio {mean:.2} below threshold {}", self.threshold
            )))
        } else {
            HaltDecision::Continue
        }
    }
}
```

---

## Error types

```rust
pub enum DecodeError {
    Reverted,
    MalformedData(String),
    MissingProfitEvent,
    Other(String),
}
```

`DecodeError` is shared between `ProfitDecoder` and `RealizedProfitDecoder` rather than each having its own error enum, since both are fundamentally "turn raw EVM output into a profit figure" operations and benefit from a consistent error vocabulary across the pipeline's telemetry.

## Putting it together

A minimal end-to-end wiring, illustrating how the three traits and `SimEngine` compose (see the [Quickstart](https://github.com/Xtley001/driftbrake#quickstart) in the README for the full runnable version):

```rust
let sim: RawSimOutput = sim_engine.simulate(&candidate_tx).await?;
let predicted = my_profit_decoder.decode_predicted(&sim)?;

// ... submit, wait for confirmation ...

let realized = my_realized_decoder.decode_realized(&receipt, &receipt.logs)?;
history.append(predicted, realized);

match halt_policy.evaluate(&history) {
    HaltDecision::Continue => { /* proceed */ }
    HaltDecision::Halt(reason) => { /* stop submitting, log reason */ }
}
```

For the full end-to-end example wired against a real testnet fork, see [`examples/toy-arbitrage`](https://github.com/Xtley001/driftbrake/tree/main/examples/toy-arbitrage).
