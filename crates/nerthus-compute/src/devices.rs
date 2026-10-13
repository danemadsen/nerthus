//! The registry of compute devices (`Sycl`'s statics): what the libraries
//! report, which devices are active, and the lookups the SYCL functions
//! pick their devices with.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::cpu;
use crate::gpu::{self, GpuDevice};

/// `Sycl.DeviceType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeviceType {
    Unknown,
    Cpu,
    CpuAccelerated,
    GpuIntegrated,
    GpuDiscrete,
    Accelerator,
}

impl DeviceType {
    /// The member's name, as device ids spell it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Cpu => "Cpu",
            Self::CpuAccelerated => "CpuAccelerated",
            Self::GpuIntegrated => "GpuIntegrated",
            Self::GpuDiscrete => "GpuDiscrete",
            Self::Accelerator => "Accelerator",
        }
    }

    /// `ComputeTab.HumanDevice`: the type as Options › Compute shows it.
    pub fn human_name(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Cpu | Self::CpuAccelerated => "CPU",
            Self::GpuIntegrated => "Integrated GPU",
            Self::GpuDiscrete => "Discrete GPU",
            Self::Accelerator => "Accelerator",
        }
    }
}

/// `Sycl.DeviceInfo`: a device as its library reports it. The id is
/// `library/type/backend/vendor/device/inner id`; it stays the same from
/// run to run, and the preferences keep the active devices by it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub library_id: String,
    pub device_type: DeviceType,
    pub backend_name: String,
    pub vendor_name: String,
    pub device_name: String,
    pub id: String,
    /// The CPU device every function falls back to.
    pub is_fallback_device: bool,
    /// The device lacks what the kernels need (its driver is too old, say);
    /// it cannot be activated.
    pub requires_driver_update: bool,
}

impl DeviceInfo {
    pub(crate) fn new(library_id: &str, device_type: DeviceType, backend_name: &str, vendor_name: &str, device_name: &str, inner_id: &str) -> DeviceInfo {
        // The id's parts are split on slashes (the status bar takes the
        // device name back out of it).
        let parts = [library_id, device_type.name(), backend_name, vendor_name, device_name, inner_id];
        let cleaned: Vec<String> = parts.iter().map(|part| part.replace('/', "-")).collect();
        DeviceInfo {
            library_id: library_id.to_string(),
            device_type,
            backend_name: backend_name.to_string(),
            vendor_name: vendor_name.to_string(),
            device_name: device_name.to_string(),
            id: cleaned.join("/"),
            is_fallback_device: false,
            requires_driver_update: false,
        }
    }
}

/// `NativeResult`: how a SYCL function ended on a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeResult {
    Success,
    Cancelled,
    InsufficientDeviceMemory,
    UndefinedError,
}

/// An active device (`Sycl.Device`): the CPU, or a GPU and its queue.
#[derive(Clone)]
pub enum Device {
    Cpu(DeviceInfo),
    Gpu(Arc<GpuDevice>),
}

impl Device {
    pub fn info(&self) -> &DeviceInfo {
        match self {
            Device::Cpu(info) => info,
            Device::Gpu(gpu) => gpu.info(),
        }
    }
}

/// The short id that names the fallback CPU device (`"cpu"`).
pub const FALLBACK_DEVICE: &str = "cpu";

/// The active devices, in the order they were activated. The fallback CPU
/// device comes first and never leaves.
struct Registry {
    active: Vec<Device>,
}

static REGISTRY: LazyLock<RwLock<Registry>> = LazyLock::new(|| {
    let fallback = Device::Cpu(cpu::device_info());
    let registry = Registry { active: vec![fallback] };
    RwLock::new(registry)
});

/// CPU-only mode, where only the fallback device loads.
static CPU_ONLY: AtomicBool = AtomicBool::new(false);

fn read_registry() -> RwLockReadGuard<'static, Registry> {
    REGISTRY.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write_registry() -> RwLockWriteGuard<'static, Registry> {
    REGISTRY.write().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Whether Nerthus runs in CPU-only mode.
pub fn is_cpu_only() -> bool {
    CPU_ONLY.load(Ordering::Relaxed)
}

pub fn set_cpu_only(cpu_only: bool) {
    CPU_ONLY.store(cpu_only, Ordering::Relaxed);
}

/// `Sycl.FallbackDeviceInfo`.
pub fn fallback_device_info() -> DeviceInfo {
    read_registry().active[0].info().clone()
}

/// `Sycl.GetAvailableDevices`: every device the libraries report, and what
/// they had to say about the devices they cannot use.
pub fn available_devices() -> (Vec<DeviceInfo>, String) {
    let mut devices = vec![fallback_device_info()];
    let (gpus, problems) = gpu::available_devices();
    devices.extend(gpus);
    (devices, problems)
}

/// `Sycl.GetActiveDevices`.
pub fn active_devices() -> Vec<DeviceInfo> {
    let registry = read_registry();
    registry.active.iter().map(|device| device.info().clone()).collect()
}

fn is_active(id: &str) -> bool {
    let registry = read_registry();
    registry.active.iter().any(|device| device.info().id == id)
}

/// The active device with this id.
pub fn active_device(id: &str) -> Option<Device> {
    let registry = read_registry();
    let found = registry.active.iter().find(|device| device.info().id == id);
    found.cloned()
}

/// `Sycl.Device.TryActivate`: makes the device with this id available to
/// the SYCL functions. The fallback is always active.
pub fn try_activate(id: &str) -> bool {
    if id == FALLBACK_DEVICE || is_active(id) {
        return true;
    }
    let library = id.split('/').next().unwrap_or_default();
    if library != gpu::LIBRARY {
        return false;
    }
    let Some(opened) = gpu::open_device(id) else {
        return false;
    };
    let mut registry = write_registry();
    // Another thread may have activated it in the meantime.
    let already = registry.active.iter().any(|device| device.info().id == id);
    if !already {
        let device = Device::Gpu(Arc::new(opened));
        registry.active.push(device);
    }
    true
}

/// `Sycl.Device.TryDeactivate`: every device but the fallback can go.
/// Functions already running on it finish there.
pub fn try_deactivate(id: &str) -> bool {
    let mut registry = write_registry();
    let fallback = registry.active[0].info().id.clone();
    if id == FALLBACK_DEVICE || id == fallback {
        return false;
    }
    registry.active.retain(|device| device.info().id != id);
    true
}

/// `Sycl.GetFirstActiveDevice`: the first active device of the first of
/// `types` that has one, else the fallback.
pub fn first_active_device(types: &[DeviceType]) -> DeviceInfo {
    let registry = read_registry();
    for &device_type in types {
        let found = registry.active.iter().find(|device| device.info().device_type == device_type);
        if let Some(device) = found {
            return device.info().clone();
        }
    }
    registry.active[0].info().clone()
}

/// `Sycl.GetDeviceIdsByTypes`: the active devices of each of `types` in
/// turn (each type's in the order they were activated), leaving out those
/// that need a driver update.
pub fn devices_by_types(types: &[DeviceType]) -> Vec<Device> {
    let registry = read_registry();
    let mut devices = Vec::new();
    for &device_type in types {
        for device in &registry.active {
            let info = device.info();
            if info.device_type == device_type && !info.requires_driver_update {
                devices.push(device.clone());
            }
        }
    }
    devices
}

/// `SyclLoader.LoadAllBackendsAsyncTask`: activates every available device
/// except the fallback (already active), integrated GPUs and devices that
/// need a driver update.
pub fn load_all_backends() {
    let (devices, _) = available_devices();
    for device in devices {
        let skipped = device.requires_driver_update || device.is_fallback_device || device.device_type == DeviceType::GpuIntegrated;
        if skipped {
            continue;
        }
        try_activate(&device.id);
        log::debug!(" Loading {} ({})", device.device_name, device.backend_name);
    }
}

/// `SyclLoader.LoadBackendsAsyncTask`: activates the devices saved in the
/// preferences.
pub fn load_backends(ids: &[String]) {
    for id in ids {
        try_activate(id);
        log::debug!(" DEVICE: {id}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_join_their_parts_without_stray_slashes() {
        let info = DeviceInfo::new("wgpu", DeviceType::GpuDiscrete, "Vulkan", "NVIDIA", "GeForce RTX 4090/PCIe", "10de:2684");
        assert_eq!(info.id, "wgpu/GpuDiscrete/Vulkan/NVIDIA/GeForce RTX 4090-PCIe/10de:2684");
        assert_eq!(info.device_name, "GeForce RTX 4090/PCIe");
    }

    #[test]
    fn the_fallback_is_always_active_and_stays() {
        let fallback = fallback_device_info();
        assert!(fallback.is_fallback_device);
        assert_eq!(fallback.device_type, DeviceType::Cpu);
        assert!(try_activate(FALLBACK_DEVICE));
        assert!(try_activate(&fallback.id));
        assert!(!try_deactivate(&fallback.id));
        assert!(!try_deactivate(FALLBACK_DEVICE));
        assert_eq!(active_devices()[0], fallback);
    }

    #[test]
    fn unknown_types_fall_back_to_the_cpu() {
        let first = first_active_device(&[DeviceType::Accelerator]);
        assert!(first.is_fallback_device);
        let devices = devices_by_types(&[DeviceType::Cpu]);
        assert!(devices.iter().any(|device| device.info().is_fallback_device));
        let none = devices_by_types(&[DeviceType::Accelerator]);
        assert!(none.is_empty());
    }
}
