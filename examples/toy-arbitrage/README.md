# toy-arbitrage

*A minimal 2-pool arbitrage strategy wired end-to-end through `driftbrake`, against a public testnet fork.*

This example exists to do two things at once: demonstrate how the three [`core` traits](../../docs/API.md) (or [rendered](https://xtley001.github.io/driftbrake/API.html)) get implemented for a real (if simplified) strategy, and serve as the first integration test of the whole pipeline — `SimEngine` → `ProfitDecoder` → submission → `RealizedProfitDecoder` → `HaltPolicy` — rather than testing each piece in isolation.

**This example is illustrative, not production-hardened.** Do not point it at real capital without your own review — see [`SECURITY.md`](../../SECURITY.md#scope-boundaries).

## What it does

Watches two pools of the same token pair on a forked testnet, and when a price divergence between them exceeds gas cost, simulates a two-leg swap (buy on the cheaper pool, sell on the more expensive one) through `driftbrake`'s `SimEngine` before submitting it. Every simulated-then-submitted trade feeds into the reconciliation history, and the default dual guard (see [`docs/whitepaper.md`](../../docs/whitepaper.md#4-mechanism-specification)) is wired in to halt the loop if realized profit starts diverging from prediction.

## Running it

```bash
cd examples/toy-arbitrage
cp .env.example .env        # set TESTNET_RPC_URL and PRIVATE_KEY
cargo run --release
```

You'll need a testnet RPC endpoint with archive access for forking (a free-tier public endpoint is generally sufficient for this example's request volume) and a funded testnet account.

## What to look at

| File | Implements |
|---|---|
| `src/decoder.rs` | `ProfitDecoder` and `RealizedProfitDecoder` for the example's two-leg swap executor |
| `src/main.rs` | The end-to-end loop: watch pools → simulate → submit → reconcile → check halt |
| `src/pools.rs` | Minimal pool-price reading, deliberately simplified — not a general-purpose venue integration |
| `src/rpc.rs` | Live-network implementations: forking state over JSON-RPC, polling receipts, signing and broadcasting the arb transaction |
| `tests/end_to_end.rs` | The integration test mentioned above — real `driftbrake` crates wired to in-memory fixtures, proving the halt guard actually trips when drift is injected |

## Forcing a halt (for demonstration)

The example includes a `--inject-drift` flag that artificially degrades the realized-profit decoder's output after a configurable number of trades, so you can watch the fast and slow guards trip without needing to wait for real market conditions to drift:

```bash
cargo run --release -- --inject-drift --drift-after 5
```

This is useful for confirming your own fork or configuration change didn't silently break the halt logic — it's a manual sanity check, not a substitute for the [benchmark sweep](../../docs/BENCHMARK.md), which is what actually validates threshold behavior statistically.

## Known limitations

- Single token pair, two pools only — this is intentionally not a general-purpose arbitrage bot.
- No inventory management: the example is flash-loan-style (borrow, swap, swap back, repay in one transaction), which is why it doesn't exercise the unwind logic that's explicitly out of scope for v1 (see [Non-goals](../../docs/ARCHITECTURE.md#non-goals)).
- Gas price handling is simplified (uses the RPC's suggested gas price directly) rather than implementing its own fee-estimation strategy.
- **No executor contract is deployed anywhere.** `decoder.rs` documents an assumed ABI (`executeArb(address,address,uint256) returns (uint256)`, `event Profit(uint256)`) as a stated assumption, not a fact about a real deployed contract. Deploy your own executor matching that ABI (or adjust `decoder.rs` to match whatever you deploy) before pointing this at a real fork.
- **`src/rpc.rs`'s live-network paths have not been round-tripped against a live archive node** as part of this repo's own test suite — `RpcDbFactory`, `RpcReceiptSource`, and `RpcTxSubmitter` are complete implementations with unit-tested parsing/encoding/signing logic (see `tests/end_to_end.rs` and the unit tests in `rpc.rs` itself), but "does this actually work against a real RPC endpoint" can only be confirmed by running it against one. Review `rpc.rs` yourself before pointing it at anything holding real value.

## Implementation notes

A few design decisions worth knowing about if you're extending this:

- **`TxSubmitter` (in `rpc.rs`) is a driftbrake-adjacent trait, not part of the core library's API** — the core traits stop at simulate → decode → reconcile → halt and deliberately don't cover the submission step. It follows the same shape as `DbFactory` and `ReceiptSource`: pluggable, so a test or an alternative signing/broadcast strategy can stand in without touching `main.rs`'s loop logic.
- **Legacy (type-0) transactions, not EIP-1559** — simpler RLP shape, and fits the simplified gas-price handling above more naturally than a `maxFeePerGas`/`maxPriorityFeePerGas` pair would.
- **Nonce management is fire-and-forget**: fetched once at startup via `eth_getTransactionCount(..., "pending")`, then incremented locally per submission. No re-sync, no stuck-transaction handling — a real long-running bot needs a real nonce manager; that's out of scope here for the same reason inventory management is.
- **`.env` loading is hand-rolled** (`load_dotenv()` in `main.rs`) rather than a `dotenv`/`dotenvy` dependency. Deliberately minimal: no quoting, no escaping, no multiline values, just `KEY=VALUE` lines matching `.env.example`.
- **`keccak256` is re-exported from `driftbrake-revm-backend`** (returning a plain `[u8; 32]`) rather than this crate taking its own hashing dependency, since `revm` already provides one transitively.
