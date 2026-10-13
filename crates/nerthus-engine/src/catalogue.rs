//! The node catalogue: every node type with its parameters, default values
//! and ports, generated into `data/nodes.toml` (and the modifiers into
//! `data/modifiers.toml`) by `tools/re/gen-specs.py` and embedded at build
//! time.

use std::borrow::Cow;
use std::sync::OnceLock;

use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

use crate::format_names::nerthus_node_name;
use crate::humanize::{humanize_title, title_case};

const NODES_TOML: &str = include_str!("../data/nodes.toml");
const MODIFIERS_TOML: &str = include_str!("../data/modifiers.toml");

/// How a parameter is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueType {
    Float,
    Double,
    Int,
    Bool,
    String,
    Byte,
    Enum,
    Float2,
    Float3,
    Float4,
    Int2,
    Color,
    Float4List,
    Float2List,
    Dictionary,
    Object,
}

/// One `[Parameter]` property of a node.
#[derive(Clone, Debug, Deserialize)]
pub struct ParameterSpec {
    pub name: String,
    #[serde(rename = "type")]
    pub value_type: ValueType,
    /// The `Parameters` kind (`Standard`, `Seed`, `HorizontalList`, …).
    pub kind: String,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub group: Option<String>,
    /// The parameter carries the `[Group]`: a new expander starts here.
    #[serde(default)]
    pub group_start: bool,
    /// That expander starts collapsed.
    #[serde(default)]
    pub group_collapsed: bool,
    #[serde(default)]
    pub label: Option<String>,
    /// An information line's text (its `[Description]`), shown in place of
    /// an editor.
    #[serde(default)]
    pub information: Option<String>,
    /// The tooltip: the node documentation's entry for the parameter.
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_true")]
    pub visible: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub separator: bool,
    #[serde(default)]
    pub force_regenerate: bool,
    #[serde(default)]
    pub curve: Option<Vec<Value>>,
    #[serde(default)]
    pub ticks: Option<Vec<Value>>,
    #[serde(default)]
    pub text_format: Option<Vec<Value>>,
    #[serde(default)]
    pub editor: Option<String>,
    #[serde(default)]
    pub command: Option<Value>,
    #[serde(default, rename = "enum")]
    pub enum_names: Vec<String>,
    #[serde(default)]
    pub enum_values: Vec<i64>,
    /// Members' `[Description]`s by member name, for the members that have
    /// one.
    #[serde(default)]
    pub enum_descriptions: IndexMap<String, String>,
}

fn default_true() -> bool {
    true
}

impl ParameterSpec {
    /// The label shown in the property panel: the `[Name]`, else the
    /// property name humanized.
    pub fn display_name(&self) -> Cow<'_, str> {
        match &self.label {
            Some(label) => Cow::Borrowed(label),
            None => Cow::Owned(humanize_title(&self.name)),
        }
    }

    /// How an enum member is shown (`Enum.Humanize(LetterCasing.Title)`):
    /// its `[Description]`, else its name humanized, in title case.
    pub fn enum_display(&self, name: &str) -> String {
        match self.enum_descriptions.get(name) {
            Some(description) => title_case(description),
            None => humanize_title(name),
        }
    }

    /// Whether new nodes draw a random value (`Global.GetRandomSeed`).
    pub fn is_seed(&self) -> bool {
        self.kind == "Seed"
    }

    /// The integer value of an enum member, by name.
    pub fn enum_value(&self, name: &str) -> Option<i64> {
        let index = self.enum_names.iter().position(|candidate| candidate == name)?;
        self.enum_values.get(index).copied()
    }

    /// The name of an enum member, by value.
    pub fn enum_name(&self, value: i64) -> Option<&str> {
        let index = self.enum_values.iter().position(|&candidate| candidate == value)?;
        self.enum_names.get(index).map(String::as_str)
    }
}

/// A port a fresh node of this type has.
#[derive(Clone, Debug, Deserialize)]
pub struct PortSpec {
    pub name: String,
    /// `PortType` flags as project files hold them (`"PrimaryIn, Required"`).
    #[serde(rename = "type")]
    pub port_type: String,
    #[serde(rename = "renderintent", default)]
    pub render_intent: Option<String>,
    #[serde(rename = "isexporting", default)]
    pub is_exporting: bool,
}

/// One node type.
#[derive(Clone, Debug, Deserialize)]
pub struct NodeSpec {
    /// The type's path under the node namespace in Nerthus's names
    /// (`Canyon`, or `Nodes.Processor.Pattern`); files name it as
    /// [`crate::format_names::node_type`] writes it.
    #[serde(rename = "type")]
    pub type_name: String,
    pub name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub toolbox: Option<String>,
    #[serde(default)]
    pub short_code: Option<String>,
    #[serde(default)]
    pub classification: Option<String>,
    #[serde(default)]
    pub keywords: Option<Value>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub flags: Vec<String>,
    #[serde(default)]
    pub custom_underlay: bool,
    /// `AccumulatorAttribute`: what this node contributes to accumulation
    /// lookups (`"Snow"`, `"Water, Shore"`, …).
    #[serde(default)]
    pub accumulator: Option<Vec<String>>,
    /// `GizmoAttribute`: the viewport gizmo the node offers (`["XY", true]`,
    /// `["SRT", true]`, …), shown as the property area's "Toggle Gizmo".
    #[serde(default)]
    pub gizmo: Option<Vec<Value>>,
    #[serde(default)]
    pub parameters: Vec<ParameterSpec>,
    /// `Node.Version` of a fresh node (0 when the constructor leaves it).
    #[serde(default)]
    pub version: i32,
    /// `Node.CanMask` as the constructor leaves it.
    #[serde(default = "default_true")]
    pub can_mask: bool,
    /// Every serialized property of a fresh node, in .terrain form.
    #[serde(default)]
    pub defaults: IndexMap<String, Value>,
    /// The node type's saved members (`Node`'s own excluded), in the order
    /// they are declared and saved.
    #[serde(default)]
    pub members: Vec<String>,
    pub ports: Vec<PortSpec>,
}

impl NodeSpec {
    /// The short type name (`Canyon`), also the catalogue key.
    pub fn key(&self) -> &str {
        self.type_name.rsplit('.').next().unwrap_or(&self.type_name)
    }

    pub fn has_flag(&self, flag: &str) -> bool {
        self.flags.iter().any(|candidate| candidate == flag)
    }

    pub fn parameter(&self, name: &str) -> Option<&ParameterSpec> {
        self.parameters.iter().find(|parameter| parameter.name == name)
    }

    /// `Node.IsMaskable`: `Generator` and `Colorizer` nodes never take a
    /// mask, other nodes unless their constructor turned `CanMask` off.
    /// Whether the node contributes `kind` (`AccummulationType` flag name)
    /// to accumulation lookups.
    pub fn accumulates(&self, kind: &str) -> bool {
        let Some(kinds) = &self.accumulator else {
            return false;
        };
        kinds.iter().any(|entry| entry.split(',').any(|flag| flag.trim() == kind))
    }

    pub fn is_maskable(&self) -> bool {
        let classification = self.classification.as_deref().unwrap_or("");
        let classification_allows = classification != "Generator" && classification != "Colorizer";
        classification_allows && self.can_mask
    }
}

/// A node modifier type.
#[derive(Clone, Debug, Deserialize)]
pub struct ModifierSpec {
    pub name: String,
    /// The class name under the modifier namespace (`Autolevel`).
    #[serde(rename = "type")]
    pub type_name: String,
    #[serde(default)]
    pub parameters: Vec<ParameterSpec>,
    /// The parameters of a fresh modifier, in .terrain form.
    #[serde(default)]
    pub defaults: IndexMap<String, Value>,
    /// `Modifier.Order` of a fresh one (66 for Min and Max).
    #[serde(default)]
    pub order: i64,
    /// `Modifier.HasUI`: whether the property area shows its parameters
    /// (otherwise just its name).
    #[serde(default)]
    pub has_ui: bool,
}

/// Every node type, keyed by short type name, and every modifier type.
pub struct Catalogue {
    nodes: IndexMap<String, NodeSpec>,
    modifiers: IndexMap<String, ModifierSpec>,
}

impl Catalogue {
    fn parse() -> Self {
        let nodes: IndexMap<String, NodeSpec> = toml::from_str(NODES_TOML).expect("the embedded node catalogue is valid");
        let modifiers: IndexMap<String, ModifierSpec> = toml::from_str(MODIFIERS_TOML).expect("the embedded modifier catalogue is valid");
        Self { nodes, modifiers }
    }

    pub fn modifier(&self, name: &str) -> Option<&ModifierSpec> {
        self.modifiers.get(name)
    }

    /// The catalogue compiled into Nerthus.
    pub fn get() -> &'static Catalogue {
        static CATALOGUE: OnceLock<Catalogue> = OnceLock::new();
        CATALOGUE.get_or_init(Catalogue::parse)
    }

    pub fn node(&self, key: &str) -> Option<&NodeSpec> {
        self.nodes.get(key)
    }

    /// Looks a node up by the type written in `$type`, by its class name.
    pub fn node_for_type(&self, dotnet_type: &str) -> Option<&NodeSpec> {
        let class_name = crate::json::type_class_name(dotnet_type);
        let key = nerthus_node_name(class_name);
        self.nodes.get(key)
    }

    pub fn nodes(&self) -> impl Iterator<Item = &NodeSpec> {
        self.nodes.values()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format_names::{format_node_name, node_type};

    #[test]
    fn catalogue_has_every_node() {
        let catalogue = Catalogue::get();
        // Every node type; the unfinished River2 and VFX are left out.
        assert_eq!(catalogue.len(), 193);
        assert!(catalogue.node("River2").is_none());
        assert!(catalogue.node("VFX").is_none());
        let canyon = catalogue.node("Canyon").expect("Canyon is in the catalogue");
        assert_eq!(canyon.family.as_deref(), Some("Landscape"));
        assert!(canyon.has_flag("RequiresBaking"));
        let style = canyon.parameter("Style").expect("Canyon has a Style");
        assert_eq!(style.enum_value("Eroded"), Some(1));
        assert_eq!(canyon.defaults["Slot"], serde_json::json!(0.36));
    }

    #[test]
    fn dotnet_type_names_resolve() {
        let catalogue = Catalogue::get();
        let spec = catalogue.node_for_type(&node_type("ErosionPro"));
        assert_eq!(spec.map(NodeSpec::key), Some("ErosionPro"));
        let pattern = catalogue.node("Pattern").expect("Pattern is in the catalogue");
        let pattern_type = node_type(&pattern.type_name);
        let found = catalogue.node_for_type(&pattern_type);
        assert_eq!(found.map(NodeSpec::key), Some("Pattern"));
        let format_class = format_node_name("ErosionPro");
        let elsewhere = catalogue.node_for_type(&format!("Vendor.Terrain.Nodes.{format_class}, Vendor.Nodes"));
        assert_eq!(elsewhere.map(NodeSpec::key), Some("ErosionPro"), "types are matched on their class name");
        assert!(catalogue.node_for_type(&node_type("NoSuchNode")).is_none());
    }

    #[test]
    fn multi_word_parameters_have_their_help() {
        let catalogue = Catalogue::get();
        let accumulator = catalogue.node("Accumulator").expect("Accumulator is in the catalogue");
        let restrict = accumulator.parameter("RestrictToGroup").expect("Accumulator has RestrictToGroup");
        let help = restrict.description.as_deref().unwrap_or_default();
        assert!(help.starts_with("Restricts accumulating"), "found {help:?}");
        // An information line keeps its text apart from its help.
        let aperture = catalogue.node("Aperture").expect("Aperture is in the catalogue");
        let information = aperture.parameter("Information").expect("Aperture has an information line");
        assert!(information.information.as_deref().is_some_and(|text| text.starts_with("PREVIEW VERSION")));
        assert!(information.description.as_deref().is_some_and(|text| text.starts_with("Displays additional information")));
    }
}
