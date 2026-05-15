# Changelog

All notable changes to this project are documented here. The format is
loosely based on [Keep a Changelog](https://keepachangelog.com), the
project follows [Semantic Versioning](https://semver.org).

## [Unreleased]

### Added

- **GitHub Actions CI** (`.github/workflows/ci.yml`) — three jobs run on
  every push to `main` and every pull request: `cargo fmt --check`,
  `cargo clippy --workspace --all-targets --release` with `-D warnings`,
  and the full test suite. CUDA Toolkit is installed via
  `Jimver/cuda-toolkit` so the kernel build succeeds on the runner.
  Cargo cache shared across runs via `Swatinem/rust-cache`.
- **Pull-request template** (`.github/PULL_REQUEST_TEMPLATE.md`) with a
  test-plan checklist and a prominent warning against attaching real
  `.ini` identity files.
- **GitHub issue templates** (`.github/ISSUE_TEMPLATE/`) for bug reports
  and feature requests, each warning against pasting identity material.
  Blank issues disabled, `config.yml` points at docs and the roadmap.
- **`CONTRIBUTING.md`** with the minimal contributor guide: tool
  versions, the commands CI runs, where the docs live, and an issue-label
  table that pairs the nine in-use labels with their meaning and how
  they combine.
- **README "Safety and scope" block** above the disclaimer — explicit
  private-key warning, reiteration of the offline / no-bypass nature.
  Closes #24.
- **Pascal (`sm_61`) and Maxwell (`sm_52`)** GPUs in the prebuilt
  fatbin. Older cards now run out of the box; the rebuild-from-source
  step in docs/compatibility.md is no longer required for them.
  Closes #4.
- **Dutch (`nl`) translation** (`po/nl.po`) alongside en/de/es/fr.
  Translator credit via the `translator-credits` msgid. CI's
  `locale-gen` now also produces `nl_NL.UTF-8`. The new
  `dutch_localized_strings_appear` test exercises the message catalog
  end-to-end. Closes #11.

### Changed

- **Minimum Supported Rust Version bumped from 1.75 to 1.82** — the
  test fixtures already use `std::iter::repeat_n`, which is stable since
  1.82. Released stable Rust is now 1.86 / 1.87, so the new floor is
  well below current.
- **Defensive wording in `docs/legal.md`** — leading "not legal advice"
  caveat hoisted to its own sentence; "risk is low" reframed as "risk
  appears low based on the current technical design and public
  documentation". Closes #26 in part.
- **GUI About dialog disclaimer** — adds the word "trademark" explicitly
  ("'TeamSpeak' is a trademark of TeamSpeak Systems GmbH and is used
  here only descriptively"). New string propagated to all five locales.
  Closes #26.
- **`PreflightError::NotReadable` split into two variants**:
  `NotReadable` retains the real metadata (uid, gid, mode), and a new
  `NotReadableNoMetadata` variant covers the case where the initial
  `stat` itself was denied. Closes #20.

### Fixed

- **Atomic `write_back` now `fsync`s the parent directory** after the
  rename and after the one-shot `.bak` creation. POSIX rename is atomic
  for visibility but not for directory-entry durability across a crash.
  Closes #14.
- **`flock` errors no longer collapse to `Error::Locked` indiscriminately**
  — only the contention case (`EWOULDBLOCK`) maps to `Locked`; `EINTR`,
  `EIO`, `ENOLCK` and other real OS faults surface as the structured
  `Error::Io { path, source }` variant. Closes #16.

### Fixed (clippy hygiene)

- Array-based char matchers in `crates/ts3level-core/src/ini.rs`
  (replaces three closure-style trim predicates).
- Removed an unnecessary `u32 → u32` cast in
  `crates/ts3level-gui/src/window.rs`.
- Workspace-wide `cargo fmt` pass.

### Changed

- **Minimum Supported Rust Version bumped from 1.75 to 1.82** — the
  test fixtures already use `std::iter::repeat_n`, which is stable since
  1.82. Released stable Rust is now 1.86 / 1.87, so the new floor is
  well below current.

### Fixed (clippy hygiene)

- Array-based char matchers in `crates/ts3level-core/src/ini.rs`
  (replaces three closure-style trim predicates).
- Removed an unnecessary `u32 → u32` cast in
  `crates/ts3level-gui/src/window.rs`.
- Workspace-wide `cargo fmt` pass.

## [0.1.0] — 2026-05-15

First public release.

### Added

- **Core algorithm**: TS3 identity `.ini` parser with byte-identical
  roundtrip, XOR + SHA-1 de-obfuscation, libtomcrypt ASN.1 DER parsing
  and public-only re-emission, reference CPU SHA-1 implementation of
  the security-level formula, atomic file writer with `flock` and a
  one-shot `.bak` backup.
- **CUDA kernel** for SHA-1 brute-force search of higher levels.
  Compiled to a fatbin covering `sm_70 … sm_90` plus PTX fallback for
  newer architectures, embedded in the Rust binary at build time.
  Verified against the reference implementation byte-for-byte.
- **Headless CLI** (`ts3level`): clap-based with identity summary,
  target auto-bump, indicatif progress, inline GPU telemetry,
  configurable batch size, Ctrl+C handling, stable exit codes.
- **GTK4 + libadwaita GUI** (`ts3level-gui`): two-column layout,
  identity details inline, live progress, NVML-based GPU stats panel
  with a Cairo utilization graph (60 s history), About window with
  references and translator credits.
- **i18n** in English, German, Spanish, French via gettext. `.po`
  files compiled by `cargo xtask msgfmt`.
- **Documentation**: README, installation guide, usage walk-through,
  algorithm specification, legal posture, build instructions, exit
  codes, compatibility matrix, roadmap.
- **68 automated tests** across the workspace, including GPU/CPU
  parity on 1 M counters and end-to-end CLI runs.

### Verified against the live TeamSpeak 3 client

The fingerprint and security level computed by this tool match what
the official client reports for the same identity, byte-identical.

### Known limitations / roadmap items

- GPU performance is on the conservative side of the field (~2.4 GH/s
  on an RTX 4060 Ti vs. ~20 GH/s for hand-tuned reference kernels).
  Kernel-level optimization (midstate precompute, in-place counter
  increment, PTX intrinsics) is package B in
  [docs/roadmap.md](docs/roadmap.md) and would close most of that gap.
- Pascal (`sm_61`) and older GPUs need a one-line patch to
  `crates/ts3level-cuda/build.rs` plus a rebuild.
- Pre-built binary requires glibc ≥ 2.39; older distros need to build
  from source.
- OpenCL/AMD support pending — the `HashEngine` trait makes this a
  drop-in.

[0.1.0]: https://github.com/Kernel-Error/ts-identities-security-level/releases/tag/v0.1.0
