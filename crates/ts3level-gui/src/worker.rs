//! Worker thread: runs the driver, forwards `Progress` events to the GTK
//! main loop through an `async_channel`. The window owns the channel
//! sender; the worker owns the receiver to listen for commands (currently
//! none beyond Stop, which is signalled via `stop_flag`).

use crate::i18n::tr;
use async_channel::Sender as AsyncSender;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use ts3level_core::IdentityFile;
use ts3level_cuda::CudaEngine;
use ts3level_engine::progress::{DoneReason, Progress};
use ts3level_engine::{run_preflight, Driver, HashEngine, StopMode};

#[derive(Debug)]
pub enum WorkerCommand {
    Stop,
}

#[derive(Debug)]
pub enum WorkerEvent {
    Tick {
        hashrate_hps: f64,
        best_level: u8,
        best_counter: u64,
        eta_next_secs: Option<f64>,
        eta_target_secs: Option<f64>,
    },
    NewBest {
        level: u8,
        counter: u64,
    },
    Finished {
        reason: String,
    },
    Error {
        message: String,
    },
}

pub fn spawn_worker(
    path: PathBuf,
    device_index: u32,
    endless: bool,
    target: u8,
    out: AsyncSender<WorkerEvent>,
) -> (Sender<WorkerCommand>, Arc<AtomicBool>) {
    let (cmd_tx, _cmd_rx) = mpsc::channel::<WorkerCommand>();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop_flag);

    std::thread::spawn(move || {
        let result = run_one(&path, device_index, endless, target, &out, stop_clone);
        if let Err(e) = result {
            let _ = out.send_blocking(WorkerEvent::Error { message: e });
        }
    });

    (cmd_tx, stop_flag)
}

fn run_one(
    path: &std::path::Path,
    device_index: u32,
    endless: bool,
    target: u8,
    out: &AsyncSender<WorkerEvent>,
    stop_flag: Arc<AtomicBool>,
) -> Result<(), String> {
    let mut engine = CudaEngine::new();
    let _report = run_preflight(path, &engine).map_err(|e| e.to_string())?;
    engine
        .select_device(device_index)
        .map_err(|e| e.to_string())?;
    let identity = IdentityFile::parse(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    let mode = if endless {
        StopMode::Endless
    } else {
        StopMode::Target(target)
    };
    let mut driver = Driver::new(Box::new(engine), path.to_owned(), identity, mode)
        .map_err(|e| e.to_string())?;

    // Replace driver's internal stop_flag with ours so the GUI's Stop
    // button affects this run.
    let internal_flag = driver.stop_handle();
    // Bridge the external GUI flag to the internal one via a tiny thread.
    let bridge_flag = Arc::clone(&stop_flag);
    let internal_clone = Arc::clone(&internal_flag);
    std::thread::spawn(move || loop {
        if bridge_flag.load(std::sync::atomic::Ordering::SeqCst) {
            internal_clone.store(true, std::sync::atomic::Ordering::SeqCst);
            break;
        }
        if internal_clone.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    });

    let (tx, rx) = mpsc::channel();
    let driver_thread = std::thread::spawn(move || driver.run(tx));

    for event in rx {
        match event {
            Progress::Tick {
                hashrate_hps,
                best_level,
                best_counter,
                eta_next_level_secs,
                eta_target_secs,
                ..
            } => {
                let _ = out.send_blocking(WorkerEvent::Tick {
                    hashrate_hps,
                    best_level,
                    best_counter,
                    eta_next_secs: eta_next_level_secs,
                    eta_target_secs,
                });
            }
            Progress::NewBest { level, counter } => {
                let _ = out.send_blocking(WorkerEvent::NewBest { level, counter });
            }
            Progress::Done { reason, .. } => {
                let msg = match reason {
                    DoneReason::TargetReached => tr("Target reached"),
                    DoneReason::Stopped => tr("Stopped"),
                    DoneReason::Error(e) => format!("{}: {e}", tr("Error")),
                };
                let _ = out.send_blocking(WorkerEvent::Finished { reason: msg });
            }
        }
    }

    driver_thread
        .join()
        .map_err(|_| "driver thread panicked".to_string())?
        .map_err(|e| e.to_string())?;
    Ok(())
}
