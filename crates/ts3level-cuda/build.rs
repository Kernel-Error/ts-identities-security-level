//! Compile the CUDA SHA-1 kernel to a fatbin via `nvcc` and embed it
//! into the crate. End users do **not** need the CUDA Toolkit — only the
//! NVIDIA driver (which provides `libcuda.so.1`).
//!
//! Build dependencies:
//!   - `nvcc` (CUDA Toolkit ≥ 12.0 recommended)
//!
//! Override CUDA location via the `CUDA_PATH` env var (Toolkit-style:
//! contains `bin/nvcc`).

use std::env;
use std::path::PathBuf;
use std::process::Command;

const KERNEL_SOURCE: &str = "kernels/sha1_hasher.cu";

/// Architectures we compile native SASS for. PTX-only `compute_90` is
/// also embedded so future architectures get JIT compilation at load.
/// Each entry pairs an `arch=` virtual architecture with a `code=` real
/// (or PTX-fallback) one.
const TARGETS: &[(&str, &str)] = &[
    ("compute_70", "sm_70"),
    ("compute_75", "sm_75"),
    ("compute_80", "sm_80"),
    ("compute_86", "sm_86"),
    ("compute_89", "sm_89"),
    ("compute_90", "sm_90"),
    ("compute_90", "compute_90"), // PTX fallback for future GPUs (JIT)
];

fn main() {
    println!("cargo:rerun-if-changed={KERNEL_SOURCE}");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CUDA_PATH");

    let nvcc = find_nvcc().unwrap_or_else(|e| {
        eprintln!("Failed to locate nvcc: {e}");
        eprintln!();
        eprintln!("This crate needs the CUDA Toolkit to build the GPU kernel.");
        eprintln!("Install it (e.g. `apt install nvidia-cuda-toolkit`) or set");
        eprintln!("the CUDA_PATH environment variable to a directory containing");
        eprintln!("`bin/nvcc`.");
        std::process::exit(1);
    });

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let fatbin = out_dir.join("sha1_hasher.fatbin");

    let mut cmd = Command::new(&nvcc);
    cmd.arg("--fatbin").arg("-O3").arg("--use_fast_math");
    for (arch, code) in TARGETS {
        cmd.arg("-gencode").arg(format!("arch={arch},code={code}"));
    }
    cmd.arg("-o").arg(&fatbin).arg(KERNEL_SOURCE);

    let status = cmd.status().expect("failed to spawn nvcc");
    if !status.success() {
        panic!("nvcc failed with status {status}");
    }

    println!("cargo:rustc-env=TS3LEVEL_FATBIN_PATH={}", fatbin.display());
}

fn find_nvcc() -> Result<PathBuf, String> {
    if let Ok(path) = env::var("CUDA_PATH") {
        let candidate = PathBuf::from(path).join("bin").join("nvcc");
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    let which = Command::new("which").arg("nvcc").output();
    if let Ok(out) = which {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            if !s.is_empty() {
                return Ok(PathBuf::from(s));
            }
        }
    }
    for candidate in [
        "/usr/local/cuda/bin/nvcc",
        "/usr/bin/nvcc",
        "/opt/cuda/bin/nvcc",
    ] {
        if std::path::Path::new(candidate).is_file() {
            return Ok(PathBuf::from(candidate));
        }
    }
    Err("nvcc not found on PATH, in CUDA_PATH, or in standard locations".into())
}
