//! Auto-tune the CUDA launch geometry for the bound device.
//!
//! Different architectures hit their occupancy / throughput sweet spot
//! at different `(blocks_per_SM, threads_per_block)` combinations. The
//! defaults (32 × 256) work but are not optimal everywhere. A short
//! probe sweep before the first real launch usually closes a 10–20 %
//! gap on architectures the defaults weren't tuned against.

use crate::cache::{CachedTuning, TuningCache};
use cudarc::driver::{CudaContext, CudaFunction, CudaStream, LaunchConfig, PushKernelArg};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info};
use ts3level_engine::EngineError;

/// Combinations probed per `auto_tune` call. Keep this small enough
/// that the total wall time stays ≤ a few seconds even on slow GPUs.
pub const PROBE_BLOCKS_PER_SM: &[u32] = &[16, 32, 48];
pub const PROBE_THREADS_PER_BLOCK: &[u32] = &[128, 256, 512];

/// Counters worked per probe. Tuned so a 2.4 GH/s card spends ≈ 200 ms
/// per combo (500M / 2.4G ≈ 208 ms). Shorter probes give an unreliable
/// measurement on fast cards: launch overhead and CUDA streaming
/// pipelining drown out the actual kernel cost.  9 combos × 200 ms
/// ≈ 1.8 s, well within the acceptance-criterion 5 s budget. Slow
/// (Pascal-era) cards take proportionally longer but still finish under
/// 5 s in practice.
const PROBE_N_COUNTERS: u64 = 500_000_000;

/// Same-shape pubkey the kernel actually sees in production — ~104 b64
/// chars covering the full SHA-1 padding-block crossover.
const PROBE_PUBKEY: &str =
    "MEwDAgcAAgEgAiEA5jUbcc+RXAJzVKLpyEnoq/Otht1JBeCdRgRJYYBuOmoCIQDwBoRP+rkICZHbAGD9XYpV9bm08yPYGT4LehKXmlYZJg==";

/// Result the kernel writes to. We don't care about the actual best
/// value during tuning, only the time the launch took.
#[derive(Debug, Clone, Copy)]
pub struct Geometry {
    pub blocks_per_sm: u32,
    pub threads_per_block: u32,
}

impl Geometry {
    pub const DEFAULT: Self = Self {
        blocks_per_sm: 32,
        threads_per_block: 256,
    };
}

/// Try to load a cached tuning entry for `device_name`. If none, run
/// the probe sweep on the bound stream and write the winner back to the
/// cache. Returns the geometry the engine should use for real launches.
pub(crate) fn pick_geometry(
    ctx: &Arc<CudaContext>,
    stream: &Arc<CudaStream>,
    func: &CudaFunction,
    mp_count: u32,
    device_name: &str,
    force_retune: bool,
) -> Result<Geometry, EngineError> {
    if !force_retune {
        if let Ok(cache) = TuningCache::load() {
            if let Some(entry) = cache.get(device_name) {
                debug!(
                    "Using cached tuning for {device_name}: \
                     blocks_per_sm={}, threads_per_block={}, ~{:.2} GH/s",
                    entry.blocks_per_sm,
                    entry.threads_per_block,
                    entry.rate_mhps / 1000.0
                );
                return Ok(Geometry {
                    blocks_per_sm: entry.blocks_per_sm,
                    threads_per_block: entry.threads_per_block,
                });
            }
        }
    }

    info!("Tuning CUDA launch geometry for {device_name} (one-time probe)…");
    let (geometry, rate_hps) = run_probe(ctx, stream, func, mp_count)?;
    info!(
        "Selected blocks_per_sm={}, threads_per_block={} at ~{:.2} GH/s",
        geometry.blocks_per_sm,
        geometry.threads_per_block,
        rate_hps / 1e9,
    );

    // Best-effort cache update. A failure to write the cache must not
    // prevent the engine from running.
    if let Err(e) = TuningCache::update(
        device_name,
        CachedTuning {
            blocks_per_sm: geometry.blocks_per_sm,
            threads_per_block: geometry.threads_per_block,
            rate_mhps: rate_hps / 1e6,
            measured_at: now_iso8601(),
        },
    ) {
        debug!("Could not persist tuning cache: {e}");
    }

    Ok(geometry)
}

fn run_probe(
    _ctx: &Arc<CudaContext>,
    stream: &Arc<CudaStream>,
    func: &CudaFunction,
    mp_count: u32,
) -> Result<(Geometry, f64), EngineError> {
    let mut best: Option<(Geometry, f64)> = None;

    for &blocks_per_sm in PROBE_BLOCKS_PER_SM {
        for &threads_per_block in PROBE_THREADS_PER_BLOCK {
            let geom = Geometry {
                blocks_per_sm,
                threads_per_block,
            };
            let rate = measure_geometry(stream, func, mp_count, geom)
                .map_err(|e| EngineError::Other(format!("probe failed: {e:?}")))?;
            debug!(
                "Probe blocks_per_sm={} threads_per_block={} → {:.2} GH/s",
                blocks_per_sm,
                threads_per_block,
                rate / 1e9
            );
            match &best {
                None => best = Some((geom, rate)),
                Some((_, prev)) if rate > *prev => best = Some((geom, rate)),
                _ => {}
            }
        }
    }

    best.ok_or_else(|| EngineError::Other("no probe geometries succeeded".into()))
}

fn measure_geometry(
    stream: &Arc<CudaStream>,
    func: &CudaFunction,
    mp_count: u32,
    geom: Geometry,
) -> Result<f64, cudarc::driver::DriverError> {
    let total_blocks = mp_count * geom.blocks_per_sm;
    let total_threads = total_blocks * geom.threads_per_block;
    let n_per_thread = PROBE_N_COUNTERS.div_ceil(total_threads as u64).max(1);
    let actual_hashes = total_threads as u64 * n_per_thread;

    let pubkey_dev = stream.memcpy_stod(PROBE_PUBKEY.as_bytes())?;
    let mut g_best_level = stream.memcpy_stod(&[0u32])?;
    let mut g_best_counter = stream.memcpy_stod(&[0u64])?;

    let cfg = LaunchConfig {
        grid_dim: (total_blocks, 1, 1),
        block_dim: (geom.threads_per_block, 1, 1),
        shared_mem_bytes: 0,
    };
    let pubkey_len: u32 = PROBE_PUBKEY.len() as u32;
    let start_counter: u64 = 0;
    let current_best_level: u32 = 0;

    // Warm-up: an unmeasured launch absorbs JIT, autotuner, and first-
    // launch driver overhead so the timed pass is representative.
    {
        let mut launch = stream.launch_builder(func);
        launch.arg(&pubkey_dev);
        launch.arg(&pubkey_len);
        launch.arg(&start_counter);
        launch.arg(&n_per_thread);
        launch.arg(&current_best_level);
        launch.arg(&mut g_best_level);
        launch.arg(&mut g_best_counter);
        unsafe { launch.launch(cfg) }?;
        stream.synchronize()?;
    }

    let started = Instant::now();
    {
        let mut launch = stream.launch_builder(func);
        launch.arg(&pubkey_dev);
        launch.arg(&pubkey_len);
        launch.arg(&start_counter);
        launch.arg(&n_per_thread);
        launch.arg(&current_best_level);
        launch.arg(&mut g_best_level);
        launch.arg(&mut g_best_counter);
        unsafe { launch.launch(cfg) }?;
        stream.synchronize()?;
    }
    let elapsed = started.elapsed().as_secs_f64().max(1e-9);

    Ok(actual_hashes as f64 / elapsed)
}

fn now_iso8601() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Minimal ISO 8601 to keep dep weight off; this is only for the
    // cache's "when did I measure this" diagnostic field.
    format!("seconds-since-unix-epoch:{dur}")
}
