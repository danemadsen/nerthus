//! Build profiles (Build Settings › Profiles): named copies of the build
//! settings kept in the project (`Asset.BuildProfiles`), which a build can
//! be run with by name (`nerthus-build build -p`) and which batch builds
//! list. Saving or deleting a profile leaves no undo step; loading one
//! changes the build settings, which can be undone as any change to them.

use nerthus_engine::region;
use nerthus_export::build::BuildDefinition;
use nerthus_export::swarm;
use serde_json::{json, Value};

use crate::document::Document;

impl Document {
    /// The profiles' names, in the file's order.
    pub fn build_profiles(&self) -> Vec<String> {
        swarm::profile_names(&self.project.assets[0])
    }

    /// A profile's settings as builds read them.
    pub fn build_profile(&self, name: &str) -> Option<BuildDefinition> {
        swarm::profile_json(&self.project.assets[0], name).map(|definition| BuildDefinition::from_json(Some(definition)))
    }

    /// `btnProfileSaveAs_Click`, `btnProfileSave_Click`: the build settings
    /// as they are now kept as the profile `name` (a copy), replacing a
    /// profile of that name in its place or added after the others.
    pub fn save_build_profile(&mut self, name: &str) {
        let extra = &mut self.project.assets[0].extra;
        let definition = extra.get("BuildDefinition").cloned().unwrap_or_else(|| json!({}));
        let profiles = extra.entry("BuildProfiles").or_insert_with(|| json!({}));
        if !profiles.is_object() {
            *profiles = json!({});
        }
        if let Some(profiles) = profiles.as_object_mut() {
            profiles.insert(name.to_string(), definition);
        }
        self.dirty = true;
    }

    /// `btnProfileDelete_Click`, once confirmed.
    pub fn delete_build_profile(&mut self, name: &str) {
        let profiles = self.project.assets[0].extra.get_mut("BuildProfiles").and_then(Value::as_object_mut);
        if let Some(profiles) = profiles
            && profiles.shift_remove(name).is_some()
        {
            self.dirty = true;
        }
    }

    /// `btnProfileLoad_Click`: the profile's settings become the build
    /// settings, and when it builds a region the terrain's region of that
    /// name takes its size (`Portion.Size` is copied back into
    /// `Terrain.Regions`).
    pub fn load_build_profile(&mut self, name: &str) {
        let Some(profile) = swarm::profile_json(&self.project.assets[0], name).cloned() else {
            return;
        };
        self.checkpoint("Build settings");
        let portion = region::portion(&profile);
        self.project.assets[0].extra.insert("BuildDefinition".to_string(), profile);
        let Some(portion) = portion else {
            return;
        };
        let stored = self.regions().into_iter().find(|candidate| candidate.name == portion.name);
        if let Some(mut stored) = stored
            && stored.size != portion.size
        {
            stored.size = portion.size;
            self.update_region(&portion.name, stored);
        }
    }
}
