//! The NVIDIA settings profile that Options › Compute's "Enforce Optimal
//! Settings" and the first-run setup make on Windows: a driver profile
//! named "Nerthus", holding the editor and the command-line build, that
//! runs them on the NVIDIA GPU of a laptop with two GPUs, keeps CUDA from
//! falling back on system memory when video memory runs out (only CUDA
//! programs read this one), and caps their frame rate at 15 while they are
//! in the background. An existing "Nerthus" profile keeps its programs and
//! has the settings set again.
//!
//! The profile is made through the driver's settings API (NVAPI's DRS
//! functions in nvapi64.dll, found through `nvapi_QueryInterface`). What
//! each step did is reported a line at a time.

/// Why the profile was not made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileError {
    /// No NVIDIA driver: nvapi64.dll is not there (always, off Windows).
    NoDriver,
    /// A step failed; the steps' report, the failure last.
    Failed(Vec<String>),
}

/// Makes the profile (or finds it) and sets its settings. Returns what
/// each step did.
pub fn enforce_optimal_settings() -> Result<Vec<String>, ProfileError> {
    imp::enforce_optimal_settings()
}

#[cfg(not(windows))]
mod imp {
    use super::ProfileError;

    pub fn enforce_optimal_settings() -> Result<Vec<String>, ProfileError> {
        Err(ProfileError::NoDriver)
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;

    use libloading::Library;

    use super::ProfileError;

    /// The profile's name.
    const PROFILE_NAME: &str = "Nerthus";

    /// The programs in the profile: (executable, name shown for it).
    const PROFILE_PROGRAMS: [(&str, &str); 2] = [("nerthus.exe", "nerthus"), ("nerthus-build.exe", "nerthus-build")];

    /// The settings, as (setting, value): `SHIM_RENDERING_MODE` on the
    /// NVIDIA GPU (`SHIM_RENDERING_MODE_ENABLE`); `CUDA - Sysmem Fallback
    /// Policy` at "Prefer No Sysmem Fallback"; `Frame Rate Limiter -
    /// Background Application` at 15 frames a second.
    const SHIM_RENDERING_MODE: (u32, u32) = (0x10F9_DC81, 1);
    const CUDA_SYSMEM_FALLBACK_POLICY: (u32, u32) = (0x10EC_ECC9, 1);
    const BACKGROUND_FRAME_RATE_LIMIT: (u32, u32) = (0x1083_5005, 15);

    /// `NvAPI_UnicodeString`: 2048 UTF-16 units, zero-terminated.
    type UnicodeString = [u16; 2048];
    type Status = i32;
    type Session = *mut c_void;
    type Profile = *mut c_void;

    const OK: Status = 0;

    /// The functions' ids for `nvapi_QueryInterface`.
    const INITIALIZE: u32 = 0x0150_E828;
    const GET_ERROR_MESSAGE: u32 = 0x6C2D_048C;
    const CREATE_SESSION: u32 = 0x0694_D52E;
    const DESTROY_SESSION: u32 = 0xDAD9_CFF8;
    const LOAD_SETTINGS: u32 = 0x375D_BD6B;
    const SAVE_SETTINGS: u32 = 0xFCBC_7E14;
    const FIND_PROFILE_BY_NAME: u32 = 0x7E4A_9A0B;
    const CREATE_PROFILE: u32 = 0xCC17_6068;
    const CREATE_APPLICATION: u32 = 0x4347_A9DE;
    /// `NvAPI_DRS_SetSetting` with two more arguments (both 0), which
    /// current drivers want; older ones only have the plain one.
    const SET_SETTING: u32 = 0x8A2C_F5F5;
    const SET_SETTING_PLAIN: u32 = 0x577D_D202;

    /// `NVDRS_PROFILE_V1`.
    #[repr(C)]
    struct ProfileInfo {
        version: u32,
        profile_name: UnicodeString,
        gpu_support: u32,
        is_predefined: u32,
        num_of_apps: u32,
        num_of_settings: u32,
    }

    /// `NVDRS_APPLICATION_V3`.
    #[repr(C)]
    struct Application {
        version: u32,
        is_predefined: u32,
        app_name: UnicodeString,
        user_friendly_name: UnicodeString,
        launcher: UnicodeString,
        file_in_folder: UnicodeString,
        flags: u32,
    }

    /// `NVDRS_SETTING_V1`, with a DWORD's values (the unions are 4100
    /// bytes: a binary setting's length and 4096 bytes).
    #[repr(C)]
    struct Setting {
        version: u32,
        setting_name: UnicodeString,
        setting_id: u32,
        setting_type: u32,
        setting_location: u32,
        is_current_predefined: u32,
        is_predefined_valid: u32,
        predefined_value: [u32; 1025],
        current_value: [u32; 1025],
    }

    /// `MAKE_NVAPI_VERSION`: the size, and the version above it.
    const fn make_version(size: usize, version: u32) -> u32 {
        size as u32 | (version << 16)
    }

    const _: () = assert!(std::mem::size_of::<ProfileInfo>() == 0x1014);
    const _: () = assert!(std::mem::size_of::<Application>() == 0x400C);
    const _: () = assert!(std::mem::size_of::<Setting>() == 0x3020);

    type QueryInterface = unsafe extern "C" fn(u32) -> *mut c_void;
    type Initialize = unsafe extern "C" fn() -> Status;
    type GetErrorMessage = unsafe extern "C" fn(Status, *mut [u8; 64]) -> Status;
    type CreateSession = unsafe extern "C" fn(*mut Session) -> Status;
    type SessionCall = unsafe extern "C" fn(Session) -> Status;
    type FindProfileByName = unsafe extern "C" fn(Session, *const u16, *mut Profile) -> Status;
    type CreateProfile = unsafe extern "C" fn(Session, *mut ProfileInfo, *mut Profile) -> Status;
    type CreateApplication = unsafe extern "C" fn(Session, Profile, *mut Application) -> Status;
    type SetSetting = unsafe extern "C" fn(Session, Profile, *mut Setting, u32, u32) -> Status;
    type SetSettingPlain = unsafe extern "C" fn(Session, Profile, *mut Setting) -> Status;

    /// The functions in use.
    struct Api {
        query: QueryInterface,
        get_error_message: Option<GetErrorMessage>,
    }

    impl Api {
        /// The function with this id, when the driver has it.
        ///
        /// # Safety
        /// `F` must be the function's signature.
        unsafe fn function<F: Copy>(&self, id: u32) -> Option<F> {
            assert_eq!(std::mem::size_of::<F>(), std::mem::size_of::<*mut c_void>());
            let pointer = unsafe { (self.query)(id) };
            if pointer.is_null() {
                return None;
            }
            Some(unsafe { std::mem::transmute_copy(&pointer) })
        }

        /// `NVAPI error: <message>`.
        fn error(&self, status: Status) -> String {
            let mut message = [0u8; 64];
            if let Some(get_error_message) = self.get_error_message {
                unsafe { get_error_message(status, &mut message) };
            }
            let end = message.iter().position(|byte| *byte == 0).unwrap_or(message.len());
            let text = String::from_utf8_lossy(&message[..end]).to_string();
            if text.is_empty() {
                return format!("NVAPI error: {status}");
            }
            format!("NVAPI error: {text}")
        }
    }

    /// `text` as an `NvAPI_UnicodeString`.
    fn unicode(text: &str) -> UnicodeString {
        let mut string = [0u16; 2048];
        for (slot, unit) in string.iter_mut().zip(text.encode_utf16().take(2047)) {
            *slot = unit;
        }
        string
    }

    /// The report so far, with the failure, as an error.
    fn failed(mut report: Vec<String>, line: String) -> ProfileError {
        log::error!("{line}");
        report.push(line);
        ProfileError::Failed(report)
    }

    fn note(report: &mut Vec<String>, line: &str) {
        log::info!("{line}");
        report.push(line.to_string());
    }

    pub fn enforce_optimal_settings() -> Result<Vec<String>, ProfileError> {
        let library = match unsafe { Library::new("nvapi64.dll") } {
            Ok(library) => library,
            Err(_) => return Err(ProfileError::NoDriver),
        };
        let query = match unsafe { library.get::<QueryInterface>(b"nvapi_QueryInterface\0") } {
            Ok(symbol) => *symbol,
            Err(_) => return Err(failed(Vec::new(), "Failed to find NvAPI_QueryInterface.".to_string())),
        };
        let mut api = Api { query, get_error_message: None };
        api.get_error_message = unsafe { api.function(GET_ERROR_MESSAGE) };
        let mut report = Vec::new();
        let result = unsafe { run(&api, &mut report) };
        // The driver's library stays loaded for the session: unloading it
        // under the driver's own threads is not safe.
        std::mem::forget(library);
        match result {
            Ok(()) => Ok(report),
            Err(line) => Err(failed(report, line)),
        }
    }

    /// The steps, until one fails (its line is the error).
    ///
    /// # Safety
    /// `api` must come from nvapi64.dll's `nvapi_QueryInterface`.
    unsafe fn run(api: &Api, report: &mut Vec<String>) -> Result<(), String> {
        let missing = |name: &str| format!("NVAPI has no {name}.");
        let initialize: Initialize = unsafe { api.function(INITIALIZE) }.ok_or_else(|| missing("NvAPI_Initialize"))?;
        let create_session: CreateSession = unsafe { api.function(CREATE_SESSION) }.ok_or_else(|| missing("NvAPI_DRS_CreateSession"))?;
        let destroy_session: SessionCall = unsafe { api.function(DESTROY_SESSION) }.ok_or_else(|| missing("NvAPI_DRS_DestroySession"))?;
        let load_settings: SessionCall = unsafe { api.function(LOAD_SETTINGS) }.ok_or_else(|| missing("NvAPI_DRS_LoadSettings"))?;
        let save_settings: SessionCall = unsafe { api.function(SAVE_SETTINGS) }.ok_or_else(|| missing("NvAPI_DRS_SaveSettings"))?;
        let find_profile: FindProfileByName = unsafe { api.function(FIND_PROFILE_BY_NAME) }.ok_or_else(|| missing("NvAPI_DRS_FindProfileByName"))?;
        let create_profile: CreateProfile = unsafe { api.function(CREATE_PROFILE) }.ok_or_else(|| missing("NvAPI_DRS_CreateProfile"))?;
        let create_application: CreateApplication = unsafe { api.function(CREATE_APPLICATION) }.ok_or_else(|| missing("NvAPI_DRS_CreateApplication"))?;
        let set_setting: Option<SetSetting> = unsafe { api.function(SET_SETTING) };
        let set_setting_plain: Option<SetSettingPlain> = unsafe { api.function(SET_SETTING_PLAIN) };
        if set_setting.is_none() && set_setting_plain.is_none() {
            return Err(missing("NvAPI_DRS_SetSetting"));
        }

        let status = unsafe { initialize() };
        if status != OK {
            return Err(api.error(status));
        }
        note(report, "NVAPI initialized successfully.");

        let mut session: Session = std::ptr::null_mut();
        let status = unsafe { create_session(&mut session) };
        if status != OK {
            return Err(api.error(status));
        }
        note(report, "DRS session created successfully.");
        let configured = unsafe { configure(api, session, report, load_settings, save_settings, find_profile, create_profile, create_application, set_setting, set_setting_plain) };
        unsafe { destroy_session(session) };
        configured?;
        note(report, "Session destroyed, configuration complete.");
        Ok(())
    }

    /// Loads the settings, finds or makes the profile, sets its settings
    /// and saves them.
    ///
    /// # Safety
    /// The functions must be NVAPI's and `session` a session they made.
    #[allow(clippy::too_many_arguments)]
    unsafe fn configure(
        api: &Api,
        session: Session,
        report: &mut Vec<String>,
        load_settings: SessionCall,
        save_settings: SessionCall,
        find_profile: FindProfileByName,
        create_profile: CreateProfile,
        create_application: CreateApplication,
        set_setting: Option<SetSetting>,
        set_setting_plain: Option<SetSettingPlain>,
    ) -> Result<(), String> {
        let status = unsafe { load_settings(session) };
        if status != OK {
            return Err(api.error(status));
        }
        note(report, "Settings loaded successfully.");

        let name = unicode(PROFILE_NAME);
        let mut profile: Profile = std::ptr::null_mut();
        let status = unsafe { find_profile(session, name.as_ptr(), &mut profile) };
        if status == OK {
            note(report, "Nerthus profile found.");
        } else {
            note(report, "Nerthus profile not found, creating new profile...");
            let mut info = Box::new(ProfileInfo {
                version: make_version(std::mem::size_of::<ProfileInfo>(), 1),
                profile_name: name,
                gpu_support: 0,
                is_predefined: 0,
                num_of_apps: 0,
                num_of_settings: 0,
            });
            let status = unsafe { create_profile(session, info.as_mut(), &mut profile) };
            if status != OK {
                return Err(api.error(status));
            }
            for (executable, shown) in PROFILE_PROGRAMS {
                let mut application = Box::new(Application {
                    version: make_version(std::mem::size_of::<Application>(), 3),
                    is_predefined: 0,
                    app_name: unicode(executable),
                    user_friendly_name: unicode(shown),
                    launcher: [0; 2048],
                    file_in_folder: [0; 2048],
                    flags: 0,
                });
                let status = unsafe { create_application(session, profile, application.as_mut()) };
                // A program the driver refuses leaves the others in.
                if status != OK {
                    let line = api.error(status);
                    log::warn!("{line}");
                    report.push(line);
                }
            }
            note(report, "Nerthus profile is created successfully.");
        }

        let set = |(setting_id, value): (u32, u32)| -> Status {
            let mut setting: Box<Setting> = Box::new(Setting {
                version: make_version(std::mem::size_of::<Setting>(), 1),
                setting_name: [0; 2048],
                setting_id,
                setting_type: 0,
                setting_location: 0,
                is_current_predefined: 0,
                is_predefined_valid: 0,
                predefined_value: [0; 1025],
                current_value: [0; 1025],
            });
            setting.current_value[0] = value;
            match (set_setting, set_setting_plain) {
                (Some(set_setting), _) => unsafe { set_setting(session, profile, setting.as_mut(), 0, 0) },
                (None, Some(set_setting_plain)) => unsafe { set_setting_plain(session, profile, setting.as_mut()) },
                (None, None) => -1,
            }
        };

        // A GPU preference the driver refuses (no second GPU) is reported
        // and passed over.
        let status = set(SHIM_RENDERING_MODE);
        if status == OK {
            note(report, "Profile set to prefer Nvidia dGPU.");
        } else {
            let line = format!("Failed to set GPU preference: {status}. {}", api.error(status));
            log::warn!("{line}");
            report.push(line);
        }
        let status = set(CUDA_SYSMEM_FALLBACK_POLICY);
        if status != OK {
            return Err(api.error(status));
        }
        note(report, "Cuda Sysmem Fallback policy set to 'Prefer No Sysmem Fallback' successfully.");
        let status = set(BACKGROUND_FRAME_RATE_LIMIT);
        if status != OK {
            return Err(api.error(status));
        }
        note(report, "Background application FPS limit applied successfully.");

        let status = unsafe { save_settings(session) };
        if status != OK {
            return Err(api.error(status));
        }
        note(report, "Nerthus profile is saved successfully.");
        Ok(())
    }
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn there_is_no_driver_off_windows() {
        assert_eq!(enforce_optimal_settings(), Err(ProfileError::NoDriver));
    }
}
