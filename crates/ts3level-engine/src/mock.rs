//! CPU-only test backend that exercises the driver loop without a GPU.
//!
//! The mock runs the reference CPU implementation over the requested
//! window and reports the highest natural level it finds — never lies.
//! Tests rely on small target levels (2..6) that statistically appear in
//! a single batch of a few hundred counters.

use crate::device::DeviceInfo;
use crate::engine::{EngineError, HashEngine, LaunchParams, LaunchResult};
use std::sync::{Arc, Mutex};
use ts3level_core::level::compute_level;

pub struct MockEngine {
    devices: Vec<DeviceInfo>,
    bound_device: Option<u32>,
    total_hashes: Arc<Mutex<u64>>,
}

impl MockEngine {
    pub fn new() -> Self {
        Self {
            devices: vec![DeviceInfo {
                kind: "MOCK",
                index: 0,
                name: "MockEngine-0".to_string(),
                compute_capability: (0, 0),
                total_memory_bytes: 0,
                multiprocessor_count: 0,
            }],
            bound_device: None,
            total_hashes: Arc::new(Mutex::new(0)),
        }
    }

    pub fn total_hashes(&self) -> u64 {
        *self.total_hashes.lock().unwrap()
    }
}

impl Default for MockEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl HashEngine for MockEngine {
    fn kind(&self) -> &'static str {
        "MOCK"
    }

    fn enumerate(&self) -> Result<Vec<DeviceInfo>, EngineError> {
        Ok(self.devices.clone())
    }

    fn select_device(&mut self, index: u32) -> Result<(), EngineError> {
        if index >= self.devices.len() as u32 {
            return Err(EngineError::DeviceIndexOutOfRange {
                requested: index,
                available: self.devices.len() as u32,
            });
        }
        self.bound_device = Some(index);
        Ok(())
    }

    fn launch(&mut self, params: &LaunchParams) -> Result<LaunchResult, EngineError> {
        if self.bound_device.is_none() {
            return Err(EngineError::Other("no device selected".into()));
        }

        let mut best_level = 0u8;
        let mut best_counter = params.start_counter;

        for offset in 0..params.n_counters {
            let counter = params.start_counter.wrapping_add(offset);
            let lvl = compute_level(&params.pubkey_b64, counter);
            if lvl > best_level && lvl > params.current_best_level {
                best_level = lvl;
                best_counter = counter;
            }
        }

        *self.total_hashes.lock().unwrap() += params.n_counters;
        Ok(LaunchResult {
            best_level,
            best_counter,
            hashes_performed: params.n_counters,
        })
    }
}
