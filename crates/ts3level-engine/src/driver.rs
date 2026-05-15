//! Main loop. Issues launches, listens for new bests, writes the file
//! atomically each time a new maximum is found, and stops on either a
//! reached target or a flipped stop flag.

use crate::engine::{HashEngine, LaunchParams};
use crate::progress::{DoneReason, Progress};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Instant;
use thiserror::Error;
use ts3level_core::pubkey::KeyPair;
use ts3level_core::writer::write_back;
use ts3level_core::IdentityFile;

/// Stop condition.
#[derive(Debug, Clone, Copy)]
pub enum StopMode {
    /// Run forever, writing the file each time a higher level is found.
    Endless,
    /// Stop when level `>= target` is reached.
    Target(u8),
}

#[derive(Debug, Error)]
pub enum DriverError {
    #[error("engine: {0}")]
    Engine(#[from] crate::engine::EngineError),

    #[error("core: {0}")]
    Core(#[from] ts3level_core::Error),
}

pub struct Driver {
    engine: Box<dyn HashEngine>,
    file_path: PathBuf,
    identity: IdentityFile,
    pubkey_b64: String,
    stop_mode: StopMode,
    stop_flag: Arc<AtomicBool>,
    /// Counters per launch. The CUDA backend picks its own grid geometry;
    /// this is the *requested* size and the actual hashes performed is
    /// reported back per [`crate::LaunchResult`].
    batch_size: u64,
}

impl Driver {
    pub fn new(
        engine: Box<dyn HashEngine>,
        file_path: PathBuf,
        identity: IdentityFile,
        stop_mode: StopMode,
    ) -> Result<Self, DriverError> {
        let kp = KeyPair::from_blob_b64(identity.blob_b64())?;
        Ok(Self {
            engine,
            file_path,
            pubkey_b64: kp.public_key_base64(),
            identity,
            stop_mode,
            stop_flag: Arc::new(AtomicBool::new(false)),
            batch_size: 1 << 24, // 16M counters per batch; tuned later
        })
    }

    /// Override the requested batch size. Tests use small values; the CLI
    /// can expose a tuning flag.
    pub fn set_batch_size(&mut self, n: u64) {
        self.batch_size = n.max(1);
    }

    /// Returns a handle the frontend can flip to stop the loop. The driver
    /// checks it between launches.
    pub fn stop_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.stop_flag)
    }

    /// Run until target reached, stop_flag flipped, or an error occurs.
    pub fn run(&mut self, progress: Sender<Progress>) -> Result<(), DriverError> {
        let start = Instant::now();
        let mut total_hashes: u64 = 0;
        let current_counter = self.identity.counter();
        let mut best_level = ts3level_core::level::compute_level(&self.pubkey_b64, current_counter);
        let mut best_counter = current_counter;
        let mut hashrate_ema: Option<f64> = None;

        // Start search at counter + 1. The current counter is already
        // committed; we want to *beat* it, so we begin from the next.
        let mut next_counter: u64 = current_counter.saturating_add(1);

        loop {
            if self.stop_flag.load(Ordering::SeqCst) {
                self.emit_done(&progress, DoneReason::Stopped, best_level, best_counter);
                return Ok(());
            }

            if let StopMode::Target(t) = self.stop_mode {
                if best_level >= t {
                    self.emit_done(
                        &progress,
                        DoneReason::TargetReached,
                        best_level,
                        best_counter,
                    );
                    return Ok(());
                }
            }

            let params = LaunchParams {
                pubkey_b64: self.pubkey_b64.clone(),
                start_counter: next_counter,
                n_counters: self.batch_size,
                current_best_level: best_level,
            };

            let launch_started = Instant::now();
            let res = self.engine.launch(&params)?;
            let launch_elapsed = launch_started.elapsed().as_secs_f64().max(1e-9);

            total_hashes = total_hashes.saturating_add(res.hashes_performed);
            let instant_rate = res.hashes_performed as f64 / launch_elapsed;
            hashrate_ema = Some(match hashrate_ema {
                None => instant_rate,
                Some(prev) => 0.8 * prev + 0.2 * instant_rate,
            });

            if res.best_level > best_level {
                // CPU-reverify the counter the backend reported. The
                // backend may have raced on its two atomic slots and
                // returned a counter whose level is lower (but still
                // higher than `best_level`) than what it claimed —
                // we trust the CPU's answer. Accept only if it
                // actually beats our current best.
                let verified =
                    ts3level_core::level::compute_level(&self.pubkey_b64, res.best_counter);
                if verified > best_level {
                    best_level = verified;
                    best_counter = res.best_counter;
                    self.identity.set_counter(best_counter);
                    write_back(&self.file_path, &self.identity)?;
                    let _ = progress.send(Progress::NewBest {
                        level: best_level,
                        counter: best_counter,
                    });
                }
                // else: the GPU result didn't survive CPU verification.
                // This is the benign race in the kernel's two-slot
                // atomic pair under contention, not a correctness bug —
                // skip this batch's "best" claim and let the next
                // launch try again.
            }

            let rate = hashrate_ema.unwrap_or(0.0);
            let (eta_next, eta_target) = compute_eta(rate, best_level, self.stop_mode);
            let _ = progress.send(Progress::Tick {
                hashrate_hps: rate,
                total_hashes,
                best_level,
                best_counter,
                elapsed_secs: start.elapsed().as_secs_f64(),
                eta_next_level_secs: eta_next,
                eta_target_secs: eta_target,
            });

            next_counter = next_counter.saturating_add(res.hashes_performed.max(1));
            // Saturated to u64::MAX is improbable but possible for very
            // long-running endless mode; stop sanely instead of looping.
            if next_counter == u64::MAX {
                self.emit_done(&progress, DoneReason::Stopped, best_level, best_counter);
                return Ok(());
            }
        }
    }

    fn emit_done(
        &self,
        progress: &Sender<Progress>,
        reason: DoneReason,
        final_level: u8,
        final_counter: u64,
    ) {
        let _ = progress.send(Progress::Done {
            reason,
            final_level,
            final_counter,
        });
    }
}

/// Expected wall-clock time to find a counter of level `≥ L` given a
/// constant hashrate of `rate` hashes/s. Probability per attempt is
/// `1 / 2^L`, mean = `2^L` attempts.
fn time_to_level(rate: f64, level: u8) -> f64 {
    if rate <= 0.0 || level > 80 {
        return f64::INFINITY;
    }
    let attempts = (1u128 << level.min(80)) as f64;
    attempts / rate
}

fn compute_eta(rate: f64, best_level: u8, mode: StopMode) -> (Option<f64>, Option<f64>) {
    if rate <= 0.0 {
        return (None, None);
    }
    let eta_next = if best_level < 80 {
        Some(time_to_level(rate, best_level.saturating_add(1)))
    } else {
        None
    };
    let eta_target = match mode {
        StopMode::Target(t) if t > best_level => Some(time_to_level(rate, t)),
        StopMode::Target(_) => Some(0.0),
        StopMode::Endless => None,
    };
    (eta_next, eta_target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockEngine;
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine as _;
    use std::sync::mpsc;
    use ts3level_core::deobfuscate::obfuscate;

    #[test]
    fn eta_calculation_endless_mode() {
        let (next, target) = compute_eta(1_000_000_000.0, 20, StopMode::Endless);
        // 2^21 / 1e9 ≈ 2.1 ms
        assert!(next.unwrap() > 0.0 && next.unwrap() < 0.01);
        assert!(target.is_none());
    }

    #[test]
    fn eta_calculation_target_mode() {
        let (next, target) = compute_eta(1_000_000_000.0, 20, StopMode::Target(40));
        // 2^21 / 1e9
        assert!(next.unwrap() > 0.0 && next.unwrap() < 0.01);
        // 2^40 / 1e9 = 1099 s
        let t = target.unwrap();
        assert!(t > 1000.0 && t < 1200.0, "got {t}");
    }

    #[test]
    fn eta_zero_when_target_reached() {
        let (_, target) = compute_eta(1e9, 50, StopMode::Target(40));
        assert_eq!(target, Some(0.0));
    }

    #[test]
    fn eta_none_when_no_hashrate() {
        let (next, target) = compute_eta(0.0, 20, StopMode::Target(30));
        assert!(next.is_none() && target.is_none());
    }

    fn make_test_ini(counter: u64) -> (tempfile::TempDir, PathBuf) {
        // Build a synthetic obfuscated blob from a fixed plaintext DER.
        // We reuse exactly the construction from the core's known_vector
        // integration test so the public-key extraction succeeds.
        let mut der = vec![0x30, 75];
        der.extend_from_slice(&[0x03, 0x02, 0x07, 0x80]);
        der.extend_from_slice(&[0x02, 0x01, 0x20]);
        // x = 32 bytes
        der.push(0x02);
        der.push(32);
        der.extend(std::iter::repeat_n(0x01u8, 32));
        // y = 32 bytes
        der.push(0x02);
        der.push(32);
        der.extend(std::iter::repeat_n(0x02u8, 32));
        // k = 32 bytes (private)
        der.push(0x02);
        der.push(32);
        der.extend(std::iter::repeat_n(0x03u8, 32));
        // Fix the SEQUENCE length to the actual body length.
        der[1] = (der.len() - 2) as u8;

        let inner = B64.encode(&der);
        let mut padded = inner.into_bytes();
        while padded.len() < 240 {
            padded.push(0);
        }
        obfuscate(&mut padded);
        let blob_b64 = B64.encode(&padded);

        let ini = format!("[Identity]\nidentity=\"{counter}V{blob_b64}\"\n");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.ini");
        std::fs::write(&path, ini).unwrap();
        (dir, path)
    }

    #[test]
    fn stops_when_target_reached() {
        // Use target=2 — for random pubkey input, P(level >= 2) = 1/4 per
        // counter, so any batch of 100 counters is essentially guaranteed
        // to find at least one such hit.
        let (_dir, path) = make_test_ini(0);
        let identity = IdentityFile::parse(&std::fs::read(&path).unwrap()).unwrap();

        let mut engine = Box::new(MockEngine::new());
        engine.select_device(0).unwrap();

        let mut driver = Driver::new(engine, path.clone(), identity, StopMode::Target(2)).unwrap();
        driver.set_batch_size(200);

        let (tx, rx) = mpsc::channel();
        driver.run(tx).unwrap();

        let events: Vec<_> = rx.iter().collect();
        let new_best = events
            .iter()
            .find(|e| matches!(e, Progress::NewBest { .. }));
        assert!(
            new_best.is_some(),
            "expected at least one NewBest, got {events:?}"
        );

        let done = events.iter().rev().find_map(|e| match e {
            Progress::Done {
                reason,
                final_level,
                ..
            } => Some((reason, final_level)),
            _ => None,
        });
        let (reason, final_level) = done.expect("no Done event");
        assert!(matches!(reason, DoneReason::TargetReached), "{events:?}");
        assert!(*final_level >= 2, "final_level={final_level} not >= 2");

        // File on disk reflects the new counter.
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(
            !after.contains("identity=\"0V"),
            "counter not updated: {after}"
        );
    }

    #[test]
    fn stops_on_stop_flag_in_endless_mode() {
        let (_dir, path) = make_test_ini(0);
        let identity = IdentityFile::parse(&std::fs::read(&path).unwrap()).unwrap();
        let mut engine = Box::new(MockEngine::new());
        engine.select_device(0).unwrap();
        let mut driver = Driver::new(engine, path.clone(), identity, StopMode::Endless).unwrap();
        driver.set_batch_size(10);
        let stop = driver.stop_handle();
        stop.store(true, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        driver.run(tx).unwrap();
        let events: Vec<_> = rx.iter().collect();
        let done = events.iter().rev().find_map(|e| match e {
            Progress::Done { reason, .. } => Some(reason),
            _ => None,
        });
        assert!(matches!(done, Some(DoneReason::Stopped)));
    }

    #[test]
    fn writes_bak_on_first_update() {
        let (_dir, path) = make_test_ini(0);
        let identity = IdentityFile::parse(&std::fs::read(&path).unwrap()).unwrap();
        let mut engine = Box::new(MockEngine::new());
        engine.select_device(0).unwrap();
        let mut driver = Driver::new(engine, path.clone(), identity, StopMode::Target(2)).unwrap();
        driver.set_batch_size(200);
        let (tx, _rx) = mpsc::channel();
        driver.run(tx).unwrap();

        let bak = ts3level_core::writer::backup_path(&path);
        assert!(bak.exists(), "backup not created");
        let bak_content = std::fs::read_to_string(&bak).unwrap();
        assert!(
            bak_content.contains("identity=\"0V"),
            "backup not pristine: {bak_content}"
        );
    }
}
