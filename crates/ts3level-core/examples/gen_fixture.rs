//! Generate the committed test fixture in
//! `crates/ts3level-core/testdata/known_identity.ini`, plus print the
//! expected fingerprint, public key and level so the verifier test can
//! be hard-coded.
//!
//! Run with:
//!     cargo run -p ts3level-core --example gen_fixture --release
//!
//! All inputs are deterministic — the X/Y/K bytes are fixed constants
//! that are *not* a valid P-256 point. That's fine: we only ever
//! exercise the parser/obfuscation/level chain, never anything that
//! treats the bytes as on-curve. No real account's key material is
//! used or could be recovered.
//!
//! After running, commit:
//!   - crates/ts3level-core/testdata/known_identity.ini
//!   - the printed fingerprint / level values into
//!     crates/ts3level-core/tests/known_identity_fixture.rs.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use std::path::PathBuf;
use ts3level_core::deobfuscate::obfuscate;
use ts3level_core::level::compute_level;
use ts3level_core::pubkey::KeyPair;
use ts3level_core::IdentityFile;

fn integer(bytes: &[u8]) -> Vec<u8> {
    let mut start = 0;
    while start < bytes.len() - 1 && bytes[start] == 0 {
        start += 1;
    }
    let stripped = &bytes[start..];
    let mut out = Vec::with_capacity(stripped.len() + 4);
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

fn full_der(x: &[u8; 32], y: &[u8; 32], k: &[u8; 32]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&[0x03, 0x02, 0x07, 0x80]); // BIT STRING flags=1
    body.extend_from_slice(&[0x02, 0x01, 0x20]); // SHORT INTEGER keysize=32
    body.extend(integer(x));
    body.extend(integer(y));
    body.extend(integer(k));

    let mut out = Vec::with_capacity(body.len() + 4);
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

fn make_blob_b64(der: &[u8]) -> String {
    let inner = B64.encode(der);
    let mut padded = inner.into_bytes();
    while padded.len() < 240 {
        padded.push(0);
    }
    obfuscate(&mut padded);
    B64.encode(&padded)
}

fn main() {
    // Deterministic synthetic key material.
    let x: [u8; 32] = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
        0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e,
        0x1f, 0x20,
    ];
    let y: [u8; 32] = [
        0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xab, 0xac, 0xad, 0xae,
        0xaf, 0xb0, 0xb1, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xbb, 0xbc, 0xbd,
        0xbe, 0xbf,
    ];
    let k: [u8; 32] = [
        0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x5b, 0x5c, 0x5d, 0x5e,
        0x5f, 0x60, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x6b, 0x6c, 0x6d,
        0x6e, 0x6f,
    ];
    // 310 yields level 10 against the synthetic public key below — gives
    // the verifier test a meaningful leading-zero-bit count to check, not
    // just the trivial level-0 case.
    let counter: u64 = 310;
    let nickname = "Fixture-User";
    let local_id = "Fixture-Standard";

    let der = full_der(&x, &y, &k);
    let blob_b64 = make_blob_b64(&der);

    let ini = format!(
        "[Identity]\n\
         id={local_id}\n\
         identity=\"{counter}V{blob_b64}\"\n\
         nickname={nickname}\n\
         phonetic_nickname=\n",
    );

    // Round-trip what we just produced to sanity-check.
    let id = IdentityFile::parse(ini.as_bytes()).expect("parse");
    let kp = KeyPair::from_blob_b64(id.blob_b64()).expect("from_blob_b64");
    let pubkey_b64 = kp.public_key_base64();
    let fingerprint = kp.fingerprint_b64();
    let level = compute_level(&pubkey_b64, id.counter());

    let testdata = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/known_identity.ini");
    std::fs::create_dir_all(testdata.parent().unwrap()).expect("create_dir_all");
    std::fs::write(&testdata, &ini).expect("write fixture");

    println!("wrote {}", testdata.display());
    println!();
    println!("--- expected values for the verifier test ---");
    println!("Counter:         {counter}");
    println!("Nickname:        {nickname}");
    println!("Local ID:        {local_id}");
    println!("Level at counter:{level}");
    println!("Public key b64:  {pubkey_b64}");
    println!("Fingerprint b64: {fingerprint}");
}
