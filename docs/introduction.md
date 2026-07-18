# driftbrake

Pre-flight simulate a transaction. Halt the moment realized profit drifts
from what simulation predicted — before drift compounds into real
capital loss.

```
predicted ──▶ simulate ──▶ submit ──▶ realized
    │                                    │
    └──────────────▶ reconcile ◀─────────┘
                        │
                     halt / continue
```

This book covers the design and reasoning behind `driftbrake`. For
installation and a working code example, see the
[repository README](https://github.com/Xtley001/driftbrake#readme) — this
book is the "why," the README is the "how."

| | |
|---|---|
| **Source** | [github.com/Xtley001/driftbrake](https://github.com/Xtley001/driftbrake) |
| **Crate** | [crates.io/crates/driftbrake](https://crates.io/crates/driftbrake) |
| **API docs** | [docs.rs/driftbrake](https://docs.rs/driftbrake) |

## Where to start

- New to the project? Read **[Architecture](./ARCHITECTURE.md)** first —
  the module map and the three design goals everything else follows from.
- Want the formal argument for *why* the halt guard works? Read the
  **[Whitepaper](./whitepaper.md)** — the properties and proofs behind
  the fast guard / slow guard split.
- Implementing your own `SimEngine` or `ProfitDecoder`? Go straight to
  the **[API Reference](./API.md)** for exact trait signatures.
- Wondering where the default thresholds (`0.50/3`, `0.70/20`) came
  from, or how to re-derive your own? See **[Benchmark Methodology](./BENCHMARK.md)**.
