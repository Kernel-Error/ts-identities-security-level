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
use cudarc::driver::{
    CudaContext, CudaFunction, CudaModule, CudaStream, LaunchConfig, PushKernelArg,
};
use cudarc::nvrtc::Ptx;
use std::io::Write;
use std::sync::Arc;
use tracing::debug;
use ts3level_core::sha1_block::{sha1_block, SHA1_INIT};
use ts3level_engine::{DeviceInfo, EngineError, HashEngine, LaunchParams, LaunchResult};

mod cache;
mod enumerate;
mod tune;

pub use tune::Geometry;

const FATBIN: &[u8] = include_bytes!(env!("TS3LEVEL_FATBIN_PATH"));

pub struct CudaEngine {
    bound: Option<BoundState>,
    /// Fatbin temp file. Must outlive the module load, hence the field.
    /// The CUDA runtime caches the module by name internally — once the
    /// file is loaded we could drop it, but keeping it does no harm.
    fatbin_temp: Option<tempfile::TempPath>,
    /// When `true`, the next `select_device` skips the persisted tuning
    /// cache and re-probes from scratch. CLI users opt into this with
    /// `--retune`; the GUI doesn't expose it (the cache is per-device
    /// and re-population is silent enough that running once on the new
    /// hardware is fine).
    force_retune: bool,
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
    geometry: Geometry,
}

impl CudaEngine {
    pub fn new() -> Self {
        Self {
            bound: None,
            fatbin_temp: None,
            force_retune: false,
        }
    }

    /// When set, `select_device` ignores the cache for this run and
    /// reruns the geometry probe. The freshly-measured result is
    /// written back to the cache on success.
    pub fn set_force_retune(&mut self, force: bool) {
        self.force_retune = force;
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

        let module = ctx
            .load_module(Ptx::from_file(&path))
            .map_err(map_driver_err)?;
        let func = module
            .load_function("sha1_hasher")
            .map_err(map_driver_err)?;

        let mp_count = ctx
            .attribute(sys::CUdevice_attribute::CU_DEVICE_ATTRIBUTE_MULTIPROCESSOR_COUNT)
            .map_err(map_driver_err)? as u32;

        let device_name = ctx.name().map_err(map_driver_err)?;
        debug!("Bound to CUDA device {index} ({device_name}), MP count {mp_count}");

        // Auto-tune (or load cached) launch geometry for this device.
        let geometry = tune::pick_geometry(
            &ctx,
            &stream,
            &func,
            mp_count,
            &device_name,
            self.force_retune,
        )?;

        self.bound = Some(BoundState {
            ctx,
            stream,
            module,
            func,
            mp_count,
            geometry,
        });
        self.fatbin_temp = Some(path);
        Ok(())
    }

    fn launch(&mut self, params: &LaunchParams) -> Result<LaunchResult, EngineError> {
        let state = self
            .bound
            .as_mut()
            .ok_or_else(|| EngineError::Other("no device selected before launch".into()))?;

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

        let total_blocks = state.mp_count * state.geometry.blocks_per_sm;
        let total_threads = total_blocks * state.geometry.threads_per_block;
        let n_per_thread = params.n_counters.div_ceil(total_threads as u64).max(1);
        let actual_hashes = total_threads as u64 * n_per_thread;

        // Precompute the SHA-1 state after every complete 64-byte block
        // of the pubkey prefix. The kernel only needs to process the
        // tail (pubkey remainder + counter + padding) per attempt.
        // For the typical 108-char TS3 pubkey this folds one block out
        // of every three into a single host-side hash per launch.
        let pubkey_bytes = params.pubkey_b64.as_bytes();
        let prefix_blocks = pubkey_bytes.len() / 64;
        let prefix_len = prefix_blocks * 64;
        let mut midstate = SHA1_INIT;
        for i in 0..prefix_blocks {
            let mut block = [0u8; 64];
            block.copy_from_slice(&pubkey_bytes[i * 64..(i + 1) * 64]);
            sha1_block(&mut midstate, &block);
        }
        let pubkey_tail = &pubkey_bytes[prefix_len..];
        let prefix_bit_len = (prefix_len as u64) * 8;

        let pubkey_dev = state
            .stream
            .memcpy_stod(pubkey_tail)
            .map_err(map_driver_err)?;
        let midstate_dev = state
            .stream
            .memcpy_stod(&midstate)
            .map_err(map_driver_err)?;
        // Two separate device slots so the level field can't be
        // corrupted by counter values ≥ 2^56 (which the previous packed
        // representation silently truncated). The kernel seeds level
        // with `current_best_level`; any `atomicMax` write below that
        // threshold is a no-op.
        let mut g_best_level = state
            .stream
            .memcpy_stod(&[params.current_best_level as u32])
            .map_err(map_driver_err)?;
        let mut g_best_counter = state.stream.memcpy_stod(&[0u64]).map_err(map_driver_err)?;

        let cfg = LaunchConfig {
            grid_dim: (total_blocks, 1, 1),
            block_dim: (state.geometry.threads_per_block, 1, 1),
            shared_mem_bytes: 0,
        };

        let pubkey_tail_len: u32 = pubkey_tail.len() as u32;
        let current_best_level: u32 = params.current_best_level as u32;

        let mut launch = state.stream.launch_builder(&state.func);
        launch.arg(&pubkey_dev);
        launch.arg(&pubkey_tail_len);
        launch.arg(&prefix_bit_len);
        launch.arg(&midstate_dev);
        launch.arg(&params.start_counter);
        launch.arg(&n_per_thread);
        launch.arg(&current_best_level);
        launch.arg(&mut g_best_level);
        launch.arg(&mut g_best_counter);
        unsafe { launch.launch(cfg) }.map_err(map_driver_err)?;

        state.stream.synchronize().map_err(map_driver_err)?;
        let level_host = state
            .stream
            .memcpy_dtov(&g_best_level)
            .map_err(map_driver_err)?;
        let counter_host = state
            .stream
            .memcpy_dtov(&g_best_counter)
            .map_err(map_driver_err)?;
        let level = level_host[0].min(u8::MAX as u32) as u8;
        let counter = counter_host[0];

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
