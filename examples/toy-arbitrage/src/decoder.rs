//! `ProfitDecoder` and `RealizedProfitDecoder` for this example's toy
//! executor contract.
//!
//! The (hypothetical, not actually deployed) executor exposes:
//!
//! ```solidity
//! function executeArb(address buyPool, address sellPool, uint256 amount)
//!     external returns (uint256 profit);
//! event Profit(uint256 amount);
//! ```
//!
//! It's a flash-loan-style single-call executor (borrow, swap on
//! `buyPool`, swap back on `sellPool`, repay, all within one
//! transaction) — see `README.md`'s "Known limitations": no inventory to
//! unwind, which is why it's a single call rather than a multi-step
//! sequence with its own state machine.

use std::sync::atomic::{AtomicU64, Ordering};

use driftbrake_core::{
    DecodeError, Log, PredictedProfit, ProfitDecoder, RawSimOutput, RealizedProfit,
    RealizedProfitDecoder, TxReceipt, TxStatus,
};

use crate::abi::{decode_u256_word, event_topic0};

pub const EXECUTE_ARB_SIGNATURE: &str = "executeArb(address,address,uint256)";
pub const PROFIT_EVENT_SIGNATURE: &str = "Profit(uint256)";

/// Decodes the executor's `executeArb` return value into a
/// `PredictedProfit`.
pub struct ToyArbProfitDecoder;

impl ProfitDecoder for ToyArbProfitDecoder {
    fn decode_predicted(&self, raw: &RawSimOutput) -> Result<PredictedProfit, DecodeError> {
        // Contract (docs/API.md): must not assume `revert_reason.is_none()`.
        if raw.revert_reason.is_some() {
            return Err(DecodeError::Reverted);
        }
        let profit = decode_u256_word(&raw.return_data, 0).ok_or_else(|| {
            DecodeError::MalformedData("executeArb return data too short for one uint256".into())
        })?;
        Ok(PredictedProfit(profit as i128))
    }
}

/// Decodes a confirmed receipt's `Profit(uint256)` log into a
/// `RealizedProfit`, net of gas cost.
pub struct ToyArbRealizedDecoder;

impl RealizedProfitDecoder for ToyArbRealizedDecoder {
    fn decode_realized(
        &self,
        receipt: &TxReceipt,
        logs: &[Log],
    ) -> Result<RealizedProfit, DecodeError> {
        // Defensive per docs/API.md: even though receipt-poller is
        // responsible for only calling this on a confirmed receipt, treat
        // an unexpected reverted status as an error rather than silently
        // returning zero.
        if receipt.status != TxStatus::Confirmed {
            return Err(DecodeError::Other(
                "decode_realized called on a non-confirmed receipt".into(),
            ));
        }

        let topic0 = event_topic0(PROFIT_EVENT_SIGNATURE);
        let profit_log = logs
            .iter()
            .find(|log| log.topics.first() == Some(&topic0))
            .ok_or(DecodeError::MissingProfitEvent)?;

        let gross = decode_u256_word(&profit_log.data, 0).ok_or_else(|| {
            DecodeError::MalformedData("Profit event data too short for one uint256".into())
        })?;

        // Implementer obligation (docs/API.md): net gas cost ourselves —
        // driftbrake does not do this automatically.
        let gas_cost = receipt.gas_used as u128 * receipt.effective_gas_price;
        let net = gross as i128 - gas_cost as i128;
        Ok(RealizedProfit(net))
    }
}

/// Wraps a `RealizedProfitDecoder` and, after `drift_after` calls,
/// artificially dampens the profit figure it returns — this is exactly
/// and only what `--inject-drift` does (see `README.md`'s "Forcing a
/// halt" section). It is a demo/testing tool, not a realistic model of
/// drift: real drift comes from market conditions changing between
/// simulation and confirmation, not from a decoder lying about what it
/// sees.
pub struct DriftInjectingDecoder<D> {
    inner: D,
    calls_so_far: AtomicU64,
    drift_after: u64,
    /// Multiplier applied to the gross (pre-gas) profit figure once
    /// drift injection kicks in. `0.3` means "this trade now realizes
    /// only 30% of what the un-drifted decoder would have reported."
    damping_factor: f64,
}

impl<D: RealizedProfitDecoder> DriftInjectingDecoder<D> {
    pub fn new(inner: D, drift_after: u64, damping_factor: f64) -> Self {
        Self {
            inner,
            calls_so_far: AtomicU64::new(0),
            drift_after,
            damping_factor,
        }
    }
}

impl<D: RealizedProfitDecoder> RealizedProfitDecoder for DriftInjectingDecoder<D> {
    fn decode_realized(
        &self,
        receipt: &TxReceipt,
        logs: &[Log],
    ) -> Result<RealizedProfit, DecodeError> {
        let realized = self.inner.decode_realized(receipt, logs)?;
        let n = self.calls_so_far.fetch_add(1, Ordering::SeqCst);
        if n < self.drift_after {
            return Ok(realized);
        }
        let dampened = (realized.0 as f64 * self.damping_factor).round() as i128;
        Ok(RealizedProfit(dampened))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::{encode_arb_call, function_selector};

    fn confirmed_receipt() -> TxReceipt {
        TxReceipt {
            tx_hash: [0u8; 32],
            block_number: 1,
            status: TxStatus::Confirmed,
            gas_used: 200_000,
            effective_gas_price: 20_000_000_000, // 20 gwei
        }
    }

    fn profit_log(amount: u128) -> Log {
        let mut data = vec![0u8; 32];
        data[16..].copy_from_slice(&amount.to_be_bytes());
        Log {
            address: vec![0xEE; 20],
            topics: vec![event_topic0(PROFIT_EVENT_SIGNATURE)],
            data,
        }
    }

    #[test]
    fn decode_predicted_reads_the_single_return_word() {
        let mut return_data = vec![0u8; 32];
        return_data[16..].copy_from_slice(&5_000u128.to_be_bytes());
        let raw = RawSimOutput {
            return_data,
            gas_used: 150_000,
            revert_reason: None,
            logs: vec![],
        };
        assert_eq!(
            ToyArbProfitDecoder.decode_predicted(&raw),
            Ok(PredictedProfit(5_000))
        );
    }

    #[test]
    fn decode_predicted_rejects_a_reverted_simulation() {
        let raw = RawSimOutput {
            return_data: vec![],
            gas_used: 21_000,
            revert_reason: Some("reverted".into()),
            logs: vec![],
        };
        assert_eq!(
            ToyArbProfitDecoder.decode_predicted(&raw),
            Err(DecodeError::Reverted)
        );
    }

    #[test]
    fn decode_realized_nets_gas_cost_from_the_profit_event() {
        let receipt = confirmed_receipt(); // 200_000 * 20e9 = 4e15 gas cost
        let logs = vec![profit_log(5_000_000_000_000_000)]; // 5e15 gross
        let realized = ToyArbRealizedDecoder
            .decode_realized(&receipt, &logs)
            .unwrap();
        assert_eq!(realized, RealizedProfit(1_000_000_000_000_000)); // 5e15 - 4e15
    }

    #[test]
    fn decode_realized_errors_on_missing_profit_event() {
        let receipt = confirmed_receipt();
        assert_eq!(
            ToyArbRealizedDecoder.decode_realized(&receipt, &[]),
            Err(DecodeError::MissingProfitEvent)
        );
    }

    #[test]
    fn decode_realized_refuses_a_non_confirmed_receipt_defensively() {
        let mut receipt = confirmed_receipt();
        receipt.status = TxStatus::Reverted;
        let logs = vec![profit_log(1_000)];
        assert!(ToyArbRealizedDecoder
            .decode_realized(&receipt, &logs)
            .is_err());
    }

    #[test]
    fn drift_injecting_decoder_passes_through_before_the_threshold() {
        let decoder = DriftInjectingDecoder::new(ToyArbRealizedDecoder, 2, 0.3);
        let receipt = confirmed_receipt();
        let logs = vec![profit_log(5_000_000_000_000_000)];
        // Calls 0 and 1 (n < drift_after=2) pass through undampened.
        assert_eq!(
            decoder.decode_realized(&receipt, &logs).unwrap(),
            RealizedProfit(1_000_000_000_000_000)
        );
        assert_eq!(
            decoder.decode_realized(&receipt, &logs).unwrap(),
            RealizedProfit(1_000_000_000_000_000)
        );
    }

    #[test]
    fn drift_injecting_decoder_dampens_after_the_threshold() {
        let decoder = DriftInjectingDecoder::new(ToyArbRealizedDecoder, 0, 0.3);
        let receipt = confirmed_receipt();
        let logs = vec![profit_log(5_000_000_000_000_000)];
        // n=0 >= drift_after=0 immediately, so this call is dampened.
        let realized = decoder.decode_realized(&receipt, &logs).unwrap();
        assert_eq!(
            realized,
            RealizedProfit((1_000_000_000_000_000f64 * 0.3).round() as i128)
        );
    }

    #[test]
    fn encode_arb_call_uses_the_documented_executor_selector() {
        let selector = function_selector(EXECUTE_ARB_SIGNATURE);
        let data = encode_arb_call(selector, [1u8; 20], [2u8; 20], 1_000);
        assert_eq!(&data[0..4], &selector);
    }
}
