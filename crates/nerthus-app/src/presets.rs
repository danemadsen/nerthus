//! Node presets (`PresetService`): a node's parameters saved under a name
//! for its type, one file each in the documents folder's `Presets`
//! (`Paths.Presets`), named `<type>--<name>` (`PresetExtension` is never
//! set, so the files have no extension). A preset named `_default`
//! is applied to every new node of its type (`Shell.G_BeforeNodeAdded`).
//!
//! The files hold the `Preset` as Newtonsoft writes it (indented, with
//! `$id`s, members at their defaults left out), its `Json`
//! the node's `[Parameter]` properties as `StoreProperties` serializes them
//! (structs and lists typed with `$type`, lists wrapped in
//! `$values`).

use std::path::{Path, PathBuf};

use nerthus_engine::automation::{dotnet_double_string, dotnet_float_string};
use nerthus_engine::format_names::{format_node_name, nerthus_node_name, value_list_type, value_type};
use nerthus_engine::{NodeData, NodeSpec, ParameterSpec, ValueType};
use serde_json::{json, Value};

/// What is written between lines (Windows line ends).
const NEW_LINE: &str = "\r\n";

/// `PresetType`: what a preset holds. Nerthus makes node presets only.
const PRESET_TYPES: [&str; 6] = ["Node", "SnapIn", "EntireNode", "NodeDefault", "Group", "GraphOrganization"];

/// A preset (`Preset`).
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    /// `Type` (`PresetType`, `Node` unless the file says otherwise).
    pub kind: String,
    /// `ApplicableType`: the node type it is for (`NodeMetadata.Name`);
    /// files hold the format's name for it ([`format_node_name`]).
    pub applicable_type: String,
    pub name: String,
    pub description: Option<String>,
    /// The parameters (`StoreProperties`), as JSON text.
    pub json: String,
}

impl Preset {
    /// `Preset.ShortCode`: the name's capital letters.
    pub fn short_code(&self) -> String {
        self.name.chars().filter(|character| character.is_uppercase()).collect()
    }

    /// Reads a preset file (`null` on failure).
    fn from_file(path: &Path) -> Option<Preset> {
        let text = std::fs::read_to_string(path).ok()?;
        let value = nerthus_engine::json::parse_lenient(&text).ok()?;
        let object = value.as_object()?;
        let text_member = |name: &str| object.get(name).and_then(Value::as_str).map(str::to_string);
        let kind = text_member("Type").unwrap_or_else(|| "Node".to_string());
        let applicable_type = text_member("ApplicableType").unwrap_or_default();
        let preset = Preset {
            kind,
            applicable_type: nerthus_node_name(&applicable_type).to_string(),
            name: text_member("Name").unwrap_or_default(),
            description: text_member("Description"),
            json: text_member("Json").unwrap_or_default(),
        };
        Some(preset)
    }

    /// The file as written: `Type` only when not `Node`,
    /// `Description` only when there is one, then `Json` and `ShortCode`.
    pub fn to_file_text(&self) -> String {
        let mut writer = NewtonsoftWriter::default();
        writer.begin_object();
        if self.kind != PRESET_TYPES[0] {
            writer.member("Type");
            writer.string(&self.kind);
        }
        writer.member("ApplicableType");
        writer.string(format_node_name(&self.applicable_type));
        writer.member("Name");
        writer.string(&self.name);
        if let Some(description) = &self.description {
            writer.member("Description");
            writer.string(description);
        }
        writer.member("Json");
        writer.string(&self.json);
        writer.member("ShortCode");
        writer.string(&self.short_code());
        writer.end_object();
        writer.text
    }
}

/// `PresetService.Presets`: every preset in the folder.
#[derive(Clone, Debug, Default)]
pub struct PresetLibrary {
    folder: PathBuf,
    presets: Vec<Preset>,
}

impl PresetLibrary {
    /// `PresetService.Initialize` and `LoadAllPresets`: the folder (made if
    /// missing) and every file in it that reads as a preset, by file name.
    pub fn load(folder: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&folder);
        let mut library = Self { folder, presets: Vec::new() };
        library.reload();
        library
    }

    /// `LoadAllPresets`.
    pub fn reload(&mut self) {
        self.presets.clear();
        let Ok(entries) = std::fs::read_dir(&self.folder) else {
            return;
        };
        let mut files: Vec<PathBuf> = entries.filter_map(Result::ok).map(|entry| entry.path()).filter(|path| path.is_file()).collect();
        files.sort();
        for file in files {
            if let Some(preset) = Preset::from_file(&file) {
                self.presets.push(preset);
            }
        }
    }

    /// `PresetService.Presets`: every preset, in load order.
    pub fn all(&self) -> &[Preset] {
        &self.presets
    }

    /// `GetPresetsByType`: the presets for a node type, in load order.
    pub fn of_type(&self, node_type: &str) -> Vec<&Preset> {
        let node_type = node_type.trim();
        self.presets.iter().filter(|preset| preset.applicable_type == node_type).collect()
    }

    /// `GetPresetByName(name, type)`.
    pub fn named(&self, name: &str, node_type: &str) -> Option<&Preset> {
        self.presets.iter().find(|preset| preset.name == name && preset.applicable_type == node_type)
    }

    /// `GetDefaultPreset`: the type's `_default` preset.
    pub fn default_for(&self, node_type: &str) -> Option<&Preset> {
        self.named("_default", node_type)
    }

    /// `PresetService.SavePreset`: `<type>--<label>` in the folder; when
    /// that file is there already (and the label is not `_default`), the
    /// preset is renamed `<label> (<six hex digits>)` and saved as
    /// `<type>--<label>--<digits>` instead. Reloads the library.
    pub fn save(&mut self, preset: &mut Preset, label: &str) -> std::io::Result<()> {
        let file_type = format_node_name(&preset.applicable_type);
        let mut file_name = format!("{file_type}--{label}");
        let mut path = self.folder.join(&file_name);
        if path.exists() && label != "_default" {
            let suffix = random_hex6();
            preset.name = format!("{label} ({suffix})");
            file_name = format!("{file_name}--{suffix}");
            path = self.folder.join(&file_name);
        }
        std::fs::create_dir_all(&self.folder)?;
        let written = std::fs::write(&path, preset.to_file_text());
        if written.is_err() {
            // What was left of the file goes.
            let _ = std::fs::remove_file(&path);
        }
        self.reload();
        written
    }
}

/// `Guid.NewGuid().ToString("N").Substring(0, 6)`.
fn random_hex6() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.as_nanos()).unwrap_or_default();
    let mixed = (nanos as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ ((nanos >> 64) as u64);
    format!("{:06x}", mixed >> 40)
}

/// `PresetService.CreatePreset`: the node's `[Parameter]` properties
/// for its type (`NodeMetadata.Name`, so a renamed node
/// still matches).
pub fn create_preset(node: &NodeData, spec: &NodeSpec, name: &str, description: Option<&str>) -> Preset {
    Preset {
        kind: PRESET_TYPES[0].to_string(),
        applicable_type: spec.name.clone(),
        name: name.to_string(),
        description: description.map(str::to_string),
        json: store_properties(node, spec),
    }
}

/// `PresetService.StoreProperties`: each parameter and its value in a
/// `Dictionary<string, object>`, serialized with each value's `$type`.
fn store_properties(node: &NodeData, spec: &NodeSpec) -> String {
    let mut writer = NewtonsoftWriter::default();
    writer.begin_object();
    for parameter in &spec.parameters {
        let value = node.values.get(&parameter.name).cloned().or_else(|| spec.defaults.get(&parameter.name).cloned()).unwrap_or(Value::Null);
        writer.member(&parameter.name);
        write_parameter(&mut writer, parameter, &value);
    }
    writer.end_object();
    writer.text
}

/// A value of `object` type as Newtonsoft writes it with
/// `TypeNameHandling.Auto`, `PreserveReferencesHandling.All` and
/// `DefaultValueHandling.IgnoreAndPopulate`.
fn write_parameter(writer: &mut NewtonsoftWriter, parameter: &ParameterSpec, value: &Value) {
    match parameter.value_type {
        ValueType::Float => match value.as_f64() {
            Some(number) => writer.raw(&newtonsoft_float(number as f32)),
            None => writer.raw("null"),
        },
        ValueType::Double => match value.as_f64() {
            Some(number) => writer.raw(&newtonsoft_double(number)),
            None => writer.raw("null"),
        },
        ValueType::Int | ValueType::Byte => match value.as_i64() {
            Some(number) => writer.raw(&number.to_string()),
            None => writer.raw("null"),
        },
        ValueType::Bool => writer.raw(if value.as_bool().unwrap_or(false) { "true" } else { "false" }),
        ValueType::Enum | ValueType::String => match value.as_str() {
            Some(text) => writer.string(text),
            None => writer.raw("null"),
        },
        ValueType::Float2 => write_struct(writer, &value_type("Float2"), value, &["X", "Y"], true),
        ValueType::Int2 => write_struct(writer, &value_type("Int2"), value, &["X", "Y"], true),
        ValueType::Color => write_struct(writer, &value_type("Color"), value, &["R", "G", "B"], true),
        ValueType::Float2List => write_list(writer, "Float2", value, &["X", "Y"]),
        ValueType::Float4List => write_list(writer, "Float4", value, &["X", "Y", "Z", "W"]),
        _ => writer.raw("null"),
    }
}

/// A struct: its `$id`, its `$type` when typed as `object`, then each
/// member that is not zero.
fn write_struct(writer: &mut NewtonsoftWriter, type_name: &str, value: &Value, members: &[&str], typed: bool) {
    writer.begin_object();
    if typed {
        writer.member("$type");
        writer.string(type_name);
    }
    for member in members {
        // `IgnoreAndPopulate`: members equal to 0 (-0 too) are left out.
        let number = value.get(*member).and_then(Value::as_f64).unwrap_or(0.0) as f32;
        if number == 0.0 {
            continue;
        }
        writer.member(member);
        writer.raw(&newtonsoft_float(number));
    }
    writer.end_object();
}

/// A `List<T>` of the value type `element_class` typed as `object`: wrapped
/// with its `$id`, `$type` and `$values`; `null` when there is none.
fn write_list(writer: &mut NewtonsoftWriter, element_class: &str, value: &Value, members: &[&str]) {
    let Some(items) = value.as_array() else {
        writer.raw("null");
        return;
    };
    writer.begin_object();
    writer.member("$type");
    writer.string(&value_list_type(element_class));
    writer.member("$values");
    writer.begin_array();
    for item in items {
        writer.array_item();
        write_struct(writer, element_class, item, members, false);
    }
    writer.end_array();
    writer.end_object();
}

/// `PresetService.ApplyPreset`: the preset's values for
/// the node's parameters, by name, in the node's own form. Enums come from
/// their names, structs from their members (missing ones zero, a value
/// that is not an object all zeros), lists from their items (a value that
/// is not a list an empty one), numbers, flags and text as they are; what
/// does not convert is left alone. Colours are applied too.
pub fn preset_values(preset: &Preset, spec: &NodeSpec) -> Vec<(String, Value)> {
    let Ok(Value::Object(properties)) = nerthus_engine::json::parse_lenient(&preset.json) else {
        return Vec::new();
    };
    let mut values = Vec::new();
    for (name, stored) in &properties {
        let Some(parameter) = spec.parameter(name) else {
            continue;
        };
        if let Some(value) = convert_value(parameter, stored) {
            values.push((name.clone(), value));
        }
    }
    values
}

/// One stored value as the parameter's type takes it.
fn convert_value(parameter: &ParameterSpec, stored: &Value) -> Option<Value> {
    let float = |value: f64| nerthus_engine::json::f32_value(value as f32);
    match parameter.value_type {
        ValueType::Enum => {
            let name = stored.as_str()?;
            enum_member(parameter, name).map(Value::from)
        }
        ValueType::Float => stored.as_f64().map(float),
        ValueType::Double => stored.as_f64().map(Value::from),
        ValueType::Int | ValueType::Byte => convert_integer(stored).map(Value::from),
        ValueType::Bool => stored.as_bool().map(Value::from),
        ValueType::String => match stored {
            Value::Null => Some(Value::Null),
            Value::String(text) => Some(Value::from(text.clone())),
            other => Some(Value::from(other.to_string())),
        },
        ValueType::Float2 | ValueType::Int2 => {
            let component = |name: &str| stored.get(name).and_then(Value::as_f64).unwrap_or(0.0);
            if parameter.value_type == ValueType::Int2 {
                return Some(json!({"X": component("X") as i64, "Y": component("Y") as i64}));
            }
            Some(json!({"X": float(component("X")), "Y": float(component("Y"))}))
        }
        ValueType::Color => {
            let object = stored.as_object()?;
            let component = |name: &str| object.get(name).and_then(Value::as_f64).unwrap_or(0.0);
            Some(json!({"R": float(component("R")), "G": float(component("G")), "B": float(component("B"))}))
        }
        ValueType::Float2List => Some(convert_list(stored, &["X", "Y"])),
        ValueType::Float4List => Some(convert_list(stored, &["X", "Y", "Z", "W"])),
        _ => None,
    }
}

/// `Enum.Parse(type, name)`: a member's name, or its number as text.
fn enum_member(parameter: &ParameterSpec, name: &str) -> Option<String> {
    let trimmed = name.trim();
    if let Some(found) = parameter.enum_names.iter().find(|candidate| candidate.as_str() == trimmed) {
        return Some(found.clone());
    }
    let number: i64 = trimmed.parse().ok()?;
    let index = parameter.enum_values.iter().position(|value| *value == number)?;
    parameter.enum_names.get(index).cloned()
}

/// `Convert.ChangeType(long or double, int)`: doubles round to even.
fn convert_integer(stored: &Value) -> Option<i64> {
    if let Some(integer) = stored.as_i64() {
        return Some(integer);
    }
    let number = stored.as_f64()?;
    if !(-2147483648.5..2147483647.5).contains(&number) {
        return None;
    }
    Some(number.round_ties_even() as i64)
}

/// `JArray.ToObject<List<FloatN>>()`: the items (from a `$values` wrapper
/// too), each member from its own number or zero.
fn convert_list(stored: &Value, members: &[&str]) -> Value {
    let items = match stored {
        Value::Array(items) => Some(items),
        Value::Object(object) => object.get("$values").and_then(Value::as_array),
        _ => None,
    };
    let converted = items.map(|items| {
        items
            .iter()
            .map(|item| {
                let mut point = serde_json::Map::new();
                for member in members {
                    let number = item.get(*member).and_then(Value::as_f64).unwrap_or(0.0);
                    point.insert(member.to_string(), nerthus_engine::json::f32_value(number as f32));
                }
                Value::Object(point)
            })
            .collect()
    });
    Value::Array(converted.unwrap_or_default())
}

/// `JsonConvert.ToString(float)`: the shortest round-trip text, `.0` added
/// when it has neither a point nor an exponent.
fn newtonsoft_float(value: f32) -> String {
    let text = dotnet_float_string(value);
    ensure_decimal(text, value.is_finite())
}

fn newtonsoft_double(value: f64) -> String {
    let text = dotnet_double_string(value);
    ensure_decimal(text, value.is_finite())
}

fn ensure_decimal(text: String, finite: bool) -> String {
    if !finite {
        return format!("\"{text}\"");
    }
    if text.contains('.') || text.contains('E') {
        return text;
    }
    format!("{text}.0")
}

/// Indented Newtonsoft output (two spaces, `"name": value`), every object
/// starting with its `$id`, numbered in writing order.
#[derive(Default)]
struct NewtonsoftWriter {
    text: String,
    /// Whether each open object or array has an entry yet.
    open: Vec<bool>,
    next_id: u32,
}

impl NewtonsoftWriter {
    fn indent(&mut self) {
        self.text.push_str(NEW_LINE);
        for _ in 0..self.open.len() {
            self.text.push_str("  ");
        }
    }

    /// Starts an entry of the innermost object or array.
    fn entry(&mut self) {
        if let Some(has_entry) = self.open.last_mut() {
            if *has_entry {
                self.text.push(',');
            }
            *has_entry = true;
            self.indent();
        }
    }

    fn begin_object(&mut self) {
        self.text.push('{');
        self.open.push(false);
        self.next_id += 1;
        let id = self.next_id;
        self.member("$id");
        self.string(&id.to_string());
    }

    fn end_object(&mut self) {
        let had_entries = self.open.pop().unwrap_or(false);
        if had_entries {
            self.indent();
        }
        self.text.push('}');
    }

    fn begin_array(&mut self) {
        self.text.push('[');
        self.open.push(false);
    }

    fn end_array(&mut self) {
        let had_entries = self.open.pop().unwrap_or(false);
        if had_entries {
            self.indent();
        }
        self.text.push(']');
    }

    fn array_item(&mut self) {
        self.entry();
    }

    fn member(&mut self, name: &str) {
        self.entry();
        write_json_string(&mut self.text, name);
        self.text.push_str(": ");
    }

    fn string(&mut self, text: &str) {
        write_json_string(&mut self.text, text);
    }

    fn raw(&mut self, text: &str) {
        self.text.push_str(text);
    }
}

/// A JSON string as Newtonsoft escapes it (`StringEscapeHandling.Default`).
fn write_json_string(output: &mut String, text: &str) {
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
    use nerthus_engine::Catalogue;

    fn node_of(type_name: &str, values: &[(&str, Value)]) -> (NodeData, &'static NodeSpec) {
        let spec = Catalogue::get().node(type_name).expect("type exists");
        let mut node = NodeData::new(spec, 1, 23734);
        for (name, value) in values {
            node.values.insert(name.to_string(), value.clone());
        }
        (node, spec)
    }

    /// What `PresetService.CreatePreset` writes, with Windows line ends.
    #[test]
    fn writes_presets_in_the_saved_layout() {
        let (node, spec) = node_of("Perlin", &[("Scale", json!(2.5))]);
        let preset = create_preset(&node, spec, "My Perlin", Some("A test"));
        let expected_json = "{\r\n  \"$id\": \"1\",\r\n  \"Type\": \"FBM\",\r\n  \"Scale\": 2.5,\r\n  \"Octaves\": 10,\r\n  \"Gain\": 0.5,\r\n  \"Clamp\": 1.0,\r\n  \"Seed\": 23734,\r\n  \"WarpType\": \"Complex\",\r\n  \"Frequency\": 0.05,\r\n  \"Amplitude\": 0.5,\r\n  \"WarpOctaves\": 10,\r\n  \"ScaleX\": 1.0,\r\n  \"ScaleY\": 1.0,\r\n  \"X\": 0.5,\r\n  \"Y\": 0.5\r\n}";
        assert_eq!(preset.json, expected_json);
        let file = preset.to_file_text();
        assert!(file.starts_with("{\r\n  \"$id\": \"1\",\r\n  \"ApplicableType\": \"Perlin\",\r\n  \"Name\": \"My Perlin\",\r\n  \"Description\": \"A test\",\r\n  \"Json\": \"{\\r\\n  \\\"$id\\\": \\\"1\\\",\\r\\n"));
        assert!(file.ends_with("\",\r\n  \"ShortCode\": \"MP\"\r\n}"));

        let (curve, curve_spec) = node_of("Curve", &[("CurveSpline", json!([{"X": 0.0, "Y": 0.0}, {"X": 0.5, "Y": 0.8}, {"X": 1.0, "Y": 1.0}]))]);
        let curve_preset = create_preset(&curve, curve_spec, "Soft", None);
        let expected_curve_template = "{\r\n  \"$id\": \"1\",\r\n  \"CurveSpline\": {\r\n    \"$id\": \"2\",\r\n    \"$type\": \"LIST_TYPE\",\r\n    \"$values\": [\r\n      {\r\n        \"$id\": \"3\"\r\n      },\r\n      {\r\n        \"$id\": \"4\",\r\n        \"X\": 0.5,\r\n        \"Y\": 0.8\r\n      },\r\n      {\r\n        \"$id\": \"5\",\r\n        \"X\": 1.0,\r\n        \"Y\": 1.0\r\n      }\r\n    ]\r\n  },\r\n  \"Mode\": \"Absolute\",\r\n  \"FitCurve\": false\r\n}";
        let expected_curve = expected_curve_template.replace("LIST_TYPE", &value_list_type("Float2"));
        assert_eq!(curve_preset.json, expected_curve);

        let (adjust, adjust_spec) = node_of("Adjust", &[("Clamp", json!({"X": 0.1, "Y": 0.9}))]);
        let adjust_preset = create_preset(&adjust, adjust_spec, "Clamped", None);
        let clip_template = "\"Clip\": {\r\n    \"$id\": \"3\",\r\n    \"$type\": \"FLOAT2_TYPE\",\r\n    \"Y\": 1.0\r\n  },";
        let expected_clip = clip_template.replace("FLOAT2_TYPE", &value_type("Float2"));
        assert!(adjust_preset.json.contains(&expected_clip));

        let (cluter, cluter_spec) = node_of("CLUTer", &[]);
        let cluter_preset = create_preset(&cluter, cluter_spec, "Warm", None);
        assert!(cluter_preset.json.starts_with("{\r\n  \"$id\": \"1\",\r\n  \"GradientData\": null,\r\n  \"AutolevelInput\": true,"));

        let (constant, constant_spec) = node_of("Constant", &[("Color", json!({"R": 1.0, "G": 0.5, "B": 0.0}))]);
        let constant_preset = create_preset(&constant, constant_spec, "Orange", None);
        let color_template = "\"Color\": {\r\n    \"$id\": \"2\",\r\n    \"$type\": \"COLOR_TYPE\",\r\n    \"R\": 1.0,\r\n    \"G\": 0.5\r\n  }\r\n}";
        let expected_color = color_template.replace("COLOR_TYPE", &value_type("Color"));
        assert!(constant_preset.json.ends_with(&expected_color));
    }

    #[test]
    fn renamed_types_are_saved_under_the_format_name() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.as_nanos()).unwrap_or_default();
        let folder = std::env::temp_dir().join(format!("nerthus-presets-{nanos}"));
        let mut library = PresetLibrary::load(folder.clone());
        let (node, spec) = node_of("SatelliteMap", &[]);
        let mut preset = create_preset(&node, spec, "Mine", None);
        library.save(&mut preset, "Mine").unwrap();

        let format_type = format_node_name("SatelliteMap");
        let file = folder.join(format!("{format_type}--Mine"));
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains(&format!("\"ApplicableType\": \"{format_type}\"")));
        let found: Vec<&str> = library.of_type("SatelliteMap").iter().map(|preset| preset.name.as_str()).collect();
        assert_eq!(found, ["Mine"]);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn applies_preset_values() {
        let (curve, spec) = node_of("Curve", &[("CurveSpline", json!([{"X": 0.0, "Y": 0.0}, {"X": 0.5, "Y": 0.8}, {"X": 1.0, "Y": 1.0}])), ("Mode", json!("Relative"))]);
        let preset = create_preset(&curve, spec, "Soft", None);
        let values = preset_values(&preset, spec);
        let spline = values.iter().find(|(name, _)| name == "CurveSpline").map(|(_, value)| value.clone());
        assert_eq!(spline, Some(json!([{"X": 0.0, "Y": 0.0}, {"X": 0.5, "Y": 0.8}, {"X": 1.0, "Y": 1.0}])));
        let mode = values.iter().find(|(name, _)| name == "Mode").map(|(_, value)| value.clone());
        assert_eq!(mode, Some(json!("Relative")));
        // A number where an enum is wanted does not convert (`Convert.ChangeType`).
        let numbered = Preset { json: "{\"Mode\": 1}".to_string(), ..preset.clone() };
        assert!(preset_values(&numbered, spec).is_empty());
        // A struct that is not an object becomes zeros.
        let adjust_spec = Catalogue::get().node("Adjust").expect("Adjust");
        let zeroed = Preset { json: "{\"Clamp\": null}".to_string(), ..preset };
        assert_eq!(preset_values(&zeroed, adjust_spec), vec![("Clamp".to_string(), json!({"X": 0.0, "Y": 0.0}))]);
    }
}
