//! Where Nerthus keeps the user's files (`Paths.Data` and `Paths.Settings`,
//! in one folder): the settings, the
//! recent files, and the compute files `cpuonly.dat` and `Benchmark.json`.

use std::path::PathBuf;

/// The user's Nerthus folder: `~/.nerthus` on macOS and Linux,
/// `%LOCALAPPDATA%\Nerthus` on Windows.
pub fn data_folder() -> Option<PathBuf> {
    let base = directories::BaseDirs::new()?;
    if cfg!(target_os = "windows") {
        return Some(base.data_local_dir().join("Nerthus"));
    }
    Some(base.home_dir().join(".nerthus"))
}

/// The engine preferences' file name in the data folder.
pub const PREFERENCES_FILE: &str = "preferences.options";
