//! Driver-API-level device enumeration. Doesn't create a CudaContext per
//! device — it just queries names, compute capabilities, and memory size.

use cudarc::driver::{result, sys, DriverError};
use ts3level_engine::{DeviceInfo, EngineError};

pub(crate) fn list_devices() -> Result<Vec<DeviceInfo>, EngineError> {
    init_driver()?;
    let count = result::device::get_count().map_err(map_err)?;
    let mut devices = Vec::with_capacity(count as usize);
    for i in 0..count {
        let dev = result::device::get(i).map_err(map_err)?;
        let name = result::device::get_name(dev).map_err(map_err)?;
        let cc_major = unsafe {
            result::device::get_attribute(
                dev,
                sys::CUdevice_attribute::CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MAJOR,
            )
        }
        .map_err(map_err)?;
        let cc_minor = unsafe {
            result::device::get_attribute(
                dev,
                sys::CUdevice_attribute::CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MINOR,
            )
        }
        .map_err(map_err)?;
        let mp_count = unsafe {
            result::device::get_attribute(
                dev,
                sys::CUdevice_attribute::CU_DEVICE_ATTRIBUTE_MULTIPROCESSOR_COUNT,
            )
        }
        .map_err(map_err)?;
        let total_mem = unsafe { result::device::total_mem(dev) }.map_err(map_err)?;

        devices.push(DeviceInfo {
            kind: "CUDA",
            index: i as u32,
            name,
            compute_capability: (cc_major as u32, cc_minor as u32),
            total_memory_bytes: total_mem as u64,
            multiprocessor_count: mp_count as u32,
        });
    }
    Ok(devices)
}

fn init_driver() -> Result<(), EngineError> {
    match result::init() {
        Ok(()) => Ok(()),
        Err(e) => {
            let DriverError(code) = e;
            // Translate well-known driver-load failures into a clear
            // DriverMissing variant; everything else bubbles up.
            match code {
                sys::cudaError_enum::CUDA_ERROR_NO_DEVICE => Err(EngineError::NoDevice),
                _ => Err(EngineError::DriverMissing(format!("cuInit: {e:?}"))),
            }
        }
    }
}

fn map_err(e: DriverError) -> EngineError {
    EngineError::Other(format!("CUDA driver error: {e:?}"))
}
