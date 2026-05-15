---
name: Bug report
about: Something doesn't behave as documented
title: ''
labels: bug
assignees: ''
---

> ⚠️ **Do not attach exported TeamSpeak identity files.**
> They contain private key material. If a parser, kernel, or writer
> bug needs a reproducer, generate a synthetic identity the way the
> existing tests do (see `crates/ts3level-cli/tests/cli_smoke.rs::synthetic_ini`).
> Never paste your real `.ini` content into this issue.

## What happened

<!-- One or two sentences describing the bug. -->

## What I expected to happen

## Steps to reproduce

1.
2.
3.

## Environment

- ts3level version: <!-- `ts3level --version` -->
- Distribution: <!-- e.g. Ubuntu 24.04 / Mint 22.3 -->
- GPU model + driver: <!-- `nvidia-smi --query-gpu=name,driver_version --format=csv` -->
- CUDA Toolkit version (if built from source): <!-- `nvcc --version` -->
- Did you build from source or use the prebuilt tarball?

## Additional context

<!-- Logs, stderr, screenshots. Remember: NO identity files. -->
