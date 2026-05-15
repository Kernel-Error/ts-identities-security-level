//! Engine glue between the parser/writer in `ts3level-core` and a GPU
//! backend.
//!
//! - [`HashEngine`] is the trait every backend implements. The CUDA backend
//!   lives in `ts3level-cuda`; tests use [`MockEngine`].
//! - [`preflight`] runs all checks the user expects before any hashing
//!   starts: file existence/perm/lock, identity structure, disk space,
//!   then delegates to the engine for driver/device checks.
//! - [`Driver`] is the main loop that issues launches, polls for new
//!   maxima, writes the file back atomically, and stops on either a
//!   reached `--target` level or an external stop signal.

pub mod device;
pub mod driver;
pub mod engine;
pub mod gpu_stats;
pub mod mock;
pub mod preflight;
pub mod progress;

pub use device::DeviceInfo;
pub use driver::{Driver, StopMode};
pub use engine::{EngineError, HashEngine, LaunchParams, LaunchResult};
pub use gpu_stats::{GpuStats, Sample as GpuSample};
pub use mock::MockEngine;
pub use preflight::{run_preflight, PreflightError, PreflightReport};
pub use progress::Progress;
