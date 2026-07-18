# driftbrake

*A chain-agnostic Rust crate that pre-flight simulates a transaction through REVM, then halts your strategy the moment realized profit drifts from the simulated prediction — before drift compounds into real capital loss.*

[![CI](https://img.shields.io/github/actions/workflow/status/Xtley001/driftbrake/ci.yml)](https://github.com/Xtley001/driftbrake/actions)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)
[![crates.io](https://img.shields.io/crates/v/driftbrake)](https://crates.io/crates/driftbrake)
[![docs.rs](https://img.shields.io/docsrs/driftbrake)](https://docs.rs/driftbrake)
[![book](https://img.shields.io/badge/book-design%20docs-3ddc84)](https://xtley001.github.io/driftbrake/)

Any bot's internal model of the world — its REVM fork, its price feed, its assumption about which venue fills first — can go stale relative to actual chain state. `driftbrake` packages three tested fixes for the failure modes that causes (ratio-direction inversion, blocking-thread starvation, unbounded RPC saturation) behind a small set of chain-agnostic traits, plus a dual-guard halt policy that catches both a sudden break and a slow bleed. For the full mechanism specification and correctness argument, see the [whitepaper](./docs/whitepaper.md) — or read it rendered at **[xtley001.github.io/driftbrake](https://xtley001.github.io/driftbrake/)**.

## Documentation

- **[xtley001.github.io/driftbrake](https://xtley001.github.io/driftbrake/)** — the design docs (architecture, whitepaper, API reference, benchmark methodology) as a browsable site, built from `docs/` via [mdBook](https://rust-lang.github.io/mdBook/). This is the "why."
- **[docs.rs/driftbrake](https://docs.rs/driftbrake)** — generated API reference (every public trait/type, from doc comments). This is the "what."
- This README is the "how" — the Quickstart below.

## Installation

```bash
cargo add driftbrake
# Add the REVM simulation backend too, unless you're bringing your own SimEngine:
cargo add driftbrake-revm-backend
```

## Quickstart

```rust
use driftbrake::{
    DecodeError, HaltDecision, HaltPolicy, Log, PredictedProfit, ProfitDecoder,
    RawSimOutput, RealizedProfit, RealizedProfitDecoder, ReconcileHistory,
    ReconcilePolicy, SimEngine, TxReceipt, TxStatus,
};
// The REVM backend is a separate, opt-in crate — `cargo add driftbrake-revm-backend`.
// (Bringing your own simulation stack instead? Implement `SimEngine` against it and
// skip this import entirely — see docs/ARCHITECTURE.md's "swappable simulation backend".)
use driftbrake_revm_backend::{BlockContext, CandidateTx, DbFactory, RevmBackend};

// 1. `SimEngine` needs a `DbFactory` — something that forks chain state.
//    Bring your own (a live RPC-backed one, or a test fixture).
#[derive(Clone)]
struct MyDbFactory;

impl DbFactory for MyDbFactory {
    type Db = revm::db::CacheDB<revm::db::EmptyDB>;
    type Error = std::convert::Infallible;
    fn fresh_db(&self) -> Result<Self::Db, Self::Error> {
        Ok(revm::db::CacheDB::new(revm::db::EmptyDB::new()))
    }
}

// 2. Implement `ProfitDecoder` / `RealizedProfitDecoder` for your executor
//    contract's specific ABI shape — see docs/API.md.
struct MyProfitDecoder;
impl ProfitDecoder for MyProfitDecoder {
    fn decode_predicted(&self, raw: &RawSimOutput) -> Result<PredictedProfit, DecodeError> {
        if raw.revert_reason.is_some() {
            return Err(DecodeError::Reverted);
        }
        Ok(PredictedProfit(0)) // decode your executor's real return value here
    }
}

struct MyRealizedDecoder;
impl RealizedProfitDecoder for MyRealizedDecoder {
    fn decode_realized(&self, receipt: &TxReceipt, _logs: &[Log]) -> Result<RealizedProfit, DecodeError> {
        if receipt.status != TxStatus::Confirmed {
            return Err(DecodeError::Other("not confirmed".into()));
        }
        Ok(RealizedProfit(0)) // decode your Profit event and net gas cost here
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let block = BlockContext { number: 0, timestamp: 0, gas_limit: 30_000_000, basefee: 0, coinbase: [0; 20] };
    let sim = RevmBackend::new(MyDbFactory, block, /* block_time_ms */ 12_000, /* timeout_fraction */ 0.5, /* concurrency */ 4);
    let predicted_decoder = MyProfitDecoder;
    let realized_decoder = MyRealizedDecoder;
    let mut guard = ReconcilePolicy::default_dual_guard();
    let mut history = ReconcileHistory::new();

    let candidate_tx = CandidateTx { caller: [0; 20], to: [0; 20], value: 0, data: vec![], gas_limit: 100_000 };
    let raw = sim.simulate(&candidate_tx).await?;
    let predicted = match predicted_decoder.decode_predicted(&raw) {
        Ok(p) if p.0 > 0 => p,
        _ => return Ok(()), // not profitable in simulation (or reverted), skip submission
    };

    let receipt = submit_and_wait(&candidate_tx).await?;
    let realized = realized_decoder.decode_realized(&receipt, &[])?;
    history.append(predicted, realized);

    match guard.evaluate(&history) {
        HaltDecision::Continue => {}
        HaltDecision::Halt(reason) => {
            eprintln!("phantom-guard halted strategy: {reason:?}");
            std::process::exit(1);
        }
    }
    Ok(())
}
```

This example is kept honest against the real API: it's a doctest in the `driftbrake` crate (`cargo test --doc -p driftbrake`), so it can't silently drift from what actually compiles the way a hand-maintained snippet can.

## Architecture

```
driftbrake/
├── driftbrake/            # facade crate: re-exports core + reconcile + receipt-poller
│   ├── core/                  # chain-agnostic traits, zero REVM/alloy dependency
│   │   ├── ProfitDecoder
│   │   ├── RealizedProfitDecoder
│   │   └── HaltPolicy
│   ├── reconcile/             # the halt-guard logic (phantom-guard) — see docs/ARCHITECTURE.md
│   └── receipt-poller/        # confirms receipts, decodes realized profit
├── revm-backend/          # separate, opt-in: concrete SimEngine (fork-and-simulate,
│                          # spawn_blocking, bounded concurrency) — not bundled into
│                          # `driftbrake` itself, see docs/ARCHITECTURE.md
├── telemetry/             # planned, not yet built — structured events for the benchmark harness
├── benchmark/              # docs/BENCHMARK.md's methodology as a runnable tool (cargo run -p driftbrake-benchmark)
└── examples/
    └── toy-arbitrage/     # minimal 2-pool arb wired end-to-end against a public testnet fork
```

For the full module breakdown, trait contracts, and why the two halt guards are independently necessary, see [`docs/ARCHITECTURE.md`](./docs/ARCHITECTURE.md).

## Benchmark

The fast/slow guard thresholds (0.50, 0.70) and window sizes (3, 20) are not asserted defaults — they're derived from a synthetic `(sim, realized)` sweep plotting false-halt rate against missed-catch rate. See [`docs/BENCHMARK.md`](./docs/BENCHMARK.md) to reproduce the sweep or re-tune it against your own historical data.

## Testing

```bash
cargo test --workspace
```

The `reconcile` module's ratio-direction regression test is the single most important test in the suite — see [`docs/whitepaper.md`](./docs/whitepaper.md#formal-properties--invariants) for why.

## Security

`driftbrake` gates a live trading loop, so a bug in `reconcile` or `revm-backend` has direct capital consequences. Report vulnerabilities per our [security policy](./SECURITY.md). This code has not been independently audited — review the `reconcile` and `HaltPolicy` logic yourself before running it against real capital.

## Contributing

See [`CONTRIBUTING.md`](./CONTRIBUTING.md) for dev environment setup, module boundaries, and PR guidelines.

## License

Released under the [MIT License](./LICENSE).
