# Compatibility

What hardware and OS combinations the prebuilt binary runs on, and what
to do when one of them doesn't match.

## NVIDIA GPU

The Fatbin compiled into the binary contains native SASS for the
following architectures, plus a PTX-only `compute_90` block that the
driver JIT-compiles for any newer card:

| Generation        | Example cards                          | Compute cap | Status            |
|-------------------|----------------------------------------|-------------|-------------------|
| Volta             | Titan V, Tesla V100                    | `sm_70`     | native SASS       |
| Turing            | RTX 2060/2070/2080, GTX 16xx, Tesla T4 | `sm_75`     | native SASS       |
| Ampere (DC)       | A100                                   | `sm_80`     | native SASS       |
| Ampere (Consumer) | RTX 3060/3070/3080/3090                | `sm_86`     | native SASS       |
| Ada               | RTX 4060/4070/4080/4090                | `sm_89`     | native SASS       |
| Hopper            | H100                                   | `sm_90`     | native SASS       |
| Blackwell / newer | RTX 5xxx, GB200, …                     | `sm_100+`   | PTX-JIT fallback  |

**Not currently supported by the prebuilt binary:**

- **Pascal** (`sm_60`/`sm_61`) — GTX 1050/1060/1070/1080, Titan X Pascal,
  Tesla P100
- **Maxwell and older** (`sm_5x`, `sm_3x`) — GTX 9xx, GTX 7xx, …

These cards would either fail at module load with *"no kernel image
available for execution on the device"* or be slow enough that SHA-1
brute-forcing is impractical anyway.

To add Pascal support, append `("compute_61", "sm_61")` to the `TARGETS`
array in [`crates/ts3level-cuda/build.rs`](../crates/ts3level-cuda/build.rs)
and rebuild.

## NVIDIA driver

- **Minimum: 525.60.x.** That is the driver shipping CUDA 12.0 runtime
  compatibility, which is what we link against (`cudarc` feature
  `cuda-12000`).
- Released late 2022. Anything resembling a current install of Ubuntu,
  Debian, Fedora, RHEL, Arch, … on a machine that's been updated in the
  last three years easily meets this.
- The driver provides both `libcuda.so.1` and `libnvidia-ml.so.1`
  (NVML). Both are resolved dynamically at runtime — there is no
  build-time link to either, so you do not need the CUDA Toolkit
  installed to *run* the binary.

## Linux distribution / glibc

The binary is a regular dynamically-linked ELF. Its glibc requirement
matches whatever distro it was built on.

| Built on                 | glibc | Runs on                                       |
|--------------------------|-------|-----------------------------------------------|
| Ubuntu 24.04 / Mint 22.x | 2.39  | Ubuntu 24.04+, Debian 13+, Fedora 39+, Arch   |
| Ubuntu 22.04             | 2.35  | Ubuntu 22.04+, RHEL 9+, Debian 12+, …         |

If you want a binary that runs on older distros, the simplest path is
to **build from source on the target machine**:

```bash
sudo apt install rustup nvidia-cuda-toolkit libgtk-4-dev libadwaita-1-dev gettext
rustup default stable
cargo build --release
```

`build.rs` finds `nvcc` automatically; you only need GTK4 + libadwaita
dev packages for `ts3level-gui` — the CLI builds without them.

A future release will provide a **Flatpak** that bundles its own GTK,
glibc, and NVIDIA-GL runtime from Flathub, sidestepping all of this.

## GUI desktop dependencies

`ts3level-gui` needs:

- **GTK 4 ≥ 4.12**
- **libadwaita ≥ 1.5**
- **gettext** runtime
- An X11 or Wayland session

Recent Ubuntu, Fedora, Mint, Arch, openSUSE Tumbleweed ship these by
default. On Debian stable or other distros with older GNOME stacks you
may need to install libadwaita 1.5 manually, or use the CLI.

The headless **CLI has no GUI dependencies** — `libcuda.so.1`
(installed by the NVIDIA driver) is the only mandatory shared library
beyond glibc.

## Portability decision matrix

| Target system                                       | What to do                              |
|-----------------------------------------------------|-----------------------------------------|
| Recent desktop, modern card (RTX 20-series or newer)| Copy the binary, run it.                |
| Older card (GTX 10-series, Pascal)                  | Patch `build.rs` (add `sm_61`), rebuild.|
| Older distro (Ubuntu 22.04, Debian 12, RHEL 9)      | Build from source on the target.        |
| Headless server                                     | Use `ts3level` (CLI); no GUI deps needed.|
| Multi-distro distribution                           | Wait for the Flatpak release (planned). |
