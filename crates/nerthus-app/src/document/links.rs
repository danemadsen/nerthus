//! The plumbing the graph edits share: ids that no node, group or note
//! has, connections made and broken without an undo step of their own,
//! portal states, and a node taken out of the project with everything
//! that pointed at it.

use std::collections::HashSet;

use nerthus_engine::{ConnectionRecord, NodeData, PortData};
use serde_json::{json, Value};

use super::Document;

/// `PortalState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortalState {
    None,
    Publishing,
    Consuming,
}

impl PortalState {
    pub fn of(port: &PortData) -> Self {
        match port.extra.get("PortalState").and_then(Value::as_str) {
            Some("Publishing") => Self::Publishing,
            Some("Consuming") => Self::Consuming,
            _ => Self::None,
        }
    }

    pub fn is_portal(self) -> bool {
        self != Self::None
    }

    fn name(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Publishing => Some("Publishing"),
            Self::Consuming => Some("Consuming"),
        }
    }
}

/// A node's primary input (`PortType.PrimaryIn`).
pub fn primary_input(node: &NodeData) -> Option<&PortData> {
    node.ports.iter().find(|port| port.is_input() && port.is_primary())
}

/// A node's primary output (`PortType.PrimaryOut`).
pub fn primary_output(node: &NodeData) -> Option<&PortData> {
    node.ports.iter().find(|port| port.is_output() && port.is_primary())
}

/// The node's inputs in order, the mask aside (`InPort(i)` counts the
/// `InPortsGrid`, which the mask port is not in).
pub fn grid_inputs(node: &NodeData) -> Vec<&PortData> {
    node.ports.iter().filter(|port| port.is_input() && port.name != "Mask").collect()
}

/// The node's outputs in order (`OutPort(i)`).
pub fn grid_outputs(node: &NodeData) -> Vec<&PortData> {
    node.ports.iter().filter(|port| port.is_output()).collect()
}

impl Document {
    /// An id no node, group or note has, nor any in
    /// `reserved`.
    pub(crate) fn new_element_id(&mut self, reserved: &HashSet<i32>) -> i32 {
        let groups: HashSet<i32> = self.groups().iter().map(|group| group.id).collect();
        let notes: HashSet<i32> = self.notes().iter().map(|(note, _)| *note).collect();
        let mut range = 900;
        loop {
            for _ in 0..64 {
                let id = 100 + self.random.below(range) as i32;
                let taken = self.terrain().nodes.contains_key(&id) || groups.contains(&id) || notes.contains(&id) || reserved.contains(&id);
                if !taken {
                    return id;
                }
            }
            range *= 10;
        }
    }

    /// `to_port` fed from `from_port`, replacing what fed
    /// it, as part of an edit already named.
    pub(crate) fn link(&mut self, from: i32, from_port: &str, to: i32, to_port: &str) {
        if from == to || self.node(from).is_none() {
            return;
        }
        let Some(target) = self.project.terrain_mut().nodes.get_mut(&to) else {
            return;
        };
        let Some(port) = target.port_mut(to_port) else {
            return;
        };
        let record = ConnectionRecord {
            from,
            to,
            from_port: from_port.to_string(),
            to_port: to_port.to_string(),
            is_valid: true,
        };
        port.record = Some(record);
        // SBomber's UpdateUI: a new input once every input is used.
        nerthus_engine::editing::grow_inputs(target);
        self.mark_stale(to);
        self.dirty = true;
    }

    /// The input fed by nothing; a portal it consumed goes
    /// with the connection.
    pub(crate) fn unlink(&mut self, to: i32, to_port: &str) {
        let Some(port) = self.project.terrain_mut().nodes.get_mut(&to).and_then(|node| node.port_mut(to_port)) else {
            return;
        };
        let had_record = port.record.take().is_some();
        if PortalState::of(port) == PortalState::Consuming {
            port.extra.shift_remove("PortalState");
        }
        if had_record {
            self.mark_stale(to);
            self.dirty = true;
        }
    }

    /// A port's portal state, saved only when it is one.
    pub(crate) fn write_portal_state(&mut self, node: i32, port: &str, state: PortalState) {
        let Some(target) = self.project.terrain_mut().nodes.get_mut(&node).and_then(|data| data.port_mut(port)) else {
            return;
        };
        match state.name() {
            Some(name) => {
                target.extra.insert("PortalState".to_string(), json!(name));
            }
            None => {
                target.extra.shift_remove("PortalState");
            }
        }
        self.dirty = true;
    }

    /// `miAddMaskPort_Click`: the node takes a mask port, as part of an
    /// edit already named.
    pub(crate) fn add_mask_port(&mut self, node: i32) {
        let Some(data) = self.project.terrain_mut().nodes.get_mut(&node) else {
            return;
        };
        if data.port("Mask").is_none() {
            data.add_port("Mask", "In");
        }
        self.dirty = true;
    }

    /// Whether the node is in a locked group (`NodeElement.IsLockedInGroup`):
    /// it then stays where it is and cannot be deleted on its own.
    pub fn node_locked_in_group(&self, node: i32) -> bool {
        let owner = self.group_of(node).and_then(|id| self.group(id));
        owner.is_some_and(|group| group.locked)
    }

    /// The inputs `node` feeds: (node, input port, output port).
    pub fn outgoing(&self, node: i32) -> Vec<(i32, String, String)> {
        let mut found = Vec::new();
        for other in self.terrain().nodes.values() {
            for port in &other.ports {
                let Some(record) = port.record.as_ref().filter(|record| record.from == node) else {
                    continue;
                };
                found.push((other.id, port.name.clone(), record.from_port.clone()));
            }
        }
        found
    }

    /// Deletes a node with what pointed at it: its connections both ways,
    /// its bookmark, its bindings, its place in a group, the underlay. Part
    /// of an edit already named.
    pub(crate) fn take_out_node(&mut self, id: i32) {
        let Some(removed) = self.project.terrain_mut().nodes.shift_remove(&id) else {
            return;
        };
        let fed: Vec<(i32, String, String)> = self.outgoing(id);
        for (node, input, _) in fed {
            self.unlink(node, &input);
        }
        if self.bookmarks().contains(&id)
            && let Some(bookmarks) = self.bookmarks_mut()
        {
            bookmarks.retain(|bookmark| bookmark.as_i64() != Some(id as i64));
        }
        if self.saved_underlay() == id {
            let extra = &mut self.project.assets[0].extra;
            if let Some(Value::Object(state)) = extra.get_mut("State") {
                state.insert("UnderlayNode".to_string(), json!(0));
            }
        }
        self.remove_node_bindings(id);
        self.forget_group_children(&[id]);
        self.mark_stale(removed.id);
        self.dirty = true;
    }

    /// The node's properties no longer bound.
    fn remove_node_bindings(&mut self, id: i32) {
        let bound = self.automation().bindings.iter().any(|binding| binding.node == id);
        if !bound {
            return;
        }
        let automation = self.automation_json_mut();
        if let Some(Value::Array(bindings)) = automation.get_mut("Bindings") {
            bindings.retain(|binding| binding.get("Node").and_then(Value::as_i64) != Some(id as i64));
        }
        self.refresh_automation();
    }
}
