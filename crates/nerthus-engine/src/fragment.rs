//! TORLET graph fragments (`.tfrag`): one graph tab's nodes, groups and
//! notes saved as a terrain of their own (File › Export › TORLET),
//! for File › Import to add to another project.

use crate::catalogue::Catalogue;
use crate::json::{parse_lenient, References};
use crate::project::{parse_terrain, ProjectError, Terrain};

/// Reads a fragment (`Terrain`), every node migrated to its type's
/// version as a project's are on open. Returns the terrain and the node
/// types that are not in the catalogue.
pub fn parse_fragment(text: &str) -> Result<(Terrain, Vec<String>), ProjectError> {
    let document = parse_lenient(text)?;
    let references = References::collect(&document);
    let catalogue = Catalogue::get();
    let mut unknown_types = Vec::new();
    let mut terrain = parse_terrain(&document, &references, catalogue, &mut unknown_types)?;
    for node in terrain.nodes.values_mut() {
        if let Some(spec) = catalogue.node(&node.type_name) {
            crate::migrate::migrate_node(node, spec);
        }
    }
    Ok((terrain, unknown_types))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format_names::node_type;
    use crate::project::Project;
    use crate::writer::to_fragment_json;

    #[test]
    fn fragments_round_trip() {
        let template = r#"{
          "Assets": {"$values": [{
            "Terrain": {
              "Id": "8b2d5f35-3c63-4c1c-9e55-0d2a6f0f9b4b",
              "Nodes": {
                "101": {"$type": "MOUNTAIN_TYPE", "Id": 101, "Name": "Mountain", "Position": {"X": 26000.0, "Y": 26000.0},
                  "Ports": {"$values": [{"Name": "In", "Type": "PrimaryIn"}, {"Name": "Out", "Type": "PrimaryOut"}]}},
                "102": {"$type": "EROSION_TYPE", "Id": 102, "Name": "ErosionPro", "Position": {"X": 26300.0, "Y": 26000.0},
                  "Ports": {"$values": [{"Name": "In", "Type": "PrimaryIn, Required", "Record": {"From": 101, "To": 102, "FromPort": "Out", "ToPort": "In", "IsValid": true}}, {"Name": "Out", "Type": "PrimaryOut"}]}}
              },
              "Groups": {}, "Notes": {},
              "GraphTabs": {"$values": [{"Name": "Graph 1", "Color": "Brass"}]},
              "Width": 5000.0, "Height": 2500.0
            }
          }]}
        }"#;
        let with_mountain = template.replace("MOUNTAIN_TYPE", &node_type("Mountain"));
        let text = with_mountain.replace("EROSION_TYPE", &node_type("ErosionPro"));
        let project = Project::parse(&text).unwrap();
        let written = to_fragment_json(project.terrain());
        assert!(written.starts_with("{\r\n  \"$id\": \"1\""));
        assert!(written.contains(&node_type("ErosionPro")), "renamed nodes are written with the format's name");
        let (terrain, unknown) = parse_fragment(&written).unwrap();
        assert!(unknown.is_empty());
        assert_eq!(terrain.nodes.len(), 2);
        assert_eq!(terrain.nodes[&102].type_name, "ErosionPro");
        let record = terrain.nodes[&102].port("In").and_then(|port| port.record.clone()).unwrap();
        assert_eq!((record.from, record.from_port.as_str()), (101, "Out"));
        assert_eq!(terrain.width, 5000.0);
    }
}
