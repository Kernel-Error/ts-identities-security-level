use crate::device::DeviceInfo;
use std::path::PathBuf;
use thiserror::Error;

/// Parameters for one kernel launch.
#[derive(Debug, Clone)]
pub struct LaunchParams {
    /// Base64 of the public-only ASN.1 DER. The kernel hashes
    /// `pubkey_b64 || decimal(counter)` for each `counter` in the launch.
    pub pubkey_b64: String,

    /// First counter value this launch will try.
    pub start_counter: u64,

    /// Number of consecutive counters to test.
    pub n_counters: u64,

    /// The kernel only reports counters whose level strictly exceeds this
    /// value, so the host can suppress duplicate "found" events for the
    /// already-known best.
    pub current_best_level: u8,
}

/// Result of one kernel launch.
#[derive(Debug, Clone, Copy)]
pub struct LaunchResult {
    /// Highest level observed in this launch. `0` means "nothing beat the
    /// current best".
    pub best_level: u8,

    /// Counter that produced `best_level` (if `best_level > 0`, otherwise
    /// undefined and the host ignores it).
    pub best_counter: u64,

    /// Number of `(pubkey, counter)` SHA-1 hashes actually performed.
    pub hashes_performed: u64,
}

/// Errors that can come from a backend.
#[derive(Debug, Error)]
pub enum EngineError {
    #[error("required driver/library not found: {0}")]
    DriverMissing(String),

    #[error("no compute device found")]
    NoDevice,

    #[error("device index {requested} out of range (have {available})")]
    DeviceIndexOutOfRange { requested: u32, available: u32 },

    #[error("permission denied for device file {path:?}: {reason}")]
    DevicePermission { path: PathBuf, reason: String },

    #[error("backend error: {0}")]
    Other(String),
}

/// A GPU hashing backend. Implementations live in their own crate
/// (`ts3level-cuda`, …).
pub trait HashEngine: Send {
    /// Stable identifier of the backend (`"CUDA"`, `"OpenCL"`, …).
    fn kind(&self) -> &'static str;

    /// All devices the backend can see right now. May be empty.
    fn enumerate(&self) -> Result<Vec<DeviceInfo>, EngineError>;

    /// Bind to a specific device (by `DeviceInfo::index`). Must be called
    /// before [`launch`](Self::launch).
    fn select_device(&mut self, index: u32) -> Result<(), EngineError>;

    /// Run one batch. The number of hashes actually performed may be less
    /// than `params.n_counters` if the engine internally rounds the batch
    /// size to its work-group geometry.
    fn launch(&mut self, params: &LaunchParams) -> Result<LaunchResult, EngineError>;
}
