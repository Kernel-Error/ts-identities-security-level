//! Headless CLI for raising the security level of a TeamSpeak 3 identity.

use anyhow::{Context, Result};
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use ts3level_core::pubkey::KeyPair;
use ts3level_cuda::CudaEngine;
use ts3level_engine::progress::DoneReason;
use ts3level_engine::{
    run_preflight, Driver, GpuSample, GpuStats, HashEngine, PreflightError, Progress, StopMode,
};

mod i18n;

/// Exit codes — stable for scripting.
mod exit {
    pub const OK: i32 = 0;
    pub const USAGE: i32 = 2;
    pub const DRIVER_MISSING: i32 = 10;
    pub const NO_DEVICE: i32 = 11;
    pub const DEVICE_PERMISSION: i32 = 12;
    pub const FILE_NOT_FOUND: i32 = 20;
    pub const FILE_PERMISSION: i32 = 21;
    pub const FILE_LOCKED: i32 = 22;
    pub const FILE_INVALID: i32 = 23;
    pub const DISK_FULL: i32 = 24;
    pub const RUNTIME_ERROR: i32 = 30;
}

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Raise the security level of a TeamSpeak 3 identity on a GPU.",
    long_about = None,
)]
struct Cli {
    /// Path to the `.ini` identity file. The file is updated in place each
    /// time a higher level is found; a one-shot `.bak` of the original is
    /// kept alongside it. Required unless `--list-devices` is set.
    #[arg(required_unless_present = "list_devices")]
    identity: Option<PathBuf>,

    /// Stop after reaching the given level. Omit to run forever.
    #[arg(short = 't', long = "target")]
    target: Option<u8>,

    /// CUDA device index. If omitted, the first device is used; pass
    /// `--list-devices` to see what's available.
    #[arg(short = 'd', long = "device")]
    device: Option<u32>,

    /// Print the available CUDA devices and exit.
    #[arg(long = "list-devices")]
    list_devices: bool,

    /// Counters per kernel launch. Higher → less per-batch overhead, but
    /// slower response to Ctrl+C. Default 16M.
    #[arg(long = "batch-size", default_value_t = 1u64 << 24)]
    batch_size: u64,

    /// Don't poll GPU telemetry. Slightly less terminal noise; useful if
    /// NVML is misbehaving on your system.
    #[arg(long = "no-gpu-stats")]
    no_gpu_stats: bool,
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    i18n::init();

    let cli = Cli::parse();
    let code = match run(&cli) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e:#}");
            exit::RUNTIME_ERROR
        }
    };
    std::process::exit(code);
}

fn run(cli: &Cli) -> Result<i32> {
    let mut engine = CudaEngine::new();

    if cli.list_devices {
        return list_devices(&engine);
    }

    let identity_path = cli.identity.as_ref().expect("required_unless_present guard");

    // Preflight: file checks + device enumeration.
    let report = match run_preflight(identity_path, &engine) {
        Ok(r) => r,
        Err(e) => return Ok(report_preflight_error(e)),
    };

    let device_index = cli.device.unwrap_or(0);
    if let Some(dev) = report.devices.get(device_index as usize) {
        eprintln!("{} {}", i18n::tr("Using device:"), dev.summary());
    } else {
        eprintln!(
            "{} {} ({} {})",
            i18n::tr("Device index out of range:"),
            device_index,
            report.devices.len(),
            i18n::tr("device(s) available"),
        );
        return Ok(exit::USAGE);
    }
    engine.select_device(device_index)?;

    // Extract identity details for an up-front summary so the user can
    // see (and verify the fingerprint of) the file they're about to grind.
    let kp = KeyPair::from_blob_b64(report.identity.blob_b64())?;
    let pubkey_b64 = kp.public_key_base64();
    let current_level =
        ts3level_core::level::compute_level(&pubkey_b64, report.current_counter);

    print_identity_summary(&report.identity, &kp, current_level, report.current_counter);

    // Target auto-bump: if the requested target is already at or below
    // the current level, raise it to current+1 so we actually do work.
    let stop_mode = match cli.target {
        Some(t) if t <= current_level => {
            let bumped = current_level.saturating_add(1);
            eprintln!(
                "{} {} ≤ {} ({}): {} {}",
                i18n::tr("note: target"),
                t,
                current_level,
                i18n::tr("current level"),
                i18n::tr("raising to"),
                bumped,
            );
            StopMode::Target(bumped)
        }
        Some(t) => StopMode::Target(t),
        None => StopMode::Endless,
    };
    eprintln!(
        "{}: {}",
        i18n::tr("Target"),
        match stop_mode {
            StopMode::Endless => i18n::tr("endless"),
            StopMode::Target(t) => t.to_string(),
        }
    );

    // GPU telemetry (NVML). Polled once per progress tick; if NVML is
    // unavailable the field is just skipped — never an error.
    let gpu_stats = if cli.no_gpu_stats {
        None
    } else {
        let stats = GpuStats::new(device_index);
        if stats.available() {
            Some(Arc::new(Mutex::new(stats)))
        } else {
            None
        }
    };

    let mut driver = Driver::new(
        Box::new(engine),
        identity_path.clone(),
        report.identity,
        stop_mode,
    )
    .context("constructing driver")?;
    driver.set_batch_size(cli.batch_size);

    let stop = driver.stop_handle();
    ctrlc::set_handler(move || stop.store(true, Ordering::SeqCst))
        .context("installing Ctrl+C handler")?;

    let (tx, rx) = mpsc::channel();
    let gpu_for_progress = gpu_stats.clone();
    let progress_handle = std::thread::spawn(move || progress_loop(rx, gpu_for_progress));

    let outcome = driver.run(tx);
    progress_handle.join().ok();

    match outcome {
        Ok(()) => Ok(exit::OK),
        Err(e) => {
            eprintln!("error: {e}");
            Ok(exit::RUNTIME_ERROR)
        }
    }
}

fn progress_loop(
    rx: mpsc::Receiver<Progress>,
    gpu_stats: Option<Arc<Mutex<GpuStats>>>,
) {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("{spinner} {msg}")
            .unwrap()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ "),
    );
    pb.enable_steady_tick(std::time::Duration::from_millis(120));

    for event in rx {
        match event {
            Progress::Tick {
                hashrate_hps,
                total_hashes: _,
                best_level,
                best_counter,
                elapsed_secs,
                eta_next_level_secs,
                eta_target_secs,
            } => {
                let next_eta = eta_next_level_secs
                    .map(format_duration)
                    .unwrap_or_else(|| "—".into());
                let target_eta = eta_target_secs
                    .map(format_duration)
                    .map(|s| format!("  {}→{}: {}", i18n::tr("ETA"), i18n::tr("target"), s))
                    .unwrap_or_default();
                let gpu_seg = gpu_segment(gpu_stats.as_ref());
                pb.set_message(format!(
                    "{}: {}  {}: {}  {:.2} GH/s  {}→+1: {}{}{}  ({:.0}s)",
                    i18n::tr("level"),
                    best_level,
                    i18n::tr("counter"),
                    best_counter,
                    hashrate_hps / 1e9,
                    i18n::tr("ETA"),
                    next_eta,
                    target_eta,
                    gpu_seg,
                    elapsed_secs,
                ));
            }
            Progress::NewBest { level, counter } => {
                pb.println(format!("✓ {} {} ({}={})", i18n::tr("new level"), level, i18n::tr("counter"), counter));
            }
            Progress::Done { reason, final_level, final_counter } => {
                let r = match reason {
                    DoneReason::TargetReached => i18n::tr("target reached"),
                    DoneReason::Stopped => i18n::tr("stopped"),
                    DoneReason::Error(e) => format!("{}: {}", i18n::tr("error"), e),
                };
                pb.finish_with_message(format!(
                    "{}. {} {}, {} {}.",
                    r,
                    i18n::tr("final level"),
                    final_level,
                    i18n::tr("counter"),
                    final_counter,
                ));
                break;
            }
        }
    }
}

fn list_devices(engine: &CudaEngine) -> Result<i32> {
    match engine.enumerate() {
        Ok(devs) if devs.is_empty() => {
            eprintln!("{}", i18n::tr("No CUDA-capable device found."));
            Ok(exit::NO_DEVICE)
        }
        Ok(devs) => {
            for d in devs {
                println!("{}", d.summary());
            }
            Ok(exit::OK)
        }
        Err(e) => {
            eprintln!("{}: {e}", i18n::tr("Failed to enumerate CUDA devices"));
            Ok(exit::DRIVER_MISSING)
        }
    }
}

fn report_preflight_error(e: PreflightError) -> i32 {
    eprintln!("error: {e}");
    match e {
        PreflightError::DriverMissing(_) => exit::DRIVER_MISSING,
        PreflightError::NoDevice { .. } => exit::NO_DEVICE,
        PreflightError::DevicePermission { .. } => exit::DEVICE_PERMISSION,
        PreflightError::NotFound { .. } => exit::FILE_NOT_FOUND,
        PreflightError::NotReadable { .. }
        | PreflightError::NotWritable { .. }
        | PreflightError::ParentNotWritable { .. } => exit::FILE_PERMISSION,
        PreflightError::Locked { .. } => exit::FILE_LOCKED,
        PreflightError::InvalidFile { .. } => exit::FILE_INVALID,
        PreflightError::InsufficientSpace { .. } => exit::DISK_FULL,
        PreflightError::BackendOther(_) => exit::RUNTIME_ERROR,
    }
}

fn print_identity_summary(
    id: &ts3level_core::IdentityFile,
    kp: &KeyPair,
    level: u8,
    counter: u64,
) {
    eprintln!();
    eprintln!(
        "{}: {}",
        i18n::tr("Nickname"),
        id.nickname().unwrap_or("—")
    );
    eprintln!(
        "{}: {}",
        i18n::tr("Local ID"),
        id.local_id().unwrap_or("—")
    );
    eprintln!("{}: {}", i18n::tr("Fingerprint"), kp.fingerprint_b64());
    eprintln!(
        "{}: {}  ({}: {})",
        i18n::tr("Current level"),
        level,
        i18n::tr("counter"),
        counter,
    );
    eprintln!();
}

/// Poll NVML once and produce a compact `  GPU: 100% 76°C 165W` segment.
/// Returns an empty string if NVML is unavailable or the sample is empty.
fn gpu_segment(stats: Option<&Arc<Mutex<GpuStats>>>) -> String {
    let Some(stats) = stats else { return String::new() };
    let mut guard = match stats.lock() {
        Ok(g) => g,
        Err(_) => return String::new(),
    };
    let sample: GpuSample = guard.poll();
    drop(guard);
    let mut bits = Vec::new();
    if let Some(u) = sample.util_pct {
        bits.push(format!("{u}%"));
    }
    if let Some(t) = sample.temp_c {
        bits.push(format!("{t}°C"));
    }
    if let Some(p) = sample.power_w {
        bits.push(format!("{p:.0}W"));
    }
    if bits.is_empty() {
        String::new()
    } else {
        format!("  GPU: {}", bits.join(" "))
    }
}

fn format_duration(secs: f64) -> String {
    if !secs.is_finite() {
        return "∞".into();
    }
    if secs < 1.0 {
        return format!("{:.0}ms", secs * 1000.0);
    }
    if secs < 60.0 {
        return format!("{secs:.1}s");
    }
    let m = secs / 60.0;
    if m < 60.0 {
        return format!("{m:.1}m");
    }
    let h = m / 60.0;
    if h < 24.0 {
        return format!("{h:.1}h");
    }
    let d = h / 24.0;
    if d < 365.0 {
        return format!("{d:.1}d");
    }
    format!("{:.1}y", d / 365.0)
}
