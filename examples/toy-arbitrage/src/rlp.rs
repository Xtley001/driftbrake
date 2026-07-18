//! A minimal RLP encoder — just enough to encode a legacy (type-0)
//! Ethereum transaction for signing and broadcast. Not a general-purpose
//! RLP library (see `abi.rs`'s module docs for the same "minimal, not
//! general-purpose" stance applied to ABI encoding).

#[derive(Debug, Clone)]
pub enum Item {
    Bytes(Vec<u8>),
    List(Vec<Item>),
}

impl Item {
    pub fn uint(value: u128) -> Self {
        Self::big_uint_bytes(&value.to_be_bytes())
    }

    /// A big-endian integer wider than `u128` — used for a signature's
    /// `r`/`s` (256-bit values). RLP requires integer fields to be
    /// encoded with no leading zero bytes (the canonical/minimal
    /// encoding); a fixed-width 32-byte `r`/`s` will have a leading zero
    /// byte about 1/256 of the time (compounding to roughly 1/128 across
    /// both `r` and `s`), and encoding it un-trimmed produces malformed
    /// RLP that strict decoders reject — this is not a corner case that
    /// only matters for unusually small values.
    pub fn big_uint_bytes(bytes: &[u8]) -> Self {
        Item::Bytes(leading_zero_trim(bytes).to_vec())
    }

    pub fn address(addr: [u8; 20]) -> Self {
        Item::Bytes(addr.to_vec())
    }
}

fn leading_zero_trim(bytes: &[u8]) -> &[u8] {
    let first_nonzero = bytes.iter().position(|&b| b != 0);
    match first_nonzero {
        Some(i) => &bytes[i..],
        None => &[],
    }
}

/// RLP-encode a single item.
pub fn encode(item: &Item) -> Vec<u8> {
    match item {
        Item::Bytes(bytes) => encode_bytes(bytes),
        Item::List(items) => {
            let mut payload = Vec::new();
            for item in items {
                payload.extend(encode(item));
            }
            encode_list_header(payload.len())
                .into_iter()
                .chain(payload)
                .collect()
        }
    }
}

fn encode_bytes(bytes: &[u8]) -> Vec<u8> {
    if bytes.len() == 1 && bytes[0] < 0x80 {
        return bytes.to_vec();
    }
    let mut out = encode_length_header(bytes.len(), 0x80);
    out.extend_from_slice(bytes);
    out
}

fn encode_list_header(len: usize) -> Vec<u8> {
    encode_length_header(len, 0xc0)
}

fn encode_length_header(len: usize, offset: u8) -> Vec<u8> {
    if len < 56 {
        vec![offset + len as u8]
    } else {
        let len_bytes_owned = len.to_be_bytes();
        let len_bytes = leading_zero_trim(&len_bytes_owned);
        let mut out = vec![offset + 55 + len_bytes.len() as u8];
        out.extend_from_slice(len_bytes);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_a_single_small_byte_as_itself() {
        assert_eq!(encode(&Item::Bytes(vec![0x01])), vec![0x01]);
    }

    #[test]
    fn encodes_the_empty_string_as_0x80() {
        assert_eq!(encode(&Item::Bytes(vec![])), vec![0x80]);
    }

    #[test]
    fn encodes_a_short_string_with_a_length_prefix() {
        // RLP of the classic "dog" test vector: 0x83 'd' 'o' 'g'
        assert_eq!(
            encode(&Item::Bytes(b"dog".to_vec())),
            vec![0x83, b'd', b'o', b'g']
        );
    }

    #[test]
    fn encodes_an_empty_list_as_0xc0() {
        assert_eq!(encode(&Item::List(vec![])), vec![0xc0]);
    }

    #[test]
    fn encodes_a_uint_trimming_leading_zero_bytes() {
        assert_eq!(encode(&Item::uint(0)), vec![0x80]); // zero encodes as empty string
        assert_eq!(encode(&Item::uint(1)), vec![0x01]);
        assert_eq!(encode(&Item::uint(1024)), vec![0x82, 0x04, 0x00]);
    }

    #[test]
    fn big_uint_bytes_trims_leading_zeros_from_a_wide_value_like_a_signature_component() {
        // Regression test: a signature's r/s is a fixed 32-byte array
        // that has a leading zero byte about 1/256 of the time. Encoding
        // it as a raw, untrimmed 32-byte string is non-canonical RLP —
        // this must trim exactly like Item::uint does, just for
        // arbitrary-width input.
        let mut r = [0xAAu8; 32];
        r[0] = 0x00; // simulate the "leading zero byte" case
        let encoded = encode(&Item::big_uint_bytes(&r));
        // 31 remaining bytes >= 56 in length? No: 31 < 56, so a short-string
        // header (0x80 + len) followed by the 31 trimmed bytes.
        assert_eq!(encoded[0], 0x80 + 31);
        assert_eq!(&encoded[1..], &r[1..]);

        // No leading zero: encodes as the full 32 bytes.
        let s = [0xBBu8; 32];
        let encoded = encode(&Item::big_uint_bytes(&s));
        assert_eq!(encoded[0], 0x80 + 32);
        assert_eq!(&encoded[1..], &s[..]);
    }

    #[test]
    fn encodes_a_list_of_strings_matching_the_classic_test_vector() {
        // ["cat", "dog"] => 0xc8 0x83 'c' 'a' 't' 0x83 'd' 'o' 'g'
        let list = Item::List(vec![
            Item::Bytes(b"cat".to_vec()),
            Item::Bytes(b"dog".to_vec()),
        ]);
        assert_eq!(
            encode(&list),
            vec![0xc8, 0x83, b'c', b'a', b't', 0x83, b'd', b'o', b'g']
        );
    }
}
