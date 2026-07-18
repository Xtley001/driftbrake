//! Runs the parameter sweep described in `docs/BENCHMARK.md` with
//! synthetic data and prints the resulting false-halt-rate /
//! missed-catch-rate curve as a table.
//!
//! ```text
//! cargo run --release -p driftbrake-benchmark
//! cargo run --release -p driftbrake-benchmark -- --seed 7 --sequences 500 --length 40 --noise 0.05 --drift-mean 0.4
//! ```
//!
//! This uses **synthetic** noise, not real chain data — see
//! `docs/BENCHMARK.md`'s "Re-running the sweep on your own data" section
//! for how to point this methodology at your own historical
//! `(predicted, realized)` pairs instead. The synthetic run here is
//! useful for sanity-checking the shipped defaults and for seeing the
//! shape of the tradeoff curve, not for deriving thresholds for a
//! specific real chain.

use driftbrake_benchmark::rng::Rng;
use driftbrake_benchmark::sweep::{sweep, SweepResult};
use driftbrake_benchmark::synthetic::{generate_batch, RegimeParams};
use driftbrake_reconcile::{FastGuardConfig, SlowGuardConfig};

struct Args {
    seed: u64,
    sequences: usize,
    length: usize,
    noise_std_dev: f64,
    drift_mean: f64,
    predicted_magnitude: i128,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            seed: 42,
            sequences: 500,
            length: 40,
            noise_std_dev: 0.05,
            drift_mean: 0.4,
            predicted_magnitude: 1_000_000,
        }
    }
}

fn parse_args() -> Args {
    let mut args = Args::default();
    let raw: Vec<String> = std::env::args().collect();
    let get = |flag: &str| {
        raw.iter()
            .position(|a| a == flag)
            .and_then(|i| raw.get(i + 1))
            .cloned()
    };

    if let Some(v) = get("--seed") {
        args.seed = v.parse().unwrap_or(args.seed);
    }
    if let Some(v) = get("--sequences") {
        args.sequences = v.parse().unwrap_or(args.sequences);
    }
    if let Some(v) = get("--length") {
        args.length = v.parse().unwrap_or(args.length);
    }
    if let Some(v) = get("--noise") {
        args.noise_std_dev = v.parse().unwrap_or(args.noise_std_dev);
    }
    if let Some(v) = get("--drift-mean") {
        args.drift_mean = v.parse().unwrap_or(args.drift_mean);
    }
    args
}

/// The sweep grid: `T_f` (and a derived `T_s = T_f + 0.2`, clamped below
/// 1.0) across a range, with `k_f`/`k_s` held at the whitepaper's
/// defaults. This traces the same one-dimensional slice of the tradeoff
/// curve `docs/BENCHMARK.md`'s ASCII diagram illustrates — a full 4D grid
/// over `(T_f, k_f, T_s, k_s)` is straightforward to add by nesting more
/// loops here, but a 1D slice is enough to see the tradeoff shape and
/// sanity-check the shipped defaults.
fn build_grid() -> Vec<(FastGuardConfig, SlowGuardConfig)> {
    let fast_thresholds: [f64; 7] = [0.20, 0.30, 0.40, 0.50, 0.60, 0.70, 0.80];
    fast_thresholds
        .iter()
        .map(|&t_f| {
            let t_s = (t_f + 0.20).min(0.95);
            (
                FastGuardConfig {
                    threshold: t_f,
                    window: 3,
                },
                SlowGuardConfig {
                    threshold: t_s,
                    window: 20,
                },
            )
        })
        .collect()
}

fn print_table(results: &[SweepResult]) {
    println!(
        "{:>6} {:>4} {:>6} {:>4}  {:>15} {:>18} {:>18}",
        "T_f", "k_f", "T_s", "k_s", "false_halt_rate", "missed_catch_rate", "mean_time_to_catch"
    );
    for r in results {
        let mean_time = r
            .mean_time_to_catch
            .map(|t| format!("{t:.1}"))
            .unwrap_or_else(|| "n/a".to_string());
        println!(
            "{:>6.2} {:>4} {:>6.2} {:>4}  {:>15.3} {:>18.3} {:>18}",
            r.fast.threshold,
            r.fast.window,
            r.slow.threshold,
            r.slow.window,
            r.false_halt_rate,
            r.missed_catch_rate,
            mean_time
        );
    }
}

fn main() {
    let args = parse_args();
    println!(
        "driftbrake-benchmark: seed={}, sequences={}, length={}, noise_std_dev={}, drift_mean={}",
        args.seed, args.sequences, args.length, args.noise_std_dev, args.drift_mean
    );
    println!(
        "(synthetic data — see docs/BENCHMARK.md to re-run against your own chain's history)\n"
    );

    let mut rng = Rng::new(args.seed);
    let healthy = generate_batch(
        &mut rng,
        RegimeParams::healthy(args.noise_std_dev),
        args.sequences,
        args.length,
        args.predicted_magnitude,
    );
    let drifted = generate_batch(
        &mut rng,
        RegimeParams::drifted(args.noise_std_dev, args.drift_mean),
        args.sequences,
        args.length,
        args.predicted_magnitude,
    );

    let grid = build_grid();
    let results = sweep(&healthy, &drifted, &grid);
    print_table(&results);

    println!(
        "\nShipped defaults are T_f=0.50, k_f=3, T_s=0.70, k_s=20 (see docs/whitepaper.md Section 7)."
    );
    println!("Re-run with different --noise / --drift-mean values to see how the curve shifts for your assumptions.");
}
