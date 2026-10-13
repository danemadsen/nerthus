//! The plumbing the graph edits share: ids that no node, group or note
//! has, connections made and broken without an undo step of their own,
//! the render intent a new connection passes on, portal states, and a
//! node taken out of the project with everything that pointed at it.

use std::collections::HashSet;

use nerthus_engine::{Catalogue, ConnectionRecord, NodeData, PortData, RenderIntent};
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

/// `Node.RenderIntentOverride` (Default when the node has none).
fn intent_override(node: &NodeData) -> RenderIntent {
    let name = node.base.get("RenderIntentOverride").and_then(Value::as_str);
    name.and_then(RenderIntent::from_name).unwrap_or_default()
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
        self.convey_render_intent(from, from_port, to, to_port);
    }

    /// A port's intent as it stands (`Port.RenderIntent`): what the node's
    /// last build left on it, else what the port was created with.
    pub(crate) fn port_intent(&self, node: i32, port: &str) -> RenderIntent {
        let built = self.built_intents.get(&node).and_then(|intents| intents.get(port));
        if let Some(intent) = built {
            return *intent;
        }
        let declared = self.node(node).and_then(|data| data.port(port)).and_then(|data| data.declared_intent);
        declared.unwrap_or_default()
    }

    /// `ConveyRenderIntent`, once `to_port` is fed from `from_port`: a node
    /// fed through its primary input takes the source node's render intent
    /// override, or Mask when the source port's intent is Mask. A colorizer
    /// keeps its own, and so does a node whose main output is a colour.
    pub(crate) fn convey_render_intent(&mut self, from: i32, from_port: &str, to: i32, to_port: &str) {
        let (Some(source), Some(target)) = (self.node(from), self.node(to)) else {
            return;
        };
        let primary = target.port(to_port).is_some_and(|port| port.is_primary());
        let spec = Catalogue::get().node(&target.type_name);
        let colorizer = spec.and_then(|spec| spec.classification.as_deref()) == Some("Colorizer");
        if !primary || colorizer || self.port_intent(to, "Out") == RenderIntent::Color {
            return;
        }
        let source_port_is_mask = self.port_intent(from, from_port) == RenderIntent::Mask;
        let conveyed = if source_port_is_mask { RenderIntent::Mask } else { intent_override(source) };
        if intent_override(target) == conveyed {
            return;
        }
        let Some(target) = self.project.terrain_mut().nodes.get_mut(&to) else {
            return;
        };
        target.set_member("RenderIntentOverride", json!(conveyed.name()));
        self.mark_stale(to);
        self.dirty = true;
    }

    /// `Port.IsConnected`: an input fed by a node, an output feeding one.
    pub fn port_connected(&self, node: i32, port: &PortData) -> bool {
        if port.is_input() {
            return port.record.as_ref().is_some_and(|record| self.node(record.from).is_some());
        }
        let fed = self.outgoing(node);
        fed.iter().any(|(_, _, output)| *output == port.name)
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
        self.forget_locked_node(id);
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use nerthus_engine::PortIntents;

    use super::*;

    fn override_of(document: &Document, node: i32) -> RenderIntent {
        intent_override(document.node(node).unwrap())
    }

    #[test]
    fn connections_pass_render_intents_on_through_primary_inputs() {
        let mut document = Document::new();
        let erosion = document.add_node("ErosionPro", [0.0, 0.0]).unwrap();
        let blur = document.add_node("Blur", [300.0, 0.0]).unwrap();
        let combine = document.add_node("Combine", [600.0, 0.0]).unwrap();
        let beside = document.add_node("Combine", [600.0, 300.0]).unwrap();
        // A mask output into a main input: the node renders as a mask, and
        // so does what it feeds through a main input.
        document.connect(erosion, "Deposits", blur, "In");
        assert_eq!(override_of(&document, blur), RenderIntent::Mask);
        document.connect(blur, "Out", combine, "In");
        assert_eq!(override_of(&document, combine), RenderIntent::Mask);
        // Other inputs pass nothing on.
        document.connect(blur, "Out", beside, "Input2");
        assert_eq!(override_of(&document, beside), RenderIntent::Default);
        // Fed from a heightfield, the node takes that node's override, even
        // Default; one undo takes both back.
        document.connect(erosion, "Out", blur, "In");
        assert_eq!(override_of(&document, blur), RenderIntent::Default);
        document.undo();
        assert_eq!(override_of(&document, blur), RenderIntent::Mask);
        document.set_member(erosion, "RenderIntentOverride", json!("Heightfield"), "Render intent", true);
        document.connect(erosion, "Out", combine, "In");
        assert_eq!(override_of(&document, combine), RenderIntent::Heightfield);
    }

    #[test]
    fn colour_nodes_keep_their_render_intent() {
        let mut document = Document::new();
        let erosion = document.add_node("ErosionPro", [0.0, 0.0]).unwrap();
        let hsl = document.add_node("HSL", [300.0, 0.0]).unwrap();
        document.connect(erosion, "Deposits", hsl, "In");
        assert_eq!(override_of(&document, hsl), RenderIntent::Default);
        // A node whose main output its last build left as colours.
        let blur = document.add_node("Blur", [300.0, 300.0]).unwrap();
        let mut built = HashMap::new();
        built.insert(blur, PortIntents::from([("Out".to_string(), RenderIntent::Color)]));
        document.set_built_intents(built);
        document.connect(erosion, "Deposits", blur, "In");
        assert_eq!(override_of(&document, blur), RenderIntent::Default);
    }

    #[test]
    fn built_intents_stand_over_declared_ones() {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [0.0, 0.0]).unwrap();
        let blur = document.add_node("Blur", [300.0, 0.0]).unwrap();
        assert_eq!(document.port_intent(mountain, "Out"), RenderIntent::Default);
        let mut built = HashMap::new();
        built.insert(mountain, PortIntents::from([("Out".to_string(), RenderIntent::Mask)]));
        document.set_built_intents(built);
        assert_eq!(document.port_intent(mountain, "Out"), RenderIntent::Mask);
        document.connect(mountain, "Out", blur, "In");
        assert_eq!(override_of(&document, blur), RenderIntent::Mask);
    }

    #[test]
    fn outputs_are_connected_when_they_feed_a_node() {
        let mut document = Document::new();
        let erosion = document.add_node("ErosionPro", [0.0, 0.0]).unwrap();
        let blur = document.add_node("Blur", [300.0, 0.0]).unwrap();
        document.connect(erosion, "Wear", blur, "In");
        let source = document.node(erosion).unwrap();
        assert!(document.port_connected(erosion, source.port("Wear").unwrap()));
        assert!(!document.port_connected(erosion, source.port("Out").unwrap()));
        let target = document.node(blur).unwrap();
        assert!(document.port_connected(blur, target.port("In").unwrap()));
        assert!(!document.port_connected(blur, target.port("Out").unwrap()));
    }
}
