//! TS3 identity blob obfuscation/deobfuscation.
//!
//! Layout reverse-engineered by `landave` in `TSIdentityTool.c`. The blob
//! held in the `.ini` is `base64(obfuscated_bytes)`, where the obfuscation
//! is symmetric: applying the same routine twice yields the original.
//!
//! Steps to deobfuscate:
//!   1. SHA-1 the bytes from offset 20 up to (but not including) the
//!      first NUL byte; XOR the result into bytes 0..20.
//!   2. XOR the first `min(100, len)` bytes with the static key `TSKEY`
//!      (which is the **ASCII representation of a 128-character hex
//!      string**, not the parsed 64-byte binary value).
//!
//! Obfuscation is the reverse — TSKEY first, then SHA-1 mask.

use sha1::{Digest, Sha1};

/// Static obfuscation key. Note: TSIdentityTool defines this as a
/// `const char *` to a hex *string* and XORs against the ASCII characters
/// themselves — so the effective key is **128 bytes** long, not 64.
pub const TSKEY: &[u8; 128] = b"\
b9dfaa7bee6ac57ac7b65f1094a1c155\
e747327bc2fe5d51c512023fe54a2802\
01004e90ad1daaae1075d53b7d571c30\
e063b5a62a4a017bb394833aa0983e6e";

/// In-place deobfuscate (reverses [`obfuscate`]).
pub fn deobfuscate(data: &mut [u8]) {
    sha1_mask(data);
    static_xor(data);
}

/// In-place obfuscate.
pub fn obfuscate(data: &mut [u8]) {
    static_xor(data);
    sha1_mask(data);
}

fn static_xor(data: &mut [u8]) {
    let n = data.len().min(100);
    for (b, k) in data[..n].iter_mut().zip(TSKEY.iter()) {
        *b ^= *k;
    }
}

fn sha1_mask(data: &mut [u8]) {
    if data.len() <= 20 {
        return;
    }
    let len = c_strlen(&data[20..]);
    let mut hasher = Sha1::new();
    hasher.update(&data[20..20 + len]);
    let h: [u8; 20] = hasher.finalize().into();
    for (b, m) in data[..20].iter_mut().zip(h.iter()) {
        *b ^= *m;
    }
}

fn c_strlen(buf: &[u8]) -> usize {
    buf.iter().position(|b| *b == 0).unwrap_or(buf.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tskey_is_128_ascii_hex() {
        assert_eq!(TSKEY.len(), 128);
        for b in TSKEY.iter() {
            assert!(matches!(*b, b'0'..=b'9' | b'a'..=b'f'), "byte {b:#x} not hex");
        }
    }

    #[test]
    fn roundtrip_below_100_bytes() {
        let original: Vec<u8> = (0..80).collect();
        let mut buf = original.clone();
        obfuscate(&mut buf);
        deobfuscate(&mut buf);
        assert_eq!(buf, original);
    }

    #[test]
    fn roundtrip_with_nul_in_tail() {
        let mut original = vec![0u8; 250];
        for (i, b) in original.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        // ensure there is at least one NUL after offset 20
        original[180] = 0;
        let mut buf = original.clone();
        obfuscate(&mut buf);
        deobfuscate(&mut buf);
        assert_eq!(buf, original);
    }

    #[test]
    fn roundtrip_no_nul_tail() {
        // No NUL after offset 20: c_strlen returns the full remainder.
        let mut original = vec![1u8; 250];
        let mut buf = original.clone();
        obfuscate(&mut buf);
        deobfuscate(&mut buf);
        assert_eq!(buf, original);
        // sanity: data was actually mutated by obfuscate
        original[0] ^= TSKEY[0];
        assert_ne!(buf, original);
    }

    #[test]
    fn short_blob_below_20_bytes_is_only_static_xored() {
        let mut buf = [1u8; 10];
        obfuscate(&mut buf);
        for (i, b) in buf.iter().enumerate() {
            assert_eq!(*b, 1 ^ TSKEY[i]);
        }
    }
}
