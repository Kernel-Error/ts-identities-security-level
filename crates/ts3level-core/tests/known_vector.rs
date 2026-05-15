//! End-to-end fixture: build a synthetic TS3 identity from known plaintext
//! material, run it through every stage (obfuscate → base64 → embed in ini
//! → parse → deobfuscate → extract public key → compute level), and assert
//! that each stage matches a reference computed independently.
//!
//! We do NOT commit a real `.ini`. The synthetic key material is enough to
//! validate the algorithm — and avoids embedding anyone's private key in
//! the repository.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use sha1::{Digest, Sha1};
use ts3level_core::deobfuscate::obfuscate;
use ts3level_core::level::{compute_level, level_of_hash};
use ts3level_core::pubkey::KeyPair;
use ts3level_core::IdentityFile;

fn strip_sign(bytes: &[u8]) -> &[u8] {
    if bytes.len() > 1 && bytes[0] == 0x00 {
        &bytes[1..]
    } else {
        bytes
    }
}

/// Build a fake libtomcrypt-style DER for a P-256 keypair with deterministic
/// X, Y, K bytes. Note: the values are not on the curve — that does not
/// matter for this test, only the byte-level encoding does.
fn synthetic_full_der(x: &[u8; 32], y: &[u8; 32], k: &[u8; 32]) -> Vec<u8> {
    fn integer(bytes: &[u8]) -> Vec<u8> {
        // libtomcrypt's INTEGER includes a leading 0x00 iff the high bit of
        // the first content byte is set, to keep it positive. Otherwise it
        // strips leading zeros (canonical DER).
        let mut start = 0;
        while start < bytes.len() - 1 && bytes[start] == 0 {
            start += 1;
        }
        let stripped = &bytes[start..];
        let mut out = Vec::new();
        out.push(0x02);
        if stripped[0] & 0x80 != 0 {
            out.push((stripped.len() + 1) as u8);
            out.push(0x00);
        } else {
            out.push(stripped.len() as u8);
        }
        out.extend_from_slice(stripped);
        out
    }

    let mut body = Vec::new();
    body.extend_from_slice(&[0x03, 0x02, 0x07, 0x80]); // BIT STRING flags=1 (private)
    body.extend_from_slice(&[0x02, 0x01, 0x20]);       // SHORT INTEGER keysize=32
    body.extend(integer(x));
    body.extend(integer(y));
    body.extend(integer(k));

    let mut out = Vec::new();
    out.push(0x30);
    if body.len() < 0x80 {
        out.push(body.len() as u8);
    } else {
        out.push(0x81);
        out.push(body.len() as u8);
    }
    out.extend(body);
    out
}

/// Build the on-disk obfuscated blob (outer-base64 string) from a clear DER.
fn make_blob_b64(der: &[u8]) -> String {
    // Inner: base64 of the DER → ASCII, ~104 chars for ~76 bytes.
    let inner = B64.encode(der);
    let mut padded = inner.into_bytes();
    // Reserve ~256 bytes total to match libtomcrypt buffer behavior. The
    // exact size does not matter for correctness, only for the SHA-1 mask
    // computation, which walks data[20..] until the first NUL byte.
    while padded.len() < 240 {
        padded.push(0);
    }
    obfuscate(&mut padded);
    B64.encode(&padded)
}

#[test]
fn roundtrip_synthetic_identity_through_full_pipeline() {
    // 1. Fixed input material.
    let x: [u8; 32] = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10,
        0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20,
    ];
    let y: [u8; 32] = [0xaa; 32];
    let k: [u8; 32] = [0x55; 32];
    let counter: u64 = 1234567;

    let der = synthetic_full_der(&x, &y, &k);
    let blob_b64 = make_blob_b64(&der);

    // 2. Assemble a fake .ini.
    let ini = format!(
        "[Identity]\n\
         id=test\n\
         identity=\"{counter}V{blob_b64}\"\n\
         nickname=Test\n\
         phonetic_nickname=\n",
    );

    // 3. Parse.
    let file = IdentityFile::parse(ini.as_bytes()).unwrap();
    assert_eq!(file.counter(), counter);
    assert_eq!(file.blob_b64(), blob_b64);

    // 4. Deobfuscate + extract public key. INTEGERs may carry a
    //    libtomcrypt-style leading 0x00 sign prefix iff the high bit of
    //    the first content byte is set — strip it for the comparison.
    let kp = KeyPair::from_blob_b64(file.blob_b64()).unwrap();
    assert!(kp.has_private);
    assert_eq!(kp.key_size, 32);
    assert_eq!(strip_sign(&kp.x), &x[..]);
    assert_eq!(strip_sign(&kp.y), &y[..]);
    assert_eq!(strip_sign(kp.k.as_deref().unwrap()), &k[..]);

    // 5. Public-only re-emission strips k and resets the flag bit.
    let pubkey_b64 = kp.public_key_base64();
    let pub_der = B64.decode(&pubkey_b64).unwrap();
    let kp_pub = KeyPair::from_der(&pub_der).unwrap();
    assert!(!kp_pub.has_private);
    assert!(kp_pub.k.is_none());
    assert_eq!(strip_sign(&kp_pub.x), &x[..]);
    assert_eq!(strip_sign(&kp_pub.y), &y[..]);

    // 6. Level computed by our reference impl must match an independent
    //    SHA-1 of `pubkey_b64 || counter.to_string()`.
    let mut h = Sha1::new();
    h.update(pubkey_b64.as_bytes());
    h.update(counter.to_string().as_bytes());
    let digest: [u8; 20] = h.finalize().into();
    let reference = level_of_hash(&digest);
    let computed = compute_level(&pubkey_b64, counter);
    assert_eq!(computed, reference);
}

#[test]
fn ini_modify_counter_preserves_recoverable_public_key() {
    let der = synthetic_full_der(&[0x11; 32], &[0x22; 32], &[0x33; 32]);
    let blob_b64 = make_blob_b64(&der);
    let ini = format!("[Identity]\nidentity=\"1V{blob_b64}\"\n");
    let mut file = IdentityFile::parse(ini.as_bytes()).unwrap();

    let before = KeyPair::from_blob_b64(file.blob_b64()).unwrap().public_key_base64();
    file.set_counter(99999);
    let after_bytes = file.to_bytes();
    let after = IdentityFile::parse(&after_bytes).unwrap();
    let after_pub = KeyPair::from_blob_b64(after.blob_b64()).unwrap().public_key_base64();

    assert_eq!(before, after_pub);
}
