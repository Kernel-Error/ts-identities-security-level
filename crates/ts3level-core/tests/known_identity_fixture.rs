//! End-to-end algorithm test against a committed fixture
//! (`testdata/known_identity.ini`). Any future change that perturbs the
//! parser, the de-obfuscation chain, the public-key DER round-trip, the
//! SHA-1 fingerprint, or the level computation makes this test fire.
//!
//! The fixture was generated deterministically by `examples/gen_fixture.rs`
//! and is committed into the repository. The expected values below were
//! produced by that same generator and hand-copied here — re-running the
//! generator must produce byte-identical output.
//!
//! Note: the X/Y/K bytes in the fixture are arbitrary and not on the
//! P-256 curve. We never treat them as a usable signing key — only as
//! deterministic test material for the parser/obfuscation/level chain.

use std::path::PathBuf;
use ts3level_core::level::compute_level;
use ts3level_core::pubkey::KeyPair;
use ts3level_core::IdentityFile;

const EXPECTED_COUNTER: u64 = 310;
const EXPECTED_NICKNAME: &str = "Fixture-User";
const EXPECTED_LOCAL_ID: &str = "Fixture-Standard";
const EXPECTED_LEVEL_AT_COUNTER: u8 = 10;

const EXPECTED_PUBKEY_B64: &str =
    "MEwDAgcAAgEgAiABAgMEBQYHCAkKCwwNDg8QERITFBUWFxgZGhscHR4fIAIhAKChoqOkpaanqKmqq6ytrq+wsbKztLW2t7i5uru8vb6/";
const EXPECTED_FINGERPRINT_B64: &str = "fN+F+TGqCOsZjJzd7rBZv/q+fKs=";

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/known_identity.ini")
}

#[test]
fn parses_committed_fixture_with_expected_metadata() {
    let bytes = std::fs::read(fixture_path()).expect("read fixture");
    let file = IdentityFile::parse(&bytes).expect("parse fixture");

    assert_eq!(file.counter(), EXPECTED_COUNTER);
    assert_eq!(file.nickname(), Some(EXPECTED_NICKNAME));
    assert_eq!(file.local_id(), Some(EXPECTED_LOCAL_ID));
}

#[test]
fn fixture_yields_expected_pubkey_and_fingerprint() {
    let bytes = std::fs::read(fixture_path()).unwrap();
    let file = IdentityFile::parse(&bytes).unwrap();

    let kp = KeyPair::from_blob_b64(file.blob_b64()).expect("from_blob_b64");
    assert!(
        kp.has_private,
        "private flag should be set in the source DER"
    );

    let pubkey_b64 = kp.public_key_base64();
    assert_eq!(pubkey_b64, EXPECTED_PUBKEY_B64);

    let fingerprint = kp.fingerprint_b64();
    assert_eq!(fingerprint, EXPECTED_FINGERPRINT_B64);
}

#[test]
fn fixture_yields_expected_level_at_committed_counter() {
    let bytes = std::fs::read(fixture_path()).unwrap();
    let file = IdentityFile::parse(&bytes).unwrap();
    let kp = KeyPair::from_blob_b64(file.blob_b64()).unwrap();

    let level = compute_level(&kp.public_key_base64(), file.counter());
    assert_eq!(level, EXPECTED_LEVEL_AT_COUNTER);
}

#[test]
fn fixture_yields_expected_levels_at_secondary_counters() {
    // Sanity-check the SHA-1 chain on a few additional counters. These
    // values were measured by `examples/gen_fixture.rs`-adjacent code and
    // hand-copied; any drift in deobfuscation, public-key DER emission,
    // or `compute_level` reshapes them.
    let bytes = std::fs::read(fixture_path()).unwrap();
    let file = IdentityFile::parse(&bytes).unwrap();
    let pubkey_b64 = KeyPair::from_blob_b64(file.blob_b64())
        .unwrap()
        .public_key_base64();

    assert_eq!(compute_level(&pubkey_b64, 0), 2);
    assert_eq!(compute_level(&pubkey_b64, 7), 5);
    assert_eq!(compute_level(&pubkey_b64, 273), 7);
    assert_eq!(compute_level(&pubkey_b64, 310), 10);
}
