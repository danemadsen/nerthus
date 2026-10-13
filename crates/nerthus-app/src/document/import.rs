//! Graphs moved between projects: File › Import › Graph and TORLET and
//! File › Repair File add another terrain's graph tabs to this project
//! (`Shell.ImportTerrain`); File › Export › TORLET saves one graph tab on
//! its own (`Shell.ExportGraph`).

use std::collections::{HashMap, HashSet};

use nerthus_engine::Terrain;
use serde_json::{json, Map, Value};

use super::Document;

impl Document {
    /// `Shell.ImportTerrain`: `imported`'s graph tabs join this project's,
    /// each at the lowest free index from 1 and named
    /// "`<file stem>` - `<tab name>`", with their nodes, groups and notes.
    /// The nodes come in unbaked and unbypassed (`StateOverride.None`,
    /// `CacheStatus.None`). Returns the index of the last tab added.
    ///
    /// Every imported node, group and note gets a new id, so none replaces
    /// one of this project's, and connections to nodes that were not
    /// imported are dropped; portal ports that lose their connection stop
    /// being portals.
    pub fn import_terrain(&mut self, imported: &Terrain, file_stem: &str) -> Option<usize> {
        let imported_tabs = imported.extra.get("GraphTabs").and_then(Value::as_array).cloned().unwrap_or_default();
        if imported_tabs.is_empty() {
            return None;
        }
        self.checkpoint(format!("Graph {file_stem} imported"));

        // Tabs: each to the next free index, renamed.
        let mut used: HashSet<usize> = self.graph_tabs().iter().map(|tab| tab.index).collect();
        let mut tab_map: HashMap<usize, usize> = HashMap::new();
        let mut index = 1;
        let mut new_tabs = Vec::new();
        for tab in &imported_tabs {
            let Value::Object(fields) = tab else {
                continue;
            };
            while used.contains(&index) {
                index += 1;
            }
            let old_index = fields.get("Index").and_then(Value::as_u64).unwrap_or(0) as usize;
            tab_map.insert(old_index, index);
            let mut renamed = fields.clone();
            let name = fields.get("Name").and_then(Value::as_str).unwrap_or_default();
            renamed.insert("Index".to_string(), json!(index));
            renamed.insert("Name".to_string(), json!(format!("{file_stem} - {name}")));
            new_tabs.push(Value::Object(renamed));
            used.insert(index);
        }
        let last_tab = index;
        let extra = &mut self.project.terrain_mut().extra;
        let tabs = extra.entry("GraphTabs".to_string()).or_insert_with(|| json!([]));
        if let Value::Array(tabs) = tabs {
            tabs.extend(new_tabs);
        }

        // Nodes on the imported tabs, with new ids.
        let mut id_map: HashMap<i32, i32> = HashMap::new();
        let mut taken = self.element_ids();
        let mut nodes = Vec::new();
        for node in imported.nodes.values() {
            let tab = node.base.get("GraphIndex").and_then(Value::as_u64).unwrap_or(0) as usize;
            let Some(new_tab) = tab_map.get(&tab) else {
                continue;
            };
            let id = self.unused_id(&mut taken);
            id_map.insert(node.id, id);
            let mut copy = node.clone();
            copy.id = id;
            copy.base.insert("GraphIndex".to_string(), json!(*new_tab));
            copy.base.insert("StateOverride".to_string(), json!("None"));
            copy.base.shift_remove("CacheStatus");
            nodes.push(copy);
        }
        for node in nodes.iter_mut() {
            for port in node.ports.iter_mut() {
                let Some(record) = port.record.as_mut() else {
                    continue;
                };
                let remapped = (id_map.get(&record.from), id_map.get(&record.to));
                if let (Some(from), Some(to)) = remapped {
                    record.from = *from;
                    record.to = *to;
                    continue;
                }
                port.record = None;
                let is_portal = port.extra.get("PortalState").and_then(Value::as_str).is_some_and(|state| state != "None");
                if is_portal {
                    port.extra.shift_remove("PortalState");
                }
            }
        }
        for node in nodes {
            self.mark_stale(node.id);
            self.project.terrain_mut().nodes.insert(node.id, node);
        }

        // Notes, then groups (whose children are nodes and notes).
        let mut note_map: HashMap<i32, i32> = HashMap::new();
        let imported_notes = imported.extra.get("Notes").and_then(Value::as_object).cloned().unwrap_or_default();
        let mut notes = Map::new();
        for (key, note) in &imported_notes {
            let (Ok(old_id), Value::Object(fields)) = (key.parse::<i32>(), note) else {
                continue;
            };
            let tab = fields.get("GraphIndex").and_then(Value::as_u64).unwrap_or(0) as usize;
            let Some(new_tab) = tab_map.get(&tab) else {
                continue;
            };
            let id = self.unused_id(&mut taken);
            note_map.insert(old_id, id);
            let mut copy = fields.clone();
            copy.insert("Id".to_string(), json!(id));
            copy.insert("GraphIndex".to_string(), json!(*new_tab));
            notes.insert(id.to_string(), Value::Object(copy));
        }
        let imported_groups = imported.extra.get("Groups").and_then(Value::as_object).cloned().unwrap_or_default();
        let mut groups = Map::new();
        let mut group_map: HashMap<i32, i32> = HashMap::new();
        for (key, group) in &imported_groups {
            let (Ok(old_id), Value::Object(fields)) = (key.parse::<i32>(), group) else {
                continue;
            };
            let tab = fields.get("GraphIndex").and_then(Value::as_u64).unwrap_or(0) as usize;
            let Some(new_tab) = tab_map.get(&tab) else {
                continue;
            };
            let id = self.unused_id(&mut taken);
            group_map.insert(old_id, id);
            let mut copy = fields.clone();
            copy.insert("Id".to_string(), json!(id));
            copy.insert("GraphIndex".to_string(), json!(*new_tab));
            let children = fields.get("Children").map(nerthus_engine::json::collection_items).unwrap_or(&[]);
            let remapped: Vec<Value> = children
                .iter()
                .filter_map(Value::as_i64)
                .filter_map(|child| id_map.get(&(child as i32)).or_else(|| note_map.get(&(child as i32))))
                .map(|child| json!(*child))
                .collect();
            copy.insert("Children".to_string(), Value::Array(remapped));
            groups.insert(id.to_string(), Value::Object(copy));
        }
        for note in notes.values_mut() {
            let group = note.get("Group").and_then(Value::as_i64).map(|group| group as i32);
            if let (Some(old), Some(fields)) = (group, note.as_object_mut()) {
                match group_map.get(&old) {
                    Some(new) => fields.insert("Group".to_string(), json!(*new)),
                    None => fields.remove("Group"),
                };
            }
        }
        let extra = &mut self.project.terrain_mut().extra;
        if !notes.is_empty() {
            let existing = extra.entry("Notes".to_string()).or_insert_with(|| json!({}));
            if let Value::Object(existing) = existing {
                existing.extend(notes);
            }
        }
        if !groups.is_empty() {
            let existing = extra.entry("Groups".to_string()).or_insert_with(|| json!({}));
            if let Value::Object(existing) = existing {
                existing.extend(groups);
            }
        }
        Some(last_tab)
    }

    /// `Shell.ExportGraph` (File › Export › TORLET): graph tab `tab`'s
    /// nodes, groups and notes as a terrain of their own, all on its first
    /// tab, in a new `Terrain` (one "Graph 1" tab, 5000 by 2500).
    /// Connections to nodes on other tabs go along as they are; importing
    /// drops them.
    pub fn graph_fragment(&mut self, tab: usize) -> Terrain {
        let source = self.terrain();
        let mut nodes = indexmap::IndexMap::new();
        for node in source.nodes.values().filter(|node| super::node_tab(node) == tab) {
            let mut copy = node.clone();
            copy.base.shift_remove("GraphIndex");
            nodes.insert(copy.id, copy);
        }
        let on_tab = |fields: &Map<String, Value>| fields.get("GraphIndex").and_then(Value::as_u64).unwrap_or(0) as usize == tab;
        let mut fragment_extra = Map::new();
        for member in ["Groups", "Notes"] {
            let mut kept = Map::new();
            if let Some(Value::Object(elements)) = source.extra.get(member) {
                for (key, element) in elements {
                    let Value::Object(fields) = element else {
                        continue;
                    };
                    if !on_tab(fields) {
                        continue;
                    }
                    let mut copy = fields.clone();
                    copy.remove("GraphIndex");
                    kept.insert(key.clone(), Value::Object(copy));
                }
            }
            fragment_extra.insert(member.to_string(), Value::Object(kept));
        }
        let first_tab = json!({"Name": "Graph 1", "Color": "Brass", "ZoomFactor": 1.0, "ViewportLocation": {"X": 26000.0, "Y": 26000.0}});
        fragment_extra.insert("GraphTabs".to_string(), json!([first_tab]));
        fragment_extra.insert("Ratio".to_string(), json!(0.5));
        fragment_extra.insert("Location".to_string(), json!({"X": 0.0, "Y": 0.0}));
        fragment_extra.insert("Regions".to_string(), json!([]));
        let id = super::random_guid(&mut self.random);
        Terrain { id, nodes, width: 5000.0, height: 2500.0, extra: fragment_extra }
    }

    /// The ids nodes, groups and notes use (drawn from one pool).
    fn element_ids(&self) -> HashSet<i32> {
        let terrain = self.terrain();
        let mut ids: HashSet<i32> = terrain.nodes.keys().copied().collect();
        for member in ["Groups", "Notes"] {
            if let Some(Value::Object(elements)) = terrain.extra.get(member) {
                ids.extend(elements.keys().filter_map(|key| key.parse::<i32>().ok()));
            }
        }
        ids
    }

    /// A node id no node, group or note has, noted as taken.
    fn unused_id(&mut self, taken: &mut HashSet<i32>) -> i32 {
        let mut id = self.new_node_id();
        while taken.contains(&id) {
            id = self.new_node_id();
        }
        taken.insert(id);
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_tab_project() -> (Document, i32, i32, i32) {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [25100.0, 25100.0]).unwrap();
        let erosion = document.add_node("ErosionPro", [25400.0, 25100.0]).unwrap();
        document.connect(mountain, "Out", erosion, "In");
        let tab = document.add_graph_tab();
        let blur = document.add_node("Blur", [25700.0, 25100.0]).unwrap();
        document.connect(erosion, "Out", blur, "In");
        document.move_nodes_to_tab(&[blur], tab);
        (document, mountain, erosion, blur)
    }

    #[test]
    fn imported_graphs_get_new_tabs_and_ids() {
        let (source, mountain, erosion, blur) = two_tab_project();
        let mut target = Document::new();
        let existing = target.add_node("Mountain", [25100.0, 25100.0]).unwrap();
        let last = target.import_terrain(source.terrain(), "Source").unwrap();
        let tabs = target.graph_tabs();
        let names: Vec<&str> = tabs.iter().map(|tab| tab.name.as_str()).collect();
        assert_eq!(names, ["Graph 1", "Source - Graph 1", "Source - Graph 2"]);
        assert_eq!((tabs[1].index, tabs[2].index, last), (1, 2, 2));
        let terrain = target.terrain();
        assert_eq!(terrain.nodes.len(), 4);
        assert!(terrain.nodes.contains_key(&existing));
        // The imported erosion reads from the imported mountain, whatever
        // their ids now are.
        let imported_erosion = terrain.nodes.values().find(|node| node.type_name == "ErosionPro").unwrap();
        let record = imported_erosion.port("In").and_then(|port| port.record.clone()).unwrap();
        let from = &terrain.nodes[&record.from];
        assert_eq!(from.type_name, "Mountain");
        assert_ne!(from.id, existing);
        assert_eq!(record.to, imported_erosion.id);
        let imported_blur = terrain.nodes.values().find(|node| node.type_name == "Blur").unwrap();
        assert_eq!(super::super::node_tab(imported_blur), 2);
        let _ = (mountain, erosion, blur);
    }

    #[test]
    fn fragments_keep_one_tab() {
        let (mut source, _, erosion, blur) = two_tab_project();
        let fragment = source.graph_fragment(1);
        assert_eq!(fragment.nodes.len(), 1);
        let node = &fragment.nodes[&blur];
        assert!(!node.base.contains_key("GraphIndex"));
        // The connection from the other tab goes along.
        assert_eq!(node.port("In").and_then(|port| port.record.clone()).map(|record| record.from), Some(erosion));

        let mut target = Document::new();
        target.import_terrain(&fragment, "Frag");
        let imported = target.terrain().nodes.values().find(|node| node.type_name == "Blur").unwrap();
        assert!(imported.port("In").unwrap().record.is_none());
    }
}
