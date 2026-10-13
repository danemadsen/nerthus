//! The fallback device: Nerthus's own CPU code, in the role of a SYCL
//! runtime's OpenMP host device. It is always active and is the last
//! device every SYCL function can run on.

use crate::devices::{DeviceInfo, DeviceType, FALLBACK_DEVICE};

/// The library id and backend name of the CPU code.
pub(crate) const LIBRARY: &str = "Native";

pub(crate) fn device_info() -> DeviceInfo {
    let device_name = processor_name();
    let vendor_name = processor_vendor(&device_name);
    let mut info = DeviceInfo::new(LIBRARY, DeviceType::Cpu, LIBRARY, vendor_name, &device_name, FALLBACK_DEVICE);
    info.is_fallback_device = true;
    info
}

/// The processor's name as the system gives it ("Apple M2 Pro"), with its
/// runs of spaces collapsed, or "CPU" when the system will not say.
fn processor_name() -> String {
    let Some(reported) = system_processor_name() else {
        return "CPU".to_string();
    };
    let words: Vec<&str> = reported.split_whitespace().collect();
    if words.is_empty() {
        return "CPU".to_string();
    }
    words.join(" ")
}

#[cfg(target_os = "macos")]
fn system_processor_name() -> Option<String> {
    let key = c"machdep.cpu.brand_string";
    let mut size: libc::size_t = 0;
    // SAFETY: with a null buffer, sysctlbyname only reports the value's
    // size into `size`.
    let status = unsafe { libc::sysctlbyname(key.as_ptr(), std::ptr::null_mut(), &mut size, std::ptr::null_mut(), 0) };
    if status != 0 || size == 0 {
        return None;
    }
    let mut buffer = vec![0u8; size];
    // SAFETY: the buffer holds `size` bytes, as much as sysctlbyname said
    // the value needs, and `size` tells it so.
    let status = unsafe { libc::sysctlbyname(key.as_ptr(), buffer.as_mut_ptr().cast(), &mut size, std::ptr::null_mut(), 0) };
    if status != 0 {
        return None;
    }
    buffer.truncate(size);
    let text = String::from_utf8_lossy(&buffer);
    Some(text.trim_end_matches('\0').to_string())
}

#[cfg(target_os = "linux")]
fn system_processor_name() -> Option<String> {
    let text = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    let line = text.lines().find(|line| line.starts_with("model name"))?;
    let (_, name) = line.split_once(':')?;
    Some(name.to_string())
}

#[cfg(target_os = "windows")]
fn system_processor_name() -> Option<String> {
    std::env::var("PROCESSOR_IDENTIFIER").ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn system_processor_name() -> Option<String> {
    None
}

/// The maker a processor name points to.
fn processor_vendor(name: &str) -> &'static str {
    let lowered = name.to_lowercase();
    let makers = [
        ("apple", "Apple"),
        ("intel", "Intel"),
        ("amd", "AMD"),
        ("qualcomm", "Qualcomm"),
        ("snapdragon", "Qualcomm"),
    ];
    for (needle, maker) in makers {
        if lowered.contains(needle) {
            return maker;
        }
    }
    "Unknown"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cpu_device_is_the_fallback() {
        let info = device_info();
        assert!(info.is_fallback_device);
        assert!(!info.requires_driver_update);
        assert!(info.id.starts_with("Native/Cpu/Native/"));
        assert!(info.id.ends_with("/cpu"));
        assert!(!info.device_name.is_empty());
    }

    #[test]
    fn vendors_come_from_the_name() {
        assert_eq!(processor_vendor("Apple M2 Pro"), "Apple");
        assert_eq!(processor_vendor("Intel(R) Core(TM) i9-9900K CPU @ 3.60GHz"), "Intel");
        assert_eq!(processor_vendor("AMD Ryzen 9 7950X 16-Core Processor"), "AMD");
        assert_eq!(processor_vendor("CPU"), "Unknown");
    }
}
