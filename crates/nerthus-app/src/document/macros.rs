//! Macros in the project: a Macro node added from the collection (the
//! toolbox's Macro menu: `M.AddToProject`, `Macro.SetupMacro`), and the
//! project made into a macro (`M.CanExport`, `M.ExportMacro`).

use nerthus_engine::macros::MacroAsset;
use nerthus_engine::{Catalogue, NodeData, Terrain};
use serde_json::{json, Value};

use super::Document;

impl Document {
    /// A Macro node running `asset` at `position` on graph tab `tab`
    /// (`Macro { Name, MacroId }`, then `SetupMacro` once its asset is in
    /// the project): an input or output for each of the macro's ports
    /// (required unless optional), the macro's variables as the node's.
    /// The macros it depends on (`dependencies`, as `M.LoadMacro` brings
    /// them), then the macro, join `Project.Macros`.
    pub fn add_macro_node(&mut self, asset: &MacroAsset, dependencies: &[String], position: [f32; 2], tab: usize) -> Option<i32> {
        let spec = Catalogue::get().node("Macro")?;
        let id_text = asset.metadata.id();
        self.checkpoint(format!("Add {}", asset.metadata.name));
        for dependency in dependencies {
            self.add_project_macro(dependency);
        }
        let id = self.new_node_id();
        let mut node = NodeData::new(spec, id, 0);
        node.position = position;
        node.name = asset.metadata.name.clone();
        node.values.insert("MacroId".to_string(), json!(id_text));
        node.values.insert("MacroVersion".to_string(), json!(asset.metadata.version));
        let variables = asset.asset.extra.get("Automation").and_then(|automation| automation.get("Variables")).cloned();
        node.values.insert("Variables".to_string(), variables.unwrap_or_else(|| json!({})));
        if tab != 0 {
            node.base.insert("GraphIndex".to_string(), json!(tab));
        }
        setup_macro_ports(&mut node, &asset.asset.terrain);
        self.project.terrain_mut().nodes.insert(id, node);
        self.add_project_macro(&id_text);
        self.mark_stale(id);
        Some(id)
    }

    /// `Project.AddMacro`: the macro's id in `Project.Macros`, once.
    fn add_project_macro(&mut self, id: &str) {
        if self.project.macro_ids().iter().any(|existing| existing == id) {
            return;
        }
        let macros = self.project.extra.entry("Macros".to_string()).or_insert_with(|| json!([]));
        if !macros.is_array() {
            let items = nerthus_engine::json::collection_items(macros).to_vec();
            *macros = Value::Array(items);
        }
        if let Value::Array(items) = macros {
            items.push(json!(id));
        }
    }

    /// Macros the project uses without listing them (dependencies of its
    /// macros, found as it opens) join `Project.Macros`. Not an edit.
    pub fn list_project_macros(&mut self, ids: &[String]) {
        for id in ids {
            self.add_project_macro(id);
        }
    }

    /// The macros this project's Macro nodes run, each once
    /// (`M.ExportMacro`'s `DependentMacros`).
    pub fn used_macros(&self) -> Vec<String> {
        let mut used: Vec<String> = Vec::new();
        for node in self.terrain().nodes.values().filter(|node| node.type_name == "Macro") {
            let id = node.values.get("MacroId").and_then(Value::as_str).unwrap_or_default().to_string();
            if !used.contains(&id) {
                used.push(id);
            }
        }
        used
    }

    /// `M.CanExport`: whether this project can be exported as a macro, and
    /// what stops it.
    pub fn can_export_macro(&self) -> Result<(), &'static str> {
        let ports: Vec<&NodeData> = self.terrain().nodes.values().filter(|node| node.type_name == "MacroPort").collect();
        let direction = |node: &NodeData| node.values.get("Direction").and_then(Value::as_str).unwrap_or("Input").to_string();
        let has_input = ports.iter().any(|port| port.name == "In" && direction(port) == "Input");
        if !has_input {
            return Err("You must have at least one MacroPort node named \"In\" and set to Input mode.");
        }
        let has_output = ports.iter().any(|port| port.name == "Out" && direction(port) == "Output");
        if !has_output {
            return Err("You must have at least one MacroPort node named \"Out\" and set to Output mode.");
        }
        if self.terrain().nodes.values().any(|node| node.type_name == "Macro") {
            return Err("You cannot have a Macro inside a Macro.");
        }
        if ports.iter().any(|port| port.name == "MacroPort") {
            return Err("You must name your MacroPorts.");
        }
        let mut names: Vec<&str> = ports.iter().map(|port| port.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        if names.len() < ports.len() {
            return Err("You cannot have duplicate MacroPort names.");
        }
        Ok(())
    }

    /// `M.ExportMacro`'s asset (`MacroAsset`): the project's terrain
    /// without groups, notes or graph tabs, its automation with every
    /// variable's default set to its value, typed a component, no build
    /// settings or state. (The project's own variables are left as they
    /// are.)
    pub fn macro_asset_json(&self, metadata: &Value) -> String {
        let source = &self.project.assets[0];
        let mut terrain: Terrain = source.terrain.clone();
        terrain.extra.insert("Groups".to_string(), json!({}));
        terrain.extra.insert("Notes".to_string(), json!({}));
        terrain.extra.insert("GraphTabs".to_string(), json!([]));
        let mut automation = source.extra.get("Automation").cloned().unwrap_or_else(|| json!({}));
        if let Some(Value::Object(variables)) = automation.get_mut("Variables") {
            for variable in variables.values_mut() {
                let Value::Object(fields) = variable else {
                    continue;
                };
                if let Some(value) = fields.get("Value").cloned() {
                    fields.insert("Default".to_string(), value);
                }
            }
        }
        let mut extra = serde_json::Map::new();
        extra.insert("Type".to_string(), json!("Component"));
        extra.insert("Automation".to_string(), automation);
        let asset = nerthus_engine::Asset { terrain, automation: source.automation.clone(), extra };
        nerthus_engine::to_macro_asset_json(&asset, metadata)
    }

    /// A value of the project's metadata (`Session.GetMetadata`), or
    /// `fallback`.
    pub fn project_metadata(&self, key: &str, fallback: &str) -> String {
        let metadata = self.project.extra.get("Metadata");
        let value = metadata.and_then(|metadata| metadata.get(key)).and_then(Value::as_str);
        value.unwrap_or(fallback).to_string()
    }

    /// Notes a value in the project's metadata (Export Macro remembers
    /// what was typed there as `_LastAuthor` and so on).
    pub fn set_project_metadata(&mut self, key: &str, value: &str) {
        let metadata = self.project.extra.entry("Metadata".to_string()).or_insert_with(|| json!({}));
        if let Value::Object(members) = metadata {
            members.insert(key.to_string(), json!(value));
        }
    }
}

/// `Macro.SetupMacro`'s ports: one per `MacroPort` of the macro not named
/// In or Out (an output for an output port; required unless optional),
/// then any port the macro has no `MacroPort` for removed.
fn setup_macro_ports(node: &mut NodeData, terrain: &Terrain) {
    let ios: Vec<&NodeData> = terrain.nodes.values().filter(|candidate| candidate.type_name == "MacroPort").collect();
    for io in ios.iter().filter(|io| io.name != "In" && io.name != "Out") {
        if node.port(&io.name).is_some() {
            continue;
        }
        let is_output = io.values.get("Direction").and_then(Value::as_str) == Some("Output");
        let optional = io.values.get("IsOptional").and_then(Value::as_bool).unwrap_or(false);
        let port_type = match (is_output, optional) {
            (true, false) => "Out, Required",
            (true, true) => "Out",
            (false, false) => "In, Required",
            (false, true) => "In",
        };
        node.add_port(&io.name, port_type);
    }
    node.ports.retain(|port| ios.iter().any(|io| io.name == port.name));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn macro_project() -> Document {
        let mut document = Document::new();
        let input = document.add_node("MacroPort", [0.0, 0.0]).unwrap();
        let blur = document.add_node("Blur", [300.0, 0.0]).unwrap();
        let output = document.add_node("MacroPort", [600.0, 0.0]).unwrap();
        let mask = document.add_node("MacroPort", [0.0, 300.0]).unwrap();
        document.rename(input, "In");
        document.rename(output, "Out");
        document.rename(mask, "Mask");
        document.set_value(output, "Direction", json!("Output"));
        document.set_value(mask, "IsOptional", json!(true));
        document.connect(input, "Out", blur, "In");
        document.connect(blur, "Out", output, "In");
        document
    }

    #[test]
    fn exporting_needs_named_ports() {
        let mut document = macro_project();
        assert_eq!(document.can_export_macro(), Ok(()));
        let extra = document.add_node("MacroPort", [0.0, 600.0]).unwrap();
        assert_eq!(document.can_export_macro(), Err("You must name your MacroPorts."));
        document.rename(extra, "Mask");
        assert_eq!(document.can_export_macro(), Err("You cannot have duplicate MacroPort names."));
    }

    #[test]
    fn macro_nodes_take_the_macros_ports() {
        let source = macro_project();
        let metadata = nerthus_engine::macros::MacroMetadata::for_export("Soften", "Ann", "", "1.0", "", "");
        let metadata_value: Value = serde_json::from_str(&metadata.to_json()).unwrap();
        let text = source.macro_asset_json(&metadata_value);
        let asset = MacroAsset::from_json(&text).unwrap();
        assert_eq!(asset.metadata.id(), "Ann.Soften");

        let mut target = Document::new();
        let id = target.add_macro_node(&asset, &[], [10.0, 20.0], 0).unwrap();
        let node = target.node(id).unwrap();
        let ports: Vec<(&str, &str)> = node.ports.iter().map(|port| (port.name.as_str(), port.port_type.as_str())).collect();
        assert_eq!(ports, [("In", "PrimaryIn, Required"), ("Out", "PrimaryOut"), ("Mask", "In")]);
        assert_eq!(node.name, "Soften");
        assert_eq!(node.values["MacroId"], json!("Ann.Soften"));
        assert_eq!(target.project.macro_ids(), ["Ann.Soften"]);
        target.add_macro_node(&asset, &[], [10.0, 20.0], 0).unwrap();
        assert_eq!(target.project.macro_ids().len(), 1);

        // Its dependencies are listed first, once.
        let mut other = Document::new();
        let dependencies = vec!["Ann.Base".to_string()];
        other.add_macro_node(&asset, &dependencies, [10.0, 20.0], 0).unwrap();
        assert_eq!(other.project.macro_ids(), ["Ann.Base", "Ann.Soften"]);
        assert_eq!(other.used_macros(), ["Ann.Soften"]);
    }
}
