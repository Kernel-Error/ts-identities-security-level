# ts-identities-security-level

GPU-accelerated tool for Linux that raises the **security level** of a
TeamSpeak 3 identity file. Point it at your `.ini`, pick a GPU, set a target
level (or let it run forever), and the tool updates the file in place each
time it finds a higher level. No copy-paste, no manual export/import.

![Main window during a hash run, showing identity details, live
progress, and GPU telemetry.](docs/screenshots/main-window.png)

Two binaries from one workspace:

- `ts3level` — headless CLI for servers, scripts, tmux sessions.
- `ts3level-gui` — small GTK4 + libadwaita desktop app, translated into
  English, German, Spanish, and French.

NVIDIA only for now (CUDA). OpenCL/AMD support is on the roadmap.

## Disclaimer

This project is not affiliated with, endorsed by, or sponsored by TeamSpeak
Systems GmbH. "TeamSpeak" is a trademark of TeamSpeak Systems GmbH and is
used here for descriptive purposes only.

The tool operates exclusively on a local identity file that the user owns.
It performs no network communication with TeamSpeak servers, does not modify
the TeamSpeak client, and does not circumvent any access control. The
"security level" is proof-of-work — leading zero bits of a SHA-1 hash — and
this tool just computes the same hashes the official client computes,
faster, on a GPU.

## Usage in one minute

1. In the TeamSpeak 3 client: **Tools → Identities**, right-click the
   identity you want to raise, pick **Export**, save the `.ini`.
2. Run this tool (`ts3level-gui` or `ts3level --target N path/to.ini`).
   The file is updated in place each time a higher level is found; a
   one-shot `.bak` of the original is kept next to it.
3. Back in TS3: **Tools → Identities → Import**, pick the same `.ini`.
   The client now shows the higher Security Level.

Full walk-through with screenshots and tips: [docs/usage.md](docs/usage.md).

## How it works

The security level of a TeamSpeak 3 identity is defined as:

```
level = leading_zero_bits( SHA1( public_key_base64 || decimal_counter ) )
```

To raise the level, you increment the counter and rehash until you find one
with more leading zero bits. The official client does this on one CPU
thread; past level ~50 that is impractically slow. A modern NVIDIA GPU can
do tens of billions of SHA-1 hashes per second.

Updating the identity file means **only the digits before the `V`** in the
`identity="..."` line change. The base64-encoded keypair stays
byte-identical. The tool writes the new file atomically (temp file + rename)
and keeps a one-time `.bak` of the original.

## Installation

Required packages per distro, plus how to set the `/dev/nvidia*` group
permissions your user needs, and what the tool checks for at every
start, are in [docs/installation.md](docs/installation.md). Quick
version for Ubuntu/Mint/Debian:

```bash
sudo apt install nvidia-driver-550 libgtk-4-1 libadwaita-1-0 gettext-base
sudo usermod -aG render $USER       # log out + back in afterwards
```

## Building from source

See [docs/building.md](docs/building.md). You need a Rust toolchain, the
CUDA Toolkit (for the kernel compile step), and GTK4 + libadwaita dev
headers if you want the GUI. End users only need the NVIDIA driver —
everything else is statically or dynamically resolved at runtime.

## Compatibility

Which GPUs, drivers, and distributions are supported by the prebuilt
binary, and what to do when one of them doesn't match, is documented in
[docs/compatibility.md](docs/compatibility.md). Short version: any
NVIDIA Volta-or-newer card with driver ≥ 525 on a recent Linux
distribution works out of the box; Pascal and older need a one-line
patch to `build.rs` and a rebuild from source.

## Algorithm provenance

The identity format and the level formula are documented by long-standing
open-source projects derived from black-box analysis of the `.ini` file
(no decompilation of the client). See [docs/algorithm.md](docs/algorithm.md)
for references and full pseudocode.

## License

MIT — see [LICENSE](LICENSE).
