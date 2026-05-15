//! Reference CPU implementation of the TS3 security level formula.
//!
//! ```text
//! level = leading_zero_bits( SHA1( pubkey_b64_ascii || decimal_counter_ascii ) )
//! ```
//!
//! Bit counting follows the order used by the TeamSpeak client and by
//! hashcat/TeamSpeakHasher: full zero bytes first, then within the first
//! non-zero byte, count trailing-zero bits (LSB to MSB).
//!
//! The GPU backend is validated against this implementation in tests.

use sha1::{Digest, Sha1};

/// Compute the level of a given SHA-1 digest.
#[inline]
pub fn level_of_hash(hash: &[u8; 20]) -> u8 {
    let mut zero_bytes = 0u8;
    while (zero_bytes as usize) < 20 && hash[zero_bytes as usize] == 0 {
        zero_bytes += 1;
    }
    if zero_bytes == 20 {
        return 160;
    }
    let first_nz = hash[zero_bytes as usize];
    let trailing = first_nz.trailing_zeros() as u8;
    zero_bytes * 8 + trailing
}

/// Compute the level of a `(pubkey_b64, counter)` pair.
pub fn compute_level(pubkey_b64: &str, counter: u64) -> u8 {
    let mut h = Sha1::new();
    h.update(pubkey_b64.as_bytes());
    h.update(counter.to_string().as_bytes());
    let digest: [u8; 20] = h.finalize().into();
    level_of_hash(&digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_hash_is_max() {
        assert_eq!(level_of_hash(&[0u8; 20]), 160);
    }

    #[test]
    fn level_0_when_first_byte_is_odd() {
        let mut h = [0u8; 20];
        h[0] = 0b0000_0001;
        assert_eq!(level_of_hash(&h), 0);
    }

    #[test]
    fn level_1_first_byte_lsb_clear() {
        let mut h = [0u8; 20];
        h[0] = 0b0000_0010;
        assert_eq!(level_of_hash(&h), 1);
    }

    #[test]
    fn level_7_high_bit_only() {
        let mut h = [0u8; 20];
        h[0] = 0b1000_0000;
        assert_eq!(level_of_hash(&h), 7);
    }

    #[test]
    fn level_8_skips_first_byte() {
        let mut h = [0u8; 20];
        h[0] = 0;
        h[1] = 0b0000_0001;
        assert_eq!(level_of_hash(&h), 8);
    }

    #[test]
    fn level_15_byte0_zero_byte1_high_bit() {
        let mut h = [0u8; 20];
        h[1] = 0b1000_0000;
        assert_eq!(level_of_hash(&h), 15);
    }

    #[test]
    fn cross_check_against_known_vector() {
        // sha1("hello42") = f40e8e0ad8e46f0a784abc837addd52a11b69655
        // First byte 0xf4 = 0b11110100, trailing_zeros = 2 → level = 2.
        assert_eq!(compute_level("hello", 42), 2);
        // sha1("hello0")  = 3a57dee5416aebc1ca12fa6206cdf090dd3ade88
        // First byte 0x3a = 0b00111010, trailing_zeros = 1 → level = 1.
        assert_eq!(compute_level("hello", 0), 1);
        // sha1("test1234") = 9bc34549d565d9505b287de0cd20ac77be1d3f2c
        // First byte 0x9b = 0b10011011, trailing_zeros = 0 → level = 0.
        assert_eq!(compute_level("test", 1234), 0);
    }
}
