//! Cut, Copy and Paste of nodes (`GraphControl.CopyNodes`, `CutNodes`,
//! `PasteNodes`) and Duplicate, which is a copy pasted at once and pushed
//! aside (`DuplicateNode`, `PushNodes`).

use std::collections::{HashMap, HashSet};

use nerthus_engine::NodeData;
use serde_json::{json, Value};

use super::links::PortalState;
use super::Document;

/// Copied nodes (`CopiedNodes`, `Ids`): each node as it was when copied,
/// under the id its copy will have.
#[derive(Clone, Debug, Default)]
pub struct NodeClipboard {
    pub nodes: Vec<NodeData>,
    /// (original id, copy id), in the order they were copied.
    pub ids: Vec<(i32, i32)>,
}

impl NodeClipboard {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// `PasteNodes` puts the copies 50 below their originals.
const PASTE_OFFSET: f32 = 50.0;
/// `DuplicateNode` then pushes them 20 right and 50 down.
const DUPLICATE_PUSH: [f32; 2] = [20.0, 50.0];

impl Document {
    /// `CopyNodes`: each node as it stands now, under a new id
    /// (`CopyNode`): its parameters, ports with their
    /// connections and portal states, modifiers, export settings, name,
    /// size, position, render intent and underlay exclusion. Not bypassed,
    /// not baked, not locked.
    pub fn copy_nodes(&mut self, ids: &[i32]) -> NodeClipboard {
        let mut clipboard = NodeClipboard::default();
        let mut reserved = HashSet::new();
        for id in ids {
            let Some(original) = self.node(*id).cloned() else {
                continue;
            };
            let copy_id = self.new_element_id(&reserved);
            reserved.insert(copy_id);
            let copy = copied_node(&original, copy_id);
            clipboard.nodes.push(copy);
            clipboard.ids.push((original.id, copy_id));
        }
        clipboard
    }

    /// `CutNodes`: "Cut Selection", the nodes copied and then deleted one by
    /// one as `DeleteNodeElement` deletes them.
    pub fn cut_nodes(&mut self, ids: &[i32]) -> NodeClipboard {
        let clipboard = self.copy_nodes(ids);
        if clipboard.is_empty() {
            return clipboard;
        }
        self.checkpoint("Cut Selection");
        for (original, _) in &clipboard.ids {
            self.delete_node_healing(*original);
        }
        clipboard
    }

    /// `PasteNodes`: "Paste Selection" (unless `label` is none), the copies
    /// on graph tab `tab`, 50 below where they were copied. Connections
    /// between copied nodes follow the copies; one from a node outside
    /// stays while that node is there, else it goes (with the portal it
    /// consumed). Returns the new nodes' ids.
    pub fn paste_nodes(&mut self, clipboard: &NodeClipboard, tab: usize, label: Option<&str>) -> Vec<i32> {
        if clipboard.is_empty() {
            return Vec::new();
        }
        if let Some(label) = label {
            self.checkpoint(label);
        }
        // A copy's id may have been taken since it was copied.
        let mut final_ids: HashMap<i32, i32> = HashMap::new();
        let mut reserved = HashSet::new();
        for copy in &clipboard.nodes {
            let taken = self.node(copy.id).is_some() || self.group(copy.id).is_some() || reserved.contains(&copy.id);
            let id = if taken { self.new_element_id(&reserved) } else { copy.id };
            reserved.insert(id);
            final_ids.insert(copy.id, id);
        }
        let originals: HashMap<i32, i32> = clipboard.ids.iter().map(|(original, copy)| (*original, final_ids[copy])).collect();

        let mut pasted = Vec::new();
        for copy in &clipboard.nodes {
            let id = final_ids[&copy.id];
            let mut node = copy.clone();
            node.id = id;
            node.position = [copy.position[0], copy.position[1] + PASTE_OFFSET];
            node.set_member("GraphIndex", json!(tab));
            if tab == 0 {
                node.base.shift_remove("GraphIndex");
            }
            if let Some(Value::Object(definition)) = node.base.get_mut("SaveDefinition") {
                definition.insert("Node".to_string(), json!(id));
            }
            for port in node.ports.iter_mut() {
                let Some(record) = port.record.as_mut() else {
                    continue;
                };
                record.to = id;
                if let Some(mapped) = originals.get(&record.from) {
                    record.from = *mapped;
                    continue;
                }
                let source_exists = self.project.terrain().nodes.contains_key(&record.from);
                if !source_exists {
                    port.record = None;
                    if PortalState::of(port) == PortalState::Consuming {
                        port.extra.shift_remove("PortalState");
                    }
                }
            }
            self.project.terrain_mut().nodes.insert(id, node);
            self.mark_stale(id);
            pasted.push(id);
        }
        self.dirty = true;
        let corners: Vec<(i32, [f32; 2], usize)> = pasted.iter().filter_map(|id| self.node(*id).map(|node| (*id, node.position, tab))).collect();
        self.settle_in_groups(&corners);
        pasted
    }

    /// `DuplicateNode`: "Node(s) duplicated .", the nodes copied and pasted
    /// at once, then pushed clear of the others. Returns the copies' ids.
    pub fn duplicate_selection(&mut self, ids: &[i32], tab: usize) -> Vec<i32> {
        let clipboard = self.copy_nodes(ids);
        if clipboard.is_empty() {
            return Vec::new();
        }
        self.checkpoint("Node(s) duplicated .");
        let copies = self.paste_nodes(&clipboard, tab, None);
        self.push_nodes(&copies, DUPLICATE_PUSH[0], DUPLICATE_PUSH[1]);
        copies
    }

    /// `PushNodes`: each node `horizontal` right and `vertical` down, then
    /// on by 10 and 50 while a node stands at that very spot or on that row
    /// to the right of it (at most 100 times; then 500 each way).
    pub fn push_nodes(&mut self, ids: &[i32], horizontal: f32, vertical: f32) {
        for id in ids {
            let Some(node) = self.node(*id) else {
                continue;
            };
            let start = [node.position[0] + horizontal, node.position[1] + vertical];
            let mut spot = start;
            let mut settled = false;
            for _ in 0..100 {
                let others = self.terrain().nodes.values();
                let mut blocked = false;
                for other in others {
                    let same_x = (other.position[0] - spot[0]).abs() < 0.2;
                    let same_y = (other.position[1] - spot[1]).abs() < 0.2;
                    let row_right = same_y && spot[0] < other.position[0] + 10.0;
                    if (same_x && same_y) || row_right {
                        blocked = true;
                        break;
                    }
                }
                if !blocked {
                    settled = true;
                    break;
                }
                spot = [spot[0] + 10.0, spot[1] + 50.0];
            }
            if !settled {
                spot = [start[0] + 500.0, start[1] + 500.0];
            }
            self.move_nodes(&[(*id, spot)], false);
        }
    }
}

/// A copy of a node (`CopyNode`): the node under `id`, its connections
/// still naming the original's sources.
fn copied_node(original: &NodeData, id: i32) -> NodeData {
    let mut copy = original.clone();
    copy.id = id;
    for member in ["StateOverride", "IsLocked", "IsExpanded", "Thumbprint"] {
        copy.base.shift_remove(member);
    }
    for port in copy.ports.iter_mut() {
        if let Some(record) = port.record.as_mut() {
            record.to = id;
        }
    }
    copy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasting_keeps_inner_and_outside_connections() {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [0.0, 0.0]).unwrap();
        let erosion = document.add_node("ErosionPro", [300.0, 0.0]).unwrap();
        let blur = document.add_node("Blur", [600.0, 0.0]).unwrap();
        document.connect(mountain, "Out", erosion, "In");
        document.connect(erosion, "Out", blur, "In");
        let clipboard = document.copy_nodes(&[erosion, blur]);
        let pasted = document.paste_nodes(&clipboard, 0, Some("Paste Selection"));
        assert_eq!(pasted.len(), 2);
        let erosion_copy = document.node(pasted[0]).unwrap();
        assert_eq!(erosion_copy.position, [300.0, 50.0]);
        // From outside: kept.
        assert_eq!(erosion_copy.port("In").unwrap().record.as_ref().unwrap().from, mountain);
        // Between the copies: follows them.
        let blur_copy = document.node(pasted[1]).unwrap();
        assert_eq!(blur_copy.port("In").unwrap().record.as_ref().unwrap().from, pasted[0]);
    }

    #[test]
    fn cutting_heals_and_paste_restores() {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [0.0, 0.0]).unwrap();
        let erosion = document.add_node("ErosionPro", [300.0, 0.0]).unwrap();
        let blur = document.add_node("Blur", [600.0, 0.0]).unwrap();
        document.connect(mountain, "Out", erosion, "In");
        document.connect(erosion, "Out", blur, "In");
        let clipboard = document.cut_nodes(&[erosion]);
        assert!(document.node(erosion).is_none());
        assert_eq!(document.node(blur).unwrap().port("In").unwrap().record.as_ref().unwrap().from, mountain);
        let pasted = document.paste_nodes(&clipboard, 0, Some("Paste Selection"));
        assert_eq!(document.node(pasted[0]).unwrap().port("In").unwrap().record.as_ref().unwrap().from, mountain);
    }

    #[test]
    fn duplicates_are_pushed_clear() {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [0.0, 0.0]).unwrap();
        let copies = document.duplicate_selection(&[mountain], 0);
        assert_eq!(document.node(copies[0]).unwrap().position, [20.0, 100.0]);
        assert_eq!(document.undo_labels()[0], "Node(s) duplicated .");
    }
}
