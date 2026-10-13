//! Writes projects as `.terrain` files (Newtonsoft):
//!
//! * `PreserveReferencesHandling.All`: every object starts with a `$id`,
//!   every collection is written as `{"$id", "$values": [...]}`, and ports
//!   and modifiers point back at their node with `{"$ref": id}`.
//! * `TypeNameHandling.Auto`: `$type` on nodes, modifiers and values held
//!   in `object` members.
//! * `DefaultValueHandling.IgnoreAndPopulate`: members equal to their
//!   default are left out. A node parameter's default is the value a fresh
//!   node of the declaring type has (`ParameterAwareContractResolver`),
//!   anything else's is its type's zero.
//! * `NullValueHandling.Ignore`, `StringEnumConverter`, indented with two
//!   spaces and CRLF line ends, no byte order mark.

use serde_json::{Map as JsonObject, Value};

use crate::automation::{dotnet_double_string, dotnet_float_string};
use crate::format_names::{modifier_type, node_type};
use crate::json::collection_items;
use crate::catalogue::{Catalogue, NodeSpec, ValueType};
use crate::project::{Asset, ConnectionRecord, ModifierData, NodeData, PortData, Project, Terrain};

const NEW_LINE: &str = "\r\n";
const INDENT: &str = "  ";
const EMPTY_GUID: &str = "00000000-0000-0000-0000-000000000000";

/// `Node` members after the node type's own, in declaration order, with
/// the value Newtonsoft leaves out (the member type's default).
const NODE_BASE_ORDER: [(&str, BaseDefault); 16] = [
    ("Id", BaseDefault::Zero),
    ("Version", BaseDefault::Zero),
    ("Name", BaseDefault::Null),
    ("NodeSize", BaseDefault::Text("Default")),
    ("Position", BaseDefault::Null),
    ("StateOverride", BaseDefault::Text("None")),
    ("IsExpanded", BaseDefault::False),
    ("GraphIndex", BaseDefault::Zero),
    ("IgnoreUnderlay", BaseDefault::False),
    ("IsLocked", BaseDefault::False),
    ("RenderIntentOverride", BaseDefault::Text("Default")),
    ("SaveDefinitions", BaseDefault::Null),
    ("SaveDefinition", BaseDefault::Null),
    ("Ports", BaseDefault::Null),
    ("Modifiers", BaseDefault::Null),
    ("Thumbprint", BaseDefault::Text(EMPTY_GUID)),
];

/// `Terrain` members in declaration order.
const TERRAIN_ORDER: [&str; 12] = ["Id", "Metadata", "Nodes", "Groups", "Notes", "GraphTabs", "Width", "Height", "SeaLevel", "Ratio", "Location", "Regions"];

/// `Asset` members in declaration order.
const ASSET_ORDER: [&str; 6] = ["Terrain", "Type", "Automation", "BuildDefinition", "State", "BuildProfiles"];

/// `Project` members in declaration order.
const PROJECT_ORDER: [&str; 6] = ["Assets", "Id", "Branch", "Iteration", "Macros", "Metadata"];

/// `Port` members in declaration order.
const PORT_ORDER: [&str; 8] = ["Name", "DisplayName", "Accumulator", "Type", "Record", "IsExporting", "Parent", "PortalState"];

#[derive(Clone, Copy)]
enum BaseDefault {
    Zero,
    False,
    Null,
    Text(&'static str),
}

impl BaseDefault {
    fn matches(self, value: &Value) -> bool {
        match self {
            BaseDefault::Zero => value.as_f64() == Some(0.0),
            BaseDefault::False => value == &Value::Bool(false),
            BaseDefault::Null => value.is_null(),
            BaseDefault::Text(text) => value.as_str() == Some(text),
        }
    }
}

/// The project as a `.terrain` file.
pub fn to_terrain_json(project: &Project) -> String {
    let mut writer = Writer::new();
    writer.write_project(project);
    writer.text
}

/// A terrain on its own, as a TORLET graph fragment (`.tfrag`, File ›
/// Export › TORLET).
pub fn to_fragment_json(terrain: &Terrain) -> String {
    let mut writer = Writer::new();
    writer.write_terrain(terrain);
    writer.text
}

/// A macro's asset (`MacroAsset`, what `data.bin` holds): the macro's
/// metadata, then the asset's own members.
pub fn to_macro_asset_json(asset: &Asset, metadata: &Value) -> String {
    let mut writer = Writer::new();
    writer.begin_object(None);
    writer.member("Metadata");
    writer.value(metadata);
    writer.ordered_members(&ASSET_ORDER, &asset.extra, |writer, name| {
        if name != "Terrain" {
            return false;
        }
        writer.member("Terrain");
        writer.write_terrain(&asset.terrain);
        true
    });
    writer.end_object();
    writer.text
}

/// Indented Newtonsoft output with reference ids.
struct Writer {
    text: String,
    /// Whether each open object or array already has an entry.
    open: Vec<bool>,
    next_id: u32,
    /// The `$id` of each region of the asset being written, by name.
    region_ids: Vec<(String, u32)>,
}

impl Writer {
    fn new() -> Self {
        Self {
            text: String::new(),
            open: Vec::new(),
            next_id: 1,
            region_ids: Vec::new(),
        }
    }

    fn new_line(&mut self) {
        self.text.push_str(NEW_LINE);
        for _ in 0..self.open.len() {
            self.text.push_str(INDENT);
        }
    }

    /// Starts the next entry of the innermost object or array.
    fn begin_entry(&mut self) {
        let Some(has_entry) = self.open.last_mut() else {
            return;
        };
        let separate = *has_entry;
        *has_entry = true;
        if separate {
            self.text.push(',');
        }
        self.new_line();
    }

    fn member(&mut self, name: &str) {
        self.begin_entry();
        write_string(&mut self.text, name);
        self.text.push_str(": ");
    }

    /// Opens an object with its `$id` (and `$type`); returns the id.
    fn begin_object(&mut self, type_name: Option<&str>) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.text.push('{');
        self.open.push(false);
        self.member("$id");
        write_string(&mut self.text, &id.to_string());
        if let Some(type_name) = type_name {
            self.member("$type");
            write_string(&mut self.text, type_name);
        }
        id
    }

    fn end_object(&mut self) {
        self.open.pop();
        self.new_line();
        self.text.push('}');
    }

    /// Opens a collection (`{"$id", "$values": [`).
    fn begin_collection(&mut self, type_name: Option<&str>) {
        self.begin_object(type_name);
        self.member("$values");
        self.text.push('[');
        self.open.push(false);
    }

    fn end_collection(&mut self) {
        let had_entries = self.open.pop().unwrap_or(false);
        if had_entries {
            self.new_line();
        }
        self.text.push(']');
        self.end_object();
    }

    fn reference(&mut self, id: u32) {
        self.text.push('{');
        self.open.push(false);
        self.member("$ref");
        write_string(&mut self.text, &id.to_string());
        self.end_object();
    }

    fn float(&mut self, value: f64) {
        let single = value as f32;
        let text = dotnet_float_string(single);
        write_number_text(&mut self.text, text);
    }

    fn double(&mut self, value: f64) {
        let text = dotnet_double_string(value);
        write_number_text(&mut self.text, text);
    }

    /// A value Nerthus does not model, written as it was read: objects keep
    /// their `$type`, arrays become collections.
    fn value(&mut self, value: &Value) {
        match value {
            Value::Object(members) => self.object_value(members),
            Value::Array(items) => {
                self.begin_collection(None);
                for item in items {
                    self.begin_entry();
                    self.value(item);
                }
                self.end_collection();
            }
            Value::Number(number) => {
                if number.is_f64() {
                    self.double(number.as_f64().unwrap_or(0.0));
                } else {
                    self.text.push_str(&number.to_string());
                }
            }
            Value::String(text) => write_string(&mut self.text, text),
            Value::Bool(flag) => self.text.push_str(if *flag { "true" } else { "false" }),
            Value::Null => self.text.push_str("null"),
        }
    }

    fn object_value(&mut self, members: &JsonObject<String, Value>) {
        let type_name = members.get("$type").and_then(Value::as_str);
        if let Some(Value::Array(items)) = members.get("$values") {
            self.begin_collection(type_name);
            for item in items {
                self.begin_entry();
                self.value(item);
            }
            self.end_collection();
            return;
        }
        self.begin_object(type_name);
        for (name, member) in members {
            if name.starts_with('$') || member.is_null() {
                continue;
            }
            self.member(name);
            self.value(member);
        }
        self.end_object();
    }

    /// Members of `extra` in the class's declaration order, then any the
    /// order does not name, as read. `typed` writes the members Nerthus
    /// models; it returns false for names it does not handle.
    fn ordered_members(&mut self, order: &[&str], extra: &JsonObject<String, Value>, mut typed: impl FnMut(&mut Self, &str) -> bool) {
        for name in order {
            if typed(self, name) {
                continue;
            }
            let Some(member) = extra.get(*name) else {
                continue;
            };
            if member.is_null() {
                continue;
            }
            self.member(name);
            self.value(member);
        }
        for (name, member) in extra {
            if order.contains(&name.as_str()) || name.starts_with('$') || member.is_null() {
                continue;
            }
            self.member(name);
            self.value(member);
        }
    }

    fn write_project(&mut self, project: &Project) {
        self.begin_object(None);
        self.ordered_members(&PROJECT_ORDER, &project.extra, |writer, name| {
            if name != "Assets" {
                return false;
            }
            writer.member("Assets");
            writer.begin_collection(None);
            for asset in &project.assets {
                writer.begin_entry();
                writer.write_asset(asset);
            }
            writer.end_collection();
            true
        });
        self.end_object();
    }

    fn write_asset(&mut self, asset: &Asset) {
        self.region_ids.clear();
        self.begin_object(None);
        self.ordered_members(&ASSET_ORDER, &asset.extra, |writer, name| match name {
            "Terrain" => {
                writer.member("Terrain");
                writer.write_terrain(&asset.terrain);
                true
            }
            "BuildDefinition" => {
                let Some(Value::Object(definition)) = asset.extra.get("BuildDefinition") else {
                    return false;
                };
                writer.member("BuildDefinition");
                writer.write_build_definition(definition);
                true
            }
            _ => false,
        });
        self.end_object();
    }

    /// `Terrain.Regions`, noting each region's `$id` so that the build
    /// definition's `Portion` can point at it.
    fn write_regions(&mut self, regions: &Value) {
        self.begin_collection(None);
        for region in collection_items(regions) {
            self.begin_entry();
            let id = self.next_id;
            self.value(region);
            if let Some(name) = region.get("Name").and_then(Value::as_str) {
                self.region_ids.push((name.to_string(), id));
            }
        }
        self.end_collection();
    }

    /// `BuildDefinition`, its `Portion` written as a reference to the
    /// region of the same name in `Terrain.Regions` (the same object once
    /// read back), or as it is when no region has its name.
    fn write_build_definition(&mut self, members: &JsonObject<String, Value>) {
        let type_name = members.get("$type").and_then(Value::as_str);
        self.begin_object(type_name);
        for (name, member) in members {
            if name.starts_with('$') || member.is_null() {
                continue;
            }
            self.member(name);
            let portion_name = if name == "Portion" { member.get("Name").and_then(Value::as_str) } else { None };
            let region = portion_name.and_then(|portion_name| self.region_ids.iter().find(|(region_name, _)| region_name == portion_name));
            match region {
                Some((_, id)) => {
                    let id = *id;
                    self.reference(id);
                }
                None => self.value(member),
            }
        }
        self.end_object();
    }

    fn write_terrain(&mut self, terrain: &Terrain) {
        self.begin_object(None);
        self.ordered_members(&TERRAIN_ORDER, &terrain.extra, |writer, name| match name {
            "Id" => {
                if !terrain.id.is_empty() && terrain.id != EMPTY_GUID {
                    writer.member("Id");
                    write_string(&mut writer.text, &terrain.id);
                }
                true
            }
            "Nodes" => {
                writer.member("Nodes");
                writer.begin_object(None);
                for (id, node) in &terrain.nodes {
                    writer.member(&id.to_string());
                    writer.write_node(node);
                }
                writer.end_object();
                true
            }
            "Width" | "Height" => {
                let size = if name == "Width" { terrain.width } else { terrain.height };
                if size != 0.0 {
                    writer.member(name);
                    writer.float(size as f64);
                }
                true
            }
            "Regions" => {
                let Some(regions) = terrain.extra.get("Regions").filter(|regions| !regions.is_null()) else {
                    return false;
                };
                writer.member("Regions");
                writer.write_regions(regions);
                true
            }
            _ => false,
        });
        self.end_object();
    }

    fn write_node(&mut self, node: &NodeData) {
        let catalogue = Catalogue::get();
        let spec = catalogue.node(&node.type_name);
        let type_path = spec.map(|spec| spec.type_name.as_str()).unwrap_or(&node.type_name);
        let dotnet_type = node_type(type_path);
        let node_reference = self.begin_object(Some(&dotnet_type));

        let declared: &[String] = spec.map(|spec| spec.members.as_slice()).unwrap_or(&[]);
        for name in declared {
            let Some(value) = node.values.get(name) else {
                continue;
            };
            self.node_member(spec, name, value);
        }
        for (name, value) in &node.values {
            if declared.contains(name) {
                continue;
            }
            self.node_member(spec, name, value);
        }

        for (name, default) in NODE_BASE_ORDER {
            match name {
                "Id" => {
                    if node.id != 0 {
                        self.member("Id");
                        self.text.push_str(&node.id.to_string());
                    }
                }
                "Name" => {
                    self.member("Name");
                    write_string(&mut self.text, &node.name);
                }
                "Position" => {
                    let [x, y] = node.position;
                    if x != 0.0 || y != 0.0 {
                        self.member("Position");
                        self.begin_object(None);
                        self.float_member("X", x);
                        self.float_member("Y", y);
                        self.end_object();
                    }
                }
                "Ports" => {
                    self.member("Ports");
                    self.begin_collection(None);
                    for port in &node.ports {
                        self.begin_entry();
                        self.write_port(port, node_reference);
                    }
                    self.end_collection();
                }
                "Modifiers" => {
                    self.member("Modifiers");
                    self.begin_collection(None);
                    for modifier in &node.modifiers {
                        self.begin_entry();
                        self.write_modifier(modifier, node_reference);
                    }
                    self.end_collection();
                }
                _ => {
                    let Some(value) = node.base.get(name) else {
                        continue;
                    };
                    if default.matches(value) {
                        continue;
                    }
                    self.member(name);
                    self.value(value);
                }
            }
        }
        for (name, value) in &node.base {
            let known = NODE_BASE_ORDER.iter().any(|(base_name, _)| base_name == name);
            if known || value.is_null() {
                continue;
            }
            self.member(name);
            self.value(value);
        }
        self.end_object();
    }

    fn node_member(&mut self, spec: Option<&NodeSpec>, name: &str, value: &Value) {
        if is_default_member(spec, name, value) {
            return;
        }
        self.member(name);
        self.parameter_value(spec, name, value);
    }

    /// A float member, left out when zero (a `Float2`'s `X`, `Y`).
    fn float_member(&mut self, name: &str, value: f32) {
        if value == 0.0 {
            return;
        }
        self.member(name);
        self.float(value as f64);
    }

    /// A node property, its numbers formatted as its .NET type.
    fn parameter_value(&mut self, spec: Option<&NodeSpec>, name: &str, value: &Value) {
        let value_type = spec.and_then(|spec| spec.parameter(name)).map(|parameter| parameter.value_type);
        match (value_type, value) {
            (Some(ValueType::Float), Value::Number(number)) if number.is_f64() => self.float(number.as_f64().unwrap_or(0.0)),
            (Some(ValueType::Float2 | ValueType::Float3 | ValueType::Float4), Value::Object(members)) => self.float_struct(members),
            _ => self.value(value),
        }
    }

    /// `Float2`/`Float3`/`Float4`: float members, zeros left out.
    fn float_struct(&mut self, members: &JsonObject<String, Value>) {
        self.begin_object(None);
        for (name, member) in members {
            if name.starts_with('$') {
                continue;
            }
            match member {
                Value::Number(number) => {
                    let component = number.as_f64().unwrap_or(0.0);
                    self.float_member(name, component as f32);
                }
                other => {
                    if is_zero_value(other) {
                        continue;
                    }
                    self.member(name);
                    self.value(other);
                }
            }
        }
        self.end_object();
    }

    fn write_port(&mut self, port: &PortData, node_reference: u32) {
        self.begin_object(None);
        self.ordered_members(&PORT_ORDER, &port.extra, |writer, name| match name {
            "Name" => {
                writer.member("Name");
                write_string(&mut writer.text, &port.name);
                true
            }
            "Type" => {
                writer.member("Type");
                write_string(&mut writer.text, &port.port_type);
                true
            }
            "Record" => {
                if let Some(record) = &port.record {
                    writer.member("Record");
                    writer.write_record(record);
                }
                true
            }
            "IsExporting" => {
                if port.is_exporting {
                    writer.member("IsExporting");
                    writer.text.push_str("true");
                }
                true
            }
            "Parent" => {
                if port.has_parent {
                    writer.member("Parent");
                    writer.reference(node_reference);
                }
                true
            }
            "PortalState" => {
                let state = port.extra.get("PortalState");
                if let Some(state) = state.filter(|state| state.as_str() != Some("None")) {
                    writer.member("PortalState");
                    writer.value(state);
                }
                true
            }
            _ => false,
        });
        self.end_object();
    }

    fn write_record(&mut self, record: &ConnectionRecord) {
        self.begin_object(None);
        if record.from != 0 {
            self.member("From");
            self.text.push_str(&record.from.to_string());
        }
        if record.to != 0 {
            self.member("To");
            self.text.push_str(&record.to.to_string());
        }
        self.member("FromPort");
        write_string(&mut self.text, &record.from_port);
        self.member("ToPort");
        write_string(&mut self.text, &record.to_port);
        if record.is_valid {
            self.member("IsValid");
            self.text.push_str("true");
        }
        self.end_object();
    }

    /// A modifier: its own members, then `Modifier`'s, with `Parent`
    /// after `Name` as declared.
    fn write_modifier(&mut self, modifier: &ModifierData, node_reference: u32) {
        let dotnet_type = modifier_type(&modifier.type_name);
        self.begin_object(Some(&dotnet_type));
        let mut parent_written = false;
        for (name, value) in &modifier.values {
            if !value.is_null() {
                self.member(name);
                self.value(value);
            }
            if name == "Name" {
                self.member("Parent");
                self.reference(node_reference);
                parent_written = true;
            }
        }
        if !parent_written {
            self.member("Parent");
            self.reference(node_reference);
        }
        self.end_object();
    }
}

/// Whether Newtonsoft leaves a node property out: parameters equal to a
/// fresh node's value (class values compare by reference, so those are
/// always written), other members equal to their type's zero.
///
/// A seed equal to the catalogue's is one the file left out: it reads back
/// as the seed drawn for the type and is left out again on save.
fn is_default_member(spec: Option<&NodeSpec>, name: &str, value: &Value) -> bool {
    if value.is_null() {
        return true;
    }
    let Some(spec) = spec else {
        return false;
    };
    let Some(parameter) = spec.parameter(name) else {
        return is_zero_value(value);
    };
    let is_class = matches!(parameter.value_type, ValueType::Float4List | ValueType::Float2List | ValueType::Dictionary | ValueType::Object);
    if is_class {
        return false;
    }
    let Some(default) = spec.defaults.get(name) else {
        return is_zero_value(value);
    };
    json_values_equal(value, default)
}

/// Value equality with numbers compared as numbers (`1` equals `1.0`).
fn json_values_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left_number), Value::Number(right_number)) => left_number.as_f64() == right_number.as_f64(),
        (Value::Object(left_members), Value::Object(right_members)) => {
            let left_plain = left_members.iter().filter(|(name, _)| !name.starts_with('$'));
            let right_plain = right_members.iter().filter(|(name, _)| !name.starts_with('$'));
            let left_count = left_plain.clone().count();
            let right_count = right_plain.count();
            if left_count != right_count {
                return false;
            }
            left_plain.into_iter().all(|(name, member)| right_members.get(name).is_some_and(|other| json_values_equal(member, other)))
        }
        (Value::Array(left_items), Value::Array(right_items)) => {
            left_items.len() == right_items.len() && left_items.iter().zip(right_items).all(|(left_item, right_item)| json_values_equal(left_item, right_item))
        }
        _ => left == right,
    }
}

/// A value Newtonsoft leaves out when it is the member type's default.
fn is_zero_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(flag) => !flag,
        Value::Number(number) => number.as_f64() == Some(0.0),
        _ => false,
    }
}

/// `JsonConvert.ToString(float/double)`: the number with `.0` added when it
/// has no decimal point or exponent; NaN and infinities as strings.
fn write_number_text(output: &mut String, text: String) {
    let is_special = text == "NaN" || text.ends_with("Infinity");
    if is_special {
        write_string(output, &text);
        return;
    }
    output.push_str(&text);
    let has_point = text.contains('.') || text.contains('E');
    if !has_point {
        output.push_str(".0");
    }
}

/// A JSON string as Newtonsoft escapes it (`StringEscapeHandling.Default`).
fn write_string(output: &mut String, text: &str) {
    output.push('"');
    for character in text.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{8}' => output.push_str("\\b"),
            '\u{c}' => output.push_str("\\f"),
            '\u{85}' | '\u{2028}' | '\u{2029}' => output.push_str(&format!("\\u{:04x}", character as u32)),
            control if (control as u32) < 0x20 => output.push_str(&format!("\\u{:04x}", control as u32)),
            other => output.push(other),
        }
    }
    output.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_are_escaped_like_newtonsoft() {
        let mut output = String::new();
        write_string(&mut output, "a\"b\\c\nd\u{1}\u{2028}");
        assert_eq!(output, "\"a\\\"b\\\\c\\nd\\u0001\\u2028\"");
    }

    #[test]
    fn numbers_keep_a_decimal_point() {
        let mut output = String::new();
        write_number_text(&mut output, dotnet_float_string(5000.0));
        assert_eq!(output, "5000.0");
        let mut small = String::new();
        write_number_text(&mut small, dotnet_float_string(0.00001));
        assert_eq!(small, "1E-05");
    }

    #[test]
    fn fresh_node_round_trips() {
        let catalogue = Catalogue::get();
        let spec = catalogue.node("SatelliteMap").expect("SatelliteMap is in the catalogue");
        let node = NodeData::new(spec, 324, 7);
        let mut writer = Writer::new();
        writer.write_node(&node);
        let satellite_map_type = format!("\"$type\": \"{}\"", node_type("SatelliteMap"));
        assert!(writer.text.contains(&satellite_map_type));
        assert!(!writer.text.contains("\"Range\""), "a default Float2 parameter is left out");
        assert!(writer.text.contains("\"$ref\": \"1\""), "ports point back at the node");
    }
}
