//! Minimal pool-price reading, deliberately simplified (see
//! `README.md`) — not a general-purpose venue integration. Reads
//! reserves via a `getReserves()` view call run through the same
//! `SimEngine` used for the actual arbitrage simulation, rather than a
//! separate direct-`eth_call` path, so pool-price reads and the arb
//! simulation itself see exactly the same forked state.

use driftbrake_core::{RawSimOutput, SimEngine};
use driftbrake_revm_backend::CandidateTx;

use crate::abi::{decode_u256_word, encode_no_arg_call, function_selector};

pub const GET_RESERVES_SIGNATURE: &str = "getReserves()";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PoolReserves {
    pub reserve0: u128,
    pub reserve1: u128,
}

impl PoolReserves {
    /// Spot price of token0 in terms of token1 (constant-product AMM,
    /// ignoring fees — deliberately simplified, see module docs).
    pub fn price_token0_in_token1(&self) -> f64 {
        self.reserve1 as f64 / self.reserve0 as f64
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PoolReadError<E> {
    #[error("simulation failed: {0}")]
    Sim(E),
    #[error("getReserves() returned data too short to decode two reserves")]
    Malformed,
}

pub fn decode_reserves(raw: &RawSimOutput) -> Option<PoolReserves> {
    let reserve0 = decode_u256_word(&raw.return_data, 0)?;
    let reserve1 = decode_u256_word(&raw.return_data, 1)?;
    Some(PoolReserves { reserve0, reserve1 })
}

/// Read a pool's reserves by simulating `getReserves()` against the
/// currently forked state.
pub async fn read_reserves<Engine>(
    engine: &Engine,
    caller: [u8; 20],
    pool: [u8; 20],
) -> Result<PoolReserves, PoolReadError<driftbrake_core::SimError>>
where
    Engine: SimEngine<Tx = CandidateTx>,
{
    let tx = CandidateTx {
        caller,
        to: pool,
        value: 0,
        data: encode_no_arg_call(function_selector(GET_RESERVES_SIGNATURE)),
        gas_limit: 100_000,
    };
    let raw = engine.simulate(&tx).await.map_err(PoolReadError::Sim)?;
    decode_reserves(&raw).ok_or(PoolReadError::Malformed)
}

/// Relative price divergence between two pools of the same pair, as a
/// fraction of the cheaper pool's price (e.g. `0.02` = 2% divergence).
/// The example's main loop compares this against a gas-cost-derived
/// threshold before bothering to simulate a real arb transaction.
pub fn price_divergence(a: PoolReserves, b: PoolReserves) -> f64 {
    let (pa, pb) = (a.price_token0_in_token1(), b.price_token0_in_token1());
    let (low, high) = if pa < pb { (pa, pb) } else { (pb, pa) };
    (high - low) / low
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reserves_word(reserve0: u128, reserve1: u128) -> Vec<u8> {
        let mut data = vec![0u8; 64];
        data[16..32].copy_from_slice(&reserve0.to_be_bytes());
        data[48..64].copy_from_slice(&reserve1.to_be_bytes());
        data
    }

    #[test]
    fn decode_reserves_reads_both_words() {
        let raw = RawSimOutput {
            return_data: reserves_word(1_000_000, 2_000_000),
            gas_used: 30_000,
            revert_reason: None,
            logs: vec![],
        };
        assert_eq!(
            decode_reserves(&raw),
            Some(PoolReserves {
                reserve0: 1_000_000,
                reserve1: 2_000_000
            })
        );
    }

    #[test]
    fn price_divergence_is_symmetric_and_relative_to_the_cheaper_pool() {
        let a = PoolReserves {
            reserve0: 1_000,
            reserve1: 2_000,
        }; // price = 2.0
        let b = PoolReserves {
            reserve0: 1_000,
            reserve1: 2_100,
        }; // price = 2.1
        let divergence = price_divergence(a, b);
        assert!((divergence - 0.05).abs() < 1e-9, "got {divergence}");
        assert_eq!(price_divergence(a, b), price_divergence(b, a));
    }

    #[test]
    fn price_divergence_is_zero_for_identical_pools() {
        let a = PoolReserves {
            reserve0: 500,
            reserve1: 1_000,
        };
        assert_eq!(price_divergence(a, a), 0.0);
    }
}
