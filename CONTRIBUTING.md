# Contributing

Thanks for thinking about contributing! This is a small Rust + CUDA
project; the surface area is friendly to newcomers. The list of open
work items is in [GitHub Issues](https://github.com/Kernel-Error/ts-identities-security-level/issues);
look for the `good first issue` label if you want a gentle start.

## ⚠️ Before you open an issue or pull request

**Do not attach an exported TeamSpeak 3 identity `.ini` file.** It
contains private key material. If a parser, kernel, or writer bug
needs a reproducer, generate a synthetic identity the way the existing
tests do — see `crates/ts3level-core/tests/known_vector.rs` or
`crates/ts3level-cli/tests/cli_smoke.rs::synthetic_ini`.

## Build prerequisites

- Rust ≥ 1.82 (the workspace MSRV; install via `rustup`)
- CUDA Toolkit ≥ 12.0 (provides `nvcc` for the kernel build; runtime
  doesn't need it, only the build does)
- For the GUI: `libgtk-4-dev`, `libadwaita-1-dev`, `gettext`

Distro-specific instructions live in
[`docs/installation.md`](docs/installation.md) (runtime) and
[`docs/building.md`](docs/building.md) (build from source).

## Commands you'll need

Same three commands that CI runs:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --release -- -D warnings
cargo test --workspace --release
```

If your change touches any user-facing string, regenerate the compiled
translations and update the affected `po/*.po` files:

```bash
cargo run -p xtask --release -- msgfmt
```

If you touch the CUDA kernel, run the parity test against a real GPU
locally — the host-runner CI cannot exercise the GPU side:

```bash
cargo test -p ts3level-cuda --release
```

## What's in scope vs out

**In scope:**

- Bug fixes (see issues labelled `bug` / `security`).
- Performance improvements to the CUDA kernel (issue #1 has the design
  sketch).
- New language translations (issue #11 shows how Dutch would land).
- Hardware coverage: additional `sm_XX` targets, OpenCL backend.
- Documentation: clarifications, typo fixes, distro-specific notes in
  `docs/installation.md` or `docs/compatibility.md`.
- Tests that cover currently uncovered paths (see the bug issues for
  pointers — `bug` + `tests` labels).

**Out of scope:**

- Anything that contacts a TS3 server, bypasses access controls, or
  modifies the official TeamSpeak client.
- Marketing-flavored changes to README/docs (terms like "crack" /
  "bypass" / "exploit" are off-limits — see
  [`docs/legal.md`](docs/legal.md) for the rationale).
- Plugins or integrations that load into the TS3 client.

## Pull request workflow

1. Fork the repo, branch off `main` (`git checkout -b your-change`).
2. Make the change. Keep the diff focused — split unrelated cleanups
   into separate commits or PRs.
3. Run the four commands above locally. Make sure they pass.
4. Push your branch, open a PR against `main`. The pull-request
   template asks for a summary, change list, test plan, and any linked
   issues.
5. CI will run the same checks on your branch. A green CI is the
   merge baseline.
6. The maintainer reviews and merges, or requests changes.

## Commit messages

Use plain English. Imperative mood is fine but not required. Keep the
summary line under ~72 characters and explain the *why* in the body if
it's not obvious from the diff.

## Licensing

By submitting a contribution you agree it is released under the same
[MIT license](LICENSE) as the rest of the project. No CLA, no DCO
signoff required.
