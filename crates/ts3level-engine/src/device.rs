/// Backend-agnostic description of one compute device. The CUDA backend
/// fills `kind = "CUDA"`, a future OpenCL/Vulkan backend would fill the
/// same struct with its own values.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub kind: &'static str,
    pub index: u32,
    pub name: String,
    pub compute_capability: (u32, u32),
    pub total_memory_bytes: u64,
    pub multiprocessor_count: u32,
}

impl DeviceInfo {
    pub fn summary(&self) -> String {
        let mem_gib = self.total_memory_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
        format!(
            "[{}:{}] {} (cc {}.{}, {} SMs, {:.1} GiB)",
            self.kind,
            self.index,
            self.name,
            self.compute_capability.0,
            self.compute_capability.1,
            self.multiprocessor_count,
            mem_gib,
        )
    }
}
