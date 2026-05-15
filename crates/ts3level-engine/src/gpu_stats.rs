//! Live GPU telemetry via NVML (`libnvidia-ml.so.1`).
//!
//! NVML ships with the NVIDIA driver, so there is no extra runtime
//! dependency. The wrapper crate loads it via `libloading` at startup.

use nvml_wrapper::enum_wrappers::device::TemperatureSensor;
use nvml_wrapper::Nvml;
use std::collections::VecDeque;
use tracing::debug;

/// Seconds of history kept for the utilization graph.
pub const HISTORY_LEN: usize = 60;

#[derive(Debug, Default, Clone, Copy)]
pub struct Sample {
    pub util_pct: Option<u32>,
    pub mem_used_mib: Option<u64>,
    pub mem_total_mib: Option<u64>,
    pub temp_c: Option<u32>,
    pub power_w: Option<f32>,
}

pub struct GpuStats {
    nvml: Option<Nvml>,
    device_index: u32,
    pub history: VecDeque<f32>,
}

impl GpuStats {
    pub fn new(device_index: u32) -> Self {
        let nvml = match Nvml::init() {
            Ok(n) => Some(n),
            Err(e) => {
                debug!("NVML init failed: {e}; GPU telemetry disabled");
                None
            }
        };
        Self {
            nvml,
            device_index,
            history: VecDeque::with_capacity(HISTORY_LEN),
        }
    }

    pub fn available(&self) -> bool {
        self.nvml.is_some()
    }

    pub fn set_device(&mut self, device_index: u32) {
        if self.device_index != device_index {
            self.device_index = device_index;
            self.history.clear();
        }
    }

    /// Take a single sample and append the utilization value to the
    /// history (or `0.0` if NVML didn't return one).
    pub fn poll(&mut self) -> Sample {
        let mut s = Sample::default();
        if let Some(nvml) = &self.nvml {
            if let Ok(dev) = nvml.device_by_index(self.device_index) {
                s.util_pct = dev.utilization_rates().ok().map(|u| u.gpu);
                if let Ok(mem) = dev.memory_info() {
                    s.mem_used_mib = Some(mem.used / (1024 * 1024));
                    s.mem_total_mib = Some(mem.total / (1024 * 1024));
                }
                s.temp_c = dev.temperature(TemperatureSensor::Gpu).ok();
                s.power_w = dev.power_usage().ok().map(|mw| mw as f32 / 1000.0);
            }
        }
        if self.history.len() >= HISTORY_LEN {
            self.history.pop_front();
        }
        self.history.push_back(s.util_pct.unwrap_or(0) as f32);
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Force NVML to be absent so the tests behave the same on machines
    /// without a working NVIDIA driver and in CI runners.
    fn no_nvml(device_index: u32) -> GpuStats {
        GpuStats {
            nvml: None,
            device_index,
            history: VecDeque::with_capacity(HISTORY_LEN),
        }
    }

    #[test]
    fn new_does_not_panic_when_nvml_unavailable() {
        // `GpuStats::new` must always return — worst case is that every
        // poll returns `Sample::default()`. No `unwrap`s on init.
        let _ = GpuStats::new(0);
    }

    #[test]
    fn poll_returns_default_sample_without_nvml() {
        let mut s = no_nvml(0);
        let sample = s.poll();
        assert!(sample.util_pct.is_none());
        assert!(sample.mem_used_mib.is_none());
        assert!(sample.mem_total_mib.is_none());
        assert!(sample.temp_c.is_none());
        assert!(sample.power_w.is_none());
    }

    #[test]
    fn history_grows_then_rotates_at_capacity() {
        let mut s = no_nvml(0);
        for _ in 0..HISTORY_LEN {
            s.poll();
        }
        assert_eq!(s.history.len(), HISTORY_LEN);
        for _ in 0..5 {
            s.poll();
        }
        assert_eq!(s.history.len(), HISTORY_LEN);
    }

    #[test]
    fn history_records_zero_when_nvml_returns_nothing() {
        let mut s = no_nvml(0);
        for _ in 0..3 {
            s.poll();
        }
        assert_eq!(s.history.len(), 3);
        for v in &s.history {
            assert_eq!(*v, 0.0);
        }
    }

    #[test]
    fn set_device_clears_history_only_on_change() {
        let mut s = no_nvml(0);
        for _ in 0..10 {
            s.poll();
        }
        assert_eq!(s.history.len(), 10);

        // Same device: history must be preserved.
        s.set_device(0);
        assert_eq!(s.history.len(), 10);

        // Different device: history must be flushed.
        s.set_device(1);
        assert_eq!(s.history.len(), 0);
        assert_eq!(s.device_index, 1);
    }

    #[test]
    fn available_reflects_nvml_state() {
        let s = no_nvml(0);
        assert!(!s.available());
        // `GpuStats::new` may or may not be available depending on the
        // machine — we just verify the accessor does not panic.
        let _ = GpuStats::new(0).available();
    }
}
