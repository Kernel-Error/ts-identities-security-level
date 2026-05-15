//! CUDA implementation of [`ts3level_engine::HashEngine`].
//!
//! The kernel is built at compile time via `nvcc --fatbin`; the resulting
//! fatbin is embedded into the binary with `include_bytes!`. At runtime the
//! end user only needs the NVIDIA driver (`libcuda.so.1`); cudarc resolves
//! it dynamically.
//!
//! Enumeration uses the low-level driver API directly so we can list
//! devices without paying for a full `CudaContext` per device probe. The
//! actual hashing context is created lazily in [`HashEngine::select_device`].

use cudarc::driver::sys;
use cudarc::driver::{CudaContext, CudaFunction, CudaModule, CudaStream, LaunchConfig, PushKernelArg};
use cudarc::nvrtc::Ptx;
use std::io::Write;
use std::sync::Arc;
use tracing::debug;
use ts3level_engine::{DeviceInfo, EngineError, HashEngine, LaunchParams, LaunchResult};

mod enumerate;

const FATBIN: &[u8] = include_bytes!(env!("TS3LEVEL_FATBIN_PATH"));

/// Number of resident blocks per SM. Tuned empirically — 32 keeps the SHA-1
/// pipeline well-fed without spilling registers on Ampere/Ada.
const BLOCKS_PER_SM: u32 = 32;

/// Threads per block. 256 is the standard sweet spot for SHA-1-shaped
/// kernels (low shared-mem use, plenty of warps).
const THREADS_PER_BLOCK: u32 = 256;

pub struct CudaEngine {
    bound: Option<BoundState>,
    /// Fatbin temp file. Must outlive the module load, hence the field.
    /// The CUDA runtime caches the module by name internally — once the
    /// file is loaded we could drop it, but keeping it does no harm.
    fatbin_temp: Option<tempfile::TempPath>,
}

struct BoundState {
    // Held to keep the CUDA context and module alive while we hold a
    // function pointer into the module. Cudarc tears them down on Drop,
    // so an early drop here would invalidate `func`.
    #[allow(dead_code)]
    ctx: Arc<CudaContext>,
    stream: Arc<CudaStream>,
    #[allow(dead_code)]
    module: Arc<CudaModule>,
    func: CudaFunction,
    mp_count: u32,
}

impl CudaEngine {
    pub fn new() -> Self {
        Self { bound: None, fatbin_temp: None }
    }
}

impl Default for CudaEngine {
    fn default() -> Self {
        Self::new()
    }
}

fn map_driver_err(e: cudarc::driver::DriverError) -> EngineError {
    use cudarc::driver::DriverError;
    let DriverError(code) = e;
    match code {
        sys::cudaError_enum::CUDA_ERROR_NO_DEVICE => EngineError::NoDevice,
        sys::cudaError_enum::CUDA_ERROR_NOT_INITIALIZED => {
            EngineError::DriverMissing(format!("CUDA driver returned {e:?}"))
        }
        _ => EngineError::Other(format!("CUDA driver error: {e:?}")),
    }
}

impl HashEngine for CudaEngine {
    fn kind(&self) -> &'static str {
        "CUDA"
    }

    fn enumerate(&self) -> Result<Vec<DeviceInfo>, EngineError> {
        enumerate::list_devices()
    }

    fn select_device(&mut self, index: u32) -> Result<(), EngineError> {
        let ctx = CudaContext::new(index as usize).map_err(map_driver_err)?;
        let stream = ctx.default_stream();

        // Materialize the fatbin to a tempfile so cudarc's safe `Ptx`
        // API can load it. cuModuleLoad reads the file once and the GPU
        // owns the module bytes after that.
        let mut tmp = tempfile::Builder::new()
            .prefix("ts3level-fatbin-")
            .suffix(".fatbin")
            .tempfile()
            .map_err(|e| EngineError::Other(format!("tempfile: {e}")))?;
        tmp.write_all(FATBIN)
            .map_err(|e| EngineError::Other(format!("write fatbin: {e}")))?;
        tmp.flush()
            .map_err(|e| EngineError::Other(format!("flush fatbin: {e}")))?;
        let path = tmp.into_temp_path();

        let module = ctx.load_module(Ptx::from_file(&path)).map_err(map_driver_err)?;
        let func = module.load_function("sha1_hasher").map_err(map_driver_err)?;

        let mp_count = ctx
            .attribute(sys::CUdevice_attribute::CU_DEVICE_ATTRIBUTE_MULTIPROCESSOR_COUNT)
            .map_err(map_driver_err)? as u32;

        debug!("Bound to CUDA device {index}, MP count {mp_count}");

        self.bound = Some(BoundState { ctx, stream, module, func, mp_count });
        self.fatbin_temp = Some(path);
        Ok(())
    }

    fn launch(&mut self, params: &LaunchParams) -> Result<LaunchResult, EngineError> {
        let state = self.bound.as_mut().ok_or_else(|| {
            EngineError::Other("no device selected before launch".into())
        })?;

        if params.pubkey_b64.len() > 110 {
            return Err(EngineError::Other(format!(
                "pubkey base64 length {} exceeds kernel limit of 110 bytes",
                params.pubkey_b64.len()
            )));
        }
        if params.n_counters == 0 {
            return Ok(LaunchResult {
                best_level: 0,
                best_counter: params.start_counter,
                hashes_performed: 0,
            });
        }

        let total_blocks = state.mp_count * BLOCKS_PER_SM;
        let total_threads = total_blocks * THREADS_PER_BLOCK;
        let n_per_thread = params
            .n_counters
            .div_ceil(total_threads as u64)
            .max(1);
        let actual_hashes = total_threads as u64 * n_per_thread;

        let pubkey_dev = state
            .stream
            .memcpy_stod(params.pubkey_b64.as_bytes())
            .map_err(map_driver_err)?;
        // Seed g_best_packed with the current best level shifted into the
        // high byte so atomicMax suppresses any equal-or-lower find.
        let seed: u64 = (params.current_best_level as u64) << 56;
        let mut g_best_dev = state
            .stream
            .memcpy_stod(&[seed])
            .map_err(map_driver_err)?;

        let cfg = LaunchConfig {
            grid_dim: (total_blocks, 1, 1),
            block_dim: (THREADS_PER_BLOCK, 1, 1),
            shared_mem_bytes: 0,
        };

        let pubkey_len: u32 = params.pubkey_b64.len() as u32;
        let current_best_level: u32 = params.current_best_level as u32;

        let mut launch = state.stream.launch_builder(&state.func);
        launch.arg(&pubkey_dev);
        launch.arg(&pubkey_len);
        launch.arg(&params.start_counter);
        launch.arg(&n_per_thread);
        launch.arg(&current_best_level);
        launch.arg(&mut g_best_dev);
        unsafe { launch.launch(cfg) }.map_err(map_driver_err)?;

        state.stream.synchronize().map_err(map_driver_err)?;
        let host_vec = state.stream.memcpy_dtov(&g_best_dev).map_err(map_driver_err)?;
        let packed = host_vec[0];
        let level = ((packed >> 56) & 0xFF) as u8;
        let counter = packed & 0x00FF_FFFF_FFFF_FFFF;

        let (best_level, best_counter) = if level > params.current_best_level {
            (level, counter)
        } else {
            (0, params.start_counter)
        };
        Ok(LaunchResult {
            best_level,
            best_counter,
            hashes_performed: actual_hashes,
        })
    }
}
