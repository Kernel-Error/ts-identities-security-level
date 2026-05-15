# Building from source

End users do **not** need any of this — the released binary only
requires the NVIDIA driver. This document covers building from source.

## Prerequisites

- Linux x86_64 (other arches untested; should work on aarch64 once the
  CUDA Toolkit supports it on the target distro).
- NVIDIA driver ≥ 525 with `libcuda.so.1` (required to *run*).
- Rust ≥ 1.82 (workspace MSRV) — install via `rustup` or your distro's
  `rustup` package.
- CUDA Toolkit ≥ 12.0 (provides `nvcc`, needed to *build* the kernel).
- GTK 4.12+ and libadwaita 1.5+ development headers (for `ts3level-gui`).
- `gettext` (for `xgettext` and `msgfmt`).
- A reasonably modern C/C++ compiler.

## Ubuntu / Linux Mint / Debian-derived

```bash
sudo apt install \
    nvidia-cuda-toolkit \
    libgtk-4-dev libadwaita-1-dev \
    gettext pkg-config libssl-dev \
    build-essential rustup

rustup default stable
```

## Build

```bash
cargo build --release
```

Build artifacts:

- `target/release/ts3level` — headless CLI
- `target/release/ts3level-gui` — GTK4 GUI

## Compile translations

The build does not produce `.mo` files automatically; run them through
the project automation:

```bash
cargo xtask msgfmt
```

This writes `target/locale/<lang>/LC_MESSAGES/ts3level.mo`. At runtime,
either install them to `/usr/share/locale` (system) or set
`TS3LEVEL_LOCALEDIR=$(pwd)/target/locale` in the environment.

## Run

```bash
./target/release/ts3level --list-devices

./target/release/ts3level --target 50 path/to/your-identity.ini
```

The file gets a one-shot `.bak` next to it on the first write; subsequent
updates of `--target` mode keep the `.bak` pristine.

## Tests

```bash
cargo test --workspace --release
```

GPU-dependent tests in `ts3level-cuda` and `ts3level-cli` skip themselves
gracefully when `/dev/nvidiactl` is not present.

## Where the CUDA kernel comes from

`crates/ts3level-cuda/build.rs` invokes `nvcc --fatbin` over
`kernels/sha1_hasher.cu`, embedding the resulting fatbin into the Rust
binary with `include_bytes!`. End users never see `nvcc`; the fatbin
contains pre-compiled SASS for `sm_70..sm_90` plus a PTX fallback for
future architectures (JIT'd at module-load time by the driver).

You can override the CUDA Toolkit location with `CUDA_PATH=/path/to/cuda`.
