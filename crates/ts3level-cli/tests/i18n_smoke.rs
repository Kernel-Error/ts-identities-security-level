//! Verify that gettext-driven UI strings appear in the user's language.

use assert_cmd::Command;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use std::path::PathBuf;
use ts3level_core::deobfuscate::obfuscate;

fn cuda_available() -> bool {
    std::path::Path::new("/dev/nvidiactl").exists()
}

/// Check whether the system has the given locale installed (e.g.
/// `fr_FR.utf8`). `setlocale(LC_ALL, "")` fails silently for missing
/// locales and gettext then falls back to source strings, masking the
/// real coverage of the translation. Tests skip on missing locales.
fn locale_installed(prefix: &str) -> bool {
    let out = std::process::Command::new("locale").arg("-a").output();
    let Ok(out) = out else { return false };
    let s = String::from_utf8_lossy(&out.stdout);
    s.lines().any(|line| line.to_lowercase().starts_with(&prefix.to_lowercase()))
}

fn synthetic_ini(counter: u64) -> (tempfile::TempDir, PathBuf) {
    let mut der = vec![0x30, 75];
    der.extend_from_slice(&[0x03, 0x02, 0x07, 0x80]);
    der.extend_from_slice(&[0x02, 0x01, 0x20]);
    der.push(0x02);
    der.push(32);
    der.extend(std::iter::repeat_n(0x77_u8, 32));
    der.push(0x02);
    der.push(32);
    der.extend(std::iter::repeat_n(0x88_u8, 32));
    der.push(0x02);
    der.push(32);
    der.extend(std::iter::repeat_n(0x99_u8, 32));
    der[1] = (der.len() - 2) as u8;
    let inner = B64.encode(&der);
    let mut padded = inner.into_bytes();
    while padded.len() < 240 {
        padded.push(0);
    }
    obfuscate(&mut padded);
    let blob_b64 = B64.encode(&padded);
    let ini = format!("[Identity]\nidentity=\"{counter}V{blob_b64}\"\n");
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("test.ini");
    std::fs::write(&p, ini).unwrap();
    (dir, p)
}

fn locale_dir() -> PathBuf {
    // Workspace target/locale, populated by `cargo xtask msgfmt`.
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .nth(2) // crates/ts3level-cli → workspace root
        .unwrap()
        .join("target")
        .join("locale")
}

#[test]
fn german_localized_strings_appear() {
    if !cuda_available() {
        eprintln!("no CUDA; skipping");
        return;
    }
    if !locale_installed("de_DE") {
        eprintln!("de_DE locale not installed; skipping");
        return;
    }
    let ld = locale_dir();
    if !ld.join("de/LC_MESSAGES/ts3level.mo").exists() {
        eprintln!("locale not compiled; run `cargo xtask msgfmt` first — skipping");
        return;
    }
    let (_dir, path) = synthetic_ini(0);
    let assert = Command::cargo_bin("ts3level")
        .unwrap()
        .env("LANG", "de_DE.UTF-8")
        .env("LANGUAGE", "de")
        .env("TS3LEVEL_LOCALEDIR", &ld)
        .args(["--target", "10"])
        .arg(&path)
        .timeout(std::time::Duration::from_secs(60))
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(
        stderr.contains("Benutze Gerät") || stderr.contains("Starte bei Level"),
        "German strings missing in stderr:\n{stderr}"
    );
}

#[test]
fn french_localized_strings_appear() {
    if !cuda_available() {
        eprintln!("no CUDA; skipping");
        return;
    }
    if !locale_installed("fr_FR") {
        eprintln!("fr_FR locale not installed; skipping");
        return;
    }
    let ld = locale_dir();
    if !ld.join("fr/LC_MESSAGES/ts3level.mo").exists() {
        eprintln!("locale not compiled; run `cargo xtask msgfmt` first — skipping");
        return;
    }
    let (_dir, path) = synthetic_ini(0);
    let assert = Command::cargo_bin("ts3level")
        .unwrap()
        .env("LANG", "fr_FR.UTF-8")
        .env("LANGUAGE", "fr")
        .env("TS3LEVEL_LOCALEDIR", &ld)
        .args(["--target", "10"])
        .arg(&path)
        .timeout(std::time::Duration::from_secs(60))
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(
        stderr.contains("Périphérique utilisé") || stderr.contains("Démarrage au niveau"),
        "French strings missing in stderr:\n{stderr}"
    );
}
