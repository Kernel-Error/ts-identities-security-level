# Installation

What you need installed on your system, and which permissions your user
account must have, to run the prebuilt binaries or build from source.

If something is missing the tool will refuse to start and tell you what
to fix — there's no silent failure mode for these.

## 1. NVIDIA driver

The only mandatory runtime dependency. Provides `libcuda.so.1` (the
CUDA driver API the tool calls) and `libnvidia-ml.so.1` (NVML, for the
live GPU stats).

Verify with:

```bash
nvidia-smi
```

If that prints a table with your card, the driver is good. If it says
*"NVIDIA-SMI has failed because it couldn't communicate with the NVIDIA
driver"*, the driver isn't loaded — install it through your distro:

| Distro                | Install command                                        |
|-----------------------|--------------------------------------------------------|
| Ubuntu / Mint / Debian| `sudo apt install nvidia-driver-550` (or `-535`)       |
| Fedora                | `sudo dnf install akmod-nvidia` (RPM Fusion)           |
| Arch                  | `sudo pacman -S nvidia nvidia-utils`                   |
| openSUSE              | `sudo zypper install nvidia-glG06-kmp-default` (varies)|

Reboot after a fresh driver install. Minimum supported version: **525**
(see [compatibility.md](compatibility.md)).

## 2. GPU device permissions

The driver creates character devices in `/dev/nvidia*`. On a stock
Linux install, these are owned by `root:video` (some distros use
`root:render`) with mode `660`. **Your user must be in the matching
group**, otherwise opening the device fails with `EACCES` and the tool
exits with `error: device file permission problem`.

Check which group:

```bash
ls -l /dev/nvidiactl /dev/nvidia0
# crw-rw-rw-+ 1 root render 195, 255  ...  /dev/nvidiactl
```

Check whether you're in it:

```bash
groups
# kernel adm cdrom sudo dip video render ...
```

If `video` (Debian/Ubuntu/Mint default) or `render` (Fedora/Arch
default) is missing, add yourself:

```bash
sudo usermod -aG render $USER     # or 'video', whichever applies
```

**Log out and log back in** (or reboot). `usermod` only takes effect for
new sessions; running `newgrp` is a partial workaround that affects only
the current shell.

A quick verify run:

```bash
ts3level --list-devices
# [CUDA:0] NVIDIA GeForce RTX 4060 Ti (cc 8.9, 34 SMs, 15.6 GiB)
```

## 3. Identity file permissions

`ts3level` reads, locks, and atomically replaces the `.ini` you point it
at. That means:

- **The user running the tool must own (or have read+write on) the
  `.ini`.** Check with `ls -l identity.ini`.
- **The parent directory must be writable** — the new content is written
  to a temp file there, then `rename(2)`'d on top of the original
  (POSIX-atomic, only works within one filesystem). The tool refuses to
  start with *"parent directory is not writable"* otherwise.
- **No other process may hold an exclusive lock** on the file. In
  practice this is the TS3 client when it's running with that identity
  active. Close TS3, or work on a freshly exported copy of the file.

There is no need for `sudo` — the tool runs entirely with your normal
user rights. If you find yourself running it as root, something is
configured wrong (and a level-bump as root could create a file owned by
root that the TS3 client can no longer overwrite cleanly).

## 4. Installing the binaries

You have three options.

### a) Prebuilt release tarball (easiest)

Attached to each [GitHub
release](https://github.com/Kernel-Error/ts-identities-security-level/releases).
Built on Ubuntu 24.04, glibc 2.39 — runs on any newer Linux.

```bash
curl -LO https://github.com/Kernel-Error/ts-identities-security-level/releases/latest/download/ts3level-v0.3.0-x86_64-linux.tar.gz
tar -xzf ts3level-v0.3.0-x86_64-linux.tar.gz
cd ts3level-v0.3.0-x86_64-linux
sudo install -m755 bin/ts3level     /usr/local/bin/
sudo install -m755 bin/ts3level-gui /usr/local/bin/
sudo cp -r share/locale/*           /usr/share/locale/
```

Verify: `ts3level --list-devices`.

### b) Run from the unpacked tarball without installing

```bash
TS3LEVEL_LOCALEDIR="$PWD/share/locale" ./bin/ts3level --list-devices
TS3LEVEL_LOCALEDIR="$PWD/share/locale" ./bin/ts3level-gui
```

The env var tells the binary where to find the compiled `.mo`
translation files. With them missing, the UI falls back to English.

### c) Build from source

See [section 7](#7-building-from-source) below. Required if your distro
predates glibc 2.39, or if you need Pascal-era GPU support.

## 5. Running the CLI (headless)

The CLI binary has no GUI dependencies. On a headless server you only
need the NVIDIA driver and your user in the right group:

```bash
ts3level --target 55 ~/identity.ini
```

That's it. Works inside SSH, tmux, systemd services, etc.

For systemd, the unit file should set `User=youruser` (not `User=root`)
and `Group=render`/`video` matching the device permissions.

## 6. Running the GUI

Additional runtime libraries required only by `ts3level-gui`:

| Distro                | Install command                                                       |
|-----------------------|-----------------------------------------------------------------------|
| Ubuntu / Mint / Debian| `sudo apt install libgtk-4-1 libadwaita-1-0 gettext-base`             |
| Fedora                | `sudo dnf install gtk4 libadwaita gettext`                            |
| Arch                  | `sudo pacman -S gtk4 libadwaita gettext`                              |
| openSUSE              | `sudo zypper install libgtk-4-1 libadwaita-1-0 gettext-runtime`       |

Minimum versions: GTK 4.12, libadwaita 1.5. Anything from mid-2024
onwards has them; on Debian stable or other distros with older GNOME
stacks you may have to backport or use the CLI instead.

## 7. Building from source

Only needed if your distro is too old for the prebuilt binary or you
want Pascal-era GPU support.

| Distro                | Build dependencies                                                                                                          |
|-----------------------|-----------------------------------------------------------------------------------------------------------------------------|
| Ubuntu / Mint / Debian| `sudo apt install rustup nvidia-cuda-toolkit libgtk-4-dev libadwaita-1-dev gettext pkg-config libssl-dev build-essential`   |
| Fedora                | `sudo dnf install rustup cuda gtk4-devel libadwaita-devel gettext-devel pkgconfig openssl-devel gcc`                        |
| Arch                  | `sudo pacman -S rustup cuda gtk4 libadwaita gettext pkgconf openssl base-devel`                                             |

Then:

```bash
rustup default stable          # one-time
cargo build --release          # both binaries
cargo xtask msgfmt             # compile translations
```

Outputs: `target/release/ts3level` and `target/release/ts3level-gui`.

You do **not** need root for any of these — the toolchain installs to
`~/.cargo` and the build writes to the project's `target/` directory.

## 8. Files the tool writes outside the project directory

`ts3level` creates exactly one file outside the directory you point it
at:

- **`$XDG_CACHE_HOME/ts3level/tuning.json`** (default
  `~/.cache/ts3level/tuning.json`) — caches the optimal CUDA launch
  geometry per GPU after the first run. Safe to delete; the tool will
  re-probe on the next start. Pass `--retune` to force a fresh probe
  without removing the file.

That's it. No state in `/etc`, no daemon, no systemd unit, no log
files anywhere.

## 9. What the preflight checks for at every start

A condensed list of what the tool verifies before doing any work, in
the order it checks them:

1. `libcuda.so.1` is loadable (NVIDIA driver installed).
2. CUDA reports at least one device.
3. Selected device's `/dev/nvidia*` is openable for read/write.
4. The `.ini` exists and is readable by your user.
5. The `.ini` is writable.
6. The parent directory is writable (for the atomic rename).
7. The `.ini` parses as a TeamSpeak 3 identity.
8. No other process holds an exclusive lock on it.
9. Free disk space ≥ 2× the file size (for the `.bak` copy).

Any failure produces a one-line error message and a documented exit
code ([exit-codes.md](exit-codes.md)). No backtraces, no surprises.
