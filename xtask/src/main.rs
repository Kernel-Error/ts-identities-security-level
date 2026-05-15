//! Project automation. Run with `cargo xtask <subcommand>`.
//!
//! Subcommands:
//!   msgfmt     compile po/*.po into ./target/locale/<lang>/LC_MESSAGES/ts3level.mo
//!   pot        run xgettext over POTFILES.in and refresh po/ts3level.pot
//!   help       show usage

use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::Command;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(|s| s.as_str()) {
        Some("msgfmt") => msgfmt(),
        Some("pot") => pot(),
        Some("help") | None => {
            print_help();
            Ok(())
        }
        Some(other) => bail!("unknown subcommand: {other}"),
    }
}

fn print_help() {
    eprintln!("Usage: cargo xtask <command>");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  msgfmt     compile po/*.po → target/locale/<lang>/LC_MESSAGES/ts3level.mo");
    eprintln!("  pot        refresh po/ts3level.pot from POTFILES.in via xgettext");
    eprintln!("  help       this message");
}

fn workspace_root() -> Result<PathBuf> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let root = std::path::Path::new(manifest_dir)
        .parent()
        .context("xtask must live in a workspace")?;
    Ok(root.to_owned())
}

fn msgfmt() -> Result<()> {
    let root = workspace_root()?;
    let po_dir = root.join("po");
    let out_root = root.join("target").join("locale");
    let mut count = 0;
    for entry in std::fs::read_dir(&po_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("po") {
            continue;
        }
        let lang = path
            .file_stem()
            .and_then(|s| s.to_str())
            .context("invalid po filename")?;
        let dest_dir = out_root.join(lang).join("LC_MESSAGES");
        std::fs::create_dir_all(&dest_dir)?;
        let dest = dest_dir.join("ts3level.mo");
        let status = Command::new("msgfmt")
            .arg("-o")
            .arg(&dest)
            .arg(&path)
            .status()
            .with_context(|| format!("running msgfmt on {path:?}"))?;
        if !status.success() {
            bail!("msgfmt failed on {path:?}");
        }
        eprintln!("compiled {} → {}", path.display(), dest.display());
        count += 1;
    }
    eprintln!("done ({count} languages)");
    eprintln!();
    eprintln!("To test, run with: LANG=de_DE.UTF-8 TS3LEVEL_LOCALEDIR={}", out_root.display());
    Ok(())
}

fn pot() -> Result<()> {
    let root = workspace_root()?;
    let potfiles = root.join("po").join("POTFILES.in");
    let out = root.join("po").join("ts3level.pot");
    let status = Command::new("xgettext")
        .arg("--from-code=UTF-8")
        .arg("--language=Rust")
        .arg("--keyword=tr")
        .arg("--keyword=gettext")
        .arg("--package-name=ts3level")
        .arg("--package-version=0.1.0")
        .arg("--msgid-bugs-address=kernel-error@kernel-error.com")
        .arg("-f")
        .arg(&potfiles)
        .arg("-o")
        .arg(&out)
        .current_dir(&root)
        .status()
        .with_context(|| "running xgettext")?;
    if !status.success() {
        bail!("xgettext failed");
    }
    eprintln!("wrote {}", out.display());
    Ok(())
}
