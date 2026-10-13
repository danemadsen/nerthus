//! Reading the graph, in brief or one node in full, showing a node in the
//! editor, and placing the 3D view's camera.

use nerthus_engine::json::f32_value;
use nerthus_engine::{Catalogue, NodeData, NodeState, StateOverride};
use serde_json::{json, Map, Value};

use super::arguments::{number_of, Arguments};
use super::build_tools::format_key;
use super::{values, Step, ToolContext, ToolResult};
use crate::document::node_tab;
use crate::editor::{Editor, NodeStatus};
use crate::ui::graph_canvas::ZoomRequest;
use crate::viewport_camera;

/// A node's output by name (any case), or its main one: `Out`, else its
/// first.
pub(crate) fn output_port(node: &NodeData, wanted: Option<&str>) -> Result<String, String> {
    let outputs: Vec<&str> = node.ports.iter().filter(|port| port.is_output()).map(|port| port.name.as_str()).collect();
    let found = match wanted {
        Some(wanted) => outputs.iter().find(|name| name.eq_ignore_ascii_case(wanted)),
        None => outputs.iter().find(|name| **name == "Out").or(outputs.first()),
    };
    match (found, wanted) {
        (Some(name), _) => Ok(name.to_string()),
        (None, Some(wanted)) => Err(format!("{} has no output {wanted}; its outputs are {}.", node.name, outputs.join(", "))),
        (None, None) => Err(format!("{} has no outputs.", node.name)),
    }
}

/// A node's input by name (any case).
pub(crate) fn input_port(node: &NodeData, wanted: &str) -> Result<String, String> {
    let inputs: Vec<&str> = node.ports.iter().filter(|port| port.is_input()).map(|port| port.name.as_str()).collect();
    let found = inputs.iter().find(|name| name.eq_ignore_ascii_case(wanted));
    found.map(|name| name.to_string()).ok_or_else(|| format!("{} has no input {wanted}; its inputs are {}.", node.name, inputs.join(", ")))
}

/// A node's state when it is not the usual one.
fn state_name(node: &NodeData) -> Option<&'static str> {
    match StateOverride::of(node) {
        StateOverride::Bypassed => Some("bypassed"),
        StateOverride::Baked => Some("baked"),
        StateOverride::None => None,
    }
}

/// A node's last build, in a few words.
fn build_text(status: &NodeStatus) -> String {
    if status.building {
        return "building".to_string();
    }
    if let Some(error) = &status.error {
        return format!("error: {error}");
    }
    match status.state {
        NodeState::Unbuilt => "not built".to_string(),
        _ => format!("built in {} ms", status.milliseconds.round()),
    }
}

/// A node in brief, as `list_nodes` and the edit tools show it.
pub(crate) fn node_brief(editor: &Editor, node: &NodeData) -> Value {
    let mut entry = Map::new();
    entry.insert("id".to_string(), json!(node.id));
    entry.insert("name".to_string(), json!(node.name));
    entry.insert("type".to_string(), json!(node.type_name));
    entry.insert("tab".to_string(), json!(node_tab(node)));
    entry.insert("position".to_string(), json!([node.position[0].round(), node.position[1].round()]));
    let mut inputs = Map::new();
    for port in node.ports.iter().filter(|port| port.is_input()) {
        let source = port.record.as_ref().map(|record| json!({"node": record.from, "port": record.from_port}));
        inputs.insert(port.name.clone(), source.unwrap_or(Value::Null));
    }
    entry.insert("inputs".to_string(), Value::Object(inputs));
    let outputs: Vec<&str> = node.ports.iter().filter(|port| port.is_output()).map(|port| port.name.as_str()).collect();
    entry.insert("outputs".to_string(), json!(outputs));
    if let Some(state) = state_name(node) {
        entry.insert("state".to_string(), json!(state));
    }
    if editor.document.export_settings(node.id).is_some() {
        entry.insert("exported".to_string(), json!(true));
    }
    let status = editor.status(node.id);
    entry.insert("build".to_string(), json!(build_text(&status)));
    Value::Object(entry)
}

/// The nodes, one compact JSON object a line: still JSON, at half the
/// size of one member a line (large graphs list hundreds of nodes).
pub(crate) fn list_nodes(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let tab = arguments.integer("tab")?;
    let editor = &*context.editor;
    let mut lines = Vec::new();
    for node in editor.document.terrain().nodes.values() {
        let on_tab = tab.is_none_or(|tab| tab == node_tab(node) as i64);
        if on_tab {
            let brief = node_brief(editor, node);
            lines.push(format!("  {brief}"));
        }
    }
    let listing = format!("{{\"count\": {}, \"nodes\": [\n{}\n]}}", lines.len(), lines.join(",\n"));
    Ok(Step::Done(ToolResult::text(listing)))
}

/// A node's parameters as the property panel has them: each with its
/// value and limits, whether the panel shows it and lets it be changed as
/// things stand, and what automates it; the panel's computed text apart.
fn parameters(editor: &Editor, node: &NodeData) -> (Vec<Value>, Map<String, Value>) {
    let Some(spec) = Catalogue::get().node(&node.type_name) else {
        return (Vec::new(), Map::new());
    };
    let asset = &editor.document.project.assets[0];
    let rules = crate::node_ui::node_ui(node, &asset.terrain.nodes, &asset.automation);
    let mut listed = Vec::new();
    let mut notes = Map::new();
    for parameter in &spec.parameters {
        if parameter.kind == "ComputedString" {
            let text = rules.texts.get(&parameter.name).filter(|text| !text.is_empty());
            if let Some(text) = text {
                notes.insert(parameter.name.clone(), json!(text));
            }
            continue;
        }
        if values::settable(parameter).is_err() {
            continue;
        }
        let mut entry = values::describe(parameter, spec.defaults.get(&parameter.name));
        let value = node.values.get(&parameter.name).cloned().unwrap_or(Value::Null);
        entry.insert("value".to_string(), value);
        if !(parameter.visible && rules.visible(&parameter.name)) {
            entry.insert("shown".to_string(), json!(false));
        }
        if !(parameter.enabled && rules.enabled(&parameter.name)) {
            entry.insert("enabled".to_string(), json!(false));
        }
        let binding = asset.automation.bindings.iter().find(|binding| binding.node == node.id && binding.property == parameter.name);
        if let Some(binding) = binding {
            entry.insert("variable".to_string(), json!(binding.variable));
        }
        let expression = editor.document.expression(node.id, &parameter.name);
        if !expression.is_empty() {
            entry.insert("expression".to_string(), json!(expression));
        }
        listed.push(Value::Object(entry));
    }
    (listed, notes)
}

/// A node's modifiers with their parameters.
fn modifiers(node: &NodeData) -> Vec<Value> {
    let mut listed = Vec::new();
    for modifier in &node.modifiers {
        let spec = Catalogue::get().modifier(&modifier.type_name);
        let mut parameters = Map::new();
        for parameter in spec.iter().flat_map(|spec| spec.parameters.iter()) {
            let value = modifier.values.get(&parameter.name).cloned().unwrap_or(Value::Null);
            parameters.insert(parameter.name.clone(), value);
        }
        let name = nerthus_engine::editing::modifier_name(modifier);
        listed.push(json!({"modifier": name, "parameters": parameters}));
    }
    listed
}

/// A node in full, as `get_node` shows it.
pub(crate) fn node_details(editor: &Editor, id: i32) -> Result<Value, String> {
    let node = editor.document.node(id).ok_or_else(|| format!("No node has the id {id}."))?;
    let spec = Catalogue::get().node(&node.type_name);
    let mut details = Map::new();
    details.insert("id".to_string(), json!(node.id));
    details.insert("name".to_string(), json!(node.name));
    details.insert("type".to_string(), json!(node.type_name));
    if let Some(description) = spec.and_then(|spec| spec.description.as_deref()) {
        details.insert("description".to_string(), json!(description));
    }
    details.insert("tab".to_string(), json!(node_tab(node)));
    details.insert("position".to_string(), json!([node.position[0].round(), node.position[1].round()]));
    if let Some(state) = state_name(node) {
        details.insert("state".to_string(), json!(state));
    }

    let mut inputs = Vec::new();
    for port in node.ports.iter().filter(|port| port.is_input()) {
        let source = port.record.as_ref().map(|record| json!({"node": record.from, "port": record.from_port}));
        inputs.push(json!({"port": port.name, "from": source, "required": port.is_required()}));
    }
    details.insert("inputs".to_string(), json!(inputs));
    let fed = editor.document.outgoing(id);
    let mut outputs = Vec::new();
    for port in node.ports.iter().filter(|port| port.is_output()) {
        let feeds: Vec<Value> = fed.iter().filter(|(_, _, output)| *output == port.name).map(|(target, input, _)| json!({"node": target, "port": input})).collect();
        outputs.push(json!({"port": port.name, "feeds": feeds}));
    }
    details.insert("outputs".to_string(), json!(outputs));

    let (listed, notes) = parameters(editor, node);
    details.insert("parameters".to_string(), json!(listed));
    if !notes.is_empty() {
        details.insert("notes".to_string(), Value::Object(notes));
    }
    let modifiers = modifiers(node);
    if !modifiers.is_empty() {
        details.insert("modifiers".to_string(), json!(modifiers));
    }
    if let Some(settings) = editor.document.export_settings(id) {
        let export = json!({"file_name": settings.filename, "format": format_key(settings.format), "enabled": settings.enabled});
        details.insert("export".to_string(), export);
    }
    let status = editor.status(id);
    let mut build = json!({"state": build_text(&status)});
    if let Some((low, high)) = status.range {
        build["range"] = json!([f32_value(low), f32_value(high)]);
    }
    details.insert("build".to_string(), build);
    Ok(Value::Object(details))
}

pub(crate) fn get_node(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let editor = &*context.editor;
    let id = arguments.node("node", editor)?;
    let details = node_details(editor, id)?;
    Ok(Step::Done(ToolResult::json(&details)))
}

/// `set_camera`: the viewport's camera moved, from its current view (or
/// the default one), and kept as the project's camera as Load Camera keeps
/// a camera file's.
pub(crate) fn set_camera(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let editor = &mut *context.editor;
    let reset = arguments.boolean("reset")?.unwrap_or(false);
    let mut camera = if reset { nerthus_render::CameraValues::DEFAULT } else { editor.camera };
    if let Some(values) = arguments.array("look_at")? {
        let numbers: Vec<f64> = values.iter().filter_map(number_of).collect();
        let [x, y, z] = numbers[..] else {
            return Err("look_at is [x, y, z].".to_string());
        };
        let x = x.clamp(-100.0, 300.0) as f32;
        let y = y.clamp(0.0, 20.0) as f32;
        let z = z.clamp(-100.0, 300.0) as f32;
        camera.look_at = bevy::math::Vec3::new(x, y, z);
    }
    if let Some(inclination) = arguments.number("inclination")? {
        camera.inclination = inclination.clamp(0.001, 1.0) as f32;
    }
    if let Some(zoom) = arguments.number("zoom")? {
        camera.zoom = zoom.clamp(0.4, 250.0) as f32;
    }
    if let Some(rotation) = arguments.number("rotation")? {
        camera.rotation = rotation.clamp(-1.0, 1.0) as f32;
    }
    let message = viewport_camera::camera_message(&camera);
    editor.document.set_saved_camera(&message);
    editor.camera_request = Some(camera);
    let look_at = [camera.look_at.x, camera.look_at.y, camera.look_at.z].map(f32_value);
    let outcome = json!({
        "look_at": look_at,
        "inclination": f32_value(camera.inclination),
        "zoom": f32_value(camera.zoom),
        "rotation": f32_value(camera.rotation),
    });
    Ok(Step::Done(ToolResult::json(&outcome)))
}

pub(crate) fn select_node(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let editor = &mut *context.editor;
    let id = arguments.node("node", editor)?;
    let node = editor.document.node(id).ok_or("The node is gone.")?;
    let port = match arguments.string("port")? {
        Some(wanted) => Some(output_port(node, Some(wanted))?),
        None => None,
    };
    // `GoToNode`: its tab shown, the node centred and selected.
    let graph = &mut context.ui_state.graph;
    graph.tab = node_tab(node);
    graph.zoom_request = Some(ZoomRequest::Center(id));
    graph.clear_element_selection();
    editor.select(Some(id));
    // Shown even while the preview is locked to another node.
    editor.preview(Some(id));
    if let Some(port) = &port {
        editor.preview_port = port.clone();
        editor.show_preview();
    }
    let shown = port.unwrap_or_else(|| editor.preview_port.clone());
    let outcome = json!({"selected": id, "previewing": shown});
    Ok(Step::Done(ToolResult::json(&outcome)))
}
