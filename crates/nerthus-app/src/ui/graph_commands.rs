//! What the graph's menus, popups and keys ask for (`GraphControl`,
//! `GraphArea` and the graph parts of `Shell`), run after the interface
//! has drawn, like every [`Command`](crate::ui::commands::Command).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use egui::Pos2;
use nerthus_engine::Catalogue;
use serde_json::Value;

use crate::document::{grid_inputs, grid_outputs, node_tab, CombineKind, DeleteRequest, Layout, TabTarget, GROUP_MIN_SIZE};
use crate::editor::Editor;
use crate::ui::commands::Command;
use crate::ui::graph_canvas::{node_extent, LazyTarget, ZoomRequest};
use crate::ui::graph_popups::GraphPopup;
use crate::ui::graph_search::{SearchMode, SearchPopup};
use crate::ui::prompt::Prompt;
use crate::ui::UiState;

/// What a node added from the search connects to (`CreateConnectedNode`).
#[derive(Clone, Debug, PartialEq)]
pub enum NodeSource {
    /// Nothing (`CreateSearchedNode`).
    None,
    /// The selected node: its first output feeds the new node, which goes
    /// to its right.
    Selected(i32),
    /// A connection dragged out of a port (`SearchType.DraggedConnection`).
    Port { node: i32, port: String, output: bool },
}

/// A search result taken: a node type, and the preset to apply to it.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchPick {
    pub type_name: String,
    pub preset: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GraphCommand {
    /// Edit › Cut, Copy and Paste (Ctrl X, C, V).
    Cut,
    Copy,
    Paste,
    /// Edit › Paste Properties: onto the node the property area shows.
    PastePropertiesToActive,
    /// Tab: the node search at the pointer, when it is over the graph.
    NodeSearch,
    /// Ctrl T: finding a node by name.
    FindNode,
    /// Comma: the selected node's neighbours.
    QuickNavigation,
    /// F1: the Lazy Menu at the pointer.
    LazyMenu,
    /// A Lazy Menu entry (`miLazyBoy_Click`'s tag).
    LazyAction(&'static str),
    /// A node picked to go to: its tab shown, it selected and framed.
    Navigate(i32),
    /// Results taken in the node search.
    CreateNodes { picks: Vec<SearchPick>, at: [f32; 2], source: NodeSource },
    /// Ctrl Q, or Replace in the node's menu: the search, to replace the
    /// selected node.
    ReplaceSearch,
    /// A node replaced by one of another type (and its preset).
    ReplaceWith { node: i32, pick: SearchPick },
    /// Ctrl G: the selection grouped.
    GroupSelection,
    /// A group chosen in the Data View (`Group.Select`), and gone to on a
    /// double click (`GoToLocation`).
    SelectGroup { id: i32, go_to: bool },
    /// Node › Group / Ungroup.
    GroupUngroup,
    ToggleGroupCollapsed(i32),
    SetGroupLocked(i32, bool),
    /// F: the preview locked to the selected node, each selected group
    /// locked or unlocked (`LockPreview`, `LockGroup`).
    LockSelection,
    SetGroupColor(i32, &'static str),
    SetGroupIcon(i32, &'static str),
    RenameGroup(i32),
    /// A node's new name from the inline editor (F2, the node menu's
    /// Rename), then the questions about its export file and its variable.
    RenameNode { node: i32, name: String },
    /// "Update Save Definition"'s Yes: the export file takes the name; then
    /// the variable question.
    RenameExportFile { node: i32, name: String },
    /// Its No: on to the variable question.
    AskRenameVariable(i32),
    /// "Rename Variable"'s Yes.
    RenameBoundVariable { key: String, name: String },
    /// Group to Tab, asked first, then done.
    GroupToTab(i32),
    GroupToTabConfirmed(i32),
    /// A tab's Tab to group, asked first, then done.
    TabToGroup { from: usize, target: TabTarget },
    TabToGroupConfirmed { from: usize, target: TabTarget },
    /// The connection menu's Insert node and Insert chokepoint.
    InsertNodeSearch { to: i32, to_port: String },
    InsertChokepoint { to: i32, to_port: String },
    InsertNodeInConnection { to: i32, to_port: String, type_name: String, corner: [f32; 2] },
    /// The node menu's Insert chokepoint: the outputs to choose from, then
    /// the chokepoint after one.
    ChokepointMenu(i32),
    InsertChokepointAt { node: i32, port: String },
    /// Ctrl P and the connection menu's Convert to Portal.
    ConvertConnectionsToPortals,
    /// P, and the node menu's Manage Portals.
    ManagePortals,
    /// Node › Convert port to portals.
    TogglePortPortal { node: i32, port: String },
    /// Manage Portals: Convert to Portal (with Shift through a chokepoint),
    /// Remove Portal, an input's Disconnect and Connect to Portal (the
    /// portal search).
    PublishPort { node: i32, port: String, through_chokepoint: bool },
    RemovePortal { node: i32, port: String },
    DisconnectPortalInput { node: i32, port: String },
    PortalSearch { node: i32, port: String },
    ConnectToPortal { from: i32, from_port: String, to: i32, to_port: String },
    /// A connection made from a popup or by dropping on a port.
    ConnectPorts { from: i32, from_port: String, to: i32, to_port: String },
    /// The ports menu's Add as Mask and Add new port, connected at once.
    AddMaskAndConnect { from: i32, from_port: String, to: i32 },
    AddPortAndConnect { from: i32, from_port: String, to: i32 },
    /// Combine nodes (F8, Ctrl F8, Ctrl Shift F8 and the toolbar's).
    Combine(CombineKind),
    /// A connection dragged out of an output let go on another node's
    /// output (`CombineOutPorts`).
    CombineOutputs { dragged: i32, dragged_port: String, other: i32, other_port: String },
    /// Node › Mutate (`MutateNode`).
    Mutate,
    /// The graph toolbar's Mutate (`BtnMutate_OnClick`) and its Mutate all
    /// nodes.
    MutateButton,
    MutateAll,
    /// F4.
    AutoLayout,
    /// T.
    CycleRenderIntent,
    /// Node › Move to Tab.
    MoveToTab(TabTarget),
    /// A toolbox entry clicked (`Toolbox.CreateNodeOnCanvas`).
    ToolboxCreate(String),
    /// A toolbox entry dropped on the graph (`canvas_Drop`), maybe onto a
    /// node.
    ToolboxDrop { type_name: String, at: [f32; 2], onto: Option<i32> },
    /// Files dropped on the graph (`DropFiles`).
    DropFiles { files: Vec<PathBuf>, at: [f32; 2] },
    /// A note's type flipped (double click), its colour, its editor, it
    /// deleted.
    ToggleNoteType(i32),
    SetNoteColor(i32, &'static str),
    EditNote(i32),
    DeleteNote(i32),
    /// Graph Surface › Snap to Items.
    ToggleSnapToItems,
}

/// A standard node's size, for nodes the map does not have.
const STANDARD_EXTENT: [f32; 2] = [230.0, 80.0];

/// The node sizes placements and the layout read (`ActualWidth`,
/// `ActualHeight`), as of now.
fn extents(editor: &Editor) -> HashMap<i32, [f32; 2]> {
    let default_size = editor.user.options.default_node_size().to_string();
    let nodes = editor.document.terrain().nodes.values();
    nodes.map(|node| (node.id, node_extent(node, &default_size))).collect()
}

fn extent_of(sizes: &HashMap<i32, [f32; 2]>, id: i32) -> [f32; 2] {
    sizes.get(&id).copied().unwrap_or(STANDARD_EXTENT)
}

pub fn run(command: GraphCommand, editor: &mut Editor, ui_state: &mut UiState) {
    match command {
        GraphCommand::Cut => {
            let selected = editor.selection.clone();
            let clipboard = editor.document.cut_nodes(&selected);
            if !clipboard.is_empty() {
                editor.node_clipboard = Some(clipboard);
                editor.select(None);
                editor.changed();
            }
        }
        GraphCommand::Copy => {
            let selected = editor.selection.clone();
            let clipboard = editor.document.copy_nodes(&selected);
            if !clipboard.is_empty() {
                editor.node_clipboard = Some(clipboard);
            }
        }
        GraphCommand::Paste => {
            let Some(clipboard) = editor.node_clipboard.take() else {
                return;
            };
            let tab = ui_state.graph.tab;
            let pasted = editor.document.paste_nodes(&clipboard, tab, Some("Paste Selection"));
            // `CopyNodes`: with Always Show Mask Port, maskable copies take
            // a mask port.
            if editor.user.options.always_show_mask_port() {
                for id in &pasted {
                    let maskable = editor.document.node(*id).and_then(|node| Catalogue::get().node(&node.type_name)).is_some_and(|spec| spec.is_maskable());
                    if maskable {
                        editor.document.add_mask_port(*id);
                    }
                }
            }
            ui_state.graph.clear_element_selection();
            editor.set_selection(pasted);
            editor.changed();
        }
        GraphCommand::PastePropertiesToActive => {
            let Some(active) = editor.active_node() else {
                return;
            };
            let Some(copied) = editor.copied_node.clone() else {
                return;
            };
            let same_document = copied.document_serial == editor.document_serial;
            let live = editor.document.node(copied.id).filter(|node| same_document && node.type_name == copied.type_name);
            let values = live.map(|node| node.values.clone()).unwrap_or(copied.values);
            editor.document.paste_properties(&[active], &copied.type_name, &values);
            editor.changed();
        }
        GraphCommand::NodeSearch => {
            let Some(pointer) = ui_state.graph.pointer else {
                return;
            };
            open_search(ui_state, SearchMode::Nodes { source: NodeSource::None }, pointer);
        }
        GraphCommand::FindNode => {
            let pointer = ui_state.graph.pointer.unwrap_or(ui_state.graph.rect.center());
            open_search(ui_state, SearchMode::Navigate, pointer);
        }
        GraphCommand::QuickNavigation => {
            let Some(node) = editor.active_node() else {
                return;
            };
            let Some(rect) = node_screen_rect(editor, ui_state, node) else {
                return;
            };
            ui_state.graph.popup = Some(GraphPopup::Navigation { node, at: rect.right_top() });
        }
        GraphCommand::LazyMenu => {
            let pointer = ui_state.graph.pointer.unwrap_or(ui_state.graph.rect.center());
            ui_state.graph.popup = Some(GraphPopup::Lazy { at: pointer });
        }
        GraphCommand::LazyAction(tag) => lazy_action(tag, editor, ui_state),
        GraphCommand::Navigate(id) => {
            let Some(tab) = editor.document.node(id).map(node_tab) else {
                return;
            };
            ui_state.graph.tab = tab;
            ui_state.graph.clear_element_selection();
            editor.select(Some(id));
            ui_state.graph.zoom_request = Some(ZoomRequest::Selection);
        }
        GraphCommand::CreateNodes { picks, at, source } => create_nodes(editor, ui_state, &picks, at, source),
        GraphCommand::ReplaceSearch => {
            let Some(node) = editor.active_node() else {
                return;
            };
            let Some(rect) = node_screen_rect(editor, ui_state, node) else {
                return;
            };
            open_search(ui_state, SearchMode::Replace { node }, rect.min);
        }
        GraphCommand::ReplaceWith { node, pick } => replace_with(editor, ui_state, node, &pick),
        GraphCommand::GroupSelection => group_selection(editor, ui_state),
        GraphCommand::SelectGroup { id, go_to } => {
            let Some(group) = editor.document.group(id) else {
                return;
            };
            ui_state.graph.tab = group.graph_index;
            ui_state.graph.clear_element_selection();
            editor.select_group();
            ui_state.graph.selected_groups = vec![id];
            if go_to {
                ui_state.graph.zoom_request = Some(ZoomRequest::CenterGroup(id));
            }
        }
        GraphCommand::GroupUngroup => {
            // `MiGroupUnGroup_OnClick`: nodes and notes (or nothing) are
            // grouped; a group alone is taken apart.
            let groups = &ui_state.graph.selected_groups;
            let only_group = groups.len() == 1 && editor.selection.is_empty() && ui_state.graph.selected_note.is_none() && ui_state.graph.selected_connections.is_empty();
            if only_group {
                let group = groups[0];
                let with_children = ui_state.graph.shift_held;
                if editor.document.delete_group(group, with_children) {
                    ui_state.graph.selected_groups.clear();
                    editor.changed();
                }
            } else if groups.is_empty() && ui_state.graph.selected_connections.is_empty() {
                group_selection(editor, ui_state);
            }
        }
        GraphCommand::ToggleGroupCollapsed(id) => editor.document.toggle_group_collapsed(id),
        GraphCommand::SetGroupLocked(id, locked) => editor.document.set_group_locked(id, locked),
        GraphCommand::LockSelection => {
            if let Some(node) = editor.active_node() {
                editor.lock_preview(node);
            }
            for id in ui_state.graph.selected_groups.clone() {
                let locked = editor.document.group(id).is_some_and(|group| group.locked);
                editor.document.set_group_locked(id, !locked);
            }
        }
        GraphCommand::SetGroupColor(id, color) => editor.document.set_group_color(id, color),
        GraphCommand::SetGroupIcon(id, icon) => editor.document.set_group_icon(id, icon),
        GraphCommand::RenameGroup(id) => {
            let name = editor.document.group(id).map(|group| group.name).unwrap_or_default();
            ui_state.graph.renaming_group = Some((id, name));
        }
        GraphCommand::RenameNode { node, name } => {
            if editor.document.rename(node, &name) {
                ask_rename_export_file(editor, ui_state, node, name);
            }
        }
        GraphCommand::RenameExportFile { node, name } => {
            editor.document.rename_export_file(node, &name);
            ask_rename_variable(editor, ui_state, node);
        }
        GraphCommand::AskRenameVariable(node) => ask_rename_variable(editor, ui_state, node),
        GraphCommand::RenameBoundVariable { key, name } => editor.document.rename_variable(&key, &name),
        GraphCommand::GroupToTab(id) => {
            let text = "Are you sure you want to convert this group to Tab.";
            let confirmed = Command::Graph(GraphCommand::GroupToTabConfirmed(id));
            ui_state.prompt = Some(Prompt::new("Convert Group to Tab?", text, confirmed));
        }
        GraphCommand::GroupToTabConfirmed(id) => {
            if let Some(tab) = editor.document.group_to_tab(id) {
                ui_state.graph.tab = tab;
                ui_state.graph.selected_groups.clear();
                editor.changed();
            }
        }
        GraphCommand::TabToGroup { from, target } => {
            let tabs = editor.document.graph_tabs();
            let name = tabs.iter().find(|tab| tab.index == from).map(|tab| tab.name.clone()).unwrap_or_default();
            let caption = format!("Convert ({name}) Tab to Group?");
            let text = "Are you sure you want to convert this Tab to group.";
            let confirmed = Command::Graph(GraphCommand::TabToGroupConfirmed { from, target });
            ui_state.prompt = Some(Prompt::new(&caption, text, confirmed));
        }
        GraphCommand::TabToGroupConfirmed { from, target } => {
            let sizes = extents(editor);
            let extent = |id: i32| extent_of(&sizes, id);
            let tab = editor.document.tab_to_group(from, target, &extent);
            ui_state.graph.tab = tab;
            ui_state.graph.clear_element_selection();
            editor.select_group();
            editor.changed();
        }
        GraphCommand::InsertNodeSearch { to, to_port } => {
            let Some(middle) = connection_middle(editor, ui_state, to, &to_port) else {
                return;
            };
            open_search(ui_state, SearchMode::Interject { to, to_port }, middle);
        }
        GraphCommand::InsertChokepoint { to, to_port } => {
            let Some(middle) = connection_middle(editor, ui_state, to, &to_port) else {
                return;
            };
            let view = ui_state.graph.view();
            let canvas = view.to_canvas(ui_state.graph.rect.min, middle);
            let corner = [canvas.x - 32.0, canvas.y - 32.0];
            if let Some(chokepoint) = editor.document.insert_node_in_connection(to, &to_port, "Chokepoint", corner, true) {
                editor.user.usage.record_node("Chokepoint");
                ui_state.graph.clear_element_selection();
                editor.select(Some(chokepoint));
                editor.changed();
            }
        }
        GraphCommand::InsertNodeInConnection { to, to_port, type_name, corner } => {
            if let Some(inserted) = editor.document.insert_node_in_connection(to, &to_port, &type_name, corner, false) {
                prepare_added_node(editor, inserted, None);
                editor.user.usage.record_node(&type_name);
                ui_state.graph.selected_connections.clear();
                editor.changed();
            }
        }
        GraphCommand::ChokepointMenu(node) => {
            let at = ui_state.graph.pointer.unwrap_or(ui_state.graph.rect.center());
            ui_state.graph.popup = Some(GraphPopup::ChokepointAt { node, at });
        }
        GraphCommand::InsertChokepointAt { node, port } => {
            let sizes = extents(editor);
            let extent = |id: i32| extent_of(&sizes, id);
            if editor.document.insert_chokepoint_at(node, &port, &extent).is_some() {
                editor.user.usage.record_node("Chokepoint");
                editor.changed();
            }
        }
        GraphCommand::ConvertConnectionsToPortals => {
            let connections = ui_state.graph.selected_connections.clone();
            editor.document.convert_connections_to_portals(&connections);
            ui_state.graph.selected_connections.clear();
        }
        GraphCommand::ManagePortals => {
            let Some(node) = editor.active_node() else {
                return;
            };
            let Some(rect) = node_screen_rect(editor, ui_state, node) else {
                return;
            };
            // `InitialMousePos`: 30 in from the node's corner.
            let zoom = ui_state.graph.view().zoom;
            let at = rect.min + egui::Vec2::splat(30.0 * zoom);
            ui_state.graph.popup = Some(GraphPopup::Portals { node, at });
        }
        GraphCommand::TogglePortPortal { node, port } => editor.document.toggle_port_portal(node, &port),
        GraphCommand::PublishPort { node, port, through_chokepoint } => {
            editor.document.publish_port(node, &port, through_chokepoint);
            if through_chokepoint {
                editor.user.usage.record_node("Chokepoint");
                editor.changed();
            }
        }
        GraphCommand::RemovePortal { node, port } => {
            editor.document.remove_portal(node, &port);
            editor.changed();
        }
        GraphCommand::DisconnectPortalInput { node, port } => {
            editor.document.disconnect_portal_input(node, &port);
            editor.changed();
        }
        GraphCommand::PortalSearch { node, port } => {
            let corner = node_screen_rect(editor, ui_state, node).map_or(ui_state.graph.rect.center(), |rect| rect.min);
            let zoom = ui_state.graph.view().zoom;
            let at = corner + egui::Vec2::splat(30.0 * zoom);
            open_search(ui_state, SearchMode::Portals { to: node, to_port: port }, at);
        }
        GraphCommand::ConnectToPortal { from, from_port, to, to_port } => {
            editor.document.connect_to_portal(from, &from_port, to, &to_port);
            record_connection(editor, from, &from_port, to);
            editor.changed();
        }
        GraphCommand::ConnectPorts { from, from_port, to, to_port } => {
            if editor.document.would_cycle(from, to) {
                return;
            }
            editor.document.connect(from, &from_port, to, &to_port);
            record_connection(editor, from, &from_port, to);
            editor.changed();
        }
        GraphCommand::AddMaskAndConnect { from, from_port, to } => {
            if editor.document.would_cycle(from, to) {
                return;
            }
            editor.document.checkpoint("Connect");
            editor.document.add_mask_port(to);
            editor.document.link(from, &from_port, to, "Mask");
            record_connection(editor, from, &from_port, to);
            editor.changed();
        }
        GraphCommand::AddPortAndConnect { from, from_port, to } => {
            if editor.document.would_cycle(from, to) {
                return;
            }
            editor.document.checkpoint("Connect");
            let Some(input) = editor.document.add_new_input(to) else {
                editor.document.undo();
                return;
            };
            editor.document.link(from, &from_port, to, &input);
            record_connection(editor, from, &from_port, to);
            editor.changed();
        }
        GraphCommand::Combine(kind) => {
            let sizes = extents(editor);
            let extent = |id: i32| extent_of(&sizes, id);
            let selected = editor.selection.clone();
            if let Some(combined) = editor.document.combine_nodes(&selected, kind, &extent) {
                let type_name = editor.document.node(combined).map(|node| node.type_name.clone()).unwrap_or_default();
                editor.user.usage.record_node(&type_name);
                ui_state.graph.clear_element_selection();
                editor.select(Some(combined));
                editor.changed();
            }
        }
        GraphCommand::CombineOutputs { dragged, dragged_port, other, other_port } => combine_outputs(editor, ui_state, dragged, &dragged_port, other, &other_port),
        GraphCommand::Mutate => {
            let selected = editor.selection.clone();
            editor.document.mutate_nodes(&selected, editor.mutation);
            editor.changed();
        }
        GraphCommand::MutateButton => {
            editor.mutation += 1;
            let selected = editor.selection.clone();
            if selected.is_empty() {
                mutate_all(editor);
            } else {
                editor.document.mutate_nodes(&selected, editor.mutation);
                editor.changed();
            }
        }
        GraphCommand::MutateAll => mutate_all(editor),
        GraphCommand::AutoLayout => {
            if editor.active_node().is_none() {
                return;
            }
            let sizes = extents(editor);
            let extent = |id: i32| extent_of(&sizes, id);
            let selection = editor.selection.clone();
            let tab = ui_state.graph.tab;
            let new_method = editor.user.options.new_autolayout();
            let layout = Layout::new(&mut editor.document, tab, &extent).with_new_method(new_method);
            layout.run(&selection);
        }
        GraphCommand::CycleRenderIntent => {
            let selected = editor.selection.clone();
            let colors: HashMap<i32, bool> = selected.iter().map(|id| (*id, is_color(editor, *id))).collect();
            let color_of = |id: i32| colors.get(&id).copied().unwrap_or(false);
            editor.document.cycle_render_intent(&selected, &color_of);
            editor.changed();
        }
        GraphCommand::MoveToTab(target) => {
            let nodes = editor.selection.clone();
            let notes: Vec<i32> = ui_state.graph.selected_note.into_iter().collect();
            let groups = ui_state.graph.selected_groups.clone();
            if nodes.is_empty() && notes.is_empty() && groups.is_empty() {
                return;
            }
            let tab = editor.document.move_elements_to_tab(&nodes, &notes, &groups, target);
            ui_state.graph.tab = tab;
            editor.changed();
        }
        GraphCommand::ToolboxCreate(type_name) => toolbox_create(editor, ui_state, &type_name),
        GraphCommand::ToolboxDrop { type_name, at, onto } => toolbox_drop(editor, ui_state, &type_name, at, onto),
        GraphCommand::DropFiles { files, at } => drop_files(editor, ui_state, &files, at),
        GraphCommand::ToggleNoteType(id) => editor.document.toggle_note_type(id),
        GraphCommand::SetNoteColor(id, color) => editor.document.set_note_color(id, color),
        GraphCommand::EditNote(id) => {
            let notes = editor.document.notes();
            let fields = notes.iter().find(|(note, _)| *note == id).map(|(_, fields)| *fields);
            let markdown = fields.and_then(|fields| fields.get("Markdown")).and_then(Value::as_str).unwrap_or_default().to_string();
            ui_state.graph.editing_note = Some((id, markdown));
        }
        GraphCommand::DeleteNote(id) => {
            let request = DeleteRequest { notes: vec![id], ..DeleteRequest::default() };
            editor.document.delete_elements(&request);
            if ui_state.graph.selected_note == Some(id) {
                ui_state.graph.selected_note = None;
            }
        }
        GraphCommand::ToggleSnapToItems => {
            let on = !editor.user.options.snap_to_items();
            editor.user.options.set_snap_to_items(on);
        }
    }
}

/// After a node's rename (`EditablePopup_EditSuccess`): a node marked for
/// export is asked about first, whether its export file takes the new
/// name; then the variable bound to it. Closing the first question
/// answers No.
fn ask_rename_export_file(editor: &Editor, ui_state: &mut UiState, node: i32, name: String) {
    if editor.document.export_settings(node).is_none() {
        ask_rename_variable(editor, ui_state, node);
        return;
    }
    let text = "Do you want to update the filename to match the new node name?";
    let yes = Command::Graph(GraphCommand::RenameExportFile { node, name });
    let next = Command::Graph(GraphCommand::AskRenameVariable(node));
    let mut question = Prompt::new("Update Save Definition", text, yes);
    question.no = Some(next.clone());
    question.dismissed = Some(next);
    ui_state.prompt = Some(question);
}

/// Whether the one variable bound to a renamed node takes its name too.
fn ask_rename_variable(editor: &Editor, ui_state: &mut UiState, node: i32) {
    let Some((key, name)) = editor.document.bound_variable_rename(node) else {
        return;
    };
    let text = format!("Would you like to rename the associated variable [{key}] to [{name}]?");
    let yes = Command::Graph(GraphCommand::RenameBoundVariable { key, name });
    ui_state.prompt = Some(Prompt::new("Rename Variable", &text, yes));
}

/// The graph menus and windows `--show` opens (for checking the editor
/// without a person at it).
const LAUNCH_MENUS: [&str; 8] = ["lazy-menu", "node-search", "find-node", "quick-navigation", "manage-portals", "connection-menu", "create-connection", "rename-node"];

pub fn opens_at_launch(panel: &str) -> bool {
    LAUNCH_MENUS.contains(&panel)
}

/// `--show lazy-menu`, `node-search[:<text>]`, `find-node[:<text>]`,
/// `quick-navigation` and `manage-portals` (for the selected node), or
/// `connection-menu` (the first connection of the tab shown), opened in
/// the middle of the graph; `create-connection`, the Create Connection
/// window from the selected node's outputs to another node's inputs; or
/// `rename-node:<name>`, the selected node renamed, with the questions a
/// rename asks.
pub fn open_at_launch(show: &str, editor: &mut Editor, ui_state: &mut UiState) {
    let (panel, detail) = show.split_once(':').unwrap_or((show, ""));
    let center = ui_state.graph.rect.center();
    match panel {
        "lazy-menu" => ui_state.graph.popup = Some(GraphPopup::Lazy { at: center }),
        "node-search" | "find-node" => {
            let mode = if panel == "find-node" { SearchMode::Navigate } else { SearchMode::Nodes { source: NodeSource::None } };
            open_search(ui_state, mode, center);
            if let Some(GraphPopup::Search(search)) = &mut ui_state.graph.popup {
                search.type_text(detail);
            }
        }
        "quick-navigation" => run(GraphCommand::QuickNavigation, editor, ui_state),
        "manage-portals" => run(GraphCommand::ManagePortals, editor, ui_state),
        "connection-menu" => {
            let tab = ui_state.graph.tab;
            let nodes = editor.document.terrain().nodes.values().filter(|node| node_tab(node) == tab);
            let mut inputs = nodes.flat_map(|node| node.ports.iter().filter(|port| port.record.is_some()).map(move |port| (node.id, port.name.clone())));
            if let Some((to, to_port)) = inputs.next() {
                ui_state.graph.clear_element_selection();
                editor.select(None);
                ui_state.graph.selected_connections = vec![(to, to_port.clone())];
                ui_state.graph.popup = Some(GraphPopup::Connection { to, to_port, at: center });
            }
        }
        "create-connection" => {
            let tab = ui_state.graph.tab;
            let mut on_tab: Vec<i32> = Vec::new();
            for node in editor.document.terrain().nodes.values() {
                if node_tab(node) == tab {
                    on_tab.push(node.id);
                }
            }
            on_tab.sort_unstable();
            let source = editor.active_node().or(on_tab.first().copied());
            let target = on_tab.iter().copied().find(|id| Some(*id) != source);
            if let (Some(source), Some(target)) = (source, target) {
                let window = crate::ui::create_connection::CreateConnection::new(source, target, true);
                ui_state.graph.create_connection = Some(window);
            }
        }
        "rename-node" => {
            if let Some(node) = editor.active_node() {
                let rename = GraphCommand::RenameNode { node, name: detail.to_string() };
                run(rename, editor, ui_state);
            }
        }
        _ => {}
    }
}

fn open_search(ui_state: &mut UiState, mode: SearchMode, at: Pos2) {
    let view = ui_state.graph.view();
    let canvas = view.to_canvas(ui_state.graph.rect.min, at);
    let popup = SearchPopup::new(mode, at, [canvas.x, canvas.y]);
    ui_state.graph.popup = Some(GraphPopup::Search(popup));
}

/// Where the node is on screen.
fn node_screen_rect(editor: &Editor, ui_state: &UiState, node: i32) -> Option<egui::Rect> {
    let data = editor.document.node(node)?;
    let default_size = editor.user.options.default_node_size().to_string();
    let extent = node_extent(data, &default_size);
    let corner = Pos2::new(data.position[0], data.position[1]);
    let canvas = egui::Rect::from_min_size(corner, egui::Vec2::new(extent[0], extent[1]));
    let view = ui_state.graph.view();
    Some(view.rect_to_screen(ui_state.graph.rect.min, canvas))
}

/// Half way along a connection, on screen (`InterjectConnection`'s start
/// and end points averaged).
fn connection_middle(editor: &Editor, ui_state: &UiState, to: i32, to_port: &str) -> Option<Pos2> {
    let (start, end) = crate::ui::graph_canvas::connection_ends(editor, &ui_state.graph, to, to_port)?;
    Some(Pos2::new((start.x + end.x) / 2.0, (start.y + end.y) / 2.0))
}

/// `Node.IsColor`: a colorizer, or a node whose main output is a colour.
fn is_color(editor: &Editor, id: i32) -> bool {
    let Some(node) = editor.document.node(id) else {
        return false;
    };
    let spec = Catalogue::get().node(&node.type_name);
    if spec.and_then(|spec| spec.classification.as_deref()) == Some("Colorizer") {
        return true;
    }
    let live = editor.node_caches.get(&id).and_then(|cache| cache.out_intent());
    if let Some(intent) = live {
        return intent == nerthus_engine::RenderIntent::Color;
    }
    let declared_port = spec.and_then(|spec| spec.ports.iter().find(|port| port.name == "Out"));
    let declared = declared_port.and_then(|port| port.render_intent.as_deref());
    declared == Some("Color")
}

/// Which node followed which port (by node names).
fn record_connection(editor: &mut Editor, from: i32, from_port: &str, to: i32) {
    let from_name = editor.document.node(from).map(|node| node.name.clone()).unwrap_or_default();
    let to_name = editor.document.node(to).map(|node| node.name.clone()).unwrap_or_default();
    editor.user.usage.record_connection(&from_name, from_port, &to_name);
}

/// The mutation counted on and every node mutated.
fn mutate_all(editor: &mut Editor) {
    editor.mutation += 1;
    let all: Vec<i32> = editor.document.terrain().nodes.keys().copied().collect();
    editor.document.mutate_nodes(&all, editor.mutation);
    editor.changed();
}

/// `GraphArea.CreateGroup` (Ctrl G): the selected nodes and note in a new
/// group around them, unless one of them is in a group already or a group
/// is chosen. With nothing chosen, an empty group at the pointer (or below
/// a chosen connection's start).
fn group_selection(editor: &mut Editor, ui_state: &mut UiState) {
    if !ui_state.graph.selected_groups.is_empty() {
        return;
    }
    let in_group = editor.selection.iter().any(|id| editor.document.group_of(*id).is_some());
    if in_group {
        return;
    }
    let mut children = editor.selection.clone();
    children.extend(ui_state.graph.selected_note);
    let tab = ui_state.graph.tab;
    let sizes = extents(editor);
    let mut boxes: Vec<([f32; 2], [f32; 2])> = Vec::new();
    for id in &editor.selection {
        if let Some(node) = editor.document.node(*id) {
            boxes.push((node.position, extent_of(&sizes, *id)));
        }
    }
    if let Some(note) = ui_state.graph.selected_note
        && let Some(rect) = crate::ui::graph_notes::note_rect(editor, tab, note)
    {
        boxes.push(([rect.left(), rect.top()], [rect.width(), rect.height()]));
    }
    let (position, size) = if boxes.is_empty() {
        let view = ui_state.graph.view();
        let origin = ui_state.graph.rect.min;
        let connection = ui_state.graph.selected_connections.first();
        let start = connection.and_then(|(to, to_port)| crate::ui::graph_canvas::connection_ends(editor, &ui_state.graph, *to, to_port));
        match start {
            Some((start, _)) => {
                let start = view.to_canvas(origin, start);
                ([start.x, start.y + 100.0], GROUP_MIN_SIZE)
            }
            None => {
                let pointer = ui_state.graph.pointer.unwrap_or(ui_state.graph.rect.center());
                let canvas = view.to_canvas(origin, pointer);
                ([canvas.x, canvas.y], GROUP_MIN_SIZE)
            }
        }
    } else {
        group_box(&boxes)
    };
    let group = editor.document.add_group(&children, position, size, tab);
    ui_state.graph.clear_element_selection();
    editor.select_group();
    ui_state.graph.selected_groups = vec![group];
}

/// `AddGroup` around elements (`GetBoundingBox`: the least corner, the
/// greatest corner plus the greatest size): 20 left and 40 above it, at
/// least 270 square, else 50 and 70 bigger than it.
fn group_box(boxes: &[([f32; 2], [f32; 2])]) -> ([f32; 2], [f32; 2]) {
    let mut left = f32::INFINITY;
    let mut top = f32::INFINITY;
    let mut greatest_x = f32::NEG_INFINITY;
    let mut greatest_y = f32::NEG_INFINITY;
    let mut widest = f32::NEG_INFINITY;
    let mut tallest = f32::NEG_INFINITY;
    for (position, size) in boxes {
        left = left.min(position[0]);
        top = top.min(position[1]);
        greatest_x = greatest_x.max(position[0]);
        greatest_y = greatest_y.max(position[1]);
        widest = widest.max(size[0]);
        tallest = tallest.max(size[1]);
    }
    let width = greatest_x + widest - left;
    let height = greatest_y + tallest - top;
    let size_x = if width < GROUP_MIN_SIZE[0] { GROUP_MIN_SIZE[0] } else { width + 50.0 };
    let size_y = if height < GROUP_MIN_SIZE[1] { GROUP_MIN_SIZE[1] } else { height + 70.0 };
    ([left - 20.0, top - 40.0], [size_x, size_y])
}

/// A node about to be added (`G_BeforeNodeAdded`): its type's default
/// preset, then the one picked in the search; shown as the options say.
fn prepare_added_node(editor: &mut Editor, id: i32, preset: Option<&str>) {
    editor.apply_default_preset(id);
    if let Some(name) = preset {
        apply_preset_quietly(editor, id, name);
    }
    editor.document.apply_node_display(&[id], &editor.user.options);
}

/// `PresetService.ApplyPreset` as part of the edit that added the node.
fn apply_preset_quietly(editor: &mut Editor, id: i32, name: &str) {
    let Some(type_name) = editor.document.node(id).map(|node| node.type_name.clone()) else {
        return;
    };
    let Some(spec) = Catalogue::get().node(&type_name) else {
        return;
    };
    let Some(preset) = editor.presets.named(name, &spec.name) else {
        return;
    };
    let values = crate::presets::preset_values(preset, spec);
    editor.document.set_values_quietly(id, &values);
}

/// `ReplaceNode` from the search, the preset applied after.
fn replace_with(editor: &mut Editor, ui_state: &mut UiState, node: i32, pick: &SearchPick) {
    let same_type = editor.document.node(node).is_some_and(|data| data.type_name == pick.type_name);
    if same_type && pick.preset.is_none() {
        return;
    }
    let Some(replacement) = editor.document.replace_node(node, &pick.type_name) else {
        return;
    };
    editor.follow_preview_lock();
    prepare_added_node(editor, replacement, pick.preset.as_deref());
    editor.user.usage.record_node(&pick.type_name);
    ui_state.graph.clear_element_selection();
    editor.select(Some(replacement));
    editor.changed();
}

/// `CreateSearchedNode` and `CreateConnectedNode`: a node at the search's
/// point, after the selected node (to its right), or on the dragged
/// connection; several (typed with commas) in a chain 300 apart.
fn create_nodes(editor: &mut Editor, ui_state: &mut UiState, picks: &[SearchPick], at: [f32; 2], source: NodeSource) {
    let picks: Vec<&SearchPick> = picks.iter().filter(|pick| Catalogue::get().node(&pick.type_name).is_some()).collect();
    let Some(first) = picks.first() else {
        return;
    };
    let tab = ui_state.graph.tab;
    let several = picks.len() > 1;
    let label = if several { "Multiple nodes added.".to_string() } else { format!("Add {}", display_name(&first.type_name)) };
    editor.document.checkpoint(label);
    let sizes = extents(editor);
    let mut previous: Option<(i32, String, bool)> = match &source {
        NodeSource::None => None,
        NodeSource::Selected(node) => first_output(editor, *node).map(|port| (*node, port, true)),
        NodeSource::Port { node, port, output } => Some((*node, port.clone(), *output)),
    };
    let mut created = Vec::new();
    for (index, pick) in picks.iter().enumerate() {
        let mut corner = at;
        if several {
            corner[0] += 300.0 * (index + 1) as f32;
        }
        // One node after the selected one goes to its right.
        if let (false, NodeSource::Selected(node)) = (several, &source)
            && let Some(position) = editor.document.node(*node).map(|data| data.position)
        {
            let width = extent_of(&sizes, *node)[0];
            corner = [position[0] + width + 100.0, position[1]];
        }
        let Some(id) = editor.document.place_node(&pick.type_name, corner, tab) else {
            continue;
        };
        prepare_added_node(editor, id, pick.preset.as_deref());
        editor.user.usage.record_node(&pick.type_name);
        if let Some((from, port, output)) = previous.take() {
            connect_new(editor, from, &port, output, id);
        }
        if let Some(output) = first_output(editor, id) {
            previous = Some((id, output, true));
        }
        created.push(id);
    }
    settle(editor, &created, tab);
    ui_state.graph.clear_element_selection();
    if let Some(last) = created.last() {
        editor.select(Some(*last));
        // `PanToElementLocation` after a connected node.
        if source != NodeSource::None {
            ui_state.graph.zoom_request = Some(ZoomRequest::Center(*last));
        }
    }
    editor.changed();
}

/// Joins a new node to a port: an output feeds its first input; an input
/// is fed by its first output.
fn connect_new(editor: &mut Editor, node: i32, port: &str, output: bool, new_node: i32) {
    if output {
        let Some(input) = first_input(editor, new_node) else {
            return;
        };
        if editor.document.would_cycle(node, new_node) {
            return;
        }
        editor.document.link(node, port, new_node, &input);
        record_connection(editor, node, port, new_node);
    } else {
        let Some(new_output) = first_output(editor, new_node) else {
            return;
        };
        editor.document.link(new_node, &new_output, node, port);
        record_connection(editor, new_node, &new_output, node);
    }
}

/// `CombineOutPorts`: a new Combine, 1.5 times the dragged node's width
/// right of the two nodes and half way between them, its first input fed
/// by the dragged output and its second by the other. It joins the group
/// it lands in and is selected.
fn combine_outputs(editor: &mut Editor, ui_state: &mut UiState, dragged: i32, dragged_port: &str, other: i32, other_port: &str) {
    if dragged == other {
        return;
    }
    let (Some(dragged_node), Some(other_node)) = (editor.document.node(dragged), editor.document.node(other)) else {
        return;
    };
    let sizes = extents(editor);
    let right = dragged_node.position[0].max(other_node.position[0]);
    let top = dragged_node.position[1].min(other_node.position[1]);
    let bottom = dragged_node.position[1].max(other_node.position[1]);
    let corner = [right + extent_of(&sizes, dragged)[0] * 1.5, top + (bottom - top) / 2.0];
    let tab = node_tab(dragged_node);
    editor.document.checkpoint("Nodes combined as inputs.");
    let Some(combined) = editor.document.place_node("Combine", corner, tab) else {
        editor.document.undo();
        return;
    };
    prepare_added_node(editor, combined, None);
    editor.user.usage.record_node("Combine");
    let Some(combine_node) = editor.document.node(combined) else {
        return;
    };
    let inputs: Vec<String> = grid_inputs(combine_node).iter().map(|port| port.name.clone()).collect();
    let (Some(first_input), Some(second_input)) = (inputs.first(), inputs.get(1)) else {
        return;
    };
    editor.document.link(dragged, dragged_port, combined, first_input);
    record_connection(editor, dragged, dragged_port, combined);
    editor.document.link(other, other_port, combined, second_input);
    record_connection(editor, other, other_port, combined);
    settle(editor, &[combined], tab);
    ui_state.graph.clear_element_selection();
    editor.select(Some(combined));
    editor.changed();
}

/// `InPort(0)`.
fn first_input(editor: &Editor, node: i32) -> Option<String> {
    let data = editor.document.node(node)?;
    grid_inputs(data).first().map(|port| port.name.clone())
}

/// `OutPort(0)`.
fn first_output(editor: &Editor, node: i32) -> Option<String> {
    let data = editor.document.node(node)?;
    grid_outputs(data).first().map(|port| port.name.clone())
}

fn display_name(type_name: &str) -> String {
    let Some(spec) = Catalogue::get().node(type_name) else {
        return type_name.to_string();
    };
    spec.display_name.clone().unwrap_or_else(|| spec.name.clone())
}

/// Added nodes join the group whose box holds their corner (`AddNode`).
fn settle(editor: &mut Editor, nodes: &[i32], tab: usize) {
    let mut corners: Vec<(i32, [f32; 2], usize)> = Vec::new();
    for id in nodes {
        if let Some(node) = editor.document.node(*id) {
            corners.push((*id, node.position, tab));
        }
    }
    editor.document.settle_in_groups(&corners);
}

/// `Toolbox.CreateNodeOnCanvas`: a node in the middle of the view; with
/// one node selected, fed by it and placed to its right (below what it
/// feeds already).
fn toolbox_create(editor: &mut Editor, ui_state: &mut UiState, type_name: &str) {
    let tab = ui_state.graph.tab;
    let sizes = extents(editor);
    let view = ui_state.graph.view();
    let visible = ui_state.graph.rect.size() / view.zoom;
    let center = view.location + visible / 2.0;
    let previous = match editor.selection.as_slice() {
        [single] => editor.document.node(*single).cloned(),
        _ => None,
    };
    editor.document.checkpoint(format!("Add {}", display_name(type_name)));
    let Some(id) = editor.document.place_node(type_name, [center.x, center.y], tab) else {
        editor.document.undo();
        return;
    };
    prepare_added_node(editor, id, None);
    editor.user.usage.record_node(type_name);
    let default_size = editor.user.options.default_node_size().to_string();
    let new_extent = editor.document.node(id).map_or(STANDARD_EXTENT, |node| node_extent(node, &default_size));
    let corner = match &previous {
        Some(previous) => {
            let previous_width = extent_of(&sizes, previous.id)[0];
            let outgoing = editor.document.outgoing(previous.id);
            let feeds: Vec<i32> = outgoing.into_iter().map(|(target, _, _)| target).filter(|target| *target != id).collect();
            match feeds.last() {
                None => [previous.position[0] + previous_width + 60.0, previous.position[1]],
                Some(last) => {
                    let heights: f32 = feeds.iter().map(|target| extent_of(&sizes, *target)[1]).sum();
                    let x = editor.document.node(*last).map_or(previous.position[0] + previous_width + 60.0, |node| node.position[0]);
                    [x, previous.position[1] + heights + feeds.len() as f32 * 20.0]
                }
            }
        }
        None => [center.x - new_extent[0] / 2.0, center.y - new_extent[1] / 2.0],
    };
    editor.document.move_nodes(&[(id, corner)], false);
    if let Some(previous) = &previous {
        let output = previous.port("Out").map(|port| port.name.clone());
        let input = editor.document.node(id).and_then(|node| node.port("In")).map(|port| port.name.clone());
        if let (Some(output), Some(input)) = (output, input) {
            editor.document.link(previous.id, &output, id, &input);
            record_connection(editor, previous.id, &output, id);
        }
    }
    settle(editor, &[id], tab);
    ui_state.graph.clear_element_selection();
    editor.select(Some(id));
    editor.changed();
}

/// `canvas_Drop`: a node 8 up and left of where the toolbox entry was
/// dropped; dropped on another node, it goes 60 right of that node, fed
/// by its `Out`.
fn toolbox_drop(editor: &mut Editor, ui_state: &mut UiState, type_name: &str, at: [f32; 2], onto: Option<i32>) {
    let tab = ui_state.graph.tab;
    let sizes = extents(editor);
    editor.document.checkpoint(format!("Add {}", display_name(type_name)));
    let corner = [at[0] - 8.0, at[1] - 8.0];
    let Some(id) = editor.document.place_node(type_name, corner, tab) else {
        editor.document.undo();
        return;
    };
    prepare_added_node(editor, id, None);
    editor.user.usage.record_node(type_name);
    let target = onto.and_then(|node| editor.document.node(node).cloned());
    if let Some(target) = target {
        let has_out = target.port("Out").is_some();
        let has_in = editor.document.node(id).is_some_and(|node| node.port("In").is_some());
        if has_out && has_in {
            let width = extent_of(&sizes, target.id)[0];
            let beside = [target.position[0] + width + 60.0, target.position[1]];
            editor.document.move_nodes(&[(id, beside)], false);
            editor.document.link(target.id, "Out", id, "In");
            record_connection(editor, target.id, "Out", id);
        }
    }
    settle(editor, &[id], tab);
    ui_state.graph.clear_element_selection();
    editor.select(Some(id));
    editor.changed();
}

/// The images a File node reads.
const IMAGE_EXTENSIONS: [&str; 21] = [
    "exr", "graw", "jpg", "png", "r16", "r32", "raw", "tif", "jpeg", "webp", "tiff", "svg", "rgb", "miff", "jp2", "hdr", "eps", "emf", "dds", "dng", "bmp",
];
/// The meshes an Object node reads.
const MESH_EXTENSIONS: [&str; 5] = ["obj", "fbx", "dae", "gltf", "glb"];

fn extension_of(path: &Path) -> String {
    path.extension().map(|extension| extension.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// `DropFiles`: two or more files of one tile set make a TileInput reading
/// the first; otherwise each image makes a File node (colour when it has
/// more than one channel) and each mesh an Object node, all where they
/// were dropped.
fn drop_files(editor: &mut Editor, ui_state: &mut UiState, files: &[PathBuf], at: [f32; 2]) {
    let tab = ui_state.graph.tab;
    let mut planned: Vec<(&str, PathBuf, bool)> = Vec::new();
    if nerthus_nodes::nodes::is_tile_set(files) {
        planned.push(("TileInput", files[0].clone(), false));
    } else {
        for file in files.iter().filter(|file| file.is_file()) {
            let extension = extension_of(file);
            if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
                let channels = nerthus_nodes::file_maps::image_channel_count(file);
                let colour = channels.is_some_and(|count| count > 1);
                planned.push(("File", file.clone(), colour));
            }
            if MESH_EXTENSIONS.contains(&extension.as_str()) {
                planned.push(("Object", file.clone(), false));
            }
        }
    }
    let Some(first) = planned.first() else {
        return;
    };
    let label = if planned.len() == 1 { format!("Add {}", display_name(first.0)) } else { "Multiple nodes added.".to_string() };
    editor.document.checkpoint(label);
    let mut created = Vec::new();
    for (type_name, file, colour) in planned {
        let Some(id) = editor.document.place_node(type_name, at, tab) else {
            continue;
        };
        let path = Value::String(file.to_string_lossy().to_string());
        let mut values = vec![("FileName".to_string(), path)];
        if colour {
            values.push(("IsRGB".to_string(), Value::Bool(true)));
        }
        editor.document.set_values_quietly(id, &values);
        prepare_added_node(editor, id, None);
        editor.user.usage.record_node(type_name);
        created.push(id);
    }
    settle(editor, &created, tab);
    ui_state.graph.clear_element_selection();
    if let Some(last) = created.last() {
        editor.select(Some(*last));
    }
    editor.changed();
}

/// `miLazyBoy_Click`: what each Lazy Menu entry opens, with the pointer
/// moved there (`SetCursorPosition`) when `MoveLazyCursor` is on.
fn lazy_action(tag: &str, editor: &mut Editor, ui_state: &mut UiState) {
    let target = match tag {
        "lighting" => {
            ui_state.sun_open = !ui_state.sun_open;
            None
        }
        "graph" => Some(LazyTarget::Graph),
        "prop" => Some(LazyTarget::Properties),
        "modifiers" => Some(LazyTarget::Modifiers),
        "datatree" => {
            ui_state.data_view.show_tab(crate::ui::data_view::DataTab::Terrain);
            Some(LazyTarget::DataView)
        }
        "databuild" => {
            ui_state.data_view.show_tab(crate::ui::data_view::DataTab::Build);
            Some(LazyTarget::DataView)
        }
        "variables" => {
            if !editor.user.options.show_advanced_tools() {
                return;
            }
            ui_state.data_view.show_tab(crate::ui::data_view::DataTab::Vars);
            Some(LazyTarget::DataView)
        }
        "buildsettings" => {
            if ui_state.build_settings.is_none() {
                let tab = crate::ui::build_settings::SettingsTab::Resolution;
                ui_state.build_settings = Some(crate::ui::build_settings::BuildSettingsState::new(tab));
            }
            None
        }
        "render-raw" => {
            editor.set_render_mode(crate::editor::RenderMode::Data);
            None
        }
        "render-clay" => {
            editor.set_render_mode(crate::editor::RenderMode::Clay);
            None
        }
        "render-realistic" => {
            editor.set_render_mode(crate::editor::RenderMode::Realistic);
            None
        }
        "toggle-skybox" => {
            ui_state.graph.toggle_skybox = true;
            None
        }
        "2d" => {
            ui_state.viewport_2d.toggle_docked();
            Some(LazyTarget::Viewport2D)
        }
        _ => None,
    };
    if editor.user.options.move_lazy_cursor() {
        ui_state.graph.lazy_cursor = target;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_output_let_go_on_another_output_makes_a_combine() {
        let mut document = crate::document::Document::new();
        let erosion = document.add_node("ErosionPro", [26000.0, 26000.0]).unwrap();
        let mountain = document.add_node("Mountain", [26000.0, 26400.0]).unwrap();
        let mut editor = Editor::new(document);
        let mut ui_state = UiState::default();
        let dropped = GraphCommand::CombineOutputs { dragged: erosion, dragged_port: "Deposits".to_string(), other: mountain, other_port: "Out".to_string() };
        run(dropped, &mut editor, &mut ui_state);

        let combined = editor.selection[0];
        let node = editor.document.node(combined).unwrap();
        assert_eq!(node.type_name, "Combine");
        // 1.5 widths of the dragged node right of the two, half way down.
        assert_eq!(node.position, [26000.0 + 230.0 * 1.5, 26200.0]);
        let source = |port: &str| node.port(port).and_then(|port| port.record.as_ref()).map(|record| (record.from, record.from_port.clone()));
        assert_eq!(source("In"), Some((erosion, "Deposits".to_string())));
        assert_eq!(source("Input2"), Some((mountain, "Out".to_string())));
        // The dragged output is a mask, so the Combine renders as one.
        assert_eq!(node.base.get("RenderIntentOverride"), Some(&Value::from("Mask")));
        assert_eq!(editor.document.undo_labels()[0], "Nodes combined as inputs.");
        editor.document.undo();
        assert!(editor.document.node(combined).is_none());
    }

    #[test]
    fn a_rename_asks_about_the_export_file_then_the_variable() {
        let mut document = crate::document::Document::new();
        let mountain = document.add_node("Mountain", [26000.0, 26000.0]).unwrap();
        document.toggle_export(&[mountain], nerthus_export::FileFormat::Exr);
        document.bind_new_variable(mountain, "Height");
        let mut editor = Editor::new(document);
        let mut ui_state = UiState::default();
        let rename = GraphCommand::RenameNode { node: mountain, name: "Peak".to_string() };
        run(rename, &mut editor, &mut ui_state);
        assert_eq!(editor.document.node(mountain).unwrap().name, "Peak");

        // The export file first; No or closing the box goes on to the variable.
        let question = ui_state.prompt.take().expect("the export file is asked about");
        assert_eq!(question.caption, "Update Save Definition");
        assert_eq!(question.text, "Do you want to update the filename to match the new node name?");
        let next = Command::Graph(GraphCommand::AskRenameVariable(mountain));
        assert_eq!(question.no, Some(next.clone()));
        assert_eq!(question.dismissed, Some(next));
        let Some(Command::Graph(yes)) = question.yes else {
            panic!("Yes renames the file");
        };
        run(yes, &mut editor, &mut ui_state);
        assert_eq!(editor.document.export_settings(mountain).unwrap().filename, "Peak");

        let question = ui_state.prompt.take().expect("the variable is asked about");
        assert_eq!(question.caption, "Rename Variable");
        assert_eq!(question.text, "Would you like to rename the associated variable [Height] to [Peak]?");
        let Some(Command::Graph(yes)) = question.yes else {
            panic!("Yes renames the variable");
        };
        run(yes, &mut editor, &mut ui_state);
        let automation = editor.document.automation();
        assert!(automation.variables.contains_key("Peak") && !automation.variables.contains_key("Height"));
        assert_eq!(automation.bindings[0].variable, "Peak");
        assert!(ui_state.prompt.is_none());

        // The variable's rename is a step of its own; the file goes back
        // with the name.
        let labels = editor.document.undo_labels();
        assert_eq!(labels[..2], ["Variable Height renamed to Peak", "Node renamed: Mountain renamed to Peak."]);
        editor.document.undo();
        editor.document.undo();
        assert_eq!(editor.document.node(mountain).unwrap().name, "Mountain");
        assert_eq!(editor.document.export_settings(mountain).unwrap().filename, "Mountain");
        assert!(editor.document.automation().variables.contains_key("Height"));
    }

    #[test]
    fn a_rename_asks_only_what_applies() {
        let mut document = crate::document::Document::new();
        let mountain = document.add_node("Mountain", [26000.0, 26000.0]).unwrap();
        let mut editor = Editor::new(document);
        let mut ui_state = UiState::default();
        // Neither exported nor bound: the name alone.
        let rename = GraphCommand::RenameNode { node: mountain, name: "Peak".to_string() };
        run(rename.clone(), &mut editor, &mut ui_state);
        assert!(ui_state.prompt.is_none());
        // The same name again: nothing at all.
        let steps = editor.document.undo_labels().len();
        run(rename, &mut editor, &mut ui_state);
        assert_eq!(editor.document.undo_labels().len(), steps);

        // Bound but not exported: straight to the variable, `Name_Id` when
        // another variable has the name.
        editor.document.bind_new_variable(mountain, "Height");
        let other = editor.document.add_variable("Float");
        editor.document.rename_variable(&other, "Ridge");
        let rename = GraphCommand::RenameNode { node: mountain, name: "Ridge".to_string() };
        run(rename, &mut editor, &mut ui_state);
        let question = ui_state.prompt.take().expect("the variable is asked about");
        assert_eq!(question.text, format!("Would you like to rename the associated variable [Height] to [Ridge_{mountain}]?"));
    }

    #[test]
    fn the_lazy_menus_2d_viewport_toggles_the_docked_view() {
        let mut editor = Editor::new(crate::document::Document::new());
        let mut ui_state = UiState::default();
        lazy_action("2d", &mut editor, &mut ui_state);
        assert!(ui_state.viewport_2d.docked);
        assert_eq!(ui_state.graph.lazy_cursor, Some(LazyTarget::Viewport2D));
        lazy_action("2d", &mut editor, &mut ui_state);
        assert!(!ui_state.viewport_2d.docked);
    }

    #[test]
    fn groups_wrap_their_elements() {
        // One node: the box is under 270 square, so 270 it is.
        let single = group_box(&[([100.0, 200.0], [230.0, 80.0])]);
        assert_eq!(single, ([80.0, 160.0], GROUP_MIN_SIZE));
        // Wider than 270: 50 more across; taller: 70 more down.
        let spread = group_box(&[([0.0, 0.0], [230.0, 80.0]), ([400.0, 300.0], [230.0, 80.0])]);
        assert_eq!(spread, ([-20.0, -40.0], [680.0, 450.0]));
    }
}
