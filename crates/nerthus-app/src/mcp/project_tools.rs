//! The project as a whole: an overview, opening, starting and saving
//! projects, project settings, and variables.

use std::path::PathBuf;

use nerthus_engine::automation::{VarType, VariableValue};
use nerthus_engine::json::f32_value;
use nerthus_export::build::exporting_nodes;
use serde_json::{json, Map, Value};

use super::arguments::{boolean_of, number_of, pair_of, Arguments};
use super::build_tools::{export_status, format_key};
use super::values::colour_of;
use super::{Step, ToolContext, ToolResult};
use crate::document::BUILD_DEFINITION_MEMBERS;
use crate::editor::{Editor, PREVIEW_RESOLUTIONS};

/// How many of the editor's status messages the overview carries.
const MESSAGES_SHOWN: usize = 8;
/// How many undo and redo steps it names.
const UNDO_SHOWN: usize = 5;
/// The terrain's size as Build Settings › Terrain bounds it (metres).
const TERRAIN_WIDTHS: (f64, f64) = (100.0, 2_400_000.0);
const TERRAIN_HEIGHTS: (f64, f64) = (100.0, 10_000.0);
/// The resolutions Build Settings offers run from 512 to 256K.
const BUILD_RESOLUTIONS: (i64, i64) = (512, 262_144);

/// The terrain's width and height in metres, as renders read them (the
/// defaults when the project has none).
fn terrain_size(editor: &Editor) -> (f32, f32) {
    let terrain = editor.document.terrain();
    let width = if terrain.width > 0.0 { terrain.width } else { 5000.0 };
    let height = if terrain.height > 0.0 { terrain.height } else { 2500.0 };
    (width, height)
}

fn build_settings(editor: &Editor) -> Value {
    let definition = editor.document.build_definition();
    let destination = if definition.destination.is_empty() { None } else { Some(definition.destination.clone()) };
    json!({
        "type": format!("{:?}", definition.build_type),
        "resolution": definition.resolution,
        "bake_resolution": definition.bake_resolution,
        "destination": destination,
        "overwrite_mode": format!("{:?}", definition.overwrite_mode),
        "organize_files": format!("{:?}", definition.organize_files),
        "output_type": format!("{:?}", definition.output_type),
        "color_space": definition.color_space,
    })
}

pub(crate) fn get_project(context: &mut ToolContext) -> Result<Step, String> {
    let editor = &*context.editor;
    let document = &editor.document;
    let terrain = document.terrain();
    let (width, height) = terrain_size(editor);
    let tabs: Vec<Value> = document.graph_tabs().iter().map(|tab| json!({"index": tab.index, "name": tab.name})).collect();
    let mut exports = Vec::new();
    for id in exporting_nodes(terrain) {
        let Some(settings) = document.export_settings(id) else {
            continue;
        };
        let name = document.node(id).map(|node| node.name.clone());
        exports.push(json!({"node": id, "name": name, "file_name": settings.filename, "format": format_key(settings.format)}));
    }
    let undo: Vec<String> = document.undo_labels().into_iter().take(UNDO_SHOWN).collect();
    let redo: Vec<String> = document.redo_labels().into_iter().take(UNDO_SHOWN).collect();
    let skipped = editor.messages.len().saturating_sub(MESSAGES_SHOWN);
    let messages = &editor.messages[skipped..];
    let preview = json!({
        "node": editor.preview_node,
        "port": editor.preview_port,
        "resolution": editor.preview_resolution,
        "locked": editor.preview_locked,
        "building": editor.building,
        "progress": f32_value((editor.build_progress * 100.0).round() / 100.0),
        "render_mode": format!("{:?}", editor.render_mode),
    });
    let overview = json!({
        "file": document.path,
        "name": document.file_name(),
        "unsaved_changes": document.dirty,
        "terrain": {"width_metres": f32_value(width), "height_metres": f32_value(height)},
        "tabs": tabs,
        "shown_tab": context.ui_state.graph.tab,
        "nodes": terrain.nodes.len(),
        "selected": editor.selection,
        "preview": preview,
        "engine_active": editor.engine_active,
        "build_settings": build_settings(editor),
        "exports": exports,
        "export_build": export_status(editor),
        "variables": document.automation().variables.len(),
        "undo": undo,
        "redo": redo,
        "messages": messages,
    });
    Ok(Step::Done(ToolResult::json(&overview)))
}

/// Opening or starting a project loses unsaved changes: refused unless
/// they are to be discarded (when the save question would be asked).
pub(crate) fn refuse_unsaved(editor: &Editor, arguments: &Arguments) -> Result<(), String> {
    let discard = arguments.boolean("discard_unsaved_changes")?.unwrap_or(false);
    let unsaved = editor.document.dirty && editor.document.can_undo();
    if unsaved && !discard {
        let name = editor.document.file_name();
        return Err(format!("{name} has unsaved changes. Save it first (save_project), or ask the user whether to discard them (discard_unsaved_changes)."));
    }
    Ok(())
}

/// The editor's latest status message (where failures are reported).
pub(crate) fn last_message(editor: &Editor) -> String {
    editor.messages.last().cloned().unwrap_or_default()
}

pub(crate) fn open_project(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let editor = &mut *context.editor;
    let path = PathBuf::from(arguments.required_string("path")?);
    if !path.is_file() {
        return Err(format!("There is no file at {}.", path.display()));
    }
    refuse_unsaved(editor, arguments)?;
    if !crate::files::open_project(editor, &path) {
        return Err(last_message(editor));
    }
    let terrain = editor.document.terrain();
    let mut outcome = json!({"opened": path, "nodes": terrain.nodes.len()});
    let unknown = &editor.document.project.unknown_types;
    if !unknown.is_empty() {
        outcome["unknown_node_types"] = json!(unknown);
    }
    Ok(Step::Done(ToolResult::json(&outcome)))
}

pub(crate) fn new_project(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let editor = &mut *context.editor;
    refuse_unsaved(editor, arguments)?;
    crate::files::new_project(editor);
    let outcome = json!({"name": editor.document.file_name(), "nodes": editor.document.terrain().nodes.len()});
    Ok(Step::Done(ToolResult::json(&outcome)))
}

pub(crate) fn save_project(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let path = save_path(context.editor, arguments.string("path")?)?;
    save_to(context.editor, &path)?;
    let outcome = json!({"saved": path});
    Ok(Step::Done(ToolResult::json(&outcome)))
}

/// Where Save (or Save As, given a path) writes: the path made a
/// `.terrain` file in a folder that exists.
pub(crate) fn save_path(editor: &Editor, given: Option<&str>) -> Result<PathBuf, String> {
    let path = match given {
        Some(text) => {
            let mut path = PathBuf::from(text.trim());
            if path.is_relative() {
                return Err("path must be absolute.".to_string());
            }
            if path.extension().is_none() {
                path.set_extension("terrain");
            }
            path
        }
        None => editor.document.path.clone().ok_or("The project has never been saved; give a path.")?,
    };
    let folder_exists = path.parent().is_some_and(std::path::Path::is_dir);
    if !folder_exists {
        return Err(format!("The folder for {} does not exist.", path.display()));
    }
    Ok(path)
}

/// Saves the open project to `path`, as File › Save As does.
pub(crate) fn save_to(editor: &mut Editor, path: &std::path::Path) -> Result<(), String> {
    editor.save_to(path);
    let saved = !editor.document.dirty && editor.document.path.as_deref() == Some(path);
    if !saved {
        return Err(last_message(editor));
    }
    Ok(())
}

pub(crate) fn set_project_settings(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let editor = &mut *context.editor;
    let width = arguments.number("terrain_width")?;
    let height = arguments.number("terrain_height")?;
    let preview_resolution = arguments.integer("preview_resolution")?;
    let build_resolution = arguments.integer("build_resolution")?;
    let destination = arguments.string("build_destination")?;
    let others = arguments.object("build_settings")?;

    // Everything checked before anything changes.
    if let Some(resolution) = preview_resolution
        && !PREVIEW_RESOLUTIONS.contains(&(resolution as usize))
    {
        return Err("preview_resolution is 512, 1024, 2048 or 4096.".to_string());
    }
    let mut members: Vec<(&str, Value)> = Vec::new();
    if let Some(resolution) = build_resolution {
        let (lowest, highest) = BUILD_RESOLUTIONS;
        if !(lowest..=highest).contains(&resolution) {
            return Err(format!("build_resolution runs from {lowest} to {highest}."));
        }
        members.push(("Resolution", json!(resolution)));
    }
    if let Some(destination) = destination {
        members.push(("Destination", json!(destination)));
    }
    for (name, value) in others.into_iter().flatten() {
        let member = BUILD_DEFINITION_MEMBERS.iter().find(|member| member.eq_ignore_ascii_case(name));
        let member = member.ok_or_else(|| format!("There is no build setting {name}; they are {}.", BUILD_DEFINITION_MEMBERS.join(", ")))?;
        members.push((member, value.clone()));
    }

    if width.is_some() || height.is_some() {
        let (current_width, current_height) = terrain_size(editor);
        let width = width.unwrap_or(current_width as f64).clamp(TERRAIN_WIDTHS.0, TERRAIN_WIDTHS.1);
        let height = height.unwrap_or(current_height as f64).clamp(TERRAIN_HEIGHTS.0, TERRAIN_HEIGHTS.1);
        editor.document.set_terrain_size(width.round() as f32, height.round() as f32);
        editor.changed();
    }
    if let Some(resolution) = preview_resolution {
        editor.set_preview_resolution(resolution as usize);
    }
    if !members.is_empty() {
        editor.document.set_build_settings(&members);
    }
    let (width, height) = terrain_size(editor);
    let outcome = json!({
        "terrain": {"width_metres": f32_value(width), "height_metres": f32_value(height)},
        "preview_resolution": editor.preview_resolution,
        "build_settings": build_settings(editor),
    });
    Ok(Step::Done(ToolResult::json(&outcome)))
}

/// A Choice variable's options (`ExtraData`, a list of text).
fn choice_options(json: &Value) -> Vec<String> {
    let extra = json.get("ExtraData");
    let list = extra.and_then(|extra| extra.get("$values").or(Some(extra))).and_then(Value::as_array);
    let options = list.map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect());
    options.unwrap_or_default()
}

pub(crate) fn list_variables(context: &mut ToolContext) -> Result<Step, String> {
    let automation = context.editor.document.automation();
    let mut listed = Vec::new();
    for (key, variable) in &automation.variables {
        if variable.var_type == VarType::Decoration {
            continue;
        }
        let mut entry = Map::new();
        entry.insert("key".to_string(), json!(key));
        if variable.name != *key {
            entry.insert("name".to_string(), json!(variable.name));
        }
        entry.insert("type".to_string(), json!(variable.var_type.name()));
        entry.insert("value".to_string(), variable.json.get("Value").cloned().unwrap_or(Value::Null));
        for (member, label) in [("Min", "min"), ("Max", "max")] {
            if let Some(limit) = variable.json.get(member) {
                entry.insert(label.to_string(), limit.clone());
            }
        }
        if variable.var_type == VarType::Choice {
            entry.insert("options".to_string(), json!(choice_options(&variable.json)));
        }
        let bound: Vec<Value> = automation.bindings.iter().filter(|binding| binding.variable == *key).map(|binding| json!({"node": binding.node, "parameter": binding.property})).collect();
        entry.insert("bound_to".to_string(), json!(bound));
        listed.push(Value::Object(entry));
    }
    let listing = json!({"variables": listed});
    Ok(Step::Done(ToolResult::json(&listing)))
}

/// A variable's value as its editor in the Vars tab stores it.
fn variable_value(variable: &nerthus_engine::automation::Variable, given: &Value) -> Result<Value, String> {
    let name = &variable.name;
    let limit = |member: &str| variable.json.get(member).map(VariableValue::from_json).and_then(|value| value.to_single().ok()).map(|value| value as f64);
    let clamp = |number: f64| {
        let low = limit("Min").unwrap_or(f64::NEG_INFINITY);
        let high = limit("Max").unwrap_or(f64::INFINITY);
        if low <= high { number.clamp(low, high) } else { number }
    };
    match variable.var_type {
        VarType::Float => {
            let number = number_of(given).ok_or_else(|| format!("{name} takes a number."))?;
            Ok(f32_value(clamp(number) as f32))
        }
        VarType::Int => {
            let number = number_of(given).ok_or_else(|| format!("{name} takes a whole number."))?;
            Ok(json!(clamp(number).round() as i64))
        }
        VarType::Range => {
            let [low, high] = pair_of(given).ok_or_else(|| format!("{name} takes a range: [low, high]."))?;
            let range = VariableValue::Float2([clamp(low.min(high)) as f32, clamp(low.max(high)) as f32]);
            Ok(range.to_json())
        }
        VarType::Choice => {
            let options = choice_options(&variable.json);
            let by_name = given.as_str().and_then(|text| options.iter().position(|option| option.eq_ignore_ascii_case(text.trim())));
            let by_index = number_of(given).map(|number| number as usize).filter(|index| *index < options.len());
            let index = by_name.or(by_index).ok_or_else(|| format!("{name} takes one of: {}.", options.join(", ")))?;
            Ok(json!(index))
        }
        VarType::Bool => {
            let flag = boolean_of(given).ok_or_else(|| format!("{name} takes true or false."))?;
            Ok(json!(flag))
        }
        VarType::Color => {
            let [red, green, blue] = colour_of(given).ok_or_else(|| format!("{name} takes a colour: \"#rrggbb\" or [red, green, blue] (scRGB)."))?;
            Ok(json!({"R": red as f64, "G": green as f64, "B": blue as f64}))
        }
        VarType::Undefined => match given {
            Value::String(text) => Ok(json!(text)),
            _ => Err(format!("{name} takes text.")),
        },
        VarType::Decoration => Err(format!("{name} arranges the Vars tab and has no value.")),
    }
}

pub(crate) fn set_variable(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let editor = &mut *context.editor;
    let wanted = arguments.required_string("variable")?.trim();
    let given = arguments.value("value").ok_or("value is required.")?;
    let automation = editor.document.automation();
    let found = automation.variables.iter().find(|(key, variable)| key.eq_ignore_ascii_case(wanted) || variable.name.eq_ignore_ascii_case(wanted));
    let Some((key, variable)) = found else {
        let keys: Vec<&str> = automation.variables.keys().map(String::as_str).collect();
        return Err(format!("There is no variable {wanted}; the variables are {}.", keys.join(", ")));
    };
    let key = key.clone();
    let undefined = variable.var_type == VarType::Undefined;
    let value = variable_value(variable, given)?;
    if undefined {
        editor.document.set_variable_text(&key, value.as_str().unwrap_or_default());
    } else {
        editor.document.set_variable_value(&key, value.clone());
    }
    editor.changed();
    let outcome = json!({"variable": key, "value": value});
    Ok(Step::Done(ToolResult::json(&outcome)))
}
