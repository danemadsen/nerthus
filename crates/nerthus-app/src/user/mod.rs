//! The user's settings (`User` and `Paths`): the interface options
//! (`user.options`), the engine preferences (`preferences.options`), the
//! files opened lately (`recent-files.mru`) and the folders the file
//! dialogs used last (`locations.mru`). They live in the user's Nerthus
//! folder: `~/.nerthus` on macOS and Linux, `%LOCALAPPDATA%\Nerthus` on
//! Windows.

mod graph_options;
pub mod locations;
mod members;
mod options;
mod preferences;
mod usage;

use std::path::{Path, PathBuf};

pub use graph_options::{ToolboxMode, ToolboxStyle};
pub use locations::Locations;
pub use options::{IconState, UserOptions};
pub use preferences::Preferences;
pub use usage::UsageStats;

const OPTIONS_FILE: &str = "user.options";
const PREFERENCES_FILE: &str = nerthus_engine::paths::PREFERENCES_FILE;
const RECENT_FILE: &str = "recent-files.mru";
/// A proxy address for the session, over the preferences' own.
const PROXY_FILE: &str = "proxy.txt";
/// `MRUList`'s default length (`MRU_TOR`).
const RECENT_LIMIT: usize = 9;

/// The user's Nerthus folder (see [`nerthus_engine::paths`]).
pub fn data_folder() -> Option<PathBuf> {
    nerthus_engine::paths::data_folder()
}

/// Everything kept for the user between runs.
#[derive(Debug)]
pub struct User {
    pub options: UserOptions,
    pub preferences: Preferences,
    pub recent: RecentFiles,
    /// The folders the open and save dialogs used last.
    pub locations: Locations,
    /// How often node types are added and connected (`usage.stats`).
    pub usage: UsageStats,
    /// Where the files are; none keeps the settings in memory only.
    folder: Option<PathBuf>,
}

impl User {
    /// Settings that are never written (a fresh `Options` and
    /// `Preferences`).
    pub fn in_memory() -> Self {
        Self {
            options: UserOptions::new(),
            preferences: Preferences::new(),
            recent: RecentFiles::default(),
            locations: Locations::default(),
            usage: UsageStats::default(),
            folder: None,
        }
    }

    /// `App.LoadPreferences`: the saved options and preferences (the
    /// defaults for any that are missing or unreadable), with
    /// `proxy.txt`'s address as the proxy; or in Safe Mode
    /// (`--safe-mode`) the defaults, written over the saved ones only once
    /// something changes.
    pub fn load(safe_mode: bool) -> Self {
        let Some(folder) = data_folder() else {
            log::warn!("no home folder: settings will not be saved");
            return Self::in_memory();
        };
        let (options, preferences) = if safe_mode {
            log::warn!("SAFE MODE: User initiated Safe Mode.");
            (UserOptions::new(), Preferences::new())
        } else {
            read_settings(&folder)
        };
        let recent = RecentFiles::load(folder.join(RECENT_FILE));
        let locations = Locations::load(folder.join(locations::LOCATIONS_FILE));
        let usage = UsageStats::load(folder.join(usage::USAGE_FILE));
        Self { options, preferences, recent, locations, usage, folder: Some(folder) }
    }

    /// Writes the options and preferences that changed since last saved.
    pub fn save_if_changed(&mut self) {
        self.usage.save_if_changed();
        let Some(folder) = self.folder.clone() else {
            return;
        };
        if self.options.changed() && write_file(&folder, OPTIONS_FILE, &self.options.to_json()) {
            self.options.mark_saved();
        }
        if self.preferences.changed() && write_file(&folder, PREFERENCES_FILE, &self.preferences.to_json()) {
            self.preferences.mark_saved();
        }
    }

    /// `--proxy`: the proxy turned on with this address, and saved.
    pub fn set_proxy(&mut self, address: &str) {
        log::info!("Setting proxy by CLI to {address}");
        self.preferences.set_member("Locations", "UseProxy", serde_json::json!(true));
        self.preferences.set_member("Locations", "Proxy", serde_json::json!(address));
    }

    /// `MRU_TOR.AddItem`: a project opened or saved, kept on disk at once.
    pub fn add_recent(&mut self, project: &Path) {
        self.recent.add(project);
        let Some(folder) = self.folder.clone() else {
            return;
        };
        write_file(&folder, RECENT_FILE, &self.recent.to_text());
    }
}

/// The options and preferences saved in the folder, the defaults for a
/// file that is missing or unreadable. A first run writes the files; a
/// file that is there but unreadable stays until something changes.
/// `proxy.txt` sets the proxy's address for the session.
fn read_settings(folder: &Path) -> (UserOptions, Preferences) {
    let options_path = folder.join(OPTIONS_FILE);
    let preferences_path = folder.join(PREFERENCES_FILE);
    let mut options = read_json(&options_path).map(|saved| UserOptions::from_json(&saved)).unwrap_or_default();
    let mut preferences = read_json(&preferences_path).map(|saved| Preferences::from_json(&saved)).unwrap_or_default();
    if !options_path.exists() {
        options.mark_changed();
    }
    if !preferences_path.exists() {
        preferences.mark_changed();
    }
    if let Ok(address) = std::fs::read_to_string(folder.join(PROXY_FILE)) {
        preferences.set_session_proxy(address.trim());
    }
    (options, preferences)
}

/// A settings file's JSON, or nothing (logged) when it is missing or bad.
fn read_json(path: &Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.trim_start_matches('\u{feff}');
    match serde_json::from_str(text) {
        Ok(saved) => Some(saved),
        Err(error) => {
            log::warn!("cannot read {}: {error}; using defaults", path.display());
            None
        }
    }
}

/// Writes a file in the folder (made if need be) through a temporary
/// file, so a crash never leaves half a file. Returns whether it worked.
fn write_file(folder: &Path, name: &str, text: &str) -> bool {
    if let Err(error) = std::fs::create_dir_all(folder) {
        log::warn!("cannot make {}: {error}", folder.display());
        return false;
    }
    let path = folder.join(name);
    let temporary = folder.join(format!("{name}.tmp"));
    let written = std::fs::write(&temporary, text).and_then(|_| std::fs::rename(&temporary, &path));
    match written {
        Ok(()) => true,
        Err(error) => {
            log::warn!("cannot write {}: {error}", path.display());
            false
        }
    }
}

/// `MRUList`: the latest first, each once, at most nine; a text file of
/// one path per line.
#[derive(Clone, Debug, Default)]
pub struct RecentFiles {
    files: Vec<PathBuf>,
}

impl RecentFiles {
    fn load(path: PathBuf) -> Self {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let lines = text.lines().filter(|line| !line.trim().is_empty());
        let files = lines.map(PathBuf::from).collect();
        Self { files }
    }

    fn add(&mut self, project: &Path) {
        let project = project.to_path_buf();
        self.files.retain(|file| *file != project);
        self.files.insert(0, project);
        self.files.truncate(RECENT_LIMIT);
    }

    fn to_text(&self) -> String {
        let lines: Vec<String> = self.files.iter().map(|file| file.to_string_lossy().to_string()).collect();
        let mut text = lines.join("\n");
        text.push('\n');
        text
    }

    /// The files, latest first.
    pub fn files(&self) -> &[PathBuf] {
        &self.files
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_files_keep_nine_latest_first() {
        let mut recent = RecentFiles::default();
        for index in 0..12 {
            recent.add(Path::new(&format!("/projects/{index}.terrain")));
        }
        recent.add(Path::new("/projects/5.terrain"));
        let files = recent.files();
        assert_eq!(files.len(), 9);
        assert_eq!(files[0], PathBuf::from("/projects/5.terrain"));
        assert_eq!(files[1], PathBuf::from("/projects/11.terrain"));
        assert_eq!(files.iter().filter(|file| file.ends_with("5.terrain")).count(), 1);
    }

    #[test]
    fn settings_save_and_load_from_their_folder() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|duration| duration.as_nanos()).unwrap_or(0);
        let folder = std::env::temp_dir().join(format!("nerthus-user-{nanos}"));
        let mut user = User { folder: Some(folder.clone()), ..User::in_memory() };
        user.options.set_expand_data_view(false);
        user.save_if_changed();
        user.add_recent(Path::new("/projects/a.terrain"));
        assert!(!user.options.changed());

        let options_text = std::fs::read_to_string(folder.join(OPTIONS_FILE)).unwrap();
        let saved: serde_json::Value = serde_json::from_str(&options_text).unwrap();
        assert!(!UserOptions::from_json(&saved).expand_data_view());
        let recent = RecentFiles::load(folder.join(RECENT_FILE));
        assert_eq!(recent.files(), [PathBuf::from("/projects/a.terrain")]);
        // Unchanged preferences are not written.
        assert!(!folder.join(PREFERENCES_FILE).exists());
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn proxy_text_sets_the_address_without_saving_it() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|duration| duration.as_nanos()).unwrap_or(0);
        let folder = std::env::temp_dir().join(format!("nerthus-proxy-{nanos}"));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(OPTIONS_FILE), UserOptions::new().to_json()).unwrap();
        std::fs::write(folder.join(PREFERENCES_FILE), Preferences::new().to_json()).unwrap();
        std::fs::write(folder.join(PROXY_FILE), "http://proxy:8080\r\n").unwrap();

        let (options, preferences) = read_settings(&folder);
        assert_eq!(preferences.member("Locations", "Proxy"), serde_json::json!("http://proxy:8080"));
        assert_eq!(preferences.member("Locations", "UseProxy"), serde_json::json!(false));
        assert!(!preferences.changed());
        assert!(!options.changed());
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn the_proxy_option_turns_the_proxy_on_and_saves_it() {
        let mut user = User::in_memory();
        user.set_proxy("http://proxy:8080");
        assert_eq!(user.preferences.member("Locations", "Proxy"), serde_json::json!("http://proxy:8080"));
        assert_eq!(user.preferences.member("Locations", "UseProxy"), serde_json::json!(true));
        assert!(user.preferences.changed());
    }
}
