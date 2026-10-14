//! Compute devices for the nodes that run as SYCL-style kernels: the
//! registry the SYCL functions pick their devices from (`Sycl`) and
//! activating devices at start (`SyclLoader`).
//!
//! Two libraries provide devices: Nerthus's own CPU code, the fallback
//! that is always active, and wgpu, whose GPUs (Metal, Vulkan, DirectX 12)
//! the user turns on in Options › Compute. A SYCL function lists the
//! device types it may run on in order
//! of preference, and runs on the first active device of those types that
//! succeeds.

// The web build leaves out what a browser page cannot do (files, other
// programs, the network, GPU compute), so code only those use goes unused.
#![cfg_attr(target_arch = "wasm32", allow(dead_code, unused_imports))]

mod benchmark;
mod cpu;
mod devices;
mod gpu;
mod nvidia_profile;

pub use benchmark::{best_device, read_scores, scan, write_scores, Scores, BENCHMARK_FILE};
pub use devices::{
    active_device, active_devices, available_devices, devices_by_types, fallback_device_info, first_active_device, is_cpu_only, load_all_backends, load_backends, set_cpu_only, try_activate,
    try_deactivate, Device, DeviceInfo, DeviceType, NativeResult, FALLBACK_DEVICE,
};
pub use gpu::{open_device, Batch, Binding, Entry, GpuDevice, GpuError, Kernel, Shader, GROUP_SIDE, GROUP_SIZE};
pub use nvidia_profile::{enforce_optimal_settings, ProfileError};
pub use wgpu;

/// The file in the data folder whose presence puts Nerthus in CPU-only
/// mode (Options › Compute writes and removes it).
pub const CPU_ONLY_FILE: &str = "cpuonly.dat";
