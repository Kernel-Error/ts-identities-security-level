# Roadmap

Open work items live as [GitHub
Issues](https://github.com/Kernel-Error/ts-identities-security-level/issues).
This file keeps a per-package overview for context — the issues are
the source of truth for what's actually in flight.

## A — Auto-tune launch geometry · *done* ✓

Landed in commit `33294e7` (closed issue #2). On the first
`select_device` call per device, the engine runs a 9-combination probe
sweep (3 × 3 over `{16, 32, 48} × {128, 256, 512}`), picks the fastest,
and caches the result in `$XDG_CACHE_HOME/ts3level/tuning.json`. CLI
gains `--retune` to ignore the cache. Measured +18 % on the RTX
4060 Ti (2.4 → 2.85 GH/s).

## B — Kernel-level optimization

> **C-2a (done):** hashcat-derived round macros, `LOP3.LUT`-friendly
> Ch/Maj, `__funnelshift_l` rotates. Performance-flat on Ada
> (nvcc already lowers the textbook expressions well), but normalises
> the inner-loop shape for the next two tranches.
>
> **C-2c (done):** host-side midstate precompute. The host SHA-1s the
> constant pubkey prefix once per launch; the kernel only processes
> the tail. Combined with the smaller resulting stack frame and a
> volatile-read fast path on the device-side `g_best_level` slot,
> sustained hashrate on the 4060 Ti went from 2.85 to 5.94 GH/s
> (+108 %). The atomic fast path mattered more than the compute
> savings — the original kernel was atomic-bound on the from-scratch
> case (every level≥1 hit serialised on one device slot).
>
> **C-2b (done, partial):** in-place decimal-counter increment.
> `utoa64_be` no longer runs per attempt; the kernel bumps the
> trailing digits of `msg[]` in place and re-bakes the 0x80 / zero
> fill / length suffix only when the counter crosses a 10^k
> boundary. Combined with C-2c, sustained hashrate on the 4060 Ti is
> now 7.66 GH/s, up from 2.85 GH/s pre-tranche (~2.7×). The remaining
> ceiling on this card is set by the per-thread `msg[]` byte buffer
> still living in local memory — the next plausible step is to
> represent the message tail as 16 u32 registers and splice the
> counter digits in via shift/mask, but that's a substantial rewrite
> and not on the near-term path.

Reference: `thissepic/TeamSpeakHasher` reaches ~20 GH/s on an RTX
4070 Ti; we currently sit at ~7.66 GH/s on a 4060 Ti.

The last remaining piece — move the per-thread `msg[]` byte buffer
from local memory into u32 registers, with the counter ASCII spliced
into the right words via shift/mask — is tracked as
[#27](https://github.com/Kernel-Error/ts-identities-security-level/issues/27).
Expected ~30-50 % beyond today (would put the 4060 Ti close to the
~10 GH/s ceiling from the original B-package estimate).

Practical wall-clock reach today at 7.66 GH/s, single 4060 Ti:

- Level 40: ~2.4 min
- Level 50: ~1.7 days
- Level 55: ~54 days
- Level 60+: still not practical on a single card.

Strict gate before any further kernel-shape changes: the GPU/CPU
parity tests in `crates/ts3level-cuda/tests/cpu_parity.rs` must stay
green for every intermediate state, not just the final one. The
`many_independent_windows_match_cpu` test in particular verifies
8 independent (counter, level) pairs against the CPU reference and
catches per-bit-position regressions that the single-winner check
would miss.

## C — Multi-GPU support (linear win per extra card, ~½ day)

`enumerate()` already returns multiple devices. Only the `--device`
flag's selection is single-valued today.

Plan:
- Allow `--devices 0,1,2,...` (or a "use all" flag).
- The driver splits the counter range across workers, one per device.
- Each worker reports its best to a shared `atomicMax`-style channel; on
  improvement the driver writes the file once (single writer).
- Hashrate display becomes the sum of per-device rates; ETA uses the
  aggregate.

Linear speedup per added card. Only interesting on rigs with multiple
GPUs.

## Other ideas, lower priority

- **Pascal / Maxwell support** in the prebuilt binary. One-line change
  to `crates/ts3level-cuda/build.rs` (`("compute_61", "sm_61")` etc.).
  Currently documented as a manual rebuild step in
  [compatibility.md](compatibility.md); could just be in the default
  fatbin instead.
- **OpenCL backend** for AMD/Intel via the existing `HashEngine` trait.
  Most of the work is porting the kernel; the engine glue is ready.
- **Vulkan compute** as a third backend — overkill for SHA-1 but
  attractive for cross-vendor support.
- **Preflight error localization** — currently the `PreflightError`
  variants render through `thiserror`'s Display, in English. The
  user-facing flow strings are translated; only structured preflight
  errors fall back.
- **`clap` help-text translation** — separate from `gettext` flow.
- **GUI integration test** with a headless Wayland compositor.
- **Additional languages** — Dutch (`nl`) and other community-driven
  translations on top of en/de/es/fr.
- **Committed end-to-end algorithm test vector** — a self-generated
  identity in real TS3 `.ini` format plus the expected fingerprint and
  level values, so the algorithm round-trip is verified in CI without
  any manual cross-check.
