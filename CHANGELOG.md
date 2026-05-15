# Changelog

All notable changes to this project are documented here. The format is
loosely based on [Keep a Changelog](https://keepachangelog.com), the
project follows [Semantic Versioning](https://semver.org).

## [Unreleased]

## [0.3.0] — 2026-05-15

### Performance

- **In-place decimal counter increment.** The kernel no longer
  re-encodes the counter ASCII from scratch every iteration via
  `utoa64_be`; instead it bumps the trailing digits in place and only
  re-bakes the message tail when the counter crosses a 10^k boundary
  (which happens log10(n_per_thread) times at most across an entire
  launch). The SHA-1 padding byte, zero fill, and 8-byte length suffix
  are also pre-baked once per thread now and only re-laid when the
  digit count changes. Per-iteration write traffic to the per-thread
  `msg[]` buffer drops from ~50 bytes to ~1 byte in the common
  no-carry case. Measured 5.94 → **7.66 GH/s** (+29 %) on the RTX
  4060 Ti. Median of 5 runs at 500 M counters.
- **Host-side SHA-1 midstate precompute and an atomic-fast-path
  rewrite roughly doubled the sustained hashrate before that.**
  Measured on the RTX 4060 Ti, 500 M-counter window: 2.85 GH/s before,
  5.94 GH/s after (+108 %). Three changes layered:
  - **Midstate precompute.** The pubkey prefix is constant across a
    launch, so the host SHA-1s every complete 64-byte block of it
    once and passes the resulting `h[0..5]` to the kernel. The kernel
    only processes the tail per attempt. For the typical TS3 P-256
    base64 pubkey (108 chars) one full SHA-1 block per attempt drops
    out. Implementation in `crates/ts3level-core/src/sha1_block.rs`
    (a dependency-free 80-line transform, cross-checked against the
    `sha1` crate by four new unit tests).
  - **Smaller per-thread stack frame.** With midstate, the
    `MAX_INPUT_BYTES` constant drops from 192 to 128, so the
    register/local-memory mix shifts and the kernel's stack frame
    shrinks from 224 to 160 bytes per thread.
  - **Atomic fast-path on `g_best_level`.** Profiling showed the
    kernel was atomic-bound, not compute-bound: with
    `current_best_level = 0` (the from-scratch case) every level≥1
    hit — roughly half of all iterations — fired an `atomicMax` on a
    single device slot. The kernel now reads `g_best_level` through a
    volatile pointer as a pre-filter; only matches that actually
    exceed the current grid-wide best reach the atomic. Correctness
    is unchanged because the CPU re-verifies every kept result.
- **CUDA SHA-1 inner loop rewritten in the shape used by hashcat's
  `OpenCL/inc_hash_sha1.h`** (MIT-licensed, attribution in
  docs/algorithm.md). Round-function macros for Ch/Maj that nvcc
  auto-lowers to `LOP3.LUT`, `__funnelshift_l` 32-bit rotates, and the
  standard 5-step register-cycling unroll. By itself this change is
  performance-flat on Ada (nvcc already lowers the textbook
  expressions to LOP3), but it normalises the kernel's inner-loop
  shape against the reference and is a prerequisite for the
  register-resident message buffer rewrite tracked under issue #1.

### Added (tests)

- **`crates/ts3level-cuda/tests/cpu_parity.rs::many_independent_windows_match_cpu`**
  — verifies 8 independent (counter, level) pairs against the CPU
  reference instead of only the single overall winner in one window.
  Off-by-one regressions affecting specific bit positions would slip
  past the single-winner test but get caught here.
- **`crates/ts3level-core/src/sha1_block.rs` tests** — four
  cross-checks against the `sha1` crate for a 64-byte prefix, a
  128-byte prefix with long suffix, the realistic TS3 input shape,
  and the empty-message edge case.

## [0.2.0] — 2026-05-15

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

- **CUDA kernel atomic packing replaced with two separate device slots**.
  Reporting `(level << 56) | counter` via a single `atomicMax` silently
  truncates the counter to 56 bits, which would corrupt the level field
  once counter values cross `2^56` (reachable around level 57). The
  kernel now uses `g_best_level` (u32) and `g_best_counter` (u64) and a
  two-step atomic update. The host re-verifies the reported counter via
  the CPU reference and accepts the verified level, so the benign
  per-batch race between the two atomics never produces a wrong write
  back to the `.ini`. Closes #12.
- **Kernel pubkey-length bounds check** at entry, defense-in-depth
  against an OOB write into the per-thread `msg[]` buffer if the host
  invariant (`pubkey_b64.len() ≤ 110`) is ever bypassed. Closes #13.
- **Driver: CPU verification mismatch no longer returns `Ok(())`**.
  The driver used to emit a `Progress::Done` with `reason: Error(...)`
  and exit success; scripts saw the run as fine when in fact a kernel
  result couldn't be reproduced. The new behavior is to trust the CPU
  reference: accept the verified level if it beats the current best,
  silently skip the batch otherwise. Closes #18.
- **`write_back`: backup creation is atomic and content comes from the
  locked fd.** Replaces the previous `if !bak.exists() { fs::copy(path,
  ...) }` (TOCTOU on the existence check, and re-opens by path so a
  path-swap attack could put foreign bytes into the backup) with
  `OpenOptions::create_new(true)` plus a `std::io::copy` from the
  already-open, already-locked source fd. Closes #15, #17.
- **`write_back`: preserve the source file's mode across the atomic
  rename.** The tempfile defaults to `0o600`; before the rename we now
  set its permissions to match the original (e.g. `0o644` or `0o660`).
  Without this, every level-up silently tightened the file's
  permission bits. Closes #22.
- **Preflight: lock the file before parsing.** Previously the sequence
  was `metadata → read → write-probe → parent-probe → parse → flock`,
  which allowed a TOCTOU window where the file could be replaced
  between `read` and `flock`. The new sequence opens once with read+
  write, takes a non-blocking flock right away, and reads + parses
  from the locked fd. The lock is released before returning, so
  `write_back` reacquires it later. Closes #19.
- **`probe_lock` opens with `read+write`** instead of read-only, so a
  pre-flight that passes accurately predicts what `write_back` will do
  next. Avoids the case where probe says "fine" on a readable-but-not-
  writable file and the actual write then fails at EACCES. Closes #21.
- **Atomic `write_back` now `fsync`s the parent directory** after the
  rename and after the one-shot `.bak` creation. POSIX rename is atomic
  for visibility but not for directory-entry durability across a crash.
  Closes #14.
- **`flock` errors no longer collapse to `Error::Locked` indiscriminately**
  — only the contention case (`EWOULDBLOCK`) maps to `Locked`; `EINTR`,
  `EIO`, `ENOLCK` and other real OS faults surface as the structured
  `Error::Io { path, source }` variant. Closes #16.

### Performance

- **Auto-tune CUDA launch geometry on first use of each device**. The
  previous hard-coded `(blocks_per_sm = 32, threads_per_block = 256)`
  was generic but suboptimal; a short 9-combination probe sweep
  (`{16, 32, 48} × {128, 256, 512}`) on first `select_device` picks
  the fastest. Result is cached as JSON in
  `$XDG_CACHE_HOME/ts3level/tuning.json` (or `~/.cache/ts3level/`)
  keyed by device name, so subsequent runs skip the probe. CLI gains
  `--retune` to ignore the cache and re-probe. Measured on the RTX
  4060 Ti: 2.4 GH/s → 2.85 GH/s (~+18 %). Sweep total: ~3.4 s, well
  inside the 5 s acceptance budget. Closes #2.

### Added (tests)

- **Committed algorithm fixture** at `crates/ts3level-core/testdata/
  known_identity.ini` with a deterministically-generated synthetic
  identity (counter=310, level=10). The matching `tests/
  known_identity_fixture.rs` hard-codes the expected fingerprint,
  public key, level at the committed counter, and levels at three
  secondary counters. Generator is `examples/gen_fixture.rs`. Closes
  #10. Any future drift in parser / deobfuscation / DER round-trip /
  fingerprint / level computation makes this test fire without needing
  manual cross-check against the live TS3 client.
- **Mode-preservation roundtrip test** in `writer::tests` verifies the
  source file's `0o640` is preserved across `write_back` instead of
  silently dropping to `0o600`.
- **Pre-existing `.bak` test** confirms `write_back` doesn't overwrite
  a `.bak` that was already on disk, even with different content.

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

[Unreleased]: https://github.com/Kernel-Error/ts-identities-security-level/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/Kernel-Error/ts-identities-security-level/releases/tag/v0.3.0
[0.2.0]: https://github.com/Kernel-Error/ts-identities-security-level/releases/tag/v0.2.0
[0.1.0]: https://github.com/Kernel-Error/ts-identities-security-level/releases/tag/v0.1.0
