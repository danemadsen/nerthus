//! `Sycl.Functions`: how a function that runs as SYCL kernels picks its
//! device. Each function lists the device types it may run on in order
//! of preference; it runs on the active devices of those types in turn
//! until one succeeds. The CPU code is the fallback device, always active,
//! and it never fails. On a GPU the function runs its WGSL kernels (see
//! [`crate::gpu`]), which follow the CPU code's arithmetic but not its
//! x86-specific rounding, so their results come close to the CPU's without
//! matching them bit for bit (as GPU and CPU results never quite do).

use nerthus_compute::{Device, DeviceType, GpuDevice, GpuError};

/// The order most functions pick devices in (`MedianFilter`,
/// `ThermalSimulation`, `ColorTransportSimulation`, `FlowSimulation`,
/// `ThermalShaper_Process` and the mesh renderers).
pub(crate) const PREFERRED: &[DeviceType] = &[
    DeviceType::GpuDiscrete,
    DeviceType::CpuAccelerated,
    DeviceType::GpuIntegrated,
    DeviceType::Accelerator,
    DeviceType::Unknown,
    DeviceType::Cpu,
];

/// `Cellular3D_Render`'s order: no CPU-accelerated devices.
pub(crate) const CELLULAR: &[DeviceType] = &[DeviceType::GpuDiscrete, DeviceType::GpuIntegrated, DeviceType::Accelerator, DeviceType::Unknown, DeviceType::Cpu];

/// `MeshRasterizer_RenderMeshWithTriplanarDisplacement`'s order.
pub(crate) const DISCRETE_OR_CPU: &[DeviceType] = &[DeviceType::GpuDiscrete, DeviceType::Cpu];

/// `ParallelErosionSimulation`, `Shade` and `SunLightIntegral`: discrete
/// GPUs only. Their callers fall back to the native CPU code.
pub(crate) const DISCRETE: &[DeviceType] = &[DeviceType::GpuDiscrete];

/// Whether a discrete GPU is active (`GetFirstActiveDevice(GpuDiscrete,
/// Cpu)` returning a discrete GPU), the test behind ErosionPro's and the
/// lighting's GPU paths.
pub(crate) fn discrete_gpu_active() -> bool {
    let types = [DeviceType::GpuDiscrete, DeviceType::Cpu];
    let first = nerthus_compute::first_active_device(&types);
    first.device_type == DeviceType::GpuDiscrete
}

/// Runs the function on each device of `types` in
/// turn, `gpu` on GPUs and `cpu` on the CPU, until one succeeds. When none
/// does (or `types` has no CPU and every GPU fails) the CPU code runs, as
/// the callers fall back to the native code. `name` and `resolution` only
/// go into the log.
pub(crate) fn run<T>(name: &str, resolution: usize, types: &[DeviceType], mut gpu: impl FnMut(&GpuDevice) -> Result<T, GpuError>, cpu: impl FnOnce() -> T) -> T {
    let mut first = true;
    for device in nerthus_compute::devices_by_types(types) {
        let info = device.info();
        if first {
            log::trace!("Executing {name} at {resolution} on device: {} ({})", info.device_name, info.backend_name);
        } else {
            log::warn!("Retrying {name} at {resolution} on device: {} ({})", info.device_name, info.backend_name);
        }
        first = false;
        let gpu_device = match &device {
            Device::Cpu(_) => return cpu(),
            Device::Gpu(gpu_device) => gpu_device,
        };
        match gpu_device.run(&mut gpu) {
            Ok(result) => return result,
            Err(GpuError::OutOfMemory(message)) => log::warn!("Not enough memory on device: {} ({}): {message}", info.device_name, info.backend_name),
            Err(error) => log::error!("Error on device: {} ({}): {error}", info.device_name, info.backend_name),
        }
    }
    if !first {
        let fallback = nerthus_compute::fallback_device_info();
        log::info!("Retrying {name} at {resolution} on device: {} ({})", fallback.device_name, fallback.backend_name);
    }
    cpu()
}
