//! The engine preferences (`Preferences`, saved as
//! `preferences.options`): caches, compute backends, export defaults,
//! folders and network builds. Nerthus uses the compute backends, the
//! export defaults, the folders, the caches and whether results are
//! clamped to 0–1.

use std::path::PathBuf;

use nerthus_engine::format_names::PURE_RAW_FORMAT_JSON;
use nerthus_export::build::ExportPreferences;
use nerthus_export::FileFormat;
use serde_json::{json, Map, Value};

use crate::user::members::{initial_values, list, object, read, value, write, Member};

/// `CacheOptions`.
const CACHE: [Member; 4] = [
    value("SmartCache", "false", "false"),
    value("AlwaysSaveCache", "false", "false"),
    value("NodeIdleThreshold", "600", "600"),
    value("OffloadBuildCache", "false", "false"),
];

/// `Compute`.
const COMPUTE: [Member; 1] = [list("Backends", "[]")];

/// `Defaults` (`FileFormats` starts at Pure Raw).
const DEFAULTS: [Member; 6] = [
    value("HeightfieldFormat", "\"EXR\"", PURE_RAW_FORMAT_JSON),
    value("MaskFormat", "\"PNG16\"", PURE_RAW_FORMAT_JSON),
    value("ColorFormat", "\"PNG16\"", PURE_RAW_FORMAT_JSON),
    value("FilenameDelimiter", "\"_\"", "null"),
    value("RemovePrimaryPortName", "false", "false"),
    value("ShowRegionReferenceImage", "false", "false"),
];

/// `Locations`: the folders start under the user's documents (set when the
/// preferences are made).
const LOCATIONS: [Member; 13] = [
    value("Proxy", "null", "null"),
    value("Projects", "null", "null"),
    value("Documents", "null", "null"),
    value("Builds", "null", "null"),
    value("Cache", "null", "null"),
    value("NetworkJobs", "null", "null"),
    value("CentralizeCacheHD", "false", "false"),
    value("CentralizeCacheBake", "false", "false"),
    value("CentralizeCacheFreeze", "false", "false"),
    value("UseProxy", "false", "false"),
    value("LicenseServer", "null", "\"\""),
    value("Custom", "null", "\"\""),
    value("ShowCustomOnly", "false", "false"),
];

/// `NetworkBuilds`.
const NETWORK: [Member; 3] = [list("KnownWorkers", "[]"), value("Port", "35771", "0"), value("RetryThreshold", "3", "0")];

/// `Preferences`' members in declaration order.
const MEMBERS: [Member; 13] = [
    value("OptionsVersion", "3", "0"),
    object("Cache", &CACHE),
    object("Compute", &COMPUTE),
    object("Defaults", &DEFAULTS),
    value("GCSensitivity", "0.800000011920929", "0.0"),
    object("Locations", &LOCATIONS),
    object("Network", &NETWORK),
    value("AllowValuesBeyondRange", "false", "false"),
    value("SendUsage", "false", "false"),
    value("SendExceptionReports", "false", "false"),
    value("ShowOrphanVariableWarning", "true", "true"),
    value("HoudiniTimeout", "5", "0"),
    value("Heartbeat", "1.0", "0.0"),
];

/// `Locations`' folders and where each starts (`_defaultDocs`, …): the
/// user's documents, `Nerthus`, then the folder's own name.
const FOLDERS: [(&str, Option<&str>); 5] = [("Documents", None), ("Projects", Some("Projects")), ("Builds", Some("Builds")), ("Cache", Some("Caches")), ("NetworkJobs", Some("NetworkJobs"))];

/// The preferences as they stand, and whether they changed since saved.
#[derive(Clone, Debug)]
pub struct Preferences {
    values: Map<String, Value>,
    changed: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self::new()
    }
}

impl Preferences {
    /// `new Preferences()`.
    pub fn new() -> Self {
        let mut preferences = Self { values: initial_values(&MEMBERS), changed: false };
        preferences.fill_missing_folders();
        preferences
    }

    /// The preferences a saved file holds; folders it leaves empty go back
    /// to their defaults (`Locations` does this as it loads).
    pub fn from_json(saved: &Value) -> Self {
        let mut preferences = Self { values: read(&MEMBERS, saved), changed: false };
        preferences.fill_missing_folders();
        preferences
    }

    pub fn to_json(&self) -> String {
        let mut next_id = 1;
        let written = write(&MEMBERS, &self.values, &mut next_id);
        serde_json::to_string_pretty(&written).unwrap_or_default()
    }

    pub fn changed(&self) -> bool {
        self.changed
    }

    pub fn mark_saved(&mut self) {
        self.changed = false;
    }

    /// Saves on the next chance even though nothing was edited.
    pub fn mark_changed(&mut self) {
        self.changed = true;
    }

    /// A member's value as it stands (null when missing): one of
    /// `section`'s (`Cache`, `Defaults`, `Locations`, …), or a top-level
    /// one for an empty section. Options' pages work on members by name.
    pub fn member(&self, section: &str, name: &str) -> Value {
        let found = if section.is_empty() { self.values.get(name) } else { self.section(section).and_then(|members| members.get(name)) };
        found.cloned().unwrap_or(Value::Null)
    }

    /// Sets a member by name (see [`Preferences::member`]).
    pub fn set_member(&mut self, section: &str, name: &str, value: Value) {
        if self.member(section, name) == value {
            return;
        }
        if section.is_empty() {
            self.values.insert(name.to_string(), value);
        } else {
            let members = self.values.entry(section).or_insert_with(|| Value::Object(Map::new()));
            if let Value::Object(members) = members {
                members.insert(name.to_string(), value);
            }
        }
        self.changed = true;
    }

    /// `Locations.Proxy` from `proxy.txt`, for the session: it is saved
    /// only along with a change.
    pub fn set_session_proxy(&mut self, address: &str) {
        let changed = self.changed;
        self.set_member("Locations", "Proxy", json!(address));
        self.changed = changed;
    }

    fn section(&self, name: &str) -> Option<&Map<String, Value>> {
        self.values.get(name).and_then(Value::as_object)
    }

    fn fill_missing_folders(&mut self) {
        let builds_root = nerthus_export::destination::default_builds_root();
        let storage_folder = builds_root.parent().map(PathBuf::from).unwrap_or_default();
        let Some(Value::Object(locations)) = self.values.get_mut("Locations") else {
            return;
        };
        for (name, child) in FOLDERS {
            let current = locations.get(name).and_then(Value::as_str).unwrap_or_default();
            if !current.is_empty() {
                continue;
            }
            let folder = match child {
                Some(child) => storage_folder.join(child),
                None => storage_folder.clone(),
            };
            locations.insert(name.to_string(), json!(folder.to_string_lossy()));
        }
    }

    /// `Compute.Backends`: the devices activated at start (Options ›
    /// Compute saves the active ones).
    pub fn compute_backends(&self) -> Vec<String> {
        let compute = self.section("Compute");
        let backends = compute.and_then(|compute| compute.get("Backends")).and_then(Value::as_array);
        let ids = backends.map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_string).collect());
        ids.unwrap_or_default()
    }

    pub fn set_compute_backends(&mut self, ids: &[String]) {
        if self.compute_backends() == ids {
            return;
        }
        let list: Vec<Value> = ids.iter().map(|id| json!(id)).collect();
        let compute = self.values.entry("Compute").or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(compute) = compute {
            compute.insert("Backends".to_string(), Value::Array(list));
        }
        self.changed = true;
    }

    /// `Locations.Builds`: where builds go (`<Builds>`).
    pub fn builds_folder(&self) -> PathBuf {
        nerthus_export::preferences::builds_folder(&self.values)
    }

    /// `Paths.Docs`: the documents folder (`Locations.Documents`).
    pub fn documents_folder(&self) -> PathBuf {
        let locations = self.section("Locations");
        let documents = locations.and_then(|locations| locations.get("Documents")).and_then(Value::as_str).unwrap_or_default();
        nerthus_export::build::project_path_text(documents)
    }

    /// `Paths.Projects`: where file dialogs start (`Locations.Projects`).
    pub fn projects_folder(&self) -> PathBuf {
        let locations = self.section("Locations");
        let projects = locations.and_then(|locations| locations.get("Projects")).and_then(Value::as_str).unwrap_or_default();
        nerthus_export::build::project_path_text(projects)
    }

    /// `Paths.Screenshots`: `Screenshots` in the documents folder
    /// (`Locations.Documents`).
    pub fn screenshots_folder(&self) -> PathBuf {
        let locations = self.section("Locations");
        let documents = locations.and_then(|locations| locations.get("Documents")).and_then(Value::as_str).unwrap_or_default();
        let documents_folder = nerthus_export::build::project_path_text(documents);
        documents_folder.join("Screenshots")
    }

    /// `Defaults.HeightfieldFormat`: what Mark for Export starts with.
    pub fn heightfield_format(&self) -> FileFormat {
        let defaults = self.section("Defaults");
        let name = defaults.and_then(|defaults| defaults.get("HeightfieldFormat")).and_then(Value::as_str).unwrap_or("EXR");
        FileFormat::from_name(name).unwrap_or(FileFormat::Exr)
    }

    /// `Locations.Cache` when `Locations.CentralizeCacheFreeze` is on: where
    /// Smart Cache and Cache to Disk keep previews instead of beside the
    /// project.
    pub fn central_preview_cache(&self) -> Option<PathBuf> {
        let locations = self.section("Locations")?;
        let centralized = locations.get("CentralizeCacheFreeze").and_then(Value::as_bool).unwrap_or(false);
        if !centralized {
            return None;
        }
        let cache = locations.get("Cache").and_then(Value::as_str).unwrap_or_default();
        Some(nerthus_export::build::project_path_text(cache))
    }

    /// `CacheOptions`: Use Smart Cache, Always Cache to Disk on Close and
    /// the Idle Threshold in seconds.
    pub fn cache_options(&self) -> (bool, bool, u64) {
        let cache = self.section("Cache");
        let flag = |name: &str| cache.and_then(|cache| cache.get(name)).and_then(Value::as_bool).unwrap_or(false);
        let threshold = cache.and_then(|cache| cache.get("NodeIdleThreshold")).and_then(Value::as_u64).unwrap_or(600);
        (flag("SmartCache"), flag("AlwaysSaveCache"), threshold)
    }

    /// What a build reads from the preferences.
    pub fn export_preferences(&self) -> ExportPreferences {
        ExportPreferences::from_preferences(&self.values)
    }

    /// What the engine reads from the preferences: `AllowValuesBeyondRange`
    /// (no page in Options shows it).
    pub fn engine_preferences(&self) -> nerthus_engine::EnginePreferences {
        nerthus_engine::EnginePreferences {
            allow_values_beyond_range: nerthus_export::preferences::allow_values_beyond_range(&self.values),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_write_with_nested_ids() {
        let preferences = Preferences::new();
        let written: Value = serde_json::from_str(&preferences.to_json()).unwrap();
        assert_eq!(written["$id"], json!("1"));
        assert_eq!(written["OptionsVersion"], json!(3));
        assert_eq!(written["Cache"], json!({"$id": "2"}));
        assert_eq!(written["Compute"]["$id"], json!("3"));
        assert_eq!(written["Compute"]["Backends"], json!({"$id": "4", "$values": []}));
        assert_eq!(written["Defaults"]["$id"], json!("5"));
        assert_eq!(written["Defaults"]["HeightfieldFormat"], json!("EXR"));
        assert_eq!(written["Locations"]["$id"], json!("6"));
        assert!(written["Locations"]["Builds"].as_str().unwrap().ends_with("Builds"));
        assert_eq!(written["Network"]["KnownWorkers"]["$id"], json!("8"));
    }

    #[test]
    fn compute_backends_read_written_lists() {
        let saved = json!({"Compute": {"$id": "3", "Backends": {"$id": "4", "$values": ["wgpu/GpuDiscrete/Vulkan/NVIDIA/RTX/1"]}}});
        let mut preferences = Preferences::from_json(&saved);
        assert_eq!(preferences.compute_backends(), ["wgpu/GpuDiscrete/Vulkan/NVIDIA/RTX/1"]);
        assert!(!preferences.changed());
        preferences.set_compute_backends(&["Native/Cpu/Native/Apple/Apple M3/cpu".to_string()]);
        assert!(preferences.changed());
        let written: Value = serde_json::from_str(&preferences.to_json()).unwrap();
        assert_eq!(written["Compute"]["Backends"]["$values"], json!(["Native/Cpu/Native/Apple/Apple M3/cpu"]));
    }

    #[test]
    fn saved_folders_and_formats_are_used() {
        let saved = json!({"Defaults": {"FilenameDelimiter": "-", "HeightfieldFormat": "PNG16"}, "Locations": {"Builds": "/tmp/builds", "Projects": ""}});
        let preferences = Preferences::from_json(&saved);
        assert_eq!(preferences.export_preferences().filename_delimiter, "-");
        assert_eq!(preferences.heightfield_format(), FileFormat::Png16);
        assert_eq!(preferences.builds_folder(), PathBuf::from("/tmp/builds"));
        let projects = preferences.section("Locations").unwrap()["Projects"].as_str().unwrap().to_string();
        assert!(projects.ends_with("Projects"));
    }
}
