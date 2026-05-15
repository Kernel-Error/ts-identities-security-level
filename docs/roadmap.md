# Roadmap

Ranked by impact-per-effort. Pick one when there's appetite for it; the
project is shippable as-is at v0.1.

## A — Auto-tune launch geometry (small win, ~½ day)

Current state: `BLOCKS_PER_SM = 32`, `THREADS_PER_BLOCK = 256` are
constants in `crates/ts3level-cuda/src/lib.rs`. Works correctly on every
supported arch but is not optimal for any specific one.

Plan:
- On first launch with a given device, run 3-4 short probe batches with
  combinations of `(blocks_per_sm ∈ {16, 24, 32, 48}, threads_per_block ∈
  {128, 256, 384, 512})`.
- Pick the fastest, cache by device name to
  `~/.cache/ts3level/tuning.json`.
- Subsequent runs read the cache and skip probing.

Expected gain: ~5-20 % on a single card. Marginal next to package B,
but cheap and self-contained.

## B — Kernel-level optimization (big win, ~1-2 days)

Where the actual ~5-8× headroom is. Reference: `thissepic/TeamSpeakHasher`
achieves ~20 GH/s on an RTX 4070 Ti; we sit at ~2.4 GH/s on a 4060 Ti
with the simple kernel.

Items, each independently testable:

1. **SHA-1 midstate precompute.** `pubkey_b64` is constant across a
   launch. The host can run the first 1-2 SHA-1 blocks once and pass the
   intermediate `h[0..5]` to the kernel; the kernel starts from there and
   only processes the final 1-2 blocks containing the counter digits.
   ~2× alone.

2. **In-place counter increment.** The current `utoa64_be` loop
   recomputes the full decimal string per attempt. With sequential
   counters, 90 %+ of iterations only need to bump the least-significant
   digit (with carry propagation for boundaries like 9999999 →
   10000000). ~1.5×.

3. **PTX intrinsics.** Use `__clz` for leading-zero counting,
   `LOP3.LUT`-friendly forms for SHA-1's `Ch`/`Maj`/`Parity` round
   functions, and `__funnelshift_l` for 32-bit rotations. Each saves a
   handful of cycles per block transform. ~1.5×.

4. **Move the message buffer out of stack-allocated arrays into
   registers** where possible — the current `uint8_t msg[192]` per
   thread spills to local memory on most archs and hurts occupancy.
   ~1.3×.

Strict gate before merging: extend the CPU/GPU parity test
(`crates/ts3level-cuda/tests/cpu_parity.rs`) to cover the new code paths
exhaustively. A subtle off-by-one in the SHA-1 padding would silently
corrupt every hash and the user would never know — the CLI/GUI would
just stop finding levels.

Expected outcome on the 4060 Ti: ~10 GH/s. Level 55 ETA drops from
~16 h to ~4 h; level 60 ETA drops from ~50 d to ~12 d.

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
