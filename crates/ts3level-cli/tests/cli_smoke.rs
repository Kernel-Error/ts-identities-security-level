//! End-to-end CLI tests. They depend on a real CUDA device — when none
//! is available the affected tests print a note and exit.

use assert_cmd::Command;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use predicates::prelude::*;
use predicates::str::contains;
use std::path::PathBuf;
use ts3level_core::deobfuscate::obfuscate;

fn cuda_available() -> bool {
    std::path::Path::new("/dev/nvidiactl").exists()
}

fn synthetic_ini(counter: u64) -> (tempfile::TempDir, PathBuf) {
    let mut der = vec![0x30, 75];
    der.extend_from_slice(&[0x03, 0x02, 0x07, 0x80]);
    der.extend_from_slice(&[0x02, 0x01, 0x20]);
    der.push(0x02);
    der.push(32);
    der.extend(std::iter::repeat_n(0xaa_u8, 32));
    der.push(0x02);
    der.push(32);
    der.extend(std::iter::repeat_n(0xbb_u8, 32));
    der.push(0x02);
    der.push(32);
    der.extend(std::iter::repeat_n(0xcc_u8, 32));
    der[1] = (der.len() - 2) as u8;

    let inner = B64.encode(&der);
    let mut padded = inner.into_bytes();
    while padded.len() < 240 {
        padded.push(0);
    }
    obfuscate(&mut padded);
    let blob_b64 = B64.encode(&padded);

    let ini = format!("[Identity]\nid=test\nidentity=\"{counter}V{blob_b64}\"\nnickname=Test\nphonetic_nickname=\n");
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("test.ini");
    std::fs::write(&p, ini).unwrap();
    (dir, p)
}

#[test]
fn help_works_and_lists_main_flags() {
    let assert = Command::cargo_bin("ts3level")
        .unwrap()
        .arg("--help")
        .assert()
        .success();
    let out = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    for needle in ["--target", "--device", "--list-devices", "--batch-size"] {
        assert!(out.contains(needle), "help missing flag {needle}\n{out}");
    }
}

#[test]
fn missing_file_returns_nonzero() {
    Command::cargo_bin("ts3level")
        .unwrap()
        .arg("/tmp/this-file-should-not-exist-ts3level.ini")
        .assert()
        .failure()
        .stderr(contains("error").or(contains("not found")));
}

#[test]
fn auto_bumps_target_when_at_or_below_current_level() {
    if !cuda_available() {
        eprintln!("no CUDA device; skipping");
        return;
    }
    let (_dir, path) = synthetic_ini(0);

    // Compute the level the file starts at — that becomes our --target
    // value so the auto-bump triggers (target ≤ current_level).
    let id = ts3level_core::IdentityFile::parse(&std::fs::read(&path).unwrap()).unwrap();
    let kp = ts3level_core::pubkey::KeyPair::from_blob_b64(id.blob_b64()).unwrap();
    let starting = ts3level_core::level::compute_level(&kp.public_key_base64(), id.counter());

    let assert = Command::cargo_bin("ts3level")
        .unwrap()
        .args(["--target", &starting.to_string(), "--no-gpu-stats"])
        .arg(&path)
        .timeout(std::time::Duration::from_secs(60))
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();

    assert!(
        stderr.contains("raising to"),
        "expected auto-bump message at boundary level {starting}, got:\n{stderr}"
    );
    let bumped = starting.saturating_add(1);
    assert!(
        stderr.contains(&bumped.to_string()),
        "expected bumped value {bumped}, got:\n{stderr}"
    );
}

#[test]
fn does_not_bump_target_when_above_current_level() {
    if !cuda_available() {
        eprintln!("no CUDA device; skipping");
        return;
    }
    let (_dir, path) = synthetic_ini(0);
    let assert = Command::cargo_bin("ts3level")
        .unwrap()
        .args(["--target", "15", "--no-gpu-stats"])
        .arg(&path)
        .timeout(std::time::Duration::from_secs(60))
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(
        !stderr.contains("raising to"),
        "no auto-bump expected for target above current level, got:\n{stderr}"
    );
    assert!(
        stderr.contains("Target: 15"),
        "expected raw target line in output, got:\n{stderr}"
    );
}

#[test]
fn raises_level_to_target_and_writes_bak() {
    if !cuda_available() {
        eprintln!("no CUDA device; skipping E2E test");
        return;
    }
    let (_dir, path) = synthetic_ini(0);
    let bak = path.with_extension("ini.bak");
    assert!(!bak.exists());

    Command::cargo_bin("ts3level")
        .unwrap()
        .args(["--target", "20"])
        .arg(&path)
        .timeout(std::time::Duration::from_secs(120))
        .assert()
        .success();

    // .bak must exist and contain the original counter=0.
    assert!(bak.exists(), ".bak file not created");
    let bak_content = std::fs::read_to_string(&bak).unwrap();
    assert!(
        bak_content.contains("identity=\"0V"),
        "bak not pristine: {bak_content}"
    );

    // The actual file must now have a counter > 0 and a level >= 20 by
    // CPU verification.
    let after = std::fs::read_to_string(&path).unwrap();
    let id = ts3level_core::IdentityFile::parse(after.as_bytes()).unwrap();
    let kp = ts3level_core::pubkey::KeyPair::from_blob_b64(id.blob_b64()).unwrap();
    let lvl = ts3level_core::level::compute_level(&kp.public_key_base64(), id.counter());
    assert!(lvl >= 20, "level reached {lvl}, expected ≥ 20");
}
