//! Portals: an output published once (`PortalState.Publishing`) and
//! consumed by inputs anywhere (`Consuming`) without a line between them.
//! Moving nodes to another graph tab turns the connections it would cut
//! into portals (`GraphArea.MoveElementsToTab`), as do Group to Tab and Tab
//! to Group.

use serde_json::Value;

use super::links::PortalState;
use super::{node_tab, Document};

/// The node menu's Move to Tab offers "New Graph" after the tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabTarget {
    Existing(usize),
    New,
}

impl Document {
    /// A port's portal state.
    pub fn portal_state(&self, node: i32, port: &str) -> PortalState {
        let found = self.node(node).and_then(|data| data.port(port));
        found.map(PortalState::of).unwrap_or(PortalState::None)
    }

    /// `ConvertSelectedConnectionsToPortals` (Ctrl P): "Connection
    /// converted to portal." (or "Connections"), each connection's output
    /// publishing and its input consuming.
    pub fn convert_connections_to_portals(&mut self, connections: &[(i32, String)]) {
        let mut sources = Vec::new();
        for (to, to_port) in connections {
            let record = self.node(*to).and_then(|node| node.port(to_port)).and_then(|port| port.record.clone());
            if let Some(record) = record {
                sources.push((record.from, record.from_port, *to, to_port.clone()));
            }
        }
        if sources.is_empty() {
            return;
        }
        let plural = if sources.len() > 1 { "s" } else { "" };
        self.checkpoint(format!("Connection{plural} converted to portal."));
        for (from, from_port, to, to_port) in sources {
            self.write_portal_state(from, &from_port, PortalState::Publishing);
            self.write_portal_state(to, &to_port, PortalState::Consuming);
        }
    }

    /// The node menu's Convert port to portals (`Shell.MiPort_Click`): an
    /// output that is a portal stops being one ("Out portal removed."),
    /// one that is not starts publishing ("Out converted to portal.").
    pub fn toggle_port_portal(&mut self, node: i32, port: &str) {
        if self.portal_state(node, port).is_portal() {
            self.checkpoint(format!("{port} portal removed."));
            self.write_portal_state(node, port, PortalState::None);
        } else {
            self.checkpoint(format!("{port} converted to portal."));
            self.write_portal_state(node, port, PortalState::Publishing);
        }
    }

    /// Manage Portals › Convert to Portal (`MiNormalPort_Click`): the
    /// output publishes. With Shift, a small Chokepoint named after the
    /// port, 300 to the right, takes the output and publishes instead.
    pub fn publish_port(&mut self, node: i32, port: &str, through_chokepoint: bool) {
        let Some(position) = self.node(node).map(|data| data.position) else {
            return;
        };
        self.checkpoint(format!("{port} converted to portal."));
        if !through_chokepoint {
            self.write_portal_state(node, port, PortalState::Publishing);
            return;
        }
        let tab = self.node(node).map(node_tab).unwrap_or(0);
        let spot = [position[0] + 300.0, position[1]];
        let Some(chokepoint) = self.place_node("Chokepoint", spot, tab) else {
            return;
        };
        if let Some(added) = self.project.terrain_mut().nodes.get_mut(&chokepoint) {
            added.name = port.to_string();
            added.set_member("NodeSize", serde_json::json!("Small"));
        }
        self.link(node, port, chokepoint, "In");
        self.write_portal_state(chokepoint, "Out", PortalState::Publishing);
    }

    /// Manage Portals › Remove Portal (`RemovePortal`): "Out portal
    /// removed.", the output and every input consuming it no longer
    /// portals (the connections stay, as lines).
    pub fn remove_portal(&mut self, node: i32, port: &str) {
        self.checkpoint(format!("{port} portal removed."));
        let consumers: Vec<(i32, String)> = self.outgoing(node).into_iter().filter(|(_, _, output)| output == port).map(|(target, input, _)| (target, input)).collect();
        for (target, input) in consumers {
            if self.consumes_portal(target, &input) {
                self.write_portal_state(target, &input, PortalState::None);
            }
        }
        self.write_portal_state(node, port, PortalState::None);
    }

    /// Manage Portals' Disconnect (`MiInputConsumingPort_Click`): the input
    /// stops consuming and is fed by nothing.
    pub fn disconnect_portal_input(&mut self, node: i32, port: &str) {
        self.checkpoint("Connection deleted.");
        self.write_portal_state(node, port, PortalState::None);
        self.unlink(node, port);
    }

    /// `ConnectToPortal`: the input fed from a published output, consuming
    /// it.
    pub fn connect_to_portal(&mut self, from: i32, from_port: &str, to: i32, to_port: &str) {
        if self.would_cycle(from, to) {
            return;
        }
        self.checkpoint("Connect");
        self.link(from, from_port, to, to_port);
        self.write_portal_state(to, to_port, PortalState::Consuming);
    }

    /// `MoveElementsToTab`'s portals: every connection between a node of
    /// `moved` and one left on another tab than `tab` becomes a portal.
    fn portal_cut_connections(&mut self, moved: &[i32], tab: usize) {
        let mut crossing = Vec::new();
        for id in moved {
            let Some(node) = self.node(*id) else {
                continue;
            };
            for port in node.ports.iter().filter(|port| port.is_input()) {
                let Some(record) = &port.record else {
                    continue;
                };
                let source_tab = self.node(record.from).map(node_tab);
                if source_tab.is_some_and(|source_tab| source_tab != tab) {
                    crossing.push((record.from, record.from_port.clone(), *id, port.name.clone()));
                }
            }
            for (target, input, output) in self.outgoing(*id) {
                let target_tab = self.node(target).map(node_tab);
                if target_tab.is_some_and(|target_tab| target_tab != tab) {
                    crossing.push((*id, output, target, input));
                }
            }
        }
        for (from, from_port, to, to_port) in crossing {
            self.write_portal_state(from, &from_port, PortalState::Publishing);
            self.write_portal_state(to, &to_port, PortalState::Consuming);
        }
    }

    /// Puts nodes and notes on graph tab `tab`; a node leaves a group of
    /// another tab.
    fn set_elements_tab(&mut self, nodes: &[i32], notes: &[i32], tab: usize) {
        for id in nodes {
            if let Some(node) = self.project.terrain_mut().nodes.get_mut(id) {
                node.set_member("GraphIndex", serde_json::json!(tab));
            }
            let owner = self.group_of(*id).and_then(|group| self.group(group));
            if let Some(owner) = owner.filter(|owner| owner.graph_index != tab) {
                self.remove_from_group(owner.id, &[*id]);
            }
        }
        for id in notes {
            self.set_note_tab(*id, tab);
        }
        self.dirty = true;
    }

    fn set_note_tab(&mut self, id: i32, tab: usize) {
        let notes = self.project.terrain_mut().extra.get_mut("Notes");
        let note = notes.and_then(|notes| notes.get_mut(id.to_string()));
        let Some(Value::Object(fields)) = note else {
            return;
        };
        if tab == 0 {
            fields.shift_remove("GraphIndex");
        } else {
            fields.insert("GraphIndex".to_string(), serde_json::json!(tab));
        }
    }

    /// Node › Move to Tab (`MoveSelectedItemClicked`): "Selected items
    /// moved to (Name) Tab", the nodes, notes and groups on that tab (or a
    /// new one), the connections it cuts made portals. Returns the tab.
    pub fn move_elements_to_tab(&mut self, nodes: &[i32], notes: &[i32], groups: &[i32], target: TabTarget) -> usize {
        // The menu entry's header names the step ("New Graph" for a new tab,
        // which then keeps a step of its own).
        let header = match target {
            TabTarget::Existing(index) => self.graph_tabs().iter().find(|candidate| candidate.index == index).map(|candidate| candidate.name.clone()).unwrap_or_default(),
            TabTarget::New => "New Graph".to_string(),
        };
        self.checkpoint(format!("Selected items moved to ({header}) Tab"));
        let tab = match target {
            TabTarget::Existing(index) => index,
            TabTarget::New => self.add_graph_tab(),
        };
        for id in groups {
            if let Some(mut group) = self.group(*id) {
                group.graph_index = tab;
                self.store_group(&group);
            }
        }
        self.set_elements_tab(nodes, notes, tab);
        self.portal_cut_connections(nodes, tab);
        tab
    }

    /// Group to Tab (`ConvertGroupToTab`): a new tab named after the group
    /// takes its nodes and notes (the connections crossing its edge made
    /// portals) and the group goes. A locked group stays. Returns the new
    /// tab.
    pub fn group_to_tab(&mut self, id: i32) -> Option<usize> {
        let group = self.group(id)?;
        if group.locked {
            return None;
        }
        // `CreateNewTab(groupElement.DisplayName)`, then `DeleteGroupElement`:
        // two steps to undo.
        self.checkpoint(format!("New graph tab ({}) added", group.name));
        let tab = self.add_tab_quietly();
        if let Some(Value::Array(tabs)) = self.project.terrain_mut().extra.get_mut("GraphTabs") {
            let created = tabs.iter_mut().filter_map(Value::as_object_mut).find(|candidate| candidate.get("Index").and_then(Value::as_u64) == Some(tab as u64));
            if let Some(created) = created {
                created.insert("Name".to_string(), serde_json::json!(group.name));
            }
        }
        self.checkpoint("Group deleted.");
        let nodes: Vec<i32> = group.children.iter().copied().filter(|child| self.node(*child).is_some()).collect();
        let note_ids: Vec<i32> = self.notes().iter().map(|(note, _)| *note).collect();
        let notes: Vec<i32> = group.children.iter().copied().filter(|child| note_ids.contains(child)).collect();
        self.set_elements_tab(&nodes, &notes, tab);
        self.portal_cut_connections(&nodes, tab);
        if let Some(Value::Object(groups)) = self.project.terrain_mut().extra.get_mut("Groups") {
            groups.shift_remove(&id.to_string());
        }
        Some(tab)
    }

    /// Tab to group (`MoveTabItemsToGroup`): "(Name) Tab converted to
    /// group", every node and note of tab `from` moved to `target` inside
    /// a grey group named after the tab around where they were (at least
    /// 560 by 400). The tab itself stays. Returns the target tab.
    pub fn tab_to_group(&mut self, from: usize, target: TabTarget, extent: &dyn Fn(i32) -> [f32; 2]) -> usize {
        let name = self.graph_tabs().iter().find(|candidate| candidate.index == from).map(|candidate| candidate.name.clone()).unwrap_or_default();
        self.checkpoint(format!("({name}) Tab converted to group"));
        let tab = match target {
            TabTarget::Existing(index) => index,
            TabTarget::New => self.add_tab_quietly(),
        };
        let nodes: Vec<i32> = self.terrain().nodes.values().filter(|node| node_tab(node) == from).map(|node| node.id).collect();
        let notes: Vec<(i32, [f32; 2], [f32; 2])> = self.notes().iter().filter(|(_, fields)| fields.get("GraphIndex").and_then(Value::as_u64).unwrap_or(0) as usize == from).map(|(note, fields)| (*note, vector(fields, "Position"), vector(fields, "Size"))).collect();
        let mut boxes: Vec<([f32; 2], [f32; 2])> = Vec::new();
        for id in &nodes {
            if let Some(node) = self.node(*id) {
                boxes.push((node.position, extent(*id)));
            }
        }
        for (_, position, size) in &notes {
            boxes.push((*position, *size));
        }
        let (corner, span) = tab_extents(&boxes);
        let size = [(span[0] + 70.0).max(560.0), (span[1] + 90.0).max(400.0)];
        let note_ids: Vec<i32> = notes.iter().map(|(note, _, _)| *note).collect();
        self.set_elements_tab(&nodes, &note_ids, tab);
        let mut children = nodes.clone();
        children.extend(note_ids);
        let group_id = self.new_element_id(&Default::default());
        let group = super::groups::GroupData {
            id: group_id,
            name,
            color: "Gray".to_string(),
            icon: String::new(),
            children,
            locked: false,
            collapsed: false,
            position: [corner[0] - 30.0, corner[1] - 40.0],
            size,
            graph_index: tab,
        };
        self.store_group(&group);
        self.portal_cut_connections(&nodes, tab);
        tab
    }

    /// `CreateNewTab(skipUndo: true)`: part of an edit already named.
    pub(crate) fn add_tab_quietly(&mut self) -> usize {
        let index = self.add_graph_tab();
        // `add_graph_tab` keeps a step of its own; the edit has one.
        self.undo.pop();
        index
    }
}

fn vector(fields: &serde_json::Map<String, Value>, name: &str) -> [f32; 2] {
    let member = fields.get(name);
    let component = |axis: &str| member.and_then(|member| member.get(axis)).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    [component("X"), component("Y")]
}

/// `GraphArea.GetExtents`: from the first element's corner, the least and
/// greatest corners of all, padded 200 across and 100 down; returns the
/// top-left corner and the size.
fn tab_extents(boxes: &[([f32; 2], [f32; 2])]) -> ([f32; 2], [f32; 2]) {
    let Some((first_position, first_size)) = boxes.first() else {
        return ([0.0, 0.0], [0.0, 0.0]);
    };
    let mut left = first_position[0];
    let mut top = first_position[1];
    let mut right = first_position[0] + first_size[0];
    // The bottom starts at twice the first element's height.
    let mut bottom = first_size[1] + first_size[1];
    for (position, _) in boxes {
        left = left.min(position[0]);
        top = top.min(position[1]);
        right = right.max(position[0]);
        bottom = bottom.max(position[1]);
    }
    let span = [right + 200.0 - left, bottom + 100.0 - top];
    ([left, top], span)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_a_node_away_makes_portals() {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [0.0, 0.0]).unwrap();
        let erosion = document.add_node("ErosionPro", [300.0, 0.0]).unwrap();
        document.connect(mountain, "Out", erosion, "In");
        let tab = document.move_elements_to_tab(&[erosion], &[], &[], TabTarget::New);
        assert_eq!(tab, 1);
        assert_eq!(document.portal_state(mountain, "Out"), PortalState::Publishing);
        assert_eq!(document.portal_state(erosion, "In"), PortalState::Consuming);
        document.remove_portal(mountain, "Out");
        assert_eq!(document.portal_state(erosion, "In"), PortalState::None);
        // The connection stays.
        assert!(document.node(erosion).unwrap().port("In").unwrap().record.is_some());
    }

    #[test]
    fn a_group_becomes_a_tab_and_back() {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [26000.0, 26000.0]).unwrap();
        let erosion = document.add_node("ErosionPro", [26300.0, 26000.0]).unwrap();
        document.connect(mountain, "Out", erosion, "In");
        let group = document.add_group(&[erosion], [26280.0, 25960.0], [300.0, 300.0], 0);
        let tab = document.group_to_tab(group).unwrap();
        assert!(document.group(group).is_none());
        assert_eq!(node_tab(document.node(erosion).unwrap()), tab);
        assert_eq!(document.graph_tabs().iter().find(|candidate| candidate.index == tab).unwrap().name, "Group");
        assert_eq!(document.portal_state(erosion, "In"), PortalState::Consuming);

        let size = |_: i32| [230.0, 80.0];
        let back = document.tab_to_group(tab, TabTarget::Existing(0), &size);
        assert_eq!(back, 0);
        assert_eq!(node_tab(document.node(erosion).unwrap()), 0);
        let groups = document.groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].children, vec![erosion]);
        assert_eq!(groups[0].size, [560.0, 400.0]);
    }
}
