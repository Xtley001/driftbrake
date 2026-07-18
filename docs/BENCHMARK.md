# Benchmark: Threshold Selection Methodology

> The methodology below is implemented as a runnable tool in `benchmark/`
> (crate `driftbrake-benchmark`): `cargo run --release -p driftbrake-benchmark`.
> It sweeps `T_f` (with `T_s` derived, `k_f`/`k_s` held at the defaults)
> against synthetic healthy/drifted data and prints the false-halt-rate /
> missed-catch-rate table described below — pass `--noise` and
> `--drift-mean` to see how the curve shifts under different assumptions.
> Synthetic noise comes from a small self-contained deterministic PRNG
> (`benchmark/src/rng.rs`), not the `rand` crate — reproducibility ("any
> reviewer can regenerate it," per the goal stated below) is a better fit
> for an explicitly-seeded, dependency-free generator than for an
> external crate whose default algorithm can change across versions. The
> full 4-dimensional grid over `(T_f, k_f, T_s, k_s)` and support for
> feeding in real historical data (see "Re-running the sweep on your own
> data" below) are natural extensions of `benchmark/src/main.rs`'s
> `build_grid()`, not yet built — `sweep()` itself already takes plain
> `(PredictedProfit, RealizedProfit)` sequences, so swapping the
> synthetic generator for a CSV/history loader is a small, isolated
> change whenever that's needed.

This document describes how the default guard parameters ($T_f = 0.50$, $k_f = 3$, $T_s = 0.70$, $k_s = 20$ — see [`whitepaper.md`](./whitepaper.md#7-parameters)) were derived, and how to reproduce or re-run the sweep against your own chain and strategy. The goal is to replace "we picked these numbers because they felt right" with a falsifiable, reproducible curve that any reviewer can regenerate without access to the private codebase these defaults were originally observed in.

## Why this exists as its own document

The [whitepaper](./whitepaper.md) states the mechanism's formal properties; it does not, on its own, justify *why* $0.50$ and $0.70$ specifically. A threshold is a tradeoff, not a fact — moving it in either direction trades false halts against missed catches, and the right point on that tradeoff depends on a chain's latency and volatility profile, not on the mechanism's correctness. This document makes that tradeoff visible and gives you the tool to re-derive it for your own conditions rather than inheriting numbers tuned for a different chain.

## Definitions

- **False halt**: the guard halts on a history that, absent noise, would have continued profitably — i.e., the underlying "true" performance was acceptable, but injected noise pushed the observed ratio below threshold anyway.
- **Missed catch**: the guard fails to halt on a history where the underlying "true" performance had genuinely degraded below an unacceptable level — i.e., real drift occurred but wasn't severe or consistent enough, at the current threshold, to trip either guard.

These are in tension by construction: tightening a threshold (raising $T_f$ or $T_s$, or shrinking $k_f$/$k_s$) reduces missed catches but increases false halts, and loosening it does the reverse. There is no threshold setting that minimizes both simultaneously — the benchmark's job is to make that tradeoff curve visible so a specific point on it can be chosen deliberately, per Property 3 in the whitepaper (fast-guard monotonicity), which guarantees the curve is well-behaved rather than erratic.

## Methodology

### 1. Synthetic pair generation

Generate a synthetic sequence of $(\hat{p}_i, r_i)$ pairs under two regimes:

- **Healthy regime**: $r_i = \hat{p}_i \cdot (1 + \varepsilon_i)$ where $\varepsilon_i$ is zero-mean noise with a configurable standard deviation representing normal simulation-to-realization variance (venue slippage, minor timing effects) that is not indicative of a strategy problem.
- **Drifted regime**: $r_i = \hat{p}_i \cdot (\mu_{\text{drift}} + \varepsilon_i)$ where $\mu_{\text{drift}} < 1$ represents genuine, sustained underperformance (the failure case the guards exist to catch), again with the same noise distribution layered on top.

A full benchmark run generates many independent sequences under each regime, at multiple noise standard deviations and multiple drift severities, so the resulting curve reflects a range of realistic conditions rather than one arbitrarily chosen scenario.

### 2. Parameter sweep

For each candidate $(T_f, k_f, T_s, k_s)$ combination in the sweep grid:

1. Run the `HaltPolicy` (Section 4.4 of the whitepaper) against every healthy-regime sequence and record whether it halted (a false halt if so) and, if so, after how many transactions.
2. Run the same `HaltPolicy` against every drifted-regime sequence and record whether it halted (a correct catch if so) and, if so, how many transactions elapsed before it did — this "time to catch" matters as much as whether it caught the drift at all, since a catch that arrives after most of the capital is already lost is a weaker result than an equally "successful" catch that arrives early.
3. Aggregate across all sequences at that parameter combination into a false-halt rate and a missed-catch rate.

### 3. Output

The sweep produces a curve — false-halt rate on one axis, missed-catch rate on the other — with each point corresponding to one parameter combination. The current defaults ($T_f = 0.50$, $k_f = 3$, $T_s = 0.70$, $k_s = 20$) sit at a specific point on this curve chosen for a particular chain's observed latency and volatility profile; they are a reasonable starting point, not a claim that this point is optimal for every chain.

```
false-halt rate
     ^
     |                                        *  (loose thresholds:
     |                                     *        few false halts,
     |                                  *           slow to catch drift)
     |                              *
     |                        *  <- current defaults sit near here
     |                  *
     |            *
     |       *
     |　*                                      (tight thresholds:
     +------------------------------------->     catches drift fast,
                missed-catch rate                 more false halts)
```

## Re-running the sweep on your own data

The recommended process for adopting `driftbrake` on a new chain, or after a strategy change that shifts its natural variance, is not to reuse the shipped defaults unexamined:

1. Run your strategy (or a shadow/paper-trading version of it) for a period long enough to accumulate a meaningful number of real `(sim, realized)` pairs under normal operating conditions.
2. Feed that historical data into the same sweep methodology described above, using your own observed noise characteristics in place of the synthetic defaults.
3. Select a threshold combination appropriate to your own risk tolerance — a team willing to accept more false halts in exchange for catching drift faster should sit further toward the tight end of the curve; a team for whom false halts are operationally expensive (e.g. each halt requires manual re-arming) should sit further toward the loose end.
4. Re-run this process after any material change to strategy logic, venue set, or chain — the curve is a property of your specific conditions, not a universal constant.

This reusability — the fact that threshold selection becomes a repeatable process rather than a one-time decision — is a deliberate part of the design, not an afterthought: it is what lets the same mechanism transfer meaningfully across chains and strategies instead of silently inheriting numbers tuned for someone else's conditions.

## Interpreting results responsibly

A few honest caveats, stated explicitly rather than left implicit:

- Synthetic noise models are an approximation. Real drift is rarely a clean shift in a single distributional parameter — it can be a step change, a slow ramp, or an intermittent flicker between healthy and drifted states. The sweep methodology above should be treated as a starting point for threshold intuition, not a guarantee that the chosen thresholds will behave identically against every real drift shape.
- A guard that never false-halts in the synthetic benchmark can still false-halt in production if real noise has heavier tails than the synthetic distribution assumed. Monitor false-halt rate in production and be willing to revisit thresholds rather than treating the benchmark as a one-time proof.
- The benchmark evaluates the `HaltPolicy` mechanism in isolation, assuming correctly implemented `ProfitDecoder` and `RealizedProfitDecoder` traits. It cannot detect or compensate for a decoder implementation bug — that remains the implementer's responsibility (see the Security Considerations table in [`whitepaper.md`](./whitepaper.md#6-security-considerations)).
