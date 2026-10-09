# toy-arbitrage

A minimal two-pool arbitrage strategy wired end-to-end through `driftbrake` against a public testnet fork.

This example demonstrates how the [`core` trait contracts](../../docs/API.md) compose within a complete execution loop: `SimEngine` $\rightarrow$ `ProfitDecoder` $\rightarrow$ transaction submission $\rightarrow$ `RealizedProfitDecoder` $\rightarrow$ `HaltPolicy`.

> **Notice**: This example is illustrative. Do not execute against real capital without independent verification.

## Running the Example

```bash
cd examples/toy-arbitrage
cp .env.example .env        # Set TESTNET_RPC_URL and PRIVATE_KEY
cargo run --release
```

Requirements:
- Archive JSON-RPC endpoint for local state forking.
- Funded testnet account for transaction execution.

## Project Structure

| File | Component | Description |
|---|---|---|
| `src/decoder.rs` | `ProfitDecoder` & `RealizedProfitDecoder` | ABI decoding for two-leg arbitrage return data and events |
| `src/main.rs` | Execution Loop | State synchronization, simulation, broadcast, and reconciliation |
| `src/pools.rs` | Reserve Ingestion | Constant-product pool reserve reading and spread calculation |
| `src/rpc.rs` | Network Adapters | In-process REVM fork factory, receipt poller, and transaction submitter |
| `tests/end_to_end.rs` | Integration Test | Verifies that injected drift trips the reconciliation halt policy |

## Simulating Drift Injection

To manually verify that the circuit breaker triggers under adverse conditions:

```bash
cargo run --release -- --inject-drift --drift-after 5
```

The strategy executes 5 healthy transactions before artificially degrading realized profits, causing the fast guard to halt the loop.

## Known Limitations

- **Fixed Scope**: Restricted to a single token pair across two constant-product pools.
- **Atomic Execution**: Assumes single-transaction flash-loan settlement with zero inventory carryover.
- **Gas Pricing**: Uses node-reported base fees rather than dynamic priority fee algorithms.
- **Contract Mocking**: Requires deployment of an executor contract conforming to the documented ABI interface (`executeArb`, `event Profit`).
