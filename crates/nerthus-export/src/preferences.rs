//! What builds read from the user's preferences (`Preferences`, saved as
//! `preferences.options` in the data folder): the file name delimiter, the
//! builds folder, where baked maps are kept, the compute devices to
//! activate, whether results are clamped to 0–1 and whether results go as
//! soon as the build is done with them. The editor and `nerthus-build`
//! read them alike.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::build::ExportPreferences;
use crate::destination::{default_builds_root, to_path};

/// The preferences as saved: their members by name, sections (`Defaults`,
/// `Locations`, …) as objects.
pub type PreferenceValues = Map<String, Value>;

/// Reads the preferences file. A missing file holds no members (every
/// preference at its default); one that cannot be read, or holds something
/// other than a JSON object, is an error.
pub fn load(path: &Path) -> Result<PreferenceValues, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(error) => return Err(format!("Cannot read {}: {error}", path.display())),
    };
    let parsed = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}'));
    match parsed {
        Ok(Value::Object(members)) => Ok(members),
        Ok(_) => Err(format!("{} does not hold preferences", path.display())),
        Err(error) => Err(format!("Cannot read {}: {error}", path.display())),
    }
}

fn section<'a>(values: &'a PreferenceValues, name: &str) -> Option<&'a PreferenceValues> {
    values.get(name).and_then(Value::as_object)
}

/// `Defaults.FilenameDelimiter`: between a node's name and its port's in
/// the files a build writes.
pub fn filename_delimiter(values: &PreferenceValues) -> String {
    let defaults = section(values, "Defaults");
    let delimiter = defaults.and_then(|defaults| defaults.get("FilenameDelimiter")).and_then(Value::as_str);
    delimiter.unwrap_or("_").to_string()
}

/// `Locations.Builds`: where builds go (`<Builds>`). An empty or missing
/// folder is the default one, as the preferences fill empty folders in when
/// they load.
pub fn builds_folder(values: &PreferenceValues) -> PathBuf {
    let locations = section(values, "Locations");
    let builds = locations.and_then(|locations| locations.get("Builds")).and_then(Value::as_str).unwrap_or_default();
    if builds.is_empty() {
        return default_builds_root();
    }
    to_path(builds)
}

/// `Locations.Cache` when `Locations.CentralizeCacheBake` is on: where
/// baked maps are kept instead of beside the project.
pub fn central_bake_cache(values: &PreferenceValues) -> Option<PathBuf> {
    let locations = section(values, "Locations")?;
    let centralized = locations.get("CentralizeCacheBake").and_then(Value::as_bool).unwrap_or(false);
    if !centralized {
        return None;
    }
    let cache = locations.get("Cache").and_then(Value::as_str).unwrap_or_default();
    Some(to_path(cache))
}

/// `Compute.Backends`: the devices activated at start (Options › Compute
/// saves the active ones). The list may be saved with or without
/// `$values`.
pub fn compute_backends(values: &PreferenceValues) -> Vec<String> {
    let Some(backends) = section(values, "Compute").and_then(|compute| compute.get("Backends")) else {
        return Vec::new();
    };
    let list = backends.get("$values").unwrap_or(backends);
    let Some(ids) = list.as_array() else {
        return Vec::new();
    };
    let names = ids.iter().filter_map(Value::as_str);
    names.map(str::to_string).collect()
}

/// `AllowValuesBeyondRange`: node results keep values outside 0–1. The
/// preference has no page in Options; it is read from the file.
pub fn allow_values_beyond_range(values: &PreferenceValues) -> bool {
    values.get("AllowValuesBeyondRange").and_then(Value::as_bool).unwrap_or(false)
}

/// `Cache.OffloadBuildCache` (Options › Build).
pub fn offload_build_cache(values: &PreferenceValues) -> bool {
    let cache = section(values, "Cache");
    let offload = cache.and_then(|cache| cache.get("OffloadBuildCache")).and_then(Value::as_bool);
    offload.unwrap_or(false)
}

impl ExportPreferences {
    /// What a build takes from the preferences.
    pub fn from_preferences(values: &PreferenceValues) -> Self {
        Self {
            filename_delimiter: filename_delimiter(values),
            builds_root: builds_folder(values),
            central_bake_cache: central_bake_cache(values),
            allow_values_beyond_range: allow_values_beyond_range(values),
            offload_build_cache: offload_build_cache(values),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn values(saved: Value) -> PreferenceValues {
        saved.as_object().cloned().unwrap()
    }

    #[test]
    fn missing_preferences_are_the_defaults() {
        let empty = PreferenceValues::new();
        let preferences = ExportPreferences::from_preferences(&empty);
        assert_eq!(preferences.filename_delimiter, "_");
        assert_eq!(preferences.builds_root, default_builds_root());
        assert_eq!(preferences.central_bake_cache, None);
        assert!(compute_backends(&empty).is_empty());
    }

    #[test]
    fn saved_preferences_are_read() {
        let saved = values(json!({
            "Compute": {"$id": "3", "Backends": {"$id": "4", "$values": ["Native/Cpu/Native/Apple/Apple M3/cpu"]}},
            "Defaults": {"$id": "5", "FilenameDelimiter": "-"},
            "Locations": {"$id": "6", "Builds": "/work/builds", "Cache": "/work/cache", "CentralizeCacheBake": true}
        }));
        let preferences = ExportPreferences::from_preferences(&saved);
        assert_eq!(preferences.filename_delimiter, "-");
        assert_eq!(preferences.builds_root, PathBuf::from("/work/builds"));
        assert_eq!(preferences.central_bake_cache, Some(PathBuf::from("/work/cache")));
        assert_eq!(compute_backends(&saved), ["Native/Cpu/Native/Apple/Apple M3/cpu"]);
        assert!(!preferences.allow_values_beyond_range);
        assert!(!preferences.offload_build_cache);
    }

    #[test]
    fn range_and_purge_preferences_are_read() {
        let saved = values(json!({"AllowValuesBeyondRange": true, "Cache": {"$id": "2", "OffloadBuildCache": true}}));
        let preferences = ExportPreferences::from_preferences(&saved);
        assert!(preferences.allow_values_beyond_range);
        assert!(preferences.offload_build_cache);
    }

    #[test]
    fn plain_lists_and_empty_folders_read_too() {
        let saved = values(json!({"Compute": {"Backends": ["wgpu/GpuDiscrete/Vulkan/NVIDIA/RTX/1"]}, "Locations": {"Builds": "", "Cache": "/c"}}));
        assert_eq!(compute_backends(&saved), ["wgpu/GpuDiscrete/Vulkan/NVIDIA/RTX/1"]);
        assert_eq!(builds_folder(&saved), default_builds_root());
        assert_eq!(central_bake_cache(&saved), None);
    }

    #[test]
    fn a_missing_file_has_no_members_and_a_broken_one_fails() {
        let folder = std::env::temp_dir().join(format!("nerthus-preferences-{}", std::process::id()));
        fs::create_dir_all(&folder).unwrap();
        let missing = folder.join("missing.options");
        assert!(load(&missing).unwrap().is_empty());
        let broken = folder.join("broken.options");
        fs::write(&broken, "{ not json").unwrap();
        assert!(load(&broken).is_err());
        let saved = folder.join("saved.options");
        fs::write(&saved, "\u{feff}{\"Defaults\": {\"FilenameDelimiter\": \".\"}}").unwrap();
        assert_eq!(filename_delimiter(&load(&saved).unwrap()), ".");
        fs::remove_dir_all(&folder).unwrap();
    }
}
