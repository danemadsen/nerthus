//! What File > Open does around deserializing a project: text rewrites
//! before, and after it, for every node: nodes older than their type are
//! migrated (`Node.Migrate()`) and take the type's version, and a node's
//! first `SaveDefinitions` entry becomes its `SaveDefinition`.

use serde_json::{json, Value};

use crate::catalogue::NodeSpec;
use crate::json::f32_value;
use crate::project::NodeData;

/// Renames applied to the file text before it is parsed. They
/// match literal patterns, spacing included.
pub fn rewrite_text(text: &str) -> String {
    let mut text = text.replace("\"BlendMode\":\"Sqrt\"", "\"BlendMode\":\"Power\"");
    for layer in 1..=9 {
        let spaced_old = format!("\"Name\": \"MaskLayer{layer}\"");
        let spaced_new = format!("\"Name\": \"MaskOut{layer}\"");
        text = text.replace(&spaced_old, &spaced_new);
    }
    for layer in 1..=9 {
        let compact_old = format!("\"Name\":\"MaskLayer{layer}\"");
        let compact_new = format!("\"Name\":\"MaskOut{layer}\"");
        text = text.replace(&compact_old, &compact_new);
    }
    text.replace(".Nodes.Trees2", ".Nodes.Trees")
}

/// The migration after deserializing, for one node.
pub fn migrate_node(node: &mut NodeData, spec: &NodeSpec) {
    if node.version() < spec.version {
        migrate(node);
        node.base.insert("Version".to_string(), Value::from(spec.version));
    }
    let first_definition = node.base.get("SaveDefinitions").and_then(Value::as_array).and_then(|definitions| definitions.first()).cloned();
    if let Some(definition) = first_definition {
        node.base.insert("SaveDefinition".to_string(), definition);
        node.base.shift_remove("SaveDefinitions");
    }
}

/// The nodes' own `Migrate` overrides.
fn migrate(node: &mut NodeData) {
    let version = node.version();
    match node.type_name.as_str() {
        "ErosionPro" => {
            if version < 2 {
                node.add_port("Precipitation", "In");
            }
        }
        "TextureBase" => {
            if version < 2 {
                node.add_port("Guide", "In");
            }
        }
        "Cellular3D" => {
            if version < 2 {
                node.add_port("Gap", "In");
            }
        }
        "ThermalShaper" => {
            if version < 2 {
                node.add_port("Intensity", "In");
            }
        }
        "Trees" => {
            if version < 2 && node.port("FreshWater").is_none() {
                node.add_port("FreshWater", "Out");
            }
        }
        "Debris" => {
            if version < 2 {
                let size = node.values.get("Size").map(read_float2).unwrap_or([0.0, 0.0]);
                let scaled = [size[0] / 10.0, size[1] / 10.0];
                node.values.insert("Size".to_string(), float2(scaled));
            }
            if version < 3 {
                node.add_port("Removal", "In");
            }
        }
        "Transform" => {
            if version < 2 {
                for name in ["OffsetX", "OffsetY"] {
                    let offset = node.values.get(name).and_then(Value::as_f64).unwrap_or(0.0) as f32;
                    if offset != 0.0 {
                        node.values.insert(name.to_string(), f32_value(offset + 0.5));
                    }
                }
            }
        }
        // Layers 7 to 15 arrived with version 2: each a fresh
        // `BlendLayer { Index, Order, Range = (0.9, 1) }`.
        "Mixer" if version == 0 => {
            for index in 6..=14 {
                let layer = new_blend_layer(index);
                node.values.insert(format!("Layer{}", index + 1), layer);
            }
        }
        _ => {}
    }
}

/// A `BlendLayer` as its field initializers make it, with the index,
/// order and range Mixer's migration sets.
fn new_blend_layer(index: i64) -> Value {
    json!({
        "Mask": "Height",
        "BlendMode": "Blend",
        "Blending": 1.0,
        "Range": {"X": 0.9, "Y": 1.0},
        "Range2": {"X": 0.0, "Y": 1.0},
        "Falloff": 0.0001,
        "Falloff2": 0.0001,
        "Warp": 0.0,
        "Size": 0.25,
        "Color": {"R": 0.0, "G": 0.0, "B": 0.0},
        "IsVisible": true,
        "IsSimplified": false,
        "Order": index,
        "Index": index,
    })
}

fn read_float2(value: &Value) -> [f32; 2] {
    let component = |name: &str| value.get(name).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    [component("X"), component("Y")]
}

fn float2(value: [f32; 2]) -> Value {
    json!({"X": f32_value(value[0]), "Y": f32_value(value[1])})
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::Catalogue;
    use crate::format_names::node_type;

    #[test]
    fn rewrites_match_literal_patterns() {
        let old_type = node_type("Trees2");
        let new_type = node_type("Trees");
        let text = format!("\"$type\": \"{old_type}\", \"Name\": \"MaskLayer3\", \"Name\":\"MaskLayer9\", \"BlendMode\":\"Sqrt\", \"BlendMode\": \"Sqrt\"");
        let rewritten = rewrite_text(&text);
        let expected = format!("\"$type\": \"{new_type}\", \"Name\": \"MaskOut3\", \"Name\":\"MaskOut9\", \"BlendMode\":\"Power\", \"BlendMode\": \"Sqrt\"");
        assert_eq!(rewritten, expected);
    }

    #[test]
    fn old_debris_is_migrated() {
        let spec = Catalogue::get().node("Debris").unwrap();
        let mut node = NodeData::new(spec, 529, 1);
        node.base.shift_remove("Version");
        node.ports.retain(|port| port.name != "Removal");
        node.values.insert("Size".to_string(), json!({"X": 3.9018564, "Y": 13.77961}));
        migrate_node(&mut node, spec);
        assert_eq!(node.version(), spec.version);
        assert!(node.port("Removal").is_some_and(|port| port.is_input()));
        let size = read_float2(&node.values["Size"]);
        assert_eq!(size, [3.9018564f32 / 10.0, 13.77961f32 / 10.0]);
    }

    #[test]
    fn current_nodes_are_left_alone() {
        let spec = Catalogue::get().node("Debris").unwrap();
        let mut node = NodeData::new(spec, 1, 1);
        node.values.insert("Size".to_string(), json!({"X": 0.5, "Y": 1.5}));
        let ports_before = node.ports.len();
        migrate_node(&mut node, spec);
        assert_eq!(node.ports.len(), ports_before);
        assert_eq!(node.values["Size"], json!({"X": 0.5, "Y": 1.5}));
    }

    #[test]
    fn first_save_definition_is_kept() {
        let spec = Catalogue::get().node("Mountain").unwrap();
        let mut node = NodeData::new(spec, 1, 1);
        node.base.insert("SaveDefinitions".to_string(), json!([{"Filename": "a"}, {"Filename": "b"}]));
        migrate_node(&mut node, spec);
        assert_eq!(node.base.get("SaveDefinition"), Some(&json!({"Filename": "a"})));
        assert!(!node.base.contains_key("SaveDefinitions"));
    }
}
