# Introduction

Pre-flight simulate a transaction. Halt the moment realized profit drifts from simulation predictions — before drift compounds into real capital loss.

```
predicted ──▶ simulate ──▶ submit ──▶ realized
    │                                    │
    └──────────────▶ reconcile ◀─────────┘
                        │
                     halt / continue
```

This book documents the design, mathematical invariants, and architectural boundaries of `driftbrake`. For installation and working examples, see the [repository README](https://github.com/Xtley001/driftbrake#readme).

## Resource Directory

| Resource | Location |
|---|---|
| **Source Repository** | [github.com/Xtley001/driftbrake](https://github.com/Xtley001/driftbrake) |
| **Rust Package** | [crates.io/crates/driftbrake](https://crates.io/crates/driftbrake) |
| **Python Package** | [pypi.org/project/driftbrake](https://pypi.org/project/driftbrake/) |
| **API Reference** | [docs.rs/driftbrake](https://docs.rs/driftbrake) |

## Navigation

- **[Architecture](./ARCHITECTURE.md)**: Workspace boundaries, concurrency patterns, persistence, and non-goals.
- **[Whitepaper](./whitepaper.md)**: Formal invariant proofs, multi-guard mathematics, and security trade-offs.
- **[API Reference](./API.md)**: Trait signatures, data types, and Python bindings.
- **[Benchmark Methodology](./BENCHMARK.md)**: Parameter calibration and empirical trade-off sweeps.
- **[Technical Post-Mortem](./technical-writeup.md)**: Case study of simulation failure modes observed in production.
