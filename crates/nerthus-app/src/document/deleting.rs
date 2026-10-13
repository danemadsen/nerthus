//! Deleting as `GraphControl.ExecuteDelete` does: one element on its own
//! (`ExecuteSingleDelete`, where a node's primary input is joined to what
//! its primary output fed), or many at once (`ExecuteMassDelete`).

use super::links::{primary_input, primary_output, PortalState};
use super::Document;

/// What a delete is asked to take away.
#[derive(Clone, Debug, Default)]
pub struct DeleteRequest {
    pub nodes: Vec<i32>,
    /// Connections by the input they feed.
    pub connections: Vec<(i32, String)>,
    pub groups: Vec<i32>,
    pub notes: Vec<i32>,
    /// Shift held: a group's nodes go with it.
    pub with_group_children: bool,
}

impl DeleteRequest {
    pub fn count(&self) -> usize {
        self.nodes.len() + self.connections.len() + self.groups.len() + self.notes.len()
    }
}

impl Document {
    /// `ExecuteDelete`: a single element as `ExecuteSingleDelete` deletes
    /// it, several as `ExecuteMassDelete` does ("Multiple nodes deleted.").
    pub fn delete_elements(&mut self, request: &DeleteRequest) {
        match request.count() {
            0 => {}
            1 => self.delete_single(request),
            _ => self.delete_many(request),
        }
    }

    fn delete_single(&mut self, request: &DeleteRequest) {
        if let Some(node) = request.nodes.first() {
            self.delete_node(*node);
        } else if let Some((to, to_port)) = request.connections.first() {
            self.delete_connection(*to, to_port);
        } else if let Some(group) = request.groups.first() {
            self.delete_group(*group, request.with_group_children);
        } else if let Some(note) = request.notes.first() {
            self.remove_note(*note);
            self.forget_group_children(&[*note]);
        }
    }

    fn delete_many(&mut self, request: &DeleteRequest) {
        self.checkpoint("Multiple nodes deleted.");
        for node in &request.nodes {
            self.take_out_node(*node);
        }
        for (to, to_port) in &request.connections {
            self.unlink(*to, to_port);
        }
        for group in &request.groups {
            self.delete_group_contents(*group, request.with_group_children);
        }
        for note in &request.notes {
            self.remove_note_quietly(*note);
        }
        self.forget_group_children(&request.notes);
    }

    /// `DeleteNodeElement`: "Node deleted: Name.", unless its group is
    /// locked. Returns whether it went.
    pub fn delete_node(&mut self, id: i32) -> bool {
        let Some(name) = self.node(id).map(|node| node.name.clone()) else {
            return false;
        };
        if self.node_locked_in_group(id) {
            return false;
        }
        self.checkpoint(format!("Node deleted: {name}."));
        self.delete_node_healing(id)
    }

    /// `DeleteNodeElement` without its undo step: when the node's primary
    /// input is connected, every input its primary output fed is fed from
    /// that source instead; then the node goes with its connections.
    pub(crate) fn delete_node_healing(&mut self, id: i32) -> bool {
        if self.node_locked_in_group(id) {
            return false;
        }
        let Some(node) = self.node(id) else {
            return false;
        };
        let source = primary_input(node).and_then(|port| port.record.clone());
        let source = source.filter(|record| self.node(record.from).is_some());
        let primary_out = primary_output(node).map(|port| port.name.clone());
        if let (Some(source), Some(primary_out)) = (source, primary_out) {
            let fed = self.outgoing(id);
            for (target, input, output) in fed {
                if output == primary_out {
                    self.link(source.from, &source.from_port, target, &input);
                }
            }
        }
        self.take_out_node(id);
        true
    }

    /// `DeleteConnection` (user initiated): "Connection deleted.".
    pub fn delete_connection(&mut self, to: i32, to_port: &str) {
        let connected = self.node(to).and_then(|node| node.port(to_port)).is_some_and(|port| port.record.is_some());
        if !connected {
            return;
        }
        self.checkpoint("Connection deleted.");
        self.unlink(to, to_port);
    }

    /// The part of `DeleteGroupElement` after its undo step.
    fn delete_group_contents(&mut self, id: i32, with_children: bool) {
        let Some(group) = self.group(id) else {
            return;
        };
        if group.locked {
            return;
        }
        if with_children {
            let nodes: Vec<i32> = group.children.iter().copied().filter(|child| self.node(*child).is_some()).collect();
            for node in nodes {
                self.delete_node_healing(node);
            }
        }
        if let Some(serde_json::Value::Object(groups)) = self.project.terrain_mut().extra.get_mut("Groups") {
            groups.shift_remove(&id.to_string());
        }
        self.dirty = true;
    }

    fn remove_note_quietly(&mut self, id: i32) {
        if let Some(serde_json::Value::Object(notes)) = self.project.terrain_mut().extra.get_mut("Notes") {
            notes.shift_remove(&id.to_string());
        }
        self.dirty = true;
    }

    /// Whether the input consumes a portal.
    pub fn consumes_portal(&self, node: i32, port: &str) -> bool {
        let found = self.node(node).and_then(|data| data.port(port));
        found.is_some_and(|data| PortalState::of(data) == PortalState::Consuming)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain() -> (Document, i32, i32, i32) {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [0.0, 0.0]).unwrap();
        let erosion = document.add_node("ErosionPro", [300.0, 0.0]).unwrap();
        let blur = document.add_node("Blur", [600.0, 0.0]).unwrap();
        document.connect(mountain, "Out", erosion, "In");
        document.connect(erosion, "Out", blur, "In");
        (document, mountain, erosion, blur)
    }

    #[test]
    fn a_single_delete_joins_the_chain_again() {
        let (mut document, mountain, erosion, blur) = chain();
        let request = DeleteRequest { nodes: vec![erosion], ..Default::default() };
        document.delete_elements(&request);
        assert!(document.node(erosion).is_none());
        let record = document.node(blur).unwrap().port("In").unwrap().record.clone().unwrap();
        assert_eq!((record.from, record.from_port.as_str()), (mountain, "Out"));
        assert_eq!(document.undo_labels()[0], "Node deleted: ErosionPro.");
    }

    #[test]
    fn a_mass_delete_only_cuts() {
        let (mut document, mountain, erosion, blur) = chain();
        let request = DeleteRequest { nodes: vec![erosion, mountain], ..Default::default() };
        document.delete_elements(&request);
        assert!(document.node(blur).unwrap().port("In").unwrap().record.is_none());
        assert_eq!(document.undo_labels()[0], "Multiple nodes deleted.");
    }

    #[test]
    fn nodes_in_a_locked_group_stay() {
        let (mut document, mountain, _, _) = chain();
        let group = document.add_group(&[mountain], [-20.0, -40.0], [270.0, 270.0], 0);
        document.set_group_locked(group, true);
        assert!(!document.delete_node(mountain));
        assert!(document.node(mountain).is_some());
    }
}
