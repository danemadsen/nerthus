//! Editing the graph as the interface edits it: each change an undo step
//! named as the interface names it, the selection and preview following
//! as they do there. Everything a call asks for is checked before its
//! edit starts, so a mistake leaves the project as it was.

use std::collections::HashSet;

use nerthus_engine::editing::{can_add_ports, modifier_name};
use nerthus_engine::{Catalogue, NodeData, StateOverride};
use serde_json::{json, Map, Value};

use super::arguments::{find_node, Arguments};
use super::catalogue_tools::{display_name, find_type};
use super::graph_tools::{input_port, node_brief, output_port};
use super::tools::MODIFIERS;
use super::{values, Step, ToolContext, ToolResult};
use crate::document::{node_tab, DeleteRequest};
use crate::editor::Editor;
use crate::ui::graph_canvas::node_extent;
use crate::ui::UiState;

/// A standard node's size, for nodes without a layout.
pub(crate) const STANDARD_EXTENT: [f32; 2] = [230.0, 80.0];
/// The gap between a node and the node it feeds (`CreateNodeOnCanvas`).
pub(crate) const FEED_GAP: f32 = 60.0;
/// The gap between nodes stacked under one another.
pub(crate) const STACK_GAP: f32 = 20.0;
/// The gap above a new row of nodes.
const ROW_GAP: f32 = 60.0;

fn done_with_notes(mut outcome: Value, notes: Vec<String>) -> Result<Step, String> {
    if !notes.is_empty() {
        outcome["notes"] = json!(notes);
    }
    Ok(Step::Done(ToolResult::json(&outcome)))
}

/// Where a new node goes, as the toolbox places one: right of the node
/// feeding it (under what that node feeds already), else under the tab's
/// nodes, else in the middle of the graph's view.
pub(crate) fn placement(editor: &Editor, ui_state: &UiState, tab: usize, source: Option<i32>, added: i32) -> [f32; 2] {
    let default_size = editor.user.options.default_node_size().to_string();
    let extent = |id: i32| editor.document.node(id).map_or(STANDARD_EXTENT, |node| node_extent(node, &default_size));
    if let Some(source) = source.and_then(|id| editor.document.node(id)) {
        let beside = source.position[0] + extent(source.id)[0] + FEED_GAP;
        let outgoing = editor.document.outgoing(source.id);
        let fed: Vec<i32> = outgoing.into_iter().map(|(target, _, _)| target).filter(|target| *target != added).collect();
        let Some(last) = fed.last() else {
            return [beside, source.position[1]];
        };
        let heights: f32 = fed.iter().map(|target| extent(*target)[1]).sum();
        let x = editor.document.node(*last).map_or(beside, |node| node.position[0]);
        return [x, source.position[1] + heights + fed.len() as f32 * STACK_GAP];
    }
    let on_tab: Vec<&NodeData> = editor.document.terrain().nodes.values().filter(|node| node.id != added && node_tab(node) == tab).collect();
    if !on_tab.is_empty() {
        let left = on_tab.iter().map(|node| node.position[0]).fold(f32::INFINITY, f32::min);
        let bottom = on_tab.iter().map(|node| node.position[1] + extent(node.id)[1]).fold(f32::NEG_INFINITY, f32::max);
        return [left, bottom + ROW_GAP];
    }
    let view = ui_state.graph.view();
    let visible = ui_state.graph.rect.size() / view.zoom;
    let center = view.location + visible / 2.0;
    let size = extent(added);
    [center.x - size[0] / 2.0, center.y - size[1] / 2.0]
}

/// Parameter values as the property panel stores them, and notes on what
/// changed on the way (clamping, rounding).
#[derive(Default)]
pub(crate) struct Converted {
    pub values: Vec<(String, Value)>,
    pub notes: Vec<String>,
}

/// Parameter values for a node of `spec` (`node` itself when it exists),
/// each checked.
pub(crate) fn converted_parameters(spec: &nerthus_engine::NodeSpec, given: &Map<String, Value>, node: Option<&NodeData>) -> Result<Converted, String> {
    let mut converted = Converted::default();
    for (name, value) in given {
        let parameter = values::find_parameter(spec, name)?;
        let current = match node {
            Some(node) => node.values.get(&parameter.name),
            None => spec.defaults.get(&parameter.name),
        };
        let stored = values::convert(parameter, value, current)?;
        converted.notes.extend(stored.note);
        converted.values.push((parameter.name.clone(), stored.value));
    }
    Ok(converted)
}

pub(crate) fn add_node(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let spec = find_type(arguments.required_string("type")?)?;
    let editor = &mut *context.editor;
    let tab = match arguments.integer("tab")? {
        Some(tab) => tab.max(0) as usize,
        None => context.ui_state.graph.tab,
    };
    let tab_exists = editor.document.graph_tabs().iter().any(|info| info.index == tab);
    if !tab_exists {
        return Err(format!("There is no graph tab {tab}; get_project lists the tabs."));
    }
    // The new node's main input, fed from `input_from` when given.
    let fresh = NodeData::new(spec, 0, 0);
    let main_input = fresh.ports.iter().filter(|port| port.is_input()).find(|port| port.is_primary()).or_else(|| fresh.ports.iter().find(|port| port.is_input()));
    let main_input = main_input.map(|port| port.name.clone());
    let feed = match arguments.optional_node("input_from", editor)? {
        Some(source) => {
            let source_node = editor.document.node(source).ok_or("The node to feed from is gone.")?;
            let port = output_port(source_node, arguments.string("input_from_port")?)?;
            let Some(input) = main_input else {
                return Err(format!("{} has no input to feed.", display_name(spec)));
            };
            Some((source, port, input))
        }
        None => None,
    };
    let converted = match arguments.object("parameters")? {
        Some(given) => converted_parameters(spec, given, None)?,
        None => Converted::default(),
    };
    let Converted { values: parameters, notes } = converted;
    let name = arguments.string("name")?.map(str::trim).filter(|name| !name.is_empty());
    let position = arguments.point("position")?;

    // `G_BeforeNodeAdded` and `CreateNodeOnCanvas`, as one edit.
    editor.document.checkpoint(format!("Add {}", display_name(spec)));
    let Some(id) = editor.document.place_node(spec.key(), [0.0, 0.0], tab) else {
        editor.document.undo();
        return Err(format!("{} could not be added.", display_name(spec)));
    };
    editor.apply_default_preset(id);
    if !parameters.is_empty() {
        editor.document.set_values_quietly(id, &parameters);
    }
    editor.document.apply_node_display(&[id], &editor.user.options);
    if let Some(name) = name
        && let Some(node) = editor.document.project.terrain_mut().nodes.get_mut(&id)
    {
        node.name = name.to_string();
    }
    let source = feed.as_ref().map(|(source, _, _)| *source);
    let corner = position.unwrap_or_else(|| placement(editor, context.ui_state, tab, source, id));
    editor.document.move_nodes(&[(id, corner)], false);
    if let Some((source, port, input)) = &feed {
        editor.document.link(*source, port, id, input);
    }
    editor.document.settle_in_groups(&[(id, corner, tab)]);
    context.ui_state.graph.clear_element_selection();
    editor.select(Some(id));
    editor.changed();

    let added = editor.document.node(id).ok_or("The node is gone.")?;
    let outcome = node_brief(editor, added);
    done_with_notes(outcome, notes)
}

pub(crate) fn set_parameters(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let editor = &mut *context.editor;
    let id = arguments.node("node", editor)?;
    let given = arguments.object("parameters")?.ok_or("parameters is required.")?;
    if given.is_empty() {
        return Err("parameters names no parameter.".to_string());
    }
    let node = editor.document.node(id).ok_or("The node is gone.")?;
    let spec = Catalogue::get().node(&node.type_name).ok_or("The node's type is unknown.")?;
    let Converted { values: changes, mut notes } = converted_parameters(spec, given, Some(node))?;
    for (name, _) in &changes {
        let bound = editor.document.automation().bindings.iter().find(|binding| binding.node == id && binding.property == *name);
        if let Some(binding) = bound {
            notes.push(format!("{name} is bound to the variable {}, whose value it takes when the node builds.", binding.variable));
        }
        if !editor.document.expression(id, name).is_empty() {
            notes.push(format!("{name} has an expression, which changes the value when the node builds."));
        }
    }
    match changes.as_slice() {
        // One parameter merges with further changes of it, as a slider does.
        [(name, value)] => editor.document.set_value(id, name, value.clone()),
        _ => {
            let names: Vec<&str> = changes.iter().map(|(name, _)| name.as_str()).collect();
            let label = format!("Change {}", names.join(", "));
            editor.document.set_values(id, &changes, &label);
        }
    }
    editor.changed();
    let set: Map<String, Value> = changes.into_iter().collect();
    let outcome = json!({"node": id, "set": set});
    done_with_notes(outcome, notes)
}

pub(crate) fn update_node(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let editor = &mut *context.editor;
    let id = arguments.node("node", editor)?;
    let name = arguments.string("name")?.map(str::trim);
    let position = arguments.point("position")?;
    let bypassed = arguments.boolean("bypassed")?;
    if name.is_none() && position.is_none() && bypassed.is_none() {
        return Err("Give a name, a position or bypassed.".to_string());
    }
    if name == Some("") {
        return Err("A node's name cannot be empty.".to_string());
    }
    if let Some(name) = name {
        editor.document.rename(id, name);
    }
    if let Some(position) = position {
        editor.document.move_nodes(&[(id, position)], true);
    }
    if let Some(bypassed) = bypassed {
        let node = editor.document.node(id).ok_or("The node is gone.")?;
        let is_bypassed = StateOverride::of(node) == StateOverride::Bypassed;
        if bypassed != is_bypassed {
            let state = if bypassed { "Bypassed" } else { "None" };
            editor.document.set_member(id, "StateOverride", json!(state), "Bypass", true);
        }
    }
    editor.changed();
    let node = editor.document.node(id).ok_or("The node is gone.")?;
    Ok(Step::Done(ToolResult::json(&node_brief(editor, node))))
}

/// Which input a connection goes into.
pub(crate) enum Feed {
    Existing(String),
    /// A mask input added first.
    AddedMask,
    /// Another input added first (Combine and the like).
    AddedPort,
}

/// The input asked for, or the first free one: the main input, any
/// other, a new one where the node grows them, else the main input again.
pub(crate) fn choose_input(node: &NodeData, requested: Option<&str>) -> Result<Feed, String> {
    if let Some(requested) = requested {
        let found = node.ports.iter().find(|port| port.is_input() && port.name.eq_ignore_ascii_case(requested));
        if let Some(port) = found {
            return Ok(Feed::Existing(port.name.clone()));
        }
        if requested.eq_ignore_ascii_case("Mask") {
            let takes_mask = Catalogue::get().node(&node.type_name).is_some_and(|spec| spec.is_maskable());
            if takes_mask {
                return Ok(Feed::AddedMask);
            }
            return Err(format!("{} takes no mask.", node.name));
        }
        return input_port(node, requested).map(Feed::Existing);
    }
    let inputs: Vec<_> = node.ports.iter().filter(|port| port.is_input() && port.name != "Mask").collect();
    let Some(main) = inputs.iter().find(|port| port.is_primary()).or(inputs.first()) else {
        return Err(format!("{} has no inputs.", node.name));
    };
    if main.record.is_none() {
        return Ok(Feed::Existing(main.name.clone()));
    }
    if let Some(free) = inputs.iter().find(|port| port.record.is_none()) {
        return Ok(Feed::Existing(free.name.clone()));
    }
    if can_add_ports(&node.type_name) {
        return Ok(Feed::AddedPort);
    }
    Ok(Feed::Existing(main.name.clone()))
}

pub(crate) fn connect_nodes(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let editor = &mut *context.editor;
    let from = arguments.node("from", editor)?;
    let to = arguments.node("to", editor)?;
    if from == to {
        return Err("A node cannot feed itself.".to_string());
    }
    let from_node = editor.document.node(from).ok_or("The node to connect from is gone.")?;
    let from_port = output_port(from_node, arguments.string("from_port")?)?;
    if editor.document.would_cycle(from, to) {
        return Err(format!("Connecting {from} to {to} would make a loop: {to} already feeds {from}."));
    }
    let to_node = editor.document.node(to).ok_or("The node to connect to is gone.")?;
    let input = choose_input(to_node, arguments.string("to_port")?)?;
    let input_name = match &input {
        Feed::Existing(name) => name.clone(),
        Feed::AddedMask => "Mask".to_string(),
        Feed::AddedPort => String::new(),
    };
    let replaced = to_node.port(&input_name).and_then(|port| port.record.as_ref());
    let replaced = replaced.map(|record| json!({"node": record.from, "port": record.from_port}));

    let to_port = match input {
        Feed::Existing(port) => {
            editor.document.connect(from, &from_port, to, &port);
            port
        }
        Feed::AddedMask => {
            editor.document.checkpoint("Connect");
            editor.document.add_mask_port(to);
            editor.document.link(from, &from_port, to, "Mask");
            "Mask".to_string()
        }
        Feed::AddedPort => {
            editor.document.checkpoint("Connect");
            let Some(port) = editor.document.add_new_input(to) else {
                editor.document.undo();
                return Err("No input could be added to the node.".to_string());
            };
            editor.document.link(from, &from_port, to, &port);
            port
        }
    };
    editor.changed();
    let mut outcome = json!({"from": {"node": from, "port": from_port}, "to": {"node": to, "port": to_port}});
    if let Some(replaced) = replaced {
        outcome["replaced"] = replaced;
    }
    Ok(Step::Done(ToolResult::json(&outcome)))
}

pub(crate) fn disconnect_input(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let editor = &mut *context.editor;
    let id = arguments.node("node", editor)?;
    let node = editor.document.node(id).ok_or("The node is gone.")?;
    let port = input_port(node, arguments.required_string("port")?)?;
    let source = node.port(&port).and_then(|data| data.record.as_ref());
    let Some(source) = source.map(|record| json!({"node": record.from, "port": record.from_port})) else {
        return Err(format!("{port} of {} is not connected.", node.name));
    };
    editor.document.delete_connection(id, &port);
    editor.changed();
    let outcome = json!({"node": id, "port": port, "was_fed_from": source});
    Ok(Step::Done(ToolResult::json(&outcome)))
}

pub(crate) fn delete_nodes(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let editor = &mut *context.editor;
    let items = arguments.array("nodes")?.ok_or("nodes is required.")?;
    let mut ids = Vec::new();
    for item in items {
        let id = find_node(editor, item)?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    if ids.is_empty() {
        return Err("nodes names no node.".to_string());
    }
    let named: Vec<(i32, String)> = ids.iter().filter_map(|id| editor.document.node(*id).map(|node| (*id, node.name.clone()))).collect();
    let request = DeleteRequest { nodes: ids, ..DeleteRequest::default() };
    editor.document.delete_elements(&request);
    // What the interface's Delete leaves.
    editor.selection.retain(|id| editor.document.terrain().nodes.contains_key(id));
    let preview_gone = editor.preview_node.is_some_and(|node| editor.document.node(node).is_none());
    if preview_gone {
        editor.preview_node = None;
        editor.preview_locked = false;
    }
    editor.changed();

    let remaining: HashSet<i32> = editor.document.terrain().nodes.keys().copied().collect();
    let deleted: Vec<Value> = named.iter().filter(|(id, _)| !remaining.contains(id)).map(|(id, name)| json!({"id": id, "name": name})).collect();
    let kept: Vec<Value> = named.iter().filter(|(id, _)| remaining.contains(id)).map(|(id, name)| json!({"id": id, "name": name})).collect();
    let mut outcome = json!({"deleted": deleted});
    let mut notes = Vec::new();
    if !kept.is_empty() {
        outcome["kept"] = json!(kept);
        notes.push("Nodes in a locked group stay; unlock the group in the editor first.".to_string());
    }
    let orphans = editor.document.orphan_variables();
    if !orphans.is_empty() {
        notes.push(format!("No parameter is bound to these variables any more: {}.", orphans.join(", ")));
    }
    done_with_notes(outcome, notes)
}

pub(crate) fn set_modifier(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let editor = &mut *context.editor;
    let id = arguments.node("node", editor)?;
    let wanted = arguments.required_string("modifier")?;
    let found = MODIFIERS.iter().find(|name| name.eq_ignore_ascii_case(wanted.trim()));
    let type_name = *found.ok_or_else(|| format!("There is no {wanted} modifier; the modifiers are {}.", MODIFIERS.join(", ")))?;
    let spec = Catalogue::get().modifier(type_name).ok_or("The modifier's type is unknown.")?;
    let enabled = arguments.boolean("enabled")?.unwrap_or(true);
    let given = arguments.object("parameters")?;
    if !enabled && given.is_some_and(|given| !given.is_empty()) {
        return Err("parameters go with enabled: true.".to_string());
    }
    let index_of = |editor: &Editor| editor.document.node(id).and_then(|node| node.modifiers.iter().position(|modifier| modifier_name(modifier) == type_name));
    let present = index_of(editor);
    // Checked before anything changes.
    let mut changes = Vec::new();
    for (name, value) in given.into_iter().flatten() {
        let parameter = values::find_modifier_parameter(spec, name)?;
        let current = present.and_then(|index| editor.document.node(id).and_then(|node| node.modifiers.get(index))).and_then(|modifier| modifier.values.get(&parameter.name));
        let stored = values::convert(parameter, value, current)?;
        changes.push((parameter.name.clone(), stored));
    }

    match (enabled, present) {
        (true, None) => editor.document.toggle_modifier(id, type_name),
        (false, Some(_)) => editor.document.toggle_modifier(id, type_name),
        _ => {}
    }
    let index = index_of(editor);
    if !enabled && index.is_some() {
        return Err(format!("The {type_name} modifier came with the project and cannot be removed here."));
    }
    let mut notes = Vec::new();
    if let Some(index) = index {
        for (name, stored) in changes {
            notes.extend(stored.note);
            editor.document.set_modifier_value(id, index, &name, stored.value);
        }
    }
    editor.changed();
    let node = editor.document.node(id).ok_or("The node is gone.")?;
    let listed: Vec<Value> = node.modifiers.iter().map(|modifier| json!(modifier_name(modifier))).collect();
    let outcome = json!({"node": id, "modifiers": listed});
    done_with_notes(outcome, notes)
}

/// Undo, or (`redo`) Redo, so many times.
pub(crate) fn undo(arguments: &Arguments, context: &mut ToolContext, redo: bool) -> Result<Step, String> {
    let steps = arguments.integer("steps")?.unwrap_or(1).clamp(1, 100);
    let editor = &mut *context.editor;
    let mut labels = Vec::new();
    for _ in 0..steps {
        let next = if redo { editor.document.redo_labels() } else { editor.document.undo_labels() };
        let done = if redo { editor.document.redo() } else { editor.document.undo() };
        if !done {
            break;
        }
        labels.extend(next.into_iter().next());
    }
    if labels.is_empty() {
        let nothing = if redo { "There is nothing to redo." } else { "There is nothing to undo." };
        return Err(nothing.to_string());
    }
    // As Undo and Redo leave the interface.
    editor.selection.retain(|id| editor.document.terrain().nodes.contains_key(id));
    context.ui_state.graph.clear_element_selection();
    editor.changed();
    let key = if redo { "redone" } else { "undone" };
    let mut outcome = json!({"can_undo": editor.document.can_undo(), "can_redo": editor.document.can_redo()});
    outcome[key] = json!(labels);
    Ok(Step::Done(ToolResult::json(&outcome)))
}
