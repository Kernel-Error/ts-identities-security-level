# CLI exit codes

`ts3level` returns the following codes. Scripts can rely on these values
remaining stable across patch releases.

| Code | Meaning |
|------|---------|
| 0    | success — target reached, or stop signal received in endless mode |
| 2    | usage error — bad CLI flags, device index out of range |
| 10   | NVIDIA driver / `libcuda.so.1` not found or not loadable |
| 11   | no CUDA-capable device found |
| 12   | device file permission denied (`/dev/nvidia*`) — user likely missing from `video` or `render` group |
| 20   | identity file does not exist |
| 21   | file permission problem — file not readable, not writable, or parent dir not writable |
| 22   | file locked by another process (typically the TS3 client) |
| 23   | file is not a valid TS3 identity (no `[Identity]` section or no `identity=` key) |
| 24   | not enough free disk space for the `.bak` copy |
| 30   | runtime error during hashing (kernel crash, CUDA OOM, …) |
