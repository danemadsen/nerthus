//! `NahimicBlocker` and `App.NahimicCountermeasures`. The Nahimic audio
//! software, which comes on many Windows computers, injects itself into
//! programs and can break their rendering. As Nerthus starts, its modules
//! are looked for in the process and unloaded; when they stay, or its
//! service is running, "Nahimic Detected!" shows on the Start dialog and
//! the status bar (a click shows the warning), and the warning shows by
//! itself unless "Do not warn me again" was ticked once (`nahimic.warn` in
//! the data folder). Elsewhere than Windows nothing is checked.

use std::sync::atomic::{AtomicBool, Ordering};

/// The modules it injects.
#[cfg(windows)]
const MODULES: [&str; 4] = ["NahimicV3OSD.dll", "NahimicV3DevProps.dll", "NahimicSvc32.dll", "NahimicSvc64.dll"];

/// The file in the data folder that keeps the warning from showing by
/// itself.
const IGNORE_FILE: &str = "nahimic.warn";

/// `O.NahimicShowWarning`: "Nahimic Detected!" shows.
static DETECTED: AtomicBool = AtomicBool::new(false);
/// The service was found running, and the warning is to show by itself.
static WARNING_ASKED: AtomicBool = AtomicBool::new(false);

/// Whether "Nahimic Detected!" shows.
pub fn detected() -> bool {
    DETECTED.load(Ordering::Relaxed)
}

/// Whether the warning should show by itself now (it asks once).
pub fn take_warning_request() -> bool {
    WARNING_ASKED.swap(false, Ordering::Relaxed)
}

/// `--show nahimic`, for checking the warning without Nahimic: as if its
/// service was found, with the warning asked for or not.
pub fn pretend_detected(warn: bool) {
    DETECTED.store(true, Ordering::Relaxed);
    WARNING_ASKED.store(warn, Ordering::Relaxed);
}

/// "Do not warn me again": `nahimic.warn` written.
pub fn stop_warning() {
    let Some(folder) = crate::user::data_folder() else {
        return;
    };
    let written = std::fs::create_dir_all(&folder).and_then(|()| std::fs::write(folder.join(IGNORE_FILE), [0u8; 100]));
    if let Err(error) = written {
        log::warn!("cannot write {IGNORE_FILE}: {error}");
    }
}

/// `NahimicCountermeasures`: the modules looked for and unloaded, then
/// looked for again, and the running programs checked in the background.
#[cfg(windows)]
pub fn countermeasures() {
    if detect() {
        unload();
    }
    let ignored = crate::user::data_folder().is_some_and(|folder| folder.join(IGNORE_FILE).exists());
    if ignored {
        log::warn!("User has chosen to ignore warning about Nahimic interference.");
    }
    DETECTED.store(detect(), Ordering::Relaxed);
    let spawned = std::thread::Builder::new().name("nahimic check".to_string()).spawn(move || {
        if !system::service_running() {
            return;
        }
        if !ignored {
            WARNING_ASKED.store(true, Ordering::Relaxed);
        }
        DETECTED.store(true, Ordering::Relaxed);
    });
    if let Err(error) = spawned {
        log::warn!("cannot look for Nahimic: {error}");
    }
}

#[cfg(not(windows))]
pub fn countermeasures() {}

/// `NahimicBlocker.Detect`: whether any of its modules is loaded.
#[cfg(windows)]
fn detect() -> bool {
    log::info!("Trying to detect Nahimic injection.");
    let mut detected = false;
    for name in MODULES {
        if system::loaded_module(name).is_some() {
            log::warn!("Nahimic: {name} injection detected.");
            detected = true;
        }
    }
    detected
}

/// `NahimicBlocker.Unload`: its loaded modules freed.
#[cfg(windows)]
fn unload() {
    log::info!("Trying to unload Nahimic injection.");
    for name in MODULES {
        let Some(module) = system::loaded_module(name) else {
            continue;
        };
        log::warn!("Nahimic: {name} injection detected.");
        system::free_module(module);
        log::warn!("Nahimic: Unloaded {name}");
    }
}

#[cfg(windows)]
mod system {
    use windows_sys::Win32::Foundation::{CloseHandle, FreeLibrary, HMODULE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

    /// The module, when it is loaded in Nerthus (`GetModuleHandle`).
    pub fn loaded_module(name: &str) -> Option<HMODULE> {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: the name ends in a nul; the handle comes back uncounted.
        let module = unsafe { GetModuleHandleW(wide.as_ptr()) };
        (!module.is_null()).then_some(module)
    }

    /// `FreeLibrary`: one load of the module undone.
    pub fn free_module(module: HMODULE) {
        // SAFETY: the handle is a loaded module's, from `GetModuleHandle`.
        unsafe { FreeLibrary(module) };
    }

    /// Whether a running program's name has "nahimic" in it.
    pub fn service_running() -> bool {
        // SAFETY: a snapshot of the processes, closed below.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut found = false;
        // SAFETY: the entry's size is set, as the walk needs.
        let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
        while more && !found {
            let length = entry.szExeFile.iter().position(|&unit| unit == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..length]);
            found = name.to_lowercase().contains("nahimic");
            // SAFETY: as above.
            more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
        }
        // SAFETY: the snapshot is ours.
        unsafe { CloseHandle(snapshot) };
        found
    }
}
