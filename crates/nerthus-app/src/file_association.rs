//! `Win32.Associate(".terrain", …)`, made at every start on Windows:
//! projects open in this Nerthus from Explorer and show its icon. The
//! keys are the user's own (`HKEY_CURRENT_USER\Software\Classes`), so no
//! administrator is needed; elsewhere than Windows nothing is registered.

/// The project files' extension, and the name and description they are
/// registered under.
#[cfg(windows)]
const EXTENSION: &str = ".terrain";
#[cfg(windows)]
const PROGRAM_ID: &str = "Nerthus.Terrain";
#[cfg(windows)]
const DESCRIPTION: &str = "Nerthus Terrain Project";

/// `.terrain` files opened by this program, with its icon, and Explorer
/// told the associations changed.
#[cfg(windows)]
pub fn register_project_files() {
    let program = match std::env::current_exe() {
        Ok(program) => program,
        Err(error) => {
            log::warn!("Cannot register {EXTENSION} files: {error}");
            return;
        }
    };
    let program = program.to_string_lossy();
    let classes = "Software\\Classes";
    let values = [
        (format!("{classes}\\{EXTENSION}"), PROGRAM_ID.to_string()),
        (format!("{classes}\\{PROGRAM_ID}"), DESCRIPTION.to_string()),
        (format!("{classes}\\{PROGRAM_ID}\\DefaultIcon"), format!("\"{program}\",0")),
        (format!("{classes}\\{PROGRAM_ID}\\shell\\open\\command"), open_command(&program)),
    ];
    for (key, value) in values {
        if let Err(error) = registry::set_default_value(&key, &value) {
            log::warn!("Cannot register {EXTENSION} files: {key}: error {error}");
            return;
        }
    }
    registry::announce_association_change();
}

#[cfg(not(windows))]
pub fn register_project_files() {}

/// What opens a project: the program with the file's path.
#[cfg(any(windows, test))]
fn open_command(program: &str) -> String {
    format!("\"{program}\" \"%1\"")
}

#[cfg(windows)]
mod registry {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{RegCloseKey, RegCreateKeyW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, REG_SZ};
    use windows_sys::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};

    /// A nul-terminated UTF-16 copy.
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// The key under the user's registry, made if need be, given its
    /// default value (`CreateSubKey`, `SetValue`). An error is the
    /// system's code.
    pub fn set_default_value(key: &str, value: &str) -> Result<(), u32> {
        let key_name = wide(key);
        let data = wide(value);
        let mut handle: HKEY = std::ptr::null_mut();
        // SAFETY: the name ends in a nul; the handle comes back filled.
        let created = unsafe { RegCreateKeyW(HKEY_CURRENT_USER, key_name.as_ptr(), &mut handle) };
        if created != ERROR_SUCCESS {
            return Err(created);
        }
        let byte_count = (data.len() * size_of::<u16>()) as u32;
        // SAFETY: the data is the value's UTF-16 with its nul, counted in
        // bytes; no name sets the key's default value.
        let set = unsafe { RegSetValueExW(handle, std::ptr::null(), 0, REG_SZ, data.as_ptr().cast(), byte_count) };
        // SAFETY: the handle is the key opened above.
        unsafe { RegCloseKey(handle) };
        if set != ERROR_SUCCESS {
            return Err(set);
        }
        Ok(())
    }

    /// `SHChangeNotify(SHCNE_ASSOCCHANGED)`: Explorer shows the new icon.
    pub fn announce_association_change() {
        // SAFETY: the event takes no items.
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, std::ptr::null(), std::ptr::null()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_open_with_their_path_quoted() {
        let command = open_command("C:\\Program Files\\Nerthus\\nerthus.exe");
        assert_eq!(command, "\"C:\\Program Files\\Nerthus\\nerthus.exe\" \"%1\"");
    }
}
