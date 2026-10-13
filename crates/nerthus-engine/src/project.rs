//! Projects (`.terrain` files). The parts the engine works with are
//! typed; everything else is kept as the file's JSON, so saving a
//! project does not lose information Nerthus does not model yet.
//!
//! Projects are serialized with Newtonsoft (`PreserveReferencesHandling.All`,
//! `TypeNameHandling.Auto`, `DefaultValueHandling.IgnoreAndPopulate`): node
//! parameters equal to their defaults are left out and come back as the
//! node's defaults, other missing members come back as zero/false/null.

use indexmap::IndexMap;
use serde_json::{Map as JsonObject, Value};
use thiserror::Error;

use crate::automation::Automation;
use crate::catalogue::{Catalogue, NodeSpec};
use crate::json::{collection_items, parse_lenient, strip_bookkeeping, strip_reference_ids, References};
use crate::map::RenderIntent;

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("the project is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the project has no terrain asset")]
    NoAsset,
    #[error("{0}")]
    Invalid(String),
}

/// A connection, stored on the input port it feeds (`ConnectionRecord`).
#[derive(Clone, Debug, PartialEq)]
pub struct ConnectionRecord {
    pub from: i32,
    pub to: i32,
    pub from_port: String,
    pub to_port: String,
    pub is_valid: bool,
}

/// A node port (`Port`).
#[derive(Clone, Debug)]
pub struct PortData {
    pub name: String,
    /// `PortType` flags as written (`"PrimaryIn, Required"`).
    pub port_type: String,
    pub record: Option<ConnectionRecord>,
    pub is_exporting: bool,
    /// Whether the port points back at its node (`Parent`); ports read
    /// without one are saved without one.
    pub has_parent: bool,
    /// Other serialized port members (`DisplayName`, `PortalState`, …).
    pub extra: JsonObject<String, Value>,
    /// The intent the port was created with (`Port.RenderIntent`). It is
    /// not saved: a new node's ports have their type's, ports read
    /// from a file or added later have none.
    pub declared_intent: Option<RenderIntent>,
}

/// `PortType` flag values.
pub mod port_flags {
    pub const IN: u32 = 1;
    pub const OUT: u32 = 2;
    pub const PRIMARY: u32 = 4;
    pub const REQUIRED: u32 = 8;
    pub const AGNOSTIC: u32 = 0x10;
    pub const COLLAPSED: u32 = 0x20;
    pub const POST_PROCESS: u32 = 0x40;
}

/// The value of one `PortType` member name (combined names included).
fn port_flag(name: &str) -> u32 {
    use port_flags::*;
    match name {
        "In" => IN,
        "Out" => OUT,
        "Primary" => PRIMARY,
        "Required" => REQUIRED,
        "Agnostic" => AGNOSTIC,
        "Collapsed" => COLLAPSED,
        "PostProcess" => POST_PROCESS,
        "CollapsedOut" => OUT | COLLAPSED,
        "CollapsedIN" => IN | COLLAPSED,
        "PrimaryIn" => IN | PRIMARY,
        "PrimaryOut" => OUT | PRIMARY | POST_PROCESS,
        other => other.parse().unwrap_or(0),
    }
}

/// Parses `PortType` flags as Newtonsoft writes them (`"PrimaryIn, Required"`).
pub fn parse_port_type(text: &str) -> u32 {
    text.split(',').map(|part| port_flag(part.trim())).fold(0, |flags, flag| flags | flag)
}

impl PortData {
    pub fn flags(&self) -> u32 {
        parse_port_type(&self.port_type)
    }

    pub fn is_input(&self) -> bool {
        self.flags() & port_flags::IN != 0
    }

    pub fn is_output(&self) -> bool {
        self.flags() & port_flags::OUT != 0
    }

    pub fn is_required(&self) -> bool {
        self.flags() & port_flags::REQUIRED != 0
    }

    /// `Port.IsPrimary()`: the node's main input or output.
    pub fn is_primary(&self) -> bool {
        self.flags() & port_flags::PRIMARY != 0
    }
}

/// A per-node modifier (a `Modifiers` type).
#[derive(Clone, Debug, PartialEq)]
pub struct ModifierData {
    /// Short type name (`Height`, `Autolevel`, …).
    pub type_name: String,
    pub values: IndexMap<String, Value>,
}

/// A node as stored in the project.
#[derive(Clone, Debug)]
pub struct NodeData {
    pub id: i32,
    /// Catalogue key (`Canyon`).
    pub type_name: String,
    pub name: String,
    pub position: [f32; 2],
    /// Node-specific properties: every catalogue default, overridden by the
    /// values in the file.
    pub values: IndexMap<String, Value>,
    /// Members of the `Node` base class other than the typed ones
    /// (`NodeSize`, `IsLocked`, `SaveDefinition`, …).
    pub base: IndexMap<String, Value>,
    pub ports: Vec<PortData>,
    pub modifiers: Vec<ModifierData>,
}

impl NodeData {
    /// A fresh node of `spec`'s type (`Activator.CreateInstance`).
    pub fn new(spec: &NodeSpec, id: i32, seed: i32) -> Self {
        let mut values = spec.defaults.clone();
        for parameter in spec.parameters.iter().filter(|parameter| parameter.is_seed()) {
            values.insert(parameter.name.clone(), Value::from(seed));
        }
        let ports = spec
            .ports
            .iter()
            .map(|port| PortData {
                name: port.name.clone(),
                port_type: port.port_type.clone(),
                record: None,
                is_exporting: port.is_exporting,
                has_parent: true,
                extra: JsonObject::new(),
                declared_intent: port.render_intent.as_deref().and_then(RenderIntent::from_name),
            })
            .collect();
        let mut base = IndexMap::new();
        if spec.version != 0 {
            base.insert("Version".to_string(), Value::from(spec.version));
        }
        Self {
            id,
            type_name: spec.key().to_string(),
            name: spec.name.clone(),
            position: [0.0, 0.0],
            values,
            base,
            ports,
            modifiers: Vec::new(),
        }
    }

    /// `Node.Group`: the group the node belongs to (0 for none).
    pub fn group(&self) -> i32 {
        self.base.get("Group").and_then(Value::as_i64).unwrap_or(0) as i32
    }

    /// `Node.Version`: 0 for nodes saved before the type's behaviour changed.
    pub fn version(&self) -> i32 {
        self.base.get("Version").and_then(Value::as_i64).unwrap_or(0) as i32
    }

    pub fn port(&self, name: &str) -> Option<&PortData> {
        self.ports.iter().find(|port| port.name == name)
    }

    pub fn port_mut(&mut self, name: &str) -> Option<&mut PortData> {
        self.ports.iter_mut().find(|port| port.name == name)
    }

    /// Sets a serialized member, wherever it lives (a node property or one
    /// of the `Node` base members).
    pub fn set_member(&mut self, name: &str, value: Value) {
        if NODE_BASE_MEMBERS.contains(&name) {
            self.base.insert(name.to_string(), value);
            return;
        }
        self.values.insert(name.to_string(), value);
    }

    /// Appends an unconnected port (`Ports.Add(new Port(this, name) { Type = … })`).
    pub fn add_port(&mut self, name: &str, port_type: &str) {
        let port = PortData {
            name: name.to_string(),
            port_type: port_type.to_string(),
            record: None,
            is_exporting: true,
            has_parent: true,
            extra: JsonObject::new(),
            declared_intent: None,
        };
        self.ports.push(port);
    }
}

/// The graph of one terrain (`Terrain`).
#[derive(Clone, Debug)]
pub struct Terrain {
    pub id: String,
    /// Nodes in file order, keyed by id.
    pub nodes: IndexMap<i32, NodeData>,
    /// Terrain width and height in metres.
    pub width: f32,
    pub height: f32,
    /// Everything else (`Metadata`, `Groups`, `Notes`, `GraphTabs`,
    /// `Regions`, …), as written.
    pub extra: JsonObject<String, Value>,
}

/// One terrain with its settings (`Asset`).
#[derive(Clone, Debug)]
pub struct Asset {
    pub terrain: Terrain,
    /// Variables, their bindings and parameter expressions.
    pub automation: Automation,
    /// `Automation`, `BuildDefinition`, `State`, `BuildProfiles`, …
    pub extra: JsonObject<String, Value>,
}

/// A whole `.terrain` file (`Project`).
#[derive(Clone, Debug)]
pub struct Project {
    pub assets: Vec<Asset>,
    /// `Id`, `Branch`, `Metadata`, …
    pub extra: JsonObject<String, Value>,
    /// Node types in the file that are not in the catalogue.
    pub unknown_types: Vec<String>,
}

impl Project {
    /// Reads a project as File > Open does: text rewrites, then
    /// the document, then every node migrated to its type's version.
    pub fn parse(text: &str) -> Result<Self, ProjectError> {
        let rewritten = crate::migrate::rewrite_text(text);
        let document = parse_lenient(&rewritten)?;
        let references = References::collect(&document);
        let catalogue = Catalogue::get();

        let mut unknown_types = Vec::new();
        let mut assets = Vec::new();
        let asset_values = document.get("Assets").map(collection_items).unwrap_or(&[]);
        for asset_value in asset_values {
            let asset_value = references.resolve(asset_value);
            let mut asset = parse_asset(asset_value, &references, catalogue, &mut unknown_types)?;
            for node in asset.terrain.nodes.values_mut() {
                if let Some(spec) = catalogue.node(&node.type_name) {
                    crate::migrate::migrate_node(node, spec);
                }
            }
            assets.push(asset);
        }
        if assets.is_empty() {
            return Err(ProjectError::NoAsset);
        }

        let extra = plain_members(&document, &["Assets"]);
        Ok(Self { assets, extra, unknown_types })
    }

    /// The ids of the macros the project uses (`Project.Macros`).
    pub fn macro_ids(&self) -> Vec<String> {
        let macros = self.extra.get("Macros").map(collection_items).unwrap_or(&[]);
        macros.iter().filter_map(Value::as_str).map(str::to_string).collect()
    }

    /// The terrain a project opens with (the first asset).
    pub fn terrain(&self) -> &Terrain {
        &self.assets[0].terrain
    }

    pub fn terrain_mut(&mut self) -> &mut Terrain {
        &mut self.assets[0].terrain
    }
}

pub(crate) fn parse_asset(value: &Value, references: &References, catalogue: &Catalogue, unknown_types: &mut Vec<String>) -> Result<Asset, ProjectError> {
    let terrain_value = value.get("Terrain").ok_or(ProjectError::NoAsset)?;
    let terrain_value = references.resolve(terrain_value);
    let terrain = parse_terrain(terrain_value, references, catalogue, unknown_types)?;
    let automation = match value.get("Automation") {
        Some(automation) => Automation::from_json(references.resolve(automation), references),
        None => Automation::default(),
    };
    let mut extra = plain_members(value, &["Terrain"]);
    if let Some(Value::Object(definition)) = extra.get_mut("BuildDefinition") {
        resolve_portion(definition, references);
    }
    if let Some(Value::Object(profiles)) = extra.get_mut("BuildProfiles") {
        for profile in profiles.values_mut() {
            if let Value::Object(definition) = profile {
                resolve_portion(definition, references);
            }
        }
    }
    Ok(Asset { terrain, automation, extra })
}

/// A build definition's `Portion` that is a `$ref` (the region it builds is
/// saved as a reference to the same object in `Terrain.Regions`), as a
/// copy of the object it points at, typed as Newtonsoft types a `Region`
/// held in a `WorldPortion` member. The writer turns it back into a
/// reference.
fn resolve_portion(definition: &mut JsonObject<String, Value>, references: &References) {
    let reference = definition.get("Portion").and_then(|portion| portion.get("$ref")).and_then(Value::as_str);
    let Some(target) = reference.and_then(|reference| references.get(reference)) else {
        return;
    };
    let Value::Object(members) = strip_reference_ids(target) else {
        return;
    };
    let mut typed = JsonObject::new();
    if !members.contains_key("$type") {
        typed.insert("$type".to_string(), Value::from(crate::format_names::value_type("Region")));
    }
    typed.extend(members);
    definition.insert("Portion".to_string(), Value::Object(typed));
}

pub(crate) fn parse_terrain(value: &Value, references: &References, catalogue: &Catalogue, unknown_types: &mut Vec<String>) -> Result<Terrain, ProjectError> {
    let mut nodes = IndexMap::new();
    if let Some(Value::Object(node_entries)) = value.get("Nodes") {
        for (key, node_value) in node_entries {
            if key.starts_with('$') {
                continue;
            }
            let node_value = references.resolve(node_value);
            match parse_node(node_value, references, catalogue)? {
                Some(node) => {
                    nodes.insert(node.id, node);
                }
                None => {
                    let type_name = node_value.get("$type").and_then(Value::as_str).unwrap_or("?").to_string();
                    unknown_types.push(type_name);
                }
            }
        }
    }

    let id = value.get("Id").and_then(Value::as_str).unwrap_or_default().to_string();
    let width = value.get("Width").and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let height = value.get("Height").and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let extra = plain_members(value, &["Id", "Nodes", "Width", "Height"]);
    Ok(Terrain { id, nodes, width, height, extra })
}

/// Members of `Node` the engine reads directly.
const TYPED_NODE_MEMBERS: [&str; 5] = ["Id", "Name", "Position", "Ports", "Modifiers"];

/// Other members declared by the `Node` base class.
const NODE_BASE_MEMBERS: [&str; 11] = [
    "Version",
    "NodeSize",
    "StateOverride",
    "IsExpanded",
    "GraphIndex",
    "IgnoreUnderlay",
    "IsLocked",
    "RenderIntentOverride",
    "SaveDefinitions",
    "SaveDefinition",
    "Thumbprint",
];

fn parse_node(value: &Value, references: &References, catalogue: &Catalogue) -> Result<Option<NodeData>, ProjectError> {
    let dotnet_type = value.get("$type").and_then(Value::as_str).unwrap_or_default();
    let Some(spec) = catalogue.node_for_type(dotnet_type) else {
        return Ok(None);
    };

    let id = value
        .get("Id")
        .and_then(Value::as_i64)
        .ok_or_else(|| ProjectError::Invalid(format!("a {} node has no id", spec.key())))? as i32;
    let name = value.get("Name").and_then(Value::as_str).unwrap_or(&spec.name).to_string();
    let position = value.get("Position").map(|position| read_float2(references.resolve(position))).unwrap_or([0.0, 0.0]);

    // IgnoreAndPopulate: start from the defaults, then apply what was written.
    let mut values = spec.defaults.clone();
    let mut base = IndexMap::new();
    let members = value.as_object().map(|members| members.iter()).into_iter().flatten();
    for (key, member) in members {
        if key.starts_with('$') || TYPED_NODE_MEMBERS.contains(&key.as_str()) {
            continue;
        }
        let member = strip_bookkeeping(references.resolve(member));
        if NODE_BASE_MEMBERS.contains(&key.as_str()) {
            base.insert(key.clone(), member);
            continue;
        }
        // A written object replaces the default as a whole
        // (`ObjectCreationHandling.Replace`); its own missing members are zero.
        values.insert(key.clone(), member);
    }

    let mut ports = Vec::new();
    if let Some(port_list) = value.get("Ports") {
        for port_value in collection_items(references.resolve(port_list)) {
            let port_value = references.resolve(port_value);
            ports.push(parse_port(port_value, references));
        }
    }

    let mut modifiers = Vec::new();
    if let Some(modifier_list) = value.get("Modifiers") {
        for modifier_value in collection_items(references.resolve(modifier_list)) {
            let modifier_value = references.resolve(modifier_value);
            modifiers.push(parse_modifier(modifier_value, references));
        }
    }

    Ok(Some(NodeData {
        id,
        type_name: spec.key().to_string(),
        name,
        position,
        values,
        base,
        ports,
        modifiers,
    }))
}

fn parse_port(value: &Value, references: &References) -> PortData {
    let name = value.get("Name").and_then(Value::as_str).unwrap_or_default().to_string();
    let port_type = value.get("Type").and_then(Value::as_str).unwrap_or_default().to_string();
    let is_exporting = value.get("IsExporting").and_then(Value::as_bool).unwrap_or(false);
    let record = value.get("Record").map(|record| parse_record(references.resolve(record)));
    let has_parent = value.get("Parent").is_some_and(|parent| !parent.is_null());
    let extra = plain_members(value, &["Name", "Type", "Record", "IsExporting", "Parent"]);
    PortData {
        name,
        port_type,
        record,
        is_exporting,
        has_parent,
        extra,
        declared_intent: None,
    }
}

fn parse_record(value: &Value) -> ConnectionRecord {
    let read_int = |key: &str| value.get(key).and_then(Value::as_i64).unwrap_or(0) as i32;
    let read_text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    ConnectionRecord {
        from: read_int("From"),
        to: read_int("To"),
        from_port: read_text("FromPort"),
        to_port: read_text("ToPort"),
        is_valid: value.get("IsValid").and_then(Value::as_bool).unwrap_or(false),
    }
}

fn parse_modifier(value: &Value, references: &References) -> ModifierData {
    let dotnet_type = value.get("$type").and_then(Value::as_str).unwrap_or_default();
    let type_name = crate::json::type_class_name(dotnet_type).to_string();
    let mut values = IndexMap::new();
    for (key, member) in value.as_object().into_iter().flatten() {
        if key.starts_with('$') || key == "Parent" {
            continue;
        }
        let member = strip_bookkeeping(references.resolve(member));
        values.insert(key.clone(), member);
    }
    ModifierData { type_name, values }
}

fn read_float2(value: &Value) -> [f32; 2] {
    let x = value.get("X").and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let y = value.get("Y").and_then(Value::as_f64).unwrap_or(0.0) as f32;
    [x, y]
}

/// The members of an object other than `typed` ones and Newtonsoft's
/// bookkeeping, with nested reference ids removed (`$type` stays, so the
/// project saves as it was read).
fn plain_members(value: &Value, typed: &[&str]) -> JsonObject<String, Value> {
    let mut members = JsonObject::new();
    for (key, member) in value.as_object().into_iter().flatten() {
        if key.starts_with('$') || typed.contains(&key.as_str()) {
            continue;
        }
        members.insert(key.clone(), strip_reference_ids(member));
    }
    members
}
