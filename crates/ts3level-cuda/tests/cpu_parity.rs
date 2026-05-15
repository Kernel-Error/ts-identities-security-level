//! Verify that the CUDA kernel computes exactly the same level as the
//! CPU reference implementation in `ts3level-core`.
//!
//! Runs only when a CUDA device is actually present. On a CI machine
//! without a GPU the test bails out quietly with a printed note.

use ts3level_core::level::compute_level;
use ts3level_cuda::CudaEngine;
use ts3level_engine::{HashEngine, LaunchParams};

const PUBKEY_B64: &str =
    "MEwDAgcAAgEgAiEA5jUbcc+RXAJzVKLpyEnoq/Otht1JBeCdRgRJYYBuOmoCIQDwBoRP+rkICZHbAGD9XYpV9bm08yPYGT4LehKXmlYZJg==";

fn engine_or_skip() -> Option<CudaEngine> {
    let mut e = CudaEngine::new();
    match e.enumerate() {
        Ok(v) if !v.is_empty() => {
            e.select_device(0).ok()?;
            Some(e)
        }
        _ => {
            eprintln!("note: no CUDA device available — skipping parity test");
            None
        }
    }
}

#[test]
fn cuda_matches_cpu_for_first_million_counters() {
    let Some(mut engine) = engine_or_skip() else {
        return;
    };

    // Window of 1M counters. We expect at least one level >= 6
    // (probability per attempt: 1/64) with overwhelmingly high confidence.
    let params = LaunchParams {
        pubkey_b64: PUBKEY_B64.to_string(),
        start_counter: 0,
        n_counters: 1_000_000,
        current_best_level: 0,
    };
    let result = engine.launch(&params).unwrap();

    // If anything was reported, it must verify on CPU.
    if result.best_level > 0 {
        let verified = compute_level(PUBKEY_B64, result.best_counter);
        assert_eq!(
            verified, result.best_level,
            "GPU reported level={} for counter={}, CPU verifies level={}",
            result.best_level, result.best_counter, verified
        );
        println!(
            "GPU best in 1M window: level={} at counter={}",
            result.best_level, result.best_counter
        );
    } else {
        // 1M counters with no level>=1 hit would be astronomically rare.
        // Treat that as a test failure: it almost certainly means the
        // kernel found a level <= current_best=0 but mis-encoded it.
        panic!("no levels reported in 1M-counter window — kernel may be broken");
    }
}

#[test]
fn cuda_finds_higher_level_when_extending_window() {
    let Some(mut engine) = engine_or_skip() else {
        return;
    };
    let mut best = 0u8;
    let mut start = 0u64;
    for _ in 0..8 {
        let params = LaunchParams {
            pubkey_b64: PUBKEY_B64.to_string(),
            start_counter: start,
            n_counters: 200_000,
            current_best_level: best,
        };
        let r = engine.launch(&params).unwrap();
        if r.best_level > best {
            // Sanity-check on CPU.
            let cpu = compute_level(PUBKEY_B64, r.best_counter);
            assert_eq!(
                cpu, r.best_level,
                "CPU mismatch at counter {}",
                r.best_counter
            );
            best = r.best_level;
        }
        start = start.saturating_add(r.hashes_performed);
        if best >= 8 {
            break;
        }
    }
    println!("Reached level {best} after sweeping {start} counters");
    assert!(
        best >= 4,
        "expected to reach at least level 4, only got {best}"
    );
}
