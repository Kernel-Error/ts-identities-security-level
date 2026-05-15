//! Minimal libtomcrypt-compatible ASN.1 DER reader/writer for the TS3
//! identity keypair, and the public-only re-emission that the security-
//! level hash is computed over.
//!
//! The keypair lives inside the deobfuscated identity blob as:
//!
//! ```text
//! SEQUENCE {
//!   BIT STRING       flags     -- 1 bit; 0 = public-only, 1 = with private k
//!   SHORT INTEGER    keysize   -- 32 for the NIST P-256 curve
//!   INTEGER          x
//!   INTEGER          y
//!   [INTEGER         k]        -- only present when flags = 1
//! }
//! ```
//!
//! The TeamSpeak server identifies the user by the SHA-1 of the **public
//! key as base64-encoded DER** — i.e. the same SEQUENCE with `flags = 0`
//! and the trailing `k` stripped.

use crate::deobfuscate::deobfuscate;
use crate::error::{Error, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

/// Decoded keypair, all big-endian-encoded big integers exactly as stored
/// in the DER (canonical form, may include a leading `0x00` sign byte).
#[derive(Debug, Clone)]
pub struct KeyPair {
    pub has_private: bool,
    pub key_size: u32,
    pub x: Vec<u8>,
    pub y: Vec<u8>,
    pub k: Option<Vec<u8>>,
}

impl KeyPair {
    /// Parse the obfuscated, base64-encoded blob from the `.ini`.
    ///
    /// Two layers of base64 + the deobfuscation pass are applied to recover
    /// the inner ASN.1 DER, which is then parsed.
    pub fn from_blob_b64(blob_b64: &str) -> Result<Self> {
        let mut outer = B64.decode(blob_b64).map_err(Error::BadBlobBase64)?;
        if outer.len() < 20 {
            return Err(Error::BlobTooShort(outer.len()));
        }
        deobfuscate(&mut outer);
        // After deobfuscation, `outer` is an ASCII base64 string of the
        // ASN.1 DER, NUL-terminated and possibly NUL-padded.
        let asciiend = outer.iter().position(|b| *b == 0).unwrap_or(outer.len());
        let inner = B64.decode(&outer[..asciiend]).map_err(|_| Error::BadInnerBase64)?;
        Self::from_der(&inner)
    }

    /// Parse the unwrapped ASN.1 DER of the keypair.
    pub fn from_der(der: &[u8]) -> Result<Self> {
        let (seq_body, rest) = read_tag(der, 0x30)?;
        if !rest.is_empty() {
            return Err(Error::Asn1("trailing bytes after SEQUENCE".into()));
        }
        let (flags_body, r) = read_tag(seq_body, 0x03)?;
        if flags_body.len() != 2 {
            return Err(Error::Asn1(format!(
                "expected 2-byte BIT STRING content, got {}",
                flags_body.len()
            )));
        }
        let unused_bits = flags_body[0];
        let flag_byte = flags_body[1];
        if unused_bits != 7 {
            return Err(Error::Asn1(format!(
                "expected 7 unused bits in flag BIT STRING, got {unused_bits}"
            )));
        }
        let has_private = match flag_byte {
            0x00 => false,
            0x80 => true,
            other => return Err(Error::Asn1(format!("unexpected flag byte {other:#x}"))),
        };

        let (keysize_body, r) = read_tag(r, 0x02)?;
        if keysize_body.is_empty() || keysize_body.len() > 4 {
            return Err(Error::Asn1("keysize INTEGER has weird length".into()));
        }
        let mut key_size: u32 = 0;
        for b in keysize_body {
            key_size = (key_size << 8) | u32::from(*b);
        }

        let (x_body, r) = read_tag(r, 0x02)?;
        let (y_body, r) = read_tag(r, 0x02)?;

        let (k, r) = if has_private {
            let (k_body, r) = read_tag(r, 0x02)?;
            (Some(k_body.to_vec()), r)
        } else {
            (None, r)
        };

        if !r.is_empty() {
            return Err(Error::Asn1("trailing bytes inside SEQUENCE".into()));
        }

        Ok(KeyPair {
            has_private,
            key_size,
            x: x_body.to_vec(),
            y: y_body.to_vec(),
            k,
        })
    }

    /// Encode the public-only DER: `flags=0`, `key_size`, `x`, `y`.
    ///
    /// libtomcrypt's `der_encode_sequence_multi` writes integers in their
    /// canonical form: a leading `0x00` sign byte iff the high bit of the
    /// first content byte is set. The parsed `x`/`y` already carry that
    /// byte (it is part of the INTEGER's content), so emission is just a
    /// matter of wrapping each field with its tag and length.
    pub fn to_public_der(&self) -> Vec<u8> {
        let mut body = Vec::with_capacity(128);

        // BIT STRING: 03 02 07 00
        body.extend_from_slice(&[0x03, 0x02, 0x07, 0x00]);

        // SHORT INTEGER (keysize). libtomcrypt emits this with a minimal
        // representation. For key_size == 32 this is `02 01 20`.
        let ks_bytes = encode_uint_minimal(self.key_size);
        body.push(0x02);
        body.extend(encode_length(ks_bytes.len()));
        body.extend(ks_bytes);

        // INTEGER x
        body.push(0x02);
        body.extend(encode_length(self.x.len()));
        body.extend_from_slice(&self.x);

        // INTEGER y
        body.push(0x02);
        body.extend(encode_length(self.y.len()));
        body.extend_from_slice(&self.y);

        // Wrap in SEQUENCE
        let mut out = Vec::with_capacity(body.len() + 4);
        out.push(0x30);
        out.extend(encode_length(body.len()));
        out.extend(body);
        out
    }

    /// Public-only DER, base64-encoded — the exact string that goes into
    /// the level hash as `pubkey_b64`.
    pub fn public_key_base64(&self) -> String {
        B64.encode(self.to_public_der())
    }

    /// The identity fingerprint TeamSpeak displays as a base64 string:
    /// base64 of SHA-1 of the public-key base64 ASCII. Reference impl is
    /// `getIDFingerprint` in `TSIdentityTool.c`.
    pub fn fingerprint_b64(&self) -> String {
        use sha1::{Digest, Sha1};
        let pk = self.public_key_base64();
        let digest: [u8; 20] = Sha1::digest(pk.as_bytes()).into();
        B64.encode(digest)
    }
}

/// Read one DER TLV with `expected_tag` from `buf` and return
/// `(content_slice, remainder)`.
fn read_tag(buf: &[u8], expected_tag: u8) -> Result<(&[u8], &[u8])> {
    if buf.is_empty() {
        return Err(Error::Asn1("unexpected end of buffer reading tag".into()));
    }
    if buf[0] != expected_tag {
        return Err(Error::Asn1(format!(
            "expected tag {:#x}, got {:#x}",
            expected_tag, buf[0]
        )));
    }
    let (len, len_size) = read_length(&buf[1..])?;
    let content_start = 1 + len_size;
    let content_end = content_start
        .checked_add(len)
        .ok_or_else(|| Error::Asn1("length overflow".into()))?;
    if content_end > buf.len() {
        return Err(Error::Asn1(format!(
            "TLV says {len} content bytes but only {} available",
            buf.len() - content_start
        )));
    }
    Ok((&buf[content_start..content_end], &buf[content_end..]))
}

/// Decode a DER length field and return `(length, length_field_size_in_bytes)`.
fn read_length(buf: &[u8]) -> Result<(usize, usize)> {
    if buf.is_empty() {
        return Err(Error::Asn1("unexpected end of buffer reading length".into()));
    }
    let first = buf[0];
    if first < 0x80 {
        return Ok((first as usize, 1));
    }
    let n = (first & 0x7F) as usize;
    if n == 0 || n > 4 {
        return Err(Error::Asn1(format!("indefinite or oversized length field ({n})")));
    }
    if buf.len() < 1 + n {
        return Err(Error::Asn1("truncated multi-byte length".into()));
    }
    let mut len: usize = 0;
    for b in &buf[1..1 + n] {
        len = (len << 8) | (*b as usize);
    }
    Ok((len, 1 + n))
}

fn encode_length(len: usize) -> Vec<u8> {
    if len < 0x80 {
        vec![len as u8]
    } else if len < 0x100 {
        vec![0x81, len as u8]
    } else if len < 0x10000 {
        vec![0x82, (len >> 8) as u8, len as u8]
    } else {
        // No legitimate TS3 keypair reaches this branch, but keep it correct.
        vec![
            0x83,
            (len >> 16) as u8,
            (len >> 8) as u8,
            len as u8,
        ]
    }
}

/// Minimal positive-integer encoding for an unsigned value.
/// 32-bit value `0x20` becomes `[0x20]`, `0x100` becomes `[0x01, 0x00]`,
/// `0x80` becomes `[0x00, 0x80]` (sign byte to keep it positive).
fn encode_uint_minimal(n: u32) -> Vec<u8> {
    if n == 0 {
        return vec![0x00];
    }
    let mut bytes = Vec::with_capacity(5);
    let mut buf = [0u8; 4];
    buf[0] = (n >> 24) as u8;
    buf[1] = (n >> 16) as u8;
    buf[2] = (n >> 8) as u8;
    buf[3] = n as u8;
    // strip leading zero bytes
    let mut i = 0;
    while i < 3 && buf[i] == 0 {
        i += 1;
    }
    if buf[i] & 0x80 != 0 {
        bytes.push(0x00);
    }
    bytes.extend_from_slice(&buf[i..]);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a synthetic libtomcrypt-style DER public-only keypair from
    /// hand-crafted X, Y values, then parse and re-emit it. Tests the
    /// reader/writer roundtrip without depending on libtomcrypt.
    fn synthetic_public_der(x: &[u8], y: &[u8]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&[0x03, 0x02, 0x07, 0x00]); // flags=0
        body.extend_from_slice(&[0x02, 0x01, 0x20]);       // keysize=32
        body.push(0x02);
        body.push(x.len() as u8);
        body.extend_from_slice(x);
        body.push(0x02);
        body.push(y.len() as u8);
        body.extend_from_slice(y);
        let mut out = vec![0x30, body.len() as u8];
        out.extend(body);
        out
    }

    #[test]
    fn parses_public_only_der() {
        let x = vec![0x00; 32];
        let y = vec![0xff; 32];
        let der = synthetic_public_der(&x, &y);
        let kp = KeyPair::from_der(&der).unwrap();
        assert!(!kp.has_private);
        assert_eq!(kp.key_size, 32);
        assert_eq!(kp.x, x);
        assert_eq!(kp.y, y);
        assert!(kp.k.is_none());
    }

    #[test]
    fn reemits_byte_identical_for_public_only_input() {
        let x = vec![0x01; 32];
        let y = vec![0x02; 32];
        let der = synthetic_public_der(&x, &y);
        let kp = KeyPair::from_der(&der).unwrap();
        assert_eq!(kp.to_public_der(), der);
    }

    #[test]
    fn drops_private_k_on_public_reemit() {
        // SEQUENCE { BIT STRING(flag=1, 0x80), key_size=32, x, y, k }
        let mut body = Vec::new();
        body.extend_from_slice(&[0x03, 0x02, 0x07, 0x80]);
        body.extend_from_slice(&[0x02, 0x01, 0x20]);
        body.extend_from_slice(&[0x02, 0x02, 0x12, 0x34]); // x = 0x1234
        body.extend_from_slice(&[0x02, 0x02, 0x56, 0x78]); // y = 0x5678
        body.extend_from_slice(&[0x02, 0x02, 0xCA, 0xFE]); // k = 0xCAFE
        let mut der = vec![0x30, body.len() as u8];
        der.extend(body);
        let kp = KeyPair::from_der(&der).unwrap();
        assert!(kp.has_private);
        assert_eq!(kp.k.as_deref(), Some(&[0xCA, 0xFE][..]));
        let public = kp.to_public_der();
        // Re-parse the re-emitted public, must lose k.
        let kp2 = KeyPair::from_der(&public).unwrap();
        assert!(!kp2.has_private);
        assert!(kp2.k.is_none());
        assert_eq!(kp2.x, [0x12, 0x34]);
        assert_eq!(kp2.y, [0x56, 0x78]);
    }

    #[test]
    fn rejects_garbage() {
        let err = KeyPair::from_der(&[0xff, 0xff]).unwrap_err();
        assert!(matches!(err, Error::Asn1(_)));
    }

    #[test]
    fn rejects_trailing_bytes() {
        let mut der = synthetic_public_der(&[0u8; 32], &[0u8; 32]);
        der.push(0xff);
        let err = KeyPair::from_der(&der).unwrap_err();
        assert!(matches!(err, Error::Asn1(_)));
    }

    #[test]
    fn encode_length_branches() {
        assert_eq!(encode_length(0), vec![0x00]);
        assert_eq!(encode_length(0x7F), vec![0x7F]);
        assert_eq!(encode_length(0x80), vec![0x81, 0x80]);
        assert_eq!(encode_length(0xFF), vec![0x81, 0xFF]);
        assert_eq!(encode_length(0x100), vec![0x82, 0x01, 0x00]);
        assert_eq!(encode_length(0x10000), vec![0x83, 0x01, 0x00, 0x00]);
    }

    #[test]
    fn fingerprint_is_sha1_of_pubkey_base64() {
        use sha1::{Digest, Sha1};
        let x = vec![0x11; 32];
        let y = vec![0x22; 32];
        let der = synthetic_public_der(&x, &y);
        let kp = KeyPair::from_der(&der).unwrap();
        let pk = kp.public_key_base64();
        let expected_digest: [u8; 20] = Sha1::digest(pk.as_bytes()).into();
        let expected_b64 = B64.encode(expected_digest);
        assert_eq!(kp.fingerprint_b64(), expected_b64);
        // Sanity: fingerprint is 28 base64 chars (20 bytes * 4 / 3 padded).
        assert_eq!(kp.fingerprint_b64().len(), 28);
    }

    #[test]
    fn preserves_libtomcrypt_sign_prefix_on_x_with_high_bit() {
        // Simulate the canonical libtomcrypt encoding where an INTEGER's
        // first content byte has its MSB set: the encoder prepends a
        // 0x00 sign byte so the value stays positive. Our reader must
        // include that prefix in `kp.x`, and our writer must emit the
        // exact same bytes back.
        let mut body = Vec::new();
        body.extend_from_slice(&[0x03, 0x02, 0x07, 0x00]); // flags=0
        body.extend_from_slice(&[0x02, 0x01, 0x20]);       // keysize=32

        // x: 33-byte INTEGER content = 0x00 + 32 bytes of 0x80.
        body.push(0x02);
        body.push(0x21);
        body.push(0x00);
        body.extend(std::iter::repeat_n(0x80u8, 32));

        // y: 32-byte INTEGER content = 32 bytes of 0x55 (MSB clear,
        // no sign prefix needed).
        body.push(0x02);
        body.push(0x20);
        body.extend(std::iter::repeat_n(0x55u8, 32));

        let mut der = vec![0x30, body.len() as u8];
        der.extend(body);

        let kp = KeyPair::from_der(&der).unwrap();
        // Sign prefix preserved verbatim.
        assert_eq!(kp.x.len(), 33);
        assert_eq!(kp.x[0], 0x00);
        assert_eq!(&kp.x[1..], &[0x80u8; 32][..]);
        assert_eq!(kp.y.len(), 32);
        assert_eq!(kp.y, &[0x55u8; 32]);

        // Re-emit byte-identical: the writer just wraps the INTEGER
        // content as-is, no recomputation of the sign prefix.
        assert_eq!(kp.to_public_der(), der);
    }

    #[test]
    fn parses_and_reemits_when_both_coords_have_sign_prefix() {
        // Stress: both X and Y have MSB-set first content byte.
        let mut body = Vec::new();
        body.extend_from_slice(&[0x03, 0x02, 0x07, 0x00]);
        body.extend_from_slice(&[0x02, 0x01, 0x20]);
        body.push(0x02);
        body.push(0x21);
        body.push(0x00);
        body.extend(std::iter::repeat_n(0xC0u8, 32));
        body.push(0x02);
        body.push(0x21);
        body.push(0x00);
        body.extend(std::iter::repeat_n(0xFFu8, 32));
        let mut der = vec![0x30, body.len() as u8];
        der.extend(body);

        let kp = KeyPair::from_der(&der).unwrap();
        assert_eq!(kp.x.len(), 33);
        assert_eq!(kp.y.len(), 33);
        assert_eq!(kp.x[0], 0x00);
        assert_eq!(kp.y[0], 0x00);
        assert_eq!(kp.to_public_der(), der);
    }

    #[test]
    fn fingerprint_differs_between_sign_prefix_variants() {
        // A DER that includes the sign prefix and one that omits it
        // (technically malformed in libtomcrypt's canon, but legal DER
        // for a value the encoder chose not to prefix) hash to different
        // fingerprints — confirming the fingerprint binds to the exact
        // public-key bytes the server sees, not to the abstract value.
        let prefixed = {
            let mut body = Vec::new();
            body.extend_from_slice(&[0x03, 0x02, 0x07, 0x00]);
            body.extend_from_slice(&[0x02, 0x01, 0x20]);
            body.push(0x02);
            body.push(0x21);
            body.push(0x00);
            body.extend(std::iter::repeat_n(0x80u8, 32));
            body.push(0x02);
            body.push(0x20);
            body.extend(std::iter::repeat_n(0x01u8, 32));
            let mut d = vec![0x30, body.len() as u8];
            d.extend(body);
            KeyPair::from_der(&d).unwrap().fingerprint_b64()
        };
        let unprefixed = {
            let mut body = Vec::new();
            body.extend_from_slice(&[0x03, 0x02, 0x07, 0x00]);
            body.extend_from_slice(&[0x02, 0x01, 0x20]);
            body.push(0x02);
            body.push(0x20);
            body.extend(std::iter::repeat_n(0x80u8, 32));
            body.push(0x02);
            body.push(0x20);
            body.extend(std::iter::repeat_n(0x01u8, 32));
            let mut d = vec![0x30, body.len() as u8];
            d.extend(body);
            KeyPair::from_der(&d).unwrap().fingerprint_b64()
        };
        assert_ne!(prefixed, unprefixed);
    }

    #[test]
    fn encode_uint_minimal_matches_libtomcrypt_short_integer() {
        assert_eq!(encode_uint_minimal(0), vec![0x00]);
        assert_eq!(encode_uint_minimal(0x20), vec![0x20]);
        assert_eq!(encode_uint_minimal(0x7F), vec![0x7F]);
        assert_eq!(encode_uint_minimal(0x80), vec![0x00, 0x80]); // sign prefix
        assert_eq!(encode_uint_minimal(0xFF), vec![0x00, 0xFF]);
        assert_eq!(encode_uint_minimal(0x100), vec![0x01, 0x00]);
    }
}
