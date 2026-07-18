# Architecture

This document describes how `driftbrake` is put together internally: the module boundaries, the trait contracts that make it chain- and strategy-agnostic, and the reasoning behind the parts of the design that aren't obvious from reading the code alone. For the argument that the design is *correct* (invariants, formal properties, the ratio-direction proof), see [`whitepaper.md`](./whitepaper.md). This document is about *shape*, not proof.

## Design goals, in priority order

1. **Chain-agnostic.** Nothing in `core` assumes a specific block time, EVM spec version, or ABI shape.
2. **Strategy-agnostic.** Nothing in `core` assumes a specific profit-decoding shape, venue, or strategy structure.
3. **Small surface, hard boundary.** `driftbrake` does exactly one thing — pre-flight simulation plus drift-based halting. Alerting, nonce isolation, and USD-denominated gas budgets are explicitly out of scope (see [Non-goals](#non-goals)).
4. **Swappable simulation backend.** A team with an existing simulation stack should be able to keep it and still get the reconciliation and halt logic for free.

## Module map

```
driftbrake/
├── core/                  # chain-agnostic traits, zero REVM/alloy dependency
│   ├── ProfitDecoder
│   ├── RealizedProfitDecoder
│   └── HaltPolicy
├── revm-backend/          # concrete implementation of SimEngine
│   ├── fork-and-simulate, spawn_blocking wrapper, bounded concurrency
│   └── configurable: timeout-as-fraction-of-block-time, concurrency cap
├── reconcile/             # the halt-guard logic (carries the "phantom-guard" name, see Naming)
│   ├── default HaltPolicy: fast guard (3-consecutive) + rolling guard (window N, threshold T)
│   └── both guards configurable, not hardcoded at 0.50 / 0.70 / 3 / 20
├── receipt-poller/        # confirms receipts, generalized polling interval/timeout
├── telemetry/             # structured events for the benchmark harness to consume
└── examples/
    └── toy-arbitrage/     # minimal 2-pool arb wired end-to-end against a public testnet fork
```

`core` has zero dependency on REVM or `alloy` types. `RawSimOutput` and `ReconcileHistory` are plain, serialization-friendly data (`serde`-derivable structs and enums, no RPC client types, no REVM database handles). This is the boundary that makes `revm-backend` swappable rather than load-bearing: a team using a different simulation stack implements `SimEngine` against their own tooling and keeps `reconcile` and `receipt-poller` unchanged.

## `core`: the trait boundaries

These three traits are the actual generalization layer — the part that doesn't already exist elsewhere as a packaged abstraction.

### `ProfitDecoder`

```rust
pub trait ProfitDecoder: Send + Sync {
    /// Decode a raw REVM execution trace/return value into a predicted profit,
    /// in the smallest denominated unit (e.g. wei).
    fn decode_predicted(&self, raw: &RawSimOutput) -> Result<PredictedProfit, DecodeError>;
}
```

Replaces what was originally a hardcoded `simulate()` call returning a single `uint256`. Any executor contract shape — single-call, multicall, flash-loan-wrapped — implements this trait once and plugs into the rest of the pipeline unchanged.

### `RealizedProfitDecoder`

```rust
pub trait RealizedProfitDecoder: Send + Sync {
    /// Decode a confirmed receipt + its logs into a realized profit,
    /// net of gas cost, in the same unit as PredictedProfit.
    fn decode_realized(&self, receipt: &TxReceipt, logs: &[Log]) -> Result<RealizedProfit, DecodeError>;
}
```

Replaces a hardcoded `Profit` event ABI. `receipt-poller` calls this once a receipt reaches confirmed status; it nets `gas_used × effective_gas_price` from whatever profit figure the implementation extracts.

### `HaltPolicy`

```rust
pub trait HaltPolicy: Send + Sync {
    /// Given the full (sim, realized) history so far, decide whether to halt.
    fn evaluate(&mut self, history: &ReconcileHistory) -> HaltDecision;
}
```

Replaces hardcoded fast/slow guard constants. The default `HaltPolicy` implementation is the dual guard described below, but it is a default, not the only option — a team can implement their own `HaltPolicy` entirely (e.g. a Bayesian change-point detector) and keep everything else in the pipeline.

Why these three and not more: everything else in the pipeline (forking state, submitting transactions, polling for receipts) is mechanical I/O that doesn't vary across strategies once you fix a chain. These three are the places where strategy-specific and chain-specific knowledge actually has to enter the system, so they're the only three traits — resist the temptation to add more surface area than this.

## `revm-backend`: `SimEngine`

`SimEngine` forks chain state at the current block, runs the candidate transaction through REVM in-process, and returns a `RawSimOutput` (profit, gas, revert reason if any) within a hard latency budget.

Three implementation details are load-bearing and are called out explicitly because a naive reimplementation gets them wrong:

### 1. `spawn_blocking` for REVM/RPC calls

REVM execution and synchronous RPC calls are CPU/IO-blocking work. Running them directly inside an `async fn` on a tokio runtime blocks whichever worker thread picked up that task — and because tokio multiplexes many tasks per worker thread, this doesn't just slow the one simulation, it starves the main block-processing loop and every other concurrent task sharing that thread pool. The fix is to wrap every REVM/RPC call in `tokio::task::spawn_blocking`, which moves the work to a dedicated blocking-thread pool and leaves the async worker threads free.

This is not a chain-specific quirk. It reproduces identically on a 400ms-block chain and a 12-second-block chain, because the bug is about thread-pool starvation, not timing.

### 2. Bounded concurrency for RPC calls

A block can surface 50+ candidate opportunities that all need simulating. Firing all of them concurrently via `futures::future::join_all` saturates the RPC provider's rate limit — turning what should be a fast local simulation into a slow, rate-limited one exactly when speed matters most (i.e., during the highest-opportunity blocks). The fix is bounding concurrency to `N`, tied to the RPC provider's actual rate limit, not a magic number picked once and forgotten — whether that's a caller-side `buffer_unordered(N)` or, as `revm-backend`'s `RevmBackend` does, a semaphore held internally by the engine itself (a stronger form of the same guarantee: the bound holds regardless of how a caller fans out its calls).

### 3. Timeout as a fraction of block time, not a fixed constant

A simulation timeout tuned for a 400ms-block chain is nonsensical on a 12-second-block chain, and vice versa — too tight starves valid simulations on a slow chain, too loose means a bot is still simulating after the block it was targeting has already passed on a fast chain. `revm-backend` takes `block_time_ms` as a required parameter and derives the simulation timeout as a configurable fraction of it (default: a sensible fraction that leaves headroom for submission after simulation completes, made explicit and tunable rather than hidden inside the code).

## `reconcile`: the dual halt guard

`reconcile` is the module that carries the `phantom-guard` name (see [Naming](#naming)). It implements the default `HaltPolicy`: two independent guards evaluated on every new `(sim, realized)` pair.

- **Fast guard** — halts immediately on 3 consecutive transactions with `realized / sim < 0.50`.
- **Slow guard** — halts if the mean ratio over the last 20 `(sim, realized)` pairs drops below 0.70.

### Why two guards, not one

The two guards catch different failure shapes and neither substitutes for the other:

- **Fast guard catches a sudden, severe break** — a venue going down, a price feed going stale all at once. Three bad trades in a row is a strong signal *right now*, and waiting for a 20-trade rolling window to reflect that would let a sudden break run for far too long before it's caught.
- **Slow guard catches a slow bleed** — small, individually-forgivable underperformance (each trade might be a 0.85 ratio, never bad enough to trip the fast guard's 0.50 threshold) that compounds into real capital loss over a session. A system with only the fast guard is blind to this; a system with only the slow guard reacts too slowly to a sudden break.

Both guards are independently configurable (threshold and window size are parameters, not constants) precisely because the numbers that were empirically tuned for one chain's latency and volatility profile are not assumed to transfer to another chain without re-validation — see the benchmark methodology in [`BENCHMARK.md`](./BENCHMARK.md) for how to re-derive them.

### Why the ratio direction matters

`reconcile` computes `realized / sim`, never `sim / realized`. This looks like it shouldn't matter, but the inverted ratio silently changes which case gets flagged: with `realized / sim`, a value below 1.0 means underperformance (the case you need to catch) and a value above 1.0 means overperformance (harmless). Invert the ratio and a value *above* 1.0 now means underperformance — which either creates false-halt noise on harmless overperformance, or, worse, fails to halt on genuine underperformance because the inverted ratio doesn't cross the configured threshold in the direction the guard is checking. This is treated as a formal invariant, not an implementation detail — see [`whitepaper.md`](./whitepaper.md#formal-properties--invariants) for the labeled statement and the regression test that guards it.

## `receipt-poller`

Polls for confirmed receipts and books profit only once two conditions are both met: confirmed transaction status *and* an emitted profit event decoded via `RealizedProfitDecoder`. Requiring both, rather than either alone, avoids two separate failure modes: a confirmed-but-reverted transaction being miscounted as profit, and a profit-shaped log appearing from an unrelated or simulated context. Gas cost (`gas_used × effective_gas_price`) is available on the `TxReceipt` passed to `RealizedProfitDecoder`, but netting it is the decoder implementation's responsibility, not something `receipt-poller` subtracts automatically — see [`API.md`](./API.md)'s `RealizedProfitDecoder` contract.

Polling interval and timeout are expressed as a function of block time, following the same reasoning as the `SimEngine` timeout: a fixed millisecond value tuned for one chain doesn't transfer to another.

## `telemetry`

Emits structured events (`RawSimOutput`, `ReconcileEvent` — a telemetry-specific summary of one reconciliation step, not a `core` type — and `HaltDecision`) that the benchmark harness consumes to reconstruct the false-halt-rate / missed-catch-rate curve described in [`BENCHMARK.md`](./BENCHMARK.md). Telemetry is a data-emission layer only — it does not implement alerting (Telegram, PagerDuty, etc.), which is explicitly out of scope (see below).

## Extensibility: WASM

Because `core` has zero dependency on REVM or `alloy` types, and `RawSimOutput`/`ReconcileHistory` are plain serializable data, the portable simulation-*logic* layer (decoding, reconciliation, halt evaluation) is a natural target for a WASM build. The fork-and-fetch I/O layer in `revm-backend` inherently depends on a live RPC connection and is not part of this extension — only the logic that consumes already-fetched data is portable. This is a structural property of the current design, not a committed roadmap item; it costs nothing to preserve now and remains available if a future need for it (e.g. a non-Rust host embedding the reconciliation logic) arises.

## Non-goals

Explicitly out of scope for this crate, and not planned for a future version without a deliberate scope decision:

- **Alerting** (Telegram, PagerDuty, or otherwise) — belongs in whatever ops stack a team already runs; `telemetry` emits structured events that any alerting layer can consume, but `driftbrake` does not ship one.
- **Per-strategy EOA nonce isolation** — a wallet/submission-layer concern, not a simulation/reconciliation concern.
- **Gas budget in USD** — requires a price oracle and a currency-conversion opinion that doesn't belong in a chain-agnostic crate.
- **Inventory-aware unwind logic** — deferred. The reference implementation this is extracted from was flash-loan-funded with no inventory to unwind. Adding unwind logic widens the applicable audience to vault/LP strategies, but introduces a genuinely different failure mode (partial fills, slippage on the unwind itself) that deserves its own design pass rather than being bolted onto the halt guard half-finished. This is a documented non-goal for v1, not an oversight.

Keeping this boundary disciplined is a design decision, not an omission: a crate that tries to be a full bot-ops platform forces every adopting team to displace their existing alerting and ops stack, which makes it harder to adopt, not easier. A crate that does exactly one thing well slots into an existing stack without a fight.

## Naming

The `reconcile` module keeps the name **phantom-guard** internally and in documentation, even though the top-level crate is `driftbrake`. "Phantom profit" is an existing, recognizable term of art for the exact failure mode this module addresses (a bot believing it made money it did not actually make), and it's worth preserving as a specific, quotable name for the halt mechanism, distinct from the umbrella project name. The two names are meant to coexist: `driftbrake` is what you depend on; `phantom-guard` is the name people use when discussing the mechanism itself.
