//! Quick hashrate probe — not a strict pass/fail bench, just prints
//! what the bound device achieves so we can spot regressions.

use std::time::Instant;
use ts3level_cuda::CudaEngine;
use ts3level_engine::{HashEngine, LaunchParams};

const PUBKEY_B64: &str =
    "MEwDAgcAAgEgAiEA5jUbcc+RXAJzVKLpyEnoq/Otht1JBeCdRgRJYYBuOmoCIQDwBoRP+rkICZHbAGD9XYpV9bm08yPYGT4LehKXmlYZJg==";

/// True if there's a CUDA device file we can hand to cudarc. Without
/// this guard, cudarc panics on `enumerate()` when `libcuda.so.1` isn't
/// installed (typical on a GitHub Actions runner with CUDA Toolkit but
/// no driver). Match the pattern used in the CLI smoke tests.
fn cuda_available() -> bool {
    std::path::Path::new("/dev/nvidiactl").exists()
}

#[test]
fn measure_hashrate() {
    if !cuda_available() {
        eprintln!("no CUDA driver; skipping hashrate bench");
        return;
    }
    let mut engine = CudaEngine::new();
    let devs = match engine.enumerate() {
        Ok(v) if !v.is_empty() => v,
        _ => {
            eprintln!("no CUDA device; skipping");
            return;
        }
    };
    println!("device: {}", devs[0].summary());
    engine.select_device(0).unwrap();

    // Warmup
    let _ = engine
        .launch(&LaunchParams {
            pubkey_b64: PUBKEY_B64.to_string(),
            start_counter: 0,
            n_counters: 1_000_000,
            current_best_level: 0,
        })
        .unwrap();

    let n: u64 = 50_000_000;
    let start = Instant::now();
    let r = engine
        .launch(&LaunchParams {
            pubkey_b64: PUBKEY_B64.to_string(),
            start_counter: 1_000_000,
            n_counters: n,
            current_best_level: 0,
        })
        .unwrap();
    let dt = start.elapsed().as_secs_f64();
    let rate = r.hashes_performed as f64 / dt;
    println!(
        "hashed {} in {:.3}s = {:.2} MH/s (best level seen: {})",
        r.hashes_performed,
        dt,
        rate / 1e6,
        r.best_level
    );
}
