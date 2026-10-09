# Benchmark Methodology

This document outlines the parameter selection methodology implemented in `driftbrake-benchmark` (`cargo run --release -p driftbrake-benchmark`).

## Overview

Guard parameters ($T_f = 0.50, k_f = 3, T_s = 0.70, k_s = 20$) balance two competing operational risks:

- **False Halt**: The guard halts on an execution sequence where performance remains acceptable, but normal market noise triggered threshold conditions.
- **Missed Catch**: The guard fails to halt on an execution sequence experiencing genuine simulation drift, allowing capital losses to accumulate.

Tightening thresholds (raising $T_f$ or $T_s$, or reducing window sizes) decreases missed catches at the expense of increased false halts. Loosening thresholds reverses the trade-off.

## Synthetic Data Regimes

The benchmark evaluates candidate parameter sets against synthetic transaction pairs $(\hat{p}_i, r_i)$ generated across two regimes:

- **Healthy Regime**: $r_i = \hat{p}_i \cdot (1 + \varepsilon_i)$ where $\varepsilon_i \sim \mathcal{N}(0, \sigma^2)$ models ordinary execution slippage.
- **Drifted Regime**: $r_i = \hat{p}_i \cdot (\mu_{\text{drift}} + \varepsilon_i)$ where $\mu_{\text{drift}} < 1.0$ models systematic divergence between simulation and on-chain reality.

To ensure deterministic reproducibility across platforms, pseudorandom sequences are generated via an internal Xorshift PRNG rather than non-deterministic system seeds.

## Parameter Sweep Procedure

For each candidate configuration tuple $(T_f, k_f, T_s, k_s)$:

1. **Healthy Evaluation**: Execute the policy across healthy sequences. Record occurrences and indices of false halts.
2. **Drifted Evaluation**: Execute the policy across drifted sequences. Record occurrences and detection latencies (time-to-catch).
3. **Metric Aggregation**: Compute the false-halt rate ($P_{\text{FH}}$) and missed-catch rate ($P_{\text{MC}}$).

```
False Halt Rate
     ^
     |                                        *  (Permissive thresholds:
     |                                     *      minimal false halts,
     |                                  *         delayed drift detection)
     |                              *
     |                        *  <- Shipped defaults (0.50 / 0.70)
     |                  *
     |            *
     |       *
     |  *                                      (Strict thresholds:
     +------------------------------------->     rapid drift detection,
                Missed Catch Rate                 frequent false halts)
```

## Running the Benchmark

```bash
# Run default parameter sweep
cargo run --release -p driftbrake-benchmark

# Run custom noise and drift scenarios
cargo run --release -p driftbrake-benchmark -- --noise 0.15 --drift-mean 0.40
```

## Calibrating on Historical Data

When deploying Driftbrake to a new blockchain or strategy profile:

1. **Collect Historical Pairs**: Record at least 500 confirmed $(\hat{p}, r)$ transactions under standard operating conditions.
2. **Measure Natural Variance**: Calculate sample standard deviation $\sigma_{\text{slippage}}$ and historical realization ratios.
3. **Run Backtest Grid**: Use `driftbrake-py`'s `run_sweep_raw` to simulate candidate threshold grids against your historical series.
4. **Select Risk Point**: Choose parameters matching your operational risk appetite (e.g. prioritizing capital preservation vs. minimizing manual reset interventions).
