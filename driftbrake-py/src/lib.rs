mod batch;

use driftbrake_core::{
    HaltDecision, HaltPolicy, HaltReason, PredictedProfit, RealizedProfit, ReconcileHistory,
    RevertEvent,
};
use driftbrake_reconcile::{
    DrawdownConfig, FastGuardConfig, ReconcilePolicy, RevertBurstConfig, RevertGasConfig,
    SlowGuardConfig, VolumeWeightedConfig,
};
use pyo3::create_exception;
use pyo3::prelude::*;
use pyo3::types::PyDict;

create_exception!(driftbrake, StrategyHaltedError, pyo3::exceptions::PyException);

#[pyclass(name = "HaltDecision")]
#[derive(Clone)]
pub struct PyHaltDecision {
    #[pyo3(get)]
    pub should_halt: bool,
    #[pyo3(get)]
    pub reason_name: Option<String>,
    decision: HaltDecision,
}

#[pymethods]
impl PyHaltDecision {
    #[getter]
    fn details<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(py);
        if let HaltDecision::Halt(ref reason) = self.decision {
            match reason {
                HaltReason::FastGuard { window, threshold } => {
                    dict.set_item("type", "FastGuard")?;
                    dict.set_item("threshold", threshold)?;
                    dict.set_item("recent_ratios", window.clone())?;
                }
                HaltReason::SlowGuard {
                    mean_ratio,
                    window_size,
                    threshold,
                } => {
                    dict.set_item("type", "SlowGuard")?;
                    dict.set_item("mean_ratio", mean_ratio)?;
                    dict.set_item("window_size", window_size)?;
                    dict.set_item("threshold", threshold)?;
                }
                HaltReason::RevertBurst {
                    consecutive_reverts,
                    limit,
                } => {
                    dict.set_item("type", "RevertBurst")?;
                    dict.set_item("consecutive_reverts", consecutive_reverts)?;
                    dict.set_item("limit", limit)?;
                }
                HaltReason::RevertGasBudgetExceeded {
                    gas_cost_burned,
                    budget_limit,
                    window,
                } => {
                    dict.set_item("type", "RevertGasBudgetExceeded")?;
                    dict.set_item("gas_cost_burned", *gas_cost_burned as u64)?;
                    dict.set_item("budget_limit", *budget_limit as u64)?;
                    dict.set_item("window", window)?;
                }
                HaltReason::VolumeWeightedDrift {
                    weighted_ratio,
                    threshold,
                    window,
                } => {
                    dict.set_item("type", "VolumeWeightedDrift")?;
                    dict.set_item("weighted_ratio", weighted_ratio)?;
                    dict.set_item("threshold", threshold)?;
                    dict.set_item("window", window)?;
                }
                HaltReason::NetDrawdownExceeded {
                    net_slippage_loss,
                    max_drawdown_limit,
                    window,
                } => {
                    dict.set_item("type", "NetDrawdownExceeded")?;
                    dict.set_item("net_slippage_loss", *net_slippage_loss as i64)?;
                    dict.set_item("max_drawdown_limit", *max_drawdown_limit as i64)?;
                    dict.set_item("window", window)?;
                }
                HaltReason::Custom(s) => {
                    dict.set_item("type", "Custom")?;
                    dict.set_item("message", s)?;
                }
            }
        }
        Ok(dict)
    }

    /// Raises `StrategyHaltedError` if `should_halt` is True; otherwise no-op.
    fn unwrap_or_raise(&self) -> PyResult<()> {
        if self.should_halt {
            let reason_str = self
                .reason_name
                .as_deref()
                .unwrap_or("Unknown driftbrake halt");
            Err(StrategyHaltedError::new_err(format!(
                "Execution halted by Driftbrake: {}",
                reason_str
            )))
        } else {
            Ok(())
        }
    }

    fn __repr__(&self) -> String {
        if self.should_halt {
            format!(
                "HaltDecision(should_halt=True, reason='{}')",
                self.reason_name.as_deref().unwrap_or("")
            )
        } else {
            "HaltDecision(should_halt=False)".to_string()
        }
    }
}

#[pyclass(name = "ReconcileHistory")]
#[derive(Default, Clone)]
pub struct PyReconcileHistory {
    inner: ReconcileHistory,
}

#[pymethods]
impl PyReconcileHistory {
    #[new]
    fn new() -> Self {
        Self {
            inner: ReconcileHistory::new(),
        }
    }

    /// Append a confirmed (predicted, realized) profit pair.
    fn append(&mut self, predicted_profit: i128, realized_profit: i128) {
        self.inner
            .append(PredictedProfit(predicted_profit), RealizedProfit(realized_profit));
    }

    /// Record a confirmed-but-reverted transaction on-chain.
    #[pyo3(signature = (tx_hash, block_number, gas_used, effective_gas_price, reason=None))]
    fn record_revert(
        &mut self,
        tx_hash: [u8; 32],
        block_number: u64,
        gas_used: u64,
        effective_gas_price: u128,
        reason: Option<String>,
    ) {
        self.inner.record_revert(RevertEvent {
            tx_hash,
            block_number,
            reason,
            gas_used,
            effective_gas_price,
        });
    }

    /// Returns the most recent n realized/predicted ratios, oldest first.
    fn recent_ratios(&self, n: usize) -> Vec<f64> {
        self.inner.recent_ratios(n)
    }

    /// Total confirmed pairs recorded.
    fn __len__(&self) -> usize {
        self.inner.pairs.len()
    }

    /// Total on-chain reverts recorded.
    fn total_reverts(&self) -> usize {
        self.inner.reverts.len()
    }

    /// Consecutive reverts at the tail of the timeline.
    fn consecutive_reverts(&self) -> usize {
        self.inner.consecutive_reverts()
    }

    /// Total gas fee burned on reverts in the last `window` timeline entries.
    fn recent_revert_gas_burned(&self, window: usize) -> u128 {
        self.inner.recent_revert_gas_burned(window)
    }

    fn __repr__(&self) -> String {
        format!(
            "ReconcileHistory(pairs={}, reverts={}, consecutive_reverts={})",
            self.inner.pairs.len(),
            self.inner.reverts.len(),
            self.inner.consecutive_reverts()
        )
    }
}

#[pyclass(name = "ReconcilePolicy")]
#[derive(Clone)]
pub struct PyReconcilePolicy {
    inner: ReconcilePolicy,
}

#[pymethods]
impl PyReconcilePolicy {
    /// Creates the default dual-guard policy (Fast: 3 @ 0.50, Slow: 20 @ 0.70).
    #[staticmethod]
    fn default_dual_guard() -> Self {
        Self {
            inner: ReconcilePolicy::default_dual_guard(),
        }
    }

    /// Creates an institutional-grade multi-guard policy.
    #[staticmethod]
    #[pyo3(signature = (
        fast_threshold=0.50,
        fast_window=3,
        slow_threshold=0.70,
        slow_window=20,
        revert_burst_limit=Some(3),
        volume_weighted_threshold=Some(0.75),
        volume_weighted_window=20,
        revert_gas_budget=None,
        revert_gas_window=10,
        drawdown_limit=None,
        drawdown_window=20
    ))]
    #[allow(clippy::too_many_arguments)]
    fn institutional(
        fast_threshold: f64,
        fast_window: usize,
        slow_threshold: f64,
        slow_window: usize,
        revert_burst_limit: Option<usize>,
        volume_weighted_threshold: Option<f64>,
        volume_weighted_window: usize,
        revert_gas_budget: Option<u128>,
        revert_gas_window: usize,
        drawdown_limit: Option<i128>,
        drawdown_window: usize,
    ) -> Self {
        let mut policy = ReconcilePolicy::new(
            FastGuardConfig {
                threshold: fast_threshold,
                window: fast_window,
            },
            SlowGuardConfig {
                threshold: slow_threshold,
                window: slow_window,
            },
        );

        if let Some(limit) = revert_burst_limit {
            policy = policy.with_revert_burst(RevertBurstConfig { limit });
        }
        if let Some(threshold) = volume_weighted_threshold {
            policy = policy.with_volume_weighted(VolumeWeightedConfig {
                threshold,
                window: volume_weighted_window,
            });
        }
        if let Some(budget_limit) = revert_gas_budget {
            policy = policy.with_revert_gas(RevertGasConfig {
                budget_limit,
                window: revert_gas_window,
            });
        }
        if let Some(max_drawdown_limit) = drawdown_limit {
            policy = policy.with_drawdown(DrawdownConfig {
                max_drawdown_limit,
                window: drawdown_window,
            });
        }

        Self { inner: policy }
    }

    /// Evaluates history against all active guards.
    fn evaluate(&mut self, history: &PyReconcileHistory) -> PyHaltDecision {
        let decision = self.inner.evaluate(&history.inner);
        match decision {
            HaltDecision::Continue => PyHaltDecision {
                should_halt: false,
                reason_name: None,
                decision,
            },
            HaltDecision::Halt(ref reason) => {
                let name = match reason {
                    HaltReason::FastGuard { .. } => "FastGuard",
                    HaltReason::SlowGuard { .. } => "SlowGuard",
                    HaltReason::RevertBurst { .. } => "RevertBurst",
                    HaltReason::RevertGasBudgetExceeded { .. } => "RevertGasBudgetExceeded",
                    HaltReason::VolumeWeightedDrift { .. } => "VolumeWeightedDrift",
                    HaltReason::NetDrawdownExceeded { .. } => "NetDrawdownExceeded",
                    HaltReason::Custom(_) => "Custom",
                };
                PyHaltDecision {
                    should_halt: true,
                    reason_name: Some(name.to_string()),
                    decision,
                }
            }
        }
    }
}

/// The driftbrake_rs extension module.
#[pymodule]
fn driftbrake_rs(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyReconcileHistory>()?;
    m.add_class::<PyReconcilePolicy>()?;
    m.add_class::<PyHaltDecision>()?;
    m.add_class::<batch::SweepResult>()?;
    m.add("StrategyHaltedError", m.py().get_type::<StrategyHaltedError>())?;
    m.add_function(wrap_pyfunction!(batch::run_sweep_raw, m)?)?;
    Ok(())
}
