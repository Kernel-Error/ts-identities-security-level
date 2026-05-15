//! Standalone SHA-1 single-block transform.
//!
//! The `sha1` crate hashes whole messages but does not expose the
//! intermediate `h[0..5]` state between blocks. The CUDA backend needs
//! that state so the host can precompute the hash of the constant
//! pubkey prefix once per launch and let the device kernel start from
//! there instead of re-hashing the same bytes inside every counter
//! iteration. This module is a small, dependency-free implementation of
//! the SHA-1 message-block transform plus the standard IV.
//!
//! Bit-for-bit identical output to the kernel's `sha1_block` device
//! function and to the `sha1` crate; verified by the unit tests below
//! (cross-check against `sha1` for multi-block messages).

/// Standard SHA-1 initial state (FIPS 180-4, §5.3.1).
pub const SHA1_INIT: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];

/// Apply one SHA-1 compression block to `state`. `block` is the next
/// 64 bytes of message in stream order. Does not perform any padding.
pub fn sha1_block(state: &mut [u32; 5], block: &[u8; 64]) {
    let mut w = [0u32; 80];
    for i in 0..16 {
        w[i] = u32::from_be_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }
    for i in 16..80 {
        w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
    }

    let mut a = state[0];
    let mut b = state[1];
    let mut c = state[2];
    let mut d = state[3];
    let mut e = state[4];

    for &w_t in &w[0..20] {
        let f = (b & c) | (!b & d);
        let t = a
            .rotate_left(5)
            .wrapping_add(f)
            .wrapping_add(e)
            .wrapping_add(0x5A82_7999)
            .wrapping_add(w_t);
        e = d;
        d = c;
        c = b.rotate_left(30);
        b = a;
        a = t;
    }
    for &w_t in &w[20..40] {
        let f = b ^ c ^ d;
        let t = a
            .rotate_left(5)
            .wrapping_add(f)
            .wrapping_add(e)
            .wrapping_add(0x6ED9_EBA1)
            .wrapping_add(w_t);
        e = d;
        d = c;
        c = b.rotate_left(30);
        b = a;
        a = t;
    }
    for &w_t in &w[40..60] {
        let f = (b & c) | (b & d) | (c & d);
        let t = a
            .rotate_left(5)
            .wrapping_add(f)
            .wrapping_add(e)
            .wrapping_add(0x8F1B_BCDC)
            .wrapping_add(w_t);
        e = d;
        d = c;
        c = b.rotate_left(30);
        b = a;
        a = t;
    }
    for &w_t in &w[60..80] {
        let f = b ^ c ^ d;
        let t = a
            .rotate_left(5)
            .wrapping_add(f)
            .wrapping_add(e)
            .wrapping_add(0xCA62_C1D6)
            .wrapping_add(w_t);
        e = d;
        d = c;
        c = b.rotate_left(30);
        b = a;
        a = t;
    }

    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha1::{Digest, Sha1};

    /// Hash `msg` by running our `sha1_block` on every complete 64-byte
    /// block, then computing the final tail (including padding) via
    /// the `sha1` crate seeded with our intermediate state. We can't
    /// seed `sha1` directly, so we hash via two routes and compare the
    /// final digest:
    ///
    ///  - reference: `Sha1::digest(msg)`
    ///  - via blocks: feed our state through `sha1_block` for every
    ///    complete block, then assert the state we get equals what
    ///    `Sha1` computes after feeding the same prefix.
    ///
    /// We can extract `Sha1`'s post-prefix state indirectly: hash the
    /// prefix-bytes, then re-create the same state by hashing the same
    /// bytes via our block fn — and the two SHA-1 implementations are
    /// equivalent iff the final digests agree for an arbitrary suffix.
    fn assert_block_matches_sha1_crate(prefix: &[u8], suffix: &[u8]) {
        let total: Vec<u8> = prefix.iter().chain(suffix.iter()).copied().collect();
        let reference = Sha1::digest(&total);

        // Path B: feed prefix block-by-block via our fn, then feed
        // suffix into a fresh sha1 hasher *initialised from our state*.
        // But sha1 doesn't let us seed state, so instead we feed the
        // prefix's full content into sha1 too and verify the final
        // digest matches. This proves end-to-end equivalence, which
        // implies our block fn matches as long as prefix is an integer
        // number of 64-byte blocks.
        assert_eq!(
            prefix.len() % 64,
            0,
            "prefix must be a whole number of blocks"
        );
        let mut state = SHA1_INIT;
        for chunk in prefix.chunks_exact(64) {
            let block: &[u8; 64] = chunk.try_into().unwrap();
            sha1_block(&mut state, block);
        }
        // Now finalise: emulate sha1 by padding+length-suffix on the
        // suffix (this also exercises the block fn through the tail).
        let total_bit_len = (total.len() as u64) * 8;
        let mut tail: Vec<u8> = suffix.to_vec();
        tail.push(0x80);
        while (tail.len() + prefix.len()) % 64 != 56 {
            tail.push(0);
        }
        tail.extend_from_slice(&total_bit_len.to_be_bytes());
        assert_eq!((tail.len() + prefix.len()) % 64, 0);
        for chunk in tail.chunks_exact(64) {
            let block: &[u8; 64] = chunk.try_into().unwrap();
            sha1_block(&mut state, block);
        }
        let mut digest = [0u8; 20];
        for (i, w) in state.iter().enumerate() {
            digest[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
        }
        assert_eq!(
            digest.as_slice(),
            reference.as_slice(),
            "block fn diverged from sha1 crate"
        );
    }

    #[test]
    fn matches_sha1_for_one_block_prefix() {
        // 64 'A' bytes as prefix, then a 7-byte suffix.
        let prefix = [b'A'; 64];
        let suffix = b"abcdefg";
        assert_block_matches_sha1_crate(&prefix, suffix);
    }

    #[test]
    fn matches_sha1_for_two_block_prefix_with_long_suffix() {
        let prefix: Vec<u8> = (0u8..128).collect();
        let suffix: Vec<u8> = (0u8..50).collect();
        assert_block_matches_sha1_crate(&prefix, &suffix);
    }

    #[test]
    fn matches_sha1_for_realistic_ts3_input() {
        // First 64 bytes of the test pubkey, then a counter ASCII tail.
        let pubkey_b64 = b"MEwDAgcAAgEgAiEA5jUbcc+RXAJzVKLpyEnoq/Otht1JBeCdRgRJYYBuOmoCIQDwBoRP+rkICZHbAGD9XYpV9bm08yPYGT4LehKXmlYZJg==";
        let suffix = b"310"; // matches the committed fixture's counter
        assert_block_matches_sha1_crate(&pubkey_b64[..64], &[&pubkey_b64[64..], suffix].concat());
    }

    #[test]
    fn empty_input_matches_sha1() {
        // sha1("") = da39a3ee5e6b4b0d3255bfef95601890afd80709
        let reference = Sha1::digest([]);
        // We need to compute via our block fn with full padding.
        let mut state = SHA1_INIT;
        let mut block = [0u8; 64];
        block[0] = 0x80; // padding marker for an empty message
                         // length suffix already zero (0 bits)
        sha1_block(&mut state, &block);
        let mut digest = [0u8; 20];
        for (i, w) in state.iter().enumerate() {
            digest[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
        }
        assert_eq!(digest.as_slice(), reference.as_slice());
    }
}
