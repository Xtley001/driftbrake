# Phantom Profit: Three Production Simulation Bugs

This document details three structural failure modes observed in production simulate-then-submit trading systems. Unlike application-level logic errors, these issues manifest as silent capital decay while individual transaction logs appear nominally successful.

## Pipeline Architecture

Algorithmic execution loops (MEV searchers, atomic arbitrageurs, liquidators) follow a four-stage lifecycle:
1. **Fork State**: Replicate current blockchain state via local EVM or JSON-RPC.
2. **Pre-flight Simulate**: Execute candidate transactions against forked state to verify profitability.
3. **Broadcast**: Submit profitable transactions to block builders or public mempools.
4. **Confirm**: Ingest transaction receipts upon block inclusion.

When execution conditions change between simulation and inclusion, this lifecycle produces simulation drift.

## Failure Mode 1: Backward Ratio Calculation

Reconciliation evaluates realized profit against simulated expectation. The natural formulation is:

$$\rho = \frac{\text{realized}}{\text{predicted}}$$

Values $\rho < 1.0$ indicate underperformance; values $\rho \ge 1.0$ indicate performance meeting or exceeding prediction.

Inverting this formula yields:

$$\rho_{\text{inv}} = \frac{\text{predicted}}{\text{realized}}$$

Under inversion, underperformance yields $\rho_{\text{inv}} > 1.0$, while profitable outperformance yields $\rho_{\text{inv}} < 1.0$. If downstream guards evaluate `ratio < threshold` without inverting comparison operators, two catastrophic conditions occur:
- Profitable trades that exceeded simulation are incorrectly flagged as anomalies.
- Trades that severely underperformed pass through without triggering circuit breakers.

Because both formulas output positive floating-point numbers, inversion produces plausible log outputs that bypass casual inspection. Driftbrake enforces ratio direction as an invariant protected by automated regression tests.

## Failure Mode 2: Worker-Thread Starvation

In asynchronous Rust runtimes (e.g. Tokio), CPU-bound operations executed inside async functions block the underlying worker thread.

Both in-process REVM execution and synchronous JSON-RPC calls occupy CPU threads for multiple milliseconds. Executing these routines directly within an `async fn` prevents the worker thread from yielding to concurrent tasks. This starves the main block-listener loop, increasing latency between block observation and candidate transaction submission.

### Remediation
Wrap CPU-intensive simulation routines in `tokio::task::spawn_blocking`. This offloads heavy computation to a dedicated thread pool, preserving asynchronous runtime throughput.

## Failure Mode 3: RPC Provider Rate-Limit Saturation

A volatile block can yield 50 or more simultaneous trading candidates. Attempting to simulate all candidates concurrently via unbounded futures (`join_all`) causes immediate request bursts.

External RPC providers enforce aggressive rate limits. Unbounded concurrency triggers HTTP 429 throttling during high-opportunity blocks—precisely when execution speed is most critical.

### Remediation
Enforce bounded concurrency via provider-calibrated semaphores (e.g. `buffer_unordered(N)`). Bounding concurrent requests to provider limits eliminates throttling while maintaining maximum sustainable throughput.

## Multi-Guard Reconciliation

Resolving execution bugs ensures mechanical reliability but does not prevent market-driven simulation drift. Price oracle delays, mempool bundle competition, and execution slippage require automated reconciliation:

- **Acute Shocks**: Handled via the Fast Guard ($k_f$ consecutive underperformances).
- **Creeping Decay**: Handled via the Slow Guard (rolling mean across $k_s$ transactions).
- **Asymmetric Capital Sizing**: Handled via Volume-Weighted Drift (VWAR).
- **Toxic Flow & Front-Running**: Handled via Revert-Burst and Revert-Gas Budget guards.

Continuously evaluating the gap between predicted and realized profit guarantees that operational divergence is intercepted before capital bleed compounds.
