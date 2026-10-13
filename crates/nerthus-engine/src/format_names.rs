//! The .NET names `.terrain` files and their companions (presets, macros)
//! carry: the namespaces and assemblies of the `$type`s they are written
//! with, and the node types and member names that are not Nerthus's own.
//! Files are written with these so other programs that read the format
//! open them; reading matches types on their class name alone
//! ([`crate::json::type_class_name`]) and takes the format's node names
//! back to Nerthus's ([`nerthus_node_name`]). This is the one place they
//! live.

/// The namespace node types live in.
pub const NODE_NAMESPACE: &str = "QuadSpinner.Gaea.Nodes";
/// The namespace node modifiers live in.
pub const MODIFIER_NAMESPACE: &str = "QuadSpinner.Gaea.Nodes.Modifiers";
/// The assembly of node and modifier types.
pub const NODE_ASSEMBLY: &str = "Gaea.Nodes";
/// The namespace of the engine's value types (`Float2`, `Color`, `Region`, …).
pub const VALUE_NAMESPACE: &str = "QuadSpinner.Gaea.Engine.Fx";
/// The assembly of the engine's types.
pub const ENGINE_ASSEMBLY: &str = "Gaea.Engine";
/// The `$type` of a `Float2` a variable holds.
pub const VARIABLE_FLOAT2_TYPE: &str = "QuadSpinner.Gaea.Float2, Gaea.Engine";
/// The `FileFormats` member name of Pure Raw (`.graw`).
pub const PURE_RAW_FORMAT: &str = "GaeaRaw";
/// [`PURE_RAW_FORMAT`] as JSON text.
pub const PURE_RAW_FORMAT_JSON: &str = "\"GaeaRaw\"";
/// The macro metadata member holding the format version a macro was made
/// with.
pub const MACRO_VERSION_MEMBER: &str = "GaeaVersion";

/// The node types Nerthus names differently from the format, as
/// (Nerthus's name, the format's name).
const RENAMED_NODES: [(&str, &str); 3] = [("ErosionPro", "Erosion2"), ("HueErosion", "ColorErosion"), ("SatelliteMap", "SatMap")];

/// The format's name for a node type: the class in its `$type`, and the
/// type a preset is for.
pub fn format_node_name(name: &str) -> &str {
    let renamed = RENAMED_NODES.iter().find(|(nerthus_name, _)| *nerthus_name == name);
    renamed.map_or(name, |(_, format_name)| format_name)
}

/// Nerthus's name for a node type the format names `format_name`.
pub fn nerthus_node_name(format_name: &str) -> &str {
    let renamed = RENAMED_NODES.iter().find(|(_, renamed_from)| *renamed_from == format_name);
    renamed.map_or(format_name, |(nerthus_name, _)| nerthus_name)
}

/// The `$type` of a node: `type_path` is its path under [`NODE_NAMESPACE`]
/// in Nerthus's names (`Mountain`, or `Nodes.Processor.Pattern`).
pub fn node_type(type_path: &str) -> String {
    let format_path = format_node_name(type_path);
    format!("{NODE_NAMESPACE}.{format_path}, {NODE_ASSEMBLY}")
}

/// The `$type` of a node modifier.
pub fn modifier_type(class_name: &str) -> String {
    format!("{MODIFIER_NAMESPACE}.{class_name}, {NODE_ASSEMBLY}")
}

/// The `$type` of an engine value type (`Float2`, `Int2`, `Color`, `Region`).
pub fn value_type(class_name: &str) -> String {
    format!("{VALUE_NAMESPACE}.{class_name}, {ENGINE_ASSEMBLY}")
}

/// The `$type` of a `List<T>` of an engine value type.
pub fn value_list_type(class_name: &str) -> String {
    let element_type = value_type(class_name);
    format!("System.Collections.Generic.List`1[[{element_type}]], System.Private.CoreLib")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_the_format() {
        assert_eq!(node_type("Mountain"), "QuadSpinner.Gaea.Nodes.Mountain, Gaea.Nodes");
        assert_eq!(node_type("Nodes.Processor.Pattern"), "QuadSpinner.Gaea.Nodes.Nodes.Processor.Pattern, Gaea.Nodes");
        assert_eq!(modifier_type("Autolevel"), "QuadSpinner.Gaea.Nodes.Modifiers.Autolevel, Gaea.Nodes");
        assert_eq!(value_type("Region"), "QuadSpinner.Gaea.Engine.Fx.Region, Gaea.Engine");
        assert_eq!(value_list_type("Float2"), "System.Collections.Generic.List`1[[QuadSpinner.Gaea.Engine.Fx.Float2, Gaea.Engine]], System.Private.CoreLib");
        assert_eq!(PURE_RAW_FORMAT_JSON, format!("\"{PURE_RAW_FORMAT}\""));
    }

    #[test]
    fn renamed_nodes_carry_the_format_names() {
        assert_eq!(node_type("ErosionPro"), "QuadSpinner.Gaea.Nodes.Erosion2, Gaea.Nodes");
        assert_eq!(node_type("HueErosion"), "QuadSpinner.Gaea.Nodes.ColorErosion, Gaea.Nodes");
        assert_eq!(node_type("SatelliteMap"), "QuadSpinner.Gaea.Nodes.SatMap, Gaea.Nodes");
        for (nerthus_name, format_name) in RENAMED_NODES {
            assert_eq!(format_node_name(nerthus_name), format_name);
            assert_eq!(nerthus_node_name(format_name), nerthus_name);
            assert_eq!(nerthus_node_name(nerthus_name), nerthus_name, "Nerthus's names read as themselves");
        }
        assert_eq!(format_node_name("Mountain"), "Mountain");
        assert_eq!(nerthus_node_name("Mountain"), "Mountain");
    }
}
