//! The folders the open and save dialogs used last (`MRULocations`),
//! kept between sessions in `locations.mru`: a dictionary of `open` and
//! `save` to a folder, written again whenever one changes. Open dialogs
//! start in `open`, save dialogs in `save`; every save of the project
//! notes its folder as `save`.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

pub const LOCATIONS_FILE: &str = "locations.mru";

/// The key File › Open and the import dialogs use.
pub const OPEN: &str = "open";

/// The key Save As and the export dialogs use.
pub const SAVE: &str = "save";

/// `MRULocations.locations`, and where it is kept (none keeps it in memory
/// only).
#[derive(Clone, Debug, Default)]
pub struct Locations {
    folders: Map<String, Value>,
    file: Option<PathBuf>,
}

impl Locations {
    /// `MRULocations.Load`: the saved dictionary, or an empty one written
    /// at once when there is none. An unreadable file reads as empty.
    pub fn load(file: PathBuf) -> Self {
        if !file.exists() {
            let locations = Self { folders: Map::new(), file: Some(file) };
            locations.save();
            return locations;
        }
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        let text = text.trim_start_matches('\u{feff}');
        let saved = match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(saved)) => saved,
            Ok(_) => Map::new(),
            Err(error) => {
                log::warn!("cannot read {}: {error}", file.display());
                Map::new()
            }
        };
        let mut folders = Map::new();
        for (key, value) in saved {
            if key.starts_with('$') || !value.is_string() {
                continue;
            }
            folders.insert(key, value);
        }
        Self { folders, file: Some(file) }
    }

    /// `GetLocation(key)`: the folder noted for `key`, when it is still a
    /// folder.
    pub fn folder(&self, key: &str) -> Option<PathBuf> {
        let noted = self.folders.get(key).and_then(Value::as_str)?;
        let folder = PathBuf::from(noted);
        folder.is_dir().then_some(folder)
    }

    /// `SetLocation(key, folder)`: noted and written at once.
    pub fn set(&mut self, key: &str, folder: &Path) {
        let text = folder.to_string_lossy().to_string();
        if self.folders.get(key).and_then(Value::as_str) == Some(text.as_str()) {
            return;
        }
        self.folders.insert(key.to_string(), Value::String(text));
        self.save();
    }

    /// Notes the folder a chosen file is in.
    pub fn set_from_file(&mut self, key: &str, file: &Path) {
        if let Some(folder) = file.parent() {
            self.set(key, folder);
        }
    }

    /// `MRULocations.Save`: the dictionary as the settings files are
    /// written, with its `$id`.
    fn save(&self) {
        let Some(file) = &self.file else {
            return;
        };
        let mut written = Map::new();
        written.insert("$id".to_string(), Value::String("1".to_string()));
        for (key, value) in &self.folders {
            written.insert(key.clone(), value.clone());
        }
        let text = serde_json::to_string_pretty(&Value::Object(written)).unwrap_or_default();
        let Some(folder) = file.parent() else {
            return;
        };
        let Some(name) = file.file_name().map(|name| name.to_string_lossy().to_string()) else {
            return;
        };
        super::write_file(folder, &name, &text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_are_kept_between_sessions() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|duration| duration.as_nanos()).unwrap_or(0);
        let folder = std::env::temp_dir().join(format!("nerthus-locations-{nanos}"));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join(LOCATIONS_FILE);

        // A first run writes an empty dictionary.
        let mut locations = Locations::load(file.clone());
        let written: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(written, serde_json::json!({"$id": "1"}));
        assert_eq!(locations.folder(OPEN), None);

        locations.set_from_file(OPEN, &folder.join("Ridge.terrain"));
        locations.set(SAVE, Path::new("/no/such/folder"));
        let reloaded = Locations::load(file);
        assert_eq!(reloaded.folder(OPEN), Some(folder.clone()));
        // A folder that is gone is not offered.
        assert_eq!(reloaded.folder(SAVE), None);
        let _ = std::fs::remove_dir_all(folder);
    }
}
