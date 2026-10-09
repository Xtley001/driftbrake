//! High-speed vectorized batch simulation and parameter sweep engine for Python.

use driftbrake_core::{HaltDecision, HaltPolicy, PredictedProfit, RealizedProfit, ReconcileHistory, RevertEvent};
use driftbrake_reconcile::{FastGuardConfig, ReconcilePolicy, RevertBurstConfig, SlowGuardConfig};
use pyo3::prelude::*;

#[pyclass]
#[derive(Debug, Clone)]
pub struct SweepResult {
    #[pyo3(get)]
    pub fast_threshold: f64,
    #[pyo3(get)]
    pub fast_window: usize,
    #[pyo3(get)]
    pub slow_threshold: f64,
    #[pyo3(get)]
    pub slow_window: usize,
    #[pyo3(get)]
    pub revert_limit: usize,
    #[pyo3(get)]
    pub total_halts: usize,
    #[pyo3(get)]
    pub first_halt_index: Option<usize>,
    #[pyo3(get)]
    pub total_drawdown_prevented: i128,
}

#[pymethods]
impl SweepResult {
    fn __repr__(&self) -> String {
        format!(
            "SweepResult(fast_T={}, slow_T={}, rev_limit={}, halts={}, first_halt={:?})",
            self.fast_threshold,
            self.slow_threshold,
            self.revert_limit,
            self.total_halts,
            self.first_halt_index
        )
    }
}

/// Evaluates a grid of parameter configurations against historical trade rows in microsecond Rust time.
#[pyfunction]
#[allow(clippy::too_many_arguments)]
pub fn run_sweep_raw(
    predicted: Vec<i128>,
    realized: Vec<i128>,
    is_revert: Vec<bool>,
    gas_used: Vec<u64>,
    gas_price: Vec<u128>,
    fast_thresholds: Vec<f64>,
    slow_thresholds: Vec<f64>,
    revert_limits: Vec<usize>,
) -> PyResult<Vec<SweepResult>> {
    let row_count = predicted.len();
    if realized.len() != row_count || is_revert.len() != row_count {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "Array lengths for predicted, realized, and is_revert must match",
        ));
    }

    let mut results = Vec::new();

    for &fast_t in &fast_thresholds {
        for &slow_t in &slow_thresholds {
            if fast_t >= slow_t {
                // Whitepaper Property 2: slow threshold must exceed fast threshold
                continue;
            }
            for &rev_lim in &revert_limits {
                let mut policy = ReconcilePolicy::new(
                    FastGuardConfig {
                        threshold: fast_t,
                        window: 3,
                    },
                    SlowGuardConfig {
                        threshold: slow_t,
                        window: 20,
                    },
                )
                .with_revert_burst(RevertBurstConfig { limit: rev_lim });

                let mut history = ReconcileHistory::new();
                let mut total_halts = 0;
                let mut first_halt_index = None;
                let mut total_drawdown = 0i128;

                for i in 0..row_count {
                    if is_revert[i] {
                        let gu = if i < gas_used.len() { gas_used[i] } else { 21_000 };
                        let gp = if i < gas_price.len() { gas_price[i] } else { 0 };
                        history.record_revert(RevertEvent {
                            tx_hash: [0u8; 32],
                            block_number: i as u64,
                            reason: None,
                            gas_used: gu,
                            effective_gas_price: gp,
                        });
                    } else {
                        history.append(PredictedProfit(predicted[i]), RealizedProfit(realized[i]));
                    }

                    match policy.evaluate(&history) {
                        HaltDecision::Continue => {}
                        HaltDecision::Halt(_) => {
                            total_halts += 1;
                            if first_halt_index.is_none() {
                                first_halt_index = Some(i);
                            }
                            if !is_revert[i] && predicted[i] > realized[i] {
                                total_drawdown += predicted[i] - realized[i];
                            }
                        }
                    }
                }

                results.push(SweepResult {
                    fast_threshold: fast_t,
                    fast_window: 3,
                    slow_threshold: slow_t,
                    slow_window: 20,
                    revert_limit: rev_lim,
                    total_halts,
                    first_halt_index,
                    total_drawdown_prevented: total_drawdown,
                });
            }
        }
    }

    Ok(results)
}
