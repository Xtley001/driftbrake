//! Deliberately minimal ABI encode/decode helpers — just enough for this
//! example's two contract interfaces (a Uniswap-V2-shaped pair's
//! `getReserves()`, and the example's own toy 2-leg-swap executor). Not a
//! general-purpose ABI library; see `docs/ARCHITECTURE.md`'s non-goals
//! for why `driftbrake` itself doesn't ship one either — that's
//! deliberately left to whatever ABI crate a real integration already
//! uses (`ethers`, `alloy`, etc.).

use driftbrake_revm_backend::keccak256;

/// First 4 bytes of `keccak256(signature)`, e.g.
/// `function_selector("getReserves()")`.
pub fn function_selector(signature: &str) -> [u8; 4] {
    let hash = keccak256(signature.as_bytes());
    [hash[0], hash[1], hash[2], hash[3]]
}

/// keccak256 of an event signature, e.g. `event_topic0("Profit(uint256)")`
/// — the value that lands in `Log::topics[0]` for a non-anonymous event.
pub fn event_topic0(signature: &str) -> [u8; 32] {
    keccak256(signature.as_bytes())
}

/// ABI-encode `(address, address, uint256)` as calldata following a
/// 4-byte selector — this example's only executor call shape (buy pool,
/// sell pool, notional amount).
pub fn encode_arb_call(
    selector: [u8; 4],
    buy_pool: [u8; 20],
    sell_pool: [u8; 20],
    amount: u128,
) -> Vec<u8> {
    let mut data = Vec::with_capacity(4 + 32 * 3);
    data.extend_from_slice(&selector);
    data.extend_from_slice(&encode_address(buy_pool));
    data.extend_from_slice(&encode_address(sell_pool));
    data.extend_from_slice(&encode_u256(amount));
    data
}

/// ABI-encode a bare `getReserves()`-style zero-argument call.
pub fn encode_no_arg_call(selector: [u8; 4]) -> Vec<u8> {
    selector.to_vec()
}

fn encode_address(addr: [u8; 20]) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[12..].copy_from_slice(&addr);
    word
}

fn encode_u256(value: u128) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[16..].copy_from_slice(&value.to_be_bytes());
    word
}

/// Decode a single right-aligned `uint256` word from `data` at word index
/// `word_index` (0-based), returning it as a `u128` truncated from the
/// low 16 bytes of the 32-byte word.
///
/// **Contract:** callers must only use this where the encoded value is
/// known to fit in `u128` (true for both this example's reserves, which
/// fit in `uint112`, and its profit figures) — this is not a general
/// `uint256` decoder.
pub fn decode_u256_word(data: &[u8], word_index: usize) -> Option<u128> {
    let start = word_index * 32;
    let word = data.get(start..start + 32)?;
    let mut buf = [0u8; 16];
    buf.copy_from_slice(word.get(16..32)?);
    Some(u128::from_be_bytes(buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_selector_matches_the_well_known_get_reserves_selector() {
        // A widely known constant for any Uniswap-V2-shaped pair — a
        // useful sanity check that our keccak256 plumbing is correct.
        assert_eq!(function_selector("getReserves()"), [0x09, 0x02, 0xf1, 0xac]);
    }

    #[test]
    fn encode_arb_call_lays_out_selector_then_three_32_byte_words() {
        let data = encode_arb_call([0xAA, 0xBB, 0xCC, 0xDD], [0x11; 20], [0x22; 20], 1_000);
        assert_eq!(data.len(), 4 + 32 * 3);
        assert_eq!(&data[0..4], [0xAA, 0xBB, 0xCC, 0xDD]);
        assert_eq!(&data[4 + 12..4 + 32], [0x11; 20]);
        assert_eq!(&data[4 + 32 + 12..4 + 64], [0x22; 20]);
        assert_eq!(decode_u256_word(&data[4..], 2), Some(1_000));
    }

    #[test]
    fn decode_u256_word_round_trips_through_encode_u256() {
        let word = encode_u256(123_456_789);
        assert_eq!(decode_u256_word(&word, 0), Some(123_456_789));
    }

    #[test]
    fn decode_u256_word_returns_none_past_the_end_of_data() {
        let data = vec![0u8; 16]; // shorter than one full word
        assert_eq!(decode_u256_word(&data, 0), None);
    }
}
