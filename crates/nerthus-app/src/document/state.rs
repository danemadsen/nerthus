//! `TerrainState`, what the editor was showing as the project was saved:
//! the selected node, the open graph tab and the node the preview is
//! locked to (which also carries `Node.IsLocked`). The project keeps them
//! without counting as an edit, and the editor opens on them again.
//!
//! Also the repairs damaged projects get: File › Open gives nodes on graph
//! tabs that no longer exist tabs of their own or deletes them
//! (`Shell.RecoverOrphans`) and fixes export settings that name another
//! node (`RecoverSaveDefinitions`); saving forgets bookmarks, a selection,
//! a preview lock and an underlay whose nodes are gone
//! (`SanitizeStateObject`).

use serde_json::{json, Value};

use super::{node_tab, Document};

/// `TerrainState`'s members in declaration order, which is how projects
/// save them (`BakeResolution` is a computed member written first).
const STATE_MEMBERS: [&str; 10] = [
    "BakeResolution",
    "PreviewResolution",
    "HDResolution",
    "SelectedNode",
    "SelectedTab",
    "NodeBookmarks",
    "Viewport",
    "LockedNode",
    "UnderlayNode",
    "GroupVariablesByNodes",
];

/// `TerrainState.SelectedNode` when nothing is selected.
const NO_NODE: i64 = -1;

/// What the orphan question asks (`RecoverOrphans`).
pub const ORPHANS_CAPTION: &str = "Review Orphan Nodes?";
pub const ORPHANS_TEXT: &str = "Nerthus found orphan nodes in this file. Should it place them in new tabs for you to review? If you say NO, then the orphan nodes will be deleted.";

impl Document {
    /// A `TerrainState` member as saved.
    fn state_member(&self, name: &str) -> Option<&Value> {
        self.project.assets[0].extra.get("State")?.get(name)
    }

    /// Writes a `TerrainState` member as projects save it: in the class's
    /// order, and left out at its type's default (`IgnoreAndPopulate`:
    /// zero, false). Not an edit: the project does not become dirty.
    fn set_state_member(&mut self, name: &str, value: Value) {
        let extra = &mut self.project.assets[0].extra;
        let state = extra.entry("State").or_insert_with(|| json!({}));
        if !state.is_object() {
            *state = json!({});
        }
        let Value::Object(members) = state else {
            return;
        };
        let omitted = match &value {
            Value::Bool(flag) => !flag,
            Value::Number(number) => number.as_f64() == Some(0.0),
            Value::Null => true,
            _ => false,
        };
        if omitted {
            members.shift_remove(name);
            return;
        }
        members.insert(name.to_string(), value);
        let mut ordered = serde_json::Map::new();
        if let Some(id) = members.remove("$id") {
            ordered.insert("$id".to_string(), id);
        }
        for member in STATE_MEMBERS {
            if let Some(found) = members.remove(member) {
                ordered.insert(member.to_string(), found);
            }
        }
        let rest = std::mem::take(members);
        ordered.extend(rest);
        *members = ordered;
    }

    fn state_number(&self, name: &str) -> Option<i64> {
        self.state_member(name).and_then(Value::as_i64)
    }

    /// `TerrainState.SelectedNode`: the node selected as the project was
    /// saved, when there was one.
    pub fn saved_selected_node(&self) -> Option<i32> {
        let saved = self.state_number("SelectedNode").unwrap_or(NO_NODE);
        (saved != NO_NODE).then_some(saved as i32)
    }

    /// `TerrainState.SelectedTab`.
    pub fn saved_tab(&self) -> usize {
        self.state_number("SelectedTab").unwrap_or(0).max(0) as usize
    }

    /// The graph tab the project opens on (`GraphTabSelectionChanged`
    /// while loading): the saved one when a tab has that index, else the
    /// first tab.
    pub fn opening_tab(&self) -> usize {
        let tabs = self.graph_tabs();
        let saved = self.saved_tab();
        if tabs.iter().any(|tab| tab.index == saved) {
            return saved;
        }
        tabs.first().map_or(0, |tab| tab.index)
    }

    /// Notes the selected node (or that none is), so that a save keeps
    /// it. Not an edit.
    pub fn note_selected_node(&mut self, selected: Option<i32>) {
        let selected = selected.map_or(NO_NODE, i64::from);
        if self.state_number("SelectedNode") != Some(selected) {
            self.set_state_member("SelectedNode", json!(selected));
        }
    }

    /// Notes the graph tab shown (`SelectGraphTab`), so that a save keeps
    /// it. Not an edit.
    pub fn note_selected_tab(&mut self, tab: usize) {
        if self.saved_tab() != tab {
            self.set_state_member("SelectedTab", json!(tab));
        }
    }

    /// The node the preview is locked to (`Node.IsLocked`): the first node
    /// so marked.
    pub fn locked_node(&self) -> Option<i32> {
        let nodes = self.terrain().nodes.values();
        nodes.filter(|node| node_is_locked(node)).map(|node| node.id).next()
    }

    /// `GraphControl.Reconstruct`'s lock: the first node marked locked
    /// keeps the lock (`LockPreview` without an undo step), any other
    /// loses it, and `TerrainState.LockedNode` follows. Not an edit.
    /// Returns the locked node.
    pub fn settle_preview_lock(&mut self) -> Option<i32> {
        let locked = self.locked_node();
        let others: Vec<i32> = self.terrain().nodes.values().filter(|node| node_is_locked(node) && Some(node.id) != locked).map(|node| node.id).collect();
        for id in others {
            set_node_locked(self, id, false);
        }
        self.set_state_member("LockedNode", json!(locked.unwrap_or(0)));
        locked
    }

    /// `GraphControl.LockPreview(node)`: a locked node is unlocked ("Node
    /// X preview lock removed."); any other node takes the lock from
    /// whichever had it ("Preview locked to Node X."). Returns whether the
    /// node is now locked, none when there is no such node.
    pub fn toggle_preview_lock(&mut self, id: i32) -> Option<bool> {
        let node = self.node(id)?;
        let name = node.name.clone();
        let was_locked = node_is_locked(node);
        let label = if was_locked { format!("Node {name} preview lock removed.") } else { format!("Preview locked to Node {name}.") };
        self.checkpoint(label);
        let others: Vec<i32> = self.terrain().nodes.values().filter(|node| node.id != id && node_is_locked(node)).map(|node| node.id).collect();
        for other in others {
            set_node_locked(self, other, false);
        }
        set_node_locked(self, id, !was_locked);
        let locked = if was_locked { 0 } else { id };
        self.set_state_member("LockedNode", json!(locked));
        Some(!was_locked)
    }

    /// The lock goes with a node replaced by another (`ReplaceNode`).
    pub(crate) fn move_preview_lock(&mut self, from: i32, to: i32) {
        let locked = self.node(from).is_some_and(node_is_locked);
        if !locked {
            return;
        }
        set_node_locked(self, from, false);
        set_node_locked(self, to, true);
        self.set_state_member("LockedNode", json!(to));
    }

    /// `TerrainState.LockedNode` once a node is gone.
    pub(crate) fn forget_locked_node(&mut self, id: i32) {
        if self.state_number("LockedNode") == Some(i64::from(id)) {
            self.set_state_member("LockedNode", json!(0));
        }
    }

    /// `Shell.SanitizeStateObject`, as a project is saved: bookmarks of
    /// nodes that are gone are dropped, and a selected, locked or underlay
    /// node that is gone is forgotten. `BakeResolution` is written from the
    /// build settings, as the computed member is.
    pub fn sanitize_state(&mut self) {
        let bookmarks = self.bookmarks();
        let kept: Vec<i32> = bookmarks.iter().copied().filter(|id| self.node(*id).is_some()).collect();
        if kept.len() != bookmarks.len()
            && let Some(saved) = self.bookmarks_mut()
        {
            *saved = kept.into_iter().map(|id| json!(id)).collect();
        }
        let selected = self.state_number("SelectedNode").unwrap_or(NO_NODE);
        if !self.node_exists(selected) {
            self.set_state_member("SelectedNode", json!(NO_NODE));
        }
        for member in ["LockedNode", "UnderlayNode"] {
            let saved = self.state_number(member).unwrap_or(0);
            if !self.node_exists(saved) {
                self.set_state_member(member, json!(0));
            }
        }
        let bake_resolution = self.build_definition().bake_resolution;
        self.set_state_member("BakeResolution", json!(bake_resolution));
    }

    fn node_exists(&self, id: i64) -> bool {
        i32::try_from(id).is_ok_and(|id| self.node(id).is_some())
    }

    /// `Shell.RecoverSaveDefinitions` (File › Open): of the exporting
    /// nodes, an Export node keeps no export settings (it exports by its
    /// own), and settings that name another node (copied with a node) are
    /// made this node's. Not an edit.
    pub fn recover_save_definitions(&mut self) {
        let exporting: Vec<i32> = self.terrain().nodes.values().filter(|node| nerthus_export::build::exports(node)).map(|node| node.id).collect();
        for id in exporting {
            let Some(node) = self.project.terrain_mut().nodes.get_mut(&id) else {
                continue;
            };
            if node.type_name == "Export" {
                node.base.shift_remove("SaveDefinition");
                continue;
            }
            let Some(Value::Object(definition)) = node.base.get_mut("SaveDefinition") else {
                continue;
            };
            let named = definition.get("Node").and_then(Value::as_i64).unwrap_or(0);
            if named == i64::from(id) {
                continue;
            }
            // `Node` is `SaveDefinition`'s first member.
            let first = definition.keys().take_while(|name| name.starts_with('$')).count();
            definition.shift_insert(first, "Node".to_string(), json!(id));
        }
    }

    /// Nodes on graph tabs the project does not have (`RecoverOrphans`).
    pub fn orphan_nodes(&self) -> Vec<i32> {
        let tabs: Vec<usize> = self.graph_tabs().iter().map(|tab| tab.index).collect();
        let nodes = self.terrain().nodes.values();
        nodes.filter(|node| !tabs.contains(&node_tab(node))).map(|node| node.id).collect()
    }

    /// The orphan question's Yes: a "Recovered Graph N" tab for each graph
    /// index the orphans are on, in the order they first come. Not an
    /// edit. Returns the new tabs' indices.
    pub fn recover_orphans(&mut self) -> Vec<usize> {
        let mut indices: Vec<usize> = Vec::new();
        for id in self.orphan_nodes() {
            let index = self.node(id).map(node_tab).unwrap_or(0);
            if !indices.contains(&index) {
                indices.push(index);
            }
        }
        for index in &indices {
            let order = self.graph_tabs().len();
            // `new GraphTab()`'s members, `Order` left out at 0.
            let mut tab = serde_json::Map::new();
            tab.insert("Name".to_string(), json!(format!("Recovered Graph {index}")));
            tab.insert("Index".to_string(), json!(index));
            if order != 0 {
                tab.insert("Order".to_string(), json!(order));
            }
            tab.insert("Color".to_string(), json!("Brass"));
            tab.insert("ZoomFactor".to_string(), json!(1.0));
            tab.insert("ViewportLocation".to_string(), json!({"X": 26000.0, "Y": 26000.0}));
            let extra = &mut self.project.terrain_mut().extra;
            let tabs = extra.entry("GraphTabs".to_string()).or_insert_with(|| json!([]));
            if let Value::Array(tabs) = tabs {
                tabs.push(Value::Object(tab));
            }
        }
        indices
    }

    /// The orphan question's No: the orphans deleted, with their
    /// connections. Not an edit.
    pub fn delete_orphans(&mut self) {
        let dirty = self.dirty;
        for id in self.orphan_nodes() {
            self.take_out_node(id);
        }
        self.dirty = dirty;
    }
}

/// `Node.IsLocked`.
fn node_is_locked(node: &nerthus_engine::NodeData) -> bool {
    node.base.get("IsLocked").and_then(Value::as_bool).unwrap_or(false)
}

/// Sets `Node.IsLocked` (left out when false, as projects save it).
fn set_node_locked(document: &mut Document, id: i32, locked: bool) {
    let Some(node) = document.project.terrain_mut().nodes.get_mut(&id) else {
        return;
    };
    if locked {
        node.set_member("IsLocked", json!(true));
    } else {
        node.base.shift_remove("IsLocked");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain() -> (Document, i32, i32) {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [0.0, 0.0]).unwrap();
        let erosion = document.add_node("ErosionPro", [300.0, 0.0]).unwrap();
        document.connect(mountain, "Out", erosion, "In");
        (document, mountain, erosion)
    }

    fn state_keys(document: &Document) -> Vec<String> {
        let state = document.project.assets[0].extra.get("State").and_then(Value::as_object).unwrap();
        state.keys().cloned().collect()
    }

    #[test]
    fn the_view_state_saves_in_member_order() {
        let (mut document, mountain, _) = chain();
        let dirty = document.dirty;
        document.note_selected_node(Some(mountain));
        document.note_selected_tab(2);
        assert_eq!(document.saved_selected_node(), Some(mountain));
        assert_eq!(document.saved_tab(), 2);
        assert_eq!(state_keys(&document), ["BakeResolution", "PreviewResolution", "SelectedNode", "SelectedTab", "NodeBookmarks", "Viewport"]);
        // Nothing selected is -1; the first tab is left out.
        document.note_selected_node(None);
        document.note_selected_tab(0);
        assert_eq!(document.state_member("SelectedNode"), Some(&json!(-1)));
        assert_eq!(document.saved_selected_node(), None);
        assert!(document.state_member("SelectedTab").is_none());
        assert_eq!(document.dirty, dirty);
    }

    #[test]
    fn the_opening_tab_falls_back_to_the_first() {
        let (mut document, _, _) = chain();
        let second = document.add_graph_tab();
        document.note_selected_tab(second);
        assert_eq!(document.opening_tab(), second);
        document.note_selected_tab(7);
        assert_eq!(document.opening_tab(), 0);
    }

    #[test]
    fn the_preview_lock_moves_between_nodes() {
        let (mut document, mountain, erosion) = chain();
        assert_eq!(document.toggle_preview_lock(mountain), Some(true));
        assert_eq!(document.undo_labels()[0], "Preview locked to Node Mountain.");
        assert_eq!(document.locked_node(), Some(mountain));
        assert_eq!(document.state_member("LockedNode"), Some(&json!(mountain)));
        assert_eq!(document.node(mountain).unwrap().base.get("IsLocked"), Some(&json!(true)));

        // Another node takes it.
        assert_eq!(document.toggle_preview_lock(erosion), Some(true));
        assert_eq!(document.locked_node(), Some(erosion));
        assert!(document.node(mountain).unwrap().base.get("IsLocked").is_none());

        assert_eq!(document.toggle_preview_lock(erosion), Some(false));
        assert_eq!(document.undo_labels()[0], "Node ErosionPro preview lock removed.");
        assert_eq!(document.locked_node(), None);
        assert!(document.state_member("LockedNode").is_none());

        // Undone, the lock is back on the node.
        document.undo();
        assert_eq!(document.locked_node(), Some(erosion));
    }

    #[test]
    fn opening_keeps_the_first_lock_only() {
        let (mut document, mountain, erosion) = chain();
        for id in [mountain, erosion] {
            document.project.terrain_mut().nodes.get_mut(&id).unwrap().set_member("IsLocked", json!(true));
        }
        assert_eq!(document.settle_preview_lock(), Some(mountain));
        assert!(document.node(erosion).unwrap().base.get("IsLocked").is_none());
        assert_eq!(document.state_member("LockedNode"), Some(&json!(mountain)));
    }

    #[test]
    fn saving_forgets_nodes_that_are_gone() {
        let (mut document, mountain, erosion) = chain();
        document.add_bookmark(mountain);
        document.add_bookmark(erosion);
        document.note_selected_node(Some(erosion));
        document.set_state_member("LockedNode", json!(erosion));
        document.set_state_member("UnderlayNode", json!(erosion));
        // Taken out behind the editor's back, as a damaged file would have it.
        document.project.terrain_mut().nodes.shift_remove(&erosion);
        document.sanitize_state();
        assert_eq!(document.bookmarks(), vec![mountain]);
        assert_eq!(document.saved_selected_node(), None);
        assert!(document.state_member("LockedNode").is_none());
        assert!(document.state_member("UnderlayNode").is_none());
    }

    #[test]
    fn export_settings_name_their_own_node() {
        let (mut document, mountain, erosion) = chain();
        document.toggle_export(&[mountain, erosion], nerthus_export::FileFormat::Exr);
        let copied = document.node(mountain).unwrap().base.get("SaveDefinition").cloned().unwrap();
        document.project.terrain_mut().nodes.get_mut(&erosion).unwrap().base.insert("SaveDefinition".to_string(), copied);
        let export = document.add_node("Export", [600.0, 0.0]).unwrap();
        document.project.terrain_mut().nodes.get_mut(&export).unwrap().base.insert("SaveDefinition".to_string(), json!({"Node": export, "Filename": "Export", "IsEnabled": true}));

        document.recover_save_definitions();
        assert_eq!(document.export_settings(erosion).unwrap().node, erosion);
        let saved = document.node(erosion).unwrap().base.get("SaveDefinition").unwrap();
        assert_eq!(saved.as_object().unwrap().keys().next().map(String::as_str), Some("Node"));
        assert!(document.node(export).unwrap().base.get("SaveDefinition").is_none());
    }

    /// A chain whose second node is on graph tab 4, which the project
    /// does not have.
    fn orphaned_chain() -> (Document, i32, i32) {
        let (mut document, mountain, erosion) = chain();
        document.project.terrain_mut().nodes.get_mut(&erosion).unwrap().set_member("GraphIndex", json!(4));
        (document, mountain, erosion)
    }

    #[test]
    fn orphans_get_tabs_of_their_own() {
        let (mut document, _, erosion) = orphaned_chain();
        assert_eq!(document.orphan_nodes(), vec![erosion]);
        assert_eq!(document.recover_orphans(), vec![4]);
        let tab = document.graph_tabs().into_iter().find(|tab| tab.index == 4).unwrap();
        assert_eq!(tab.name, "Recovered Graph 4");
        assert!(document.orphan_nodes().is_empty());
        let saved_tabs = document.terrain().extra.get("GraphTabs").unwrap().as_array().unwrap().clone();
        let keys: Vec<&String> = saved_tabs[1].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["Name", "Index", "Order", "Color", "ZoomFactor", "ViewportLocation"]);
    }

    #[test]
    fn orphans_can_go_instead() {
        let (mut document, mountain, erosion) = orphaned_chain();
        let dirty = document.dirty;
        document.delete_orphans();
        assert!(document.node(erosion).is_none());
        assert!(document.node(mountain).is_some());
        assert_eq!(document.dirty, dirty);
    }
}
