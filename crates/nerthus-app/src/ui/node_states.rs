//! The property area's node-state commands (`PropertyArea`): Revert
//! Changes back to the node as it was when it was selected
//! (`CreateRevertPoint`, `RevertChanges_Click`), Save State and Load State
//! (`SaveState_OnChecked`, `SaveState_OnUnchecked`), Bind all properties
//! (`BindProperties_OnClick`) and "Connect for me!" (`ConnectMe_OnClick`).

use egui::Ui;
use nerthus_engine::{Catalogue, NodeData, NodeState, RenderIntent};
use serde_json::Value;

use crate::editor::Editor;
use crate::ui::commands::Command;

/// What the node-state menus and the Height notice asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StateAction {
    Revert,
    /// Save State, or Load State once a state is saved.
    ToggleSavedState,
    BindAll,
    ConnectHeight,
}

/// `PropertyArea.initialState` and `savedState`.
#[derive(Clone, Default)]
struct NodeStates {
    /// The document and node the revert point is of.
    shown: Option<(u64, i32)>,
    revert_point: Option<NodeData>,
    /// What Save State copied, until Load State.
    saved: Option<NodeData>,
}

fn states_id() -> egui::Id {
    egui::Id::new("node-states")
}

fn states(ui: &Ui) -> NodeStates {
    ui.data(|data| data.get_temp(states_id())).unwrap_or_default()
}

fn store(ui: &Ui, states: NodeStates) {
    ui.data_mut(|data| data.insert_temp(states_id(), states));
}

/// Whether Save State holds a state: the menus then read "Load State".
pub fn has_saved_state(ui: &Ui) -> bool {
    states(ui).saved.is_some()
}

/// `Graph_AfterNodeChanged` → `CreateRevertPoint`: a node coming into the
/// property area is remembered as it is.
pub fn note_shown_node(ui: &Ui, editor: &Editor, node: i32) {
    let mut states = states(ui);
    let shown = Some((editor.document_serial, node));
    if states.shown == shown {
        return;
    }
    states.shown = shown;
    states.revert_point = editor.document.node(node).cloned();
    store(ui, states);
}

/// `ChangeNode`: another node in the property area ends Preview Tiling
/// (`Comms.Send(36, 0)`). True when the shown node changed.
pub fn node_changed(ui: &Ui, editor: &Editor, node: Option<i32>) -> bool {
    let shown_id = egui::Id::new("property-area-node");
    let shown = (editor.document_serial, node);
    let previous: Option<(u64, Option<i32>)> = ui.data(|data| data.get_temp(shown_id));
    ui.data_mut(|data| data.insert_temp(shown_id, shown));
    previous.is_some_and(|previous| previous != shown)
}

/// `ChangeNode`'s `InfoHeightRef`: the node takes a terrain (`Terrain` or
/// `Height` port) that is not connected.
pub fn needs_height(node: &NodeData) -> bool {
    let terrain = node.port("Terrain").or_else(|| node.port("Height"));
    terrain.is_some_and(|port| port.record.is_none())
}

/// Runs a node-state command for the node the property area shows.
pub fn apply(ui: &Ui, editor: &mut Editor, node: i32, action: StateAction, commands: &mut Vec<Command>) {
    match action {
        StateAction::Revert => {
            let mut states = states(ui);
            let point = states.revert_point.take().filter(|point| point.id == node);
            if let Some(point) = &point
                && editor.document.restore_node(node, point, "Revert Changes")
            {
                editor.changed();
            }
            states.revert_point = point;
            store(ui, states);
        }
        StateAction::ToggleSavedState => toggle_saved_state(ui, editor, node),
        StateAction::BindAll => bind_all(editor, node, commands),
        StateAction::ConnectHeight => connect_height(editor, node, commands),
    }
}

/// Save State keeps a copy of the node; Load State puts it back (onto a
/// node of the same type; another type's state is dropped) and forgets it.
fn toggle_saved_state(ui: &Ui, editor: &mut Editor, node: i32) {
    let mut states = states(ui);
    match states.saved.take() {
        None => states.saved = editor.document.node(node).cloned(),
        Some(saved) => {
            let same_type = editor.document.node(node).is_some_and(|current| current.type_name == saved.type_name);
            if same_type && editor.document.restore_node(node, &saved, "Load State") {
                editor.changed();
            }
        }
    }
    store(ui, states);
}

/// `BindProperties_OnClick`: not for macros.
fn bind_all(editor: &mut Editor, node: i32, commands: &mut Vec<Command>) {
    let is_macro = editor.document.node(node).is_some_and(|data| data.type_name == "Macro");
    if is_macro {
        let caption = "Command not available".to_string();
        let text = "This command is not available for macro node".to_string();
        commands.push(Command::ShowMessage { caption, text });
        return;
    }
    if editor.document.bind_all_properties(node) > 0 {
        editor.changed();
    }
}

/// `ConnectMe_OnClick`: the nearest heightfield (`B.FindNearestHeightfield`)
/// connected to the node's `Height` port.
fn connect_height(editor: &mut Editor, node: i32, commands: &mut Vec<Command>) {
    let has_height = editor.document.node(node).is_some_and(|data| data.port("Height").is_some());
    if !has_height {
        return;
    }
    let Some(source) = nearest_heightfield(editor, node) else {
        return;
    };
    let would_cycle = editor.document.would_cycle(source, node);
    if would_cycle {
        let caption = "Could not connect".to_string();
        let text = "Could not connect automatically. Please connect the port yourself.".to_string();
        commands.push(Command::ShowMessage { caption, text });
        return;
    }
    editor.document.connect(source, "Out", node, "Height");
    editor.changed();
}

/// `B.FindNearestHeightfield`: the project's underlay when it is built (or
/// baked), else the nearest node up the chain of `In` ports
/// that is built, a heightfield and not excluded from
/// the underlay.
fn nearest_heightfield(editor: &Editor, node: i32) -> Option<i32> {
    let terrain = editor.document.terrain();
    let underlay = editor.document.underlay_node();
    if terrain.nodes.contains_key(&underlay) {
        let state = editor.status(underlay).state;
        if matches!(state, NodeState::Built | NodeState::Baked) {
            return Some(underlay);
        }
    }
    let mut visited = std::collections::HashSet::from([node]);
    let mut current = terrain.nodes.get(&node)?;
    while let Some(record) = current.ports.first().and_then(|port| port.record.as_ref()) {
        if !visited.insert(record.from) {
            break;
        }
        let parent = terrain.nodes.get(&record.from)?;
        let built = editor.status(parent.id).state == NodeState::Built;
        let ignored = parent.base.get("IgnoreUnderlay").and_then(Value::as_bool).unwrap_or(false);
        if built && !ignored && is_heightfield(editor, parent) {
            return Some(parent.id);
        }
        current = parent;
    }
    None
}

/// `Node.IsHeightfield` for a node the builds have left in the cache: its
/// main output renders as a heightfield, or has no intent of its own and
/// the node is not a mask or colour node.
fn is_heightfield(editor: &Editor, node: &NodeData) -> bool {
    let saved_override = node.base.get("RenderIntentOverride").and_then(Value::as_str);
    let intent_override = saved_override.and_then(RenderIntent::from_name).unwrap_or_default();
    let out_intent = editor.node_caches.get(&node.id).and_then(|cache| cache.out_intent()).unwrap_or_default();
    let explicit = matches!(intent_override, RenderIntent::Default | RenderIntent::Heightfield);
    if explicit && out_intent == RenderIntent::Heightfield {
        return true;
    }
    if intent_override != RenderIntent::Default || out_intent != RenderIntent::Default {
        return false;
    }
    let spec = Catalogue::get().node(&node.type_name);
    let classification = spec.and_then(|spec| spec.classification.as_deref()).unwrap_or_default();
    !matches!(classification, "Mask" | "Colorizer")
}
