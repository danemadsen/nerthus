//! Making many nodes at once: a whole graph from a list of nodes and
//! connections (`create_graph`), and the example projects Nerthus comes
//! with as starting points (`list_examples`, `open_example`). A graph is
//! planned (every type, parameter, modifier and export checked) before
//! anything changes, then made as one undoable edit.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use nerthus_engine::{Engine, NodeSpec, Project};
use nerthus_export::FileFormat;
use serde_json::{json, Map, Value};

use super::arguments::{find_in, pair_of, Arguments};
use super::build_tools::{format_key, format_named};
use super::catalogue_tools::{display_name, find_type};
use super::edit_tools::{choose_input, converted_parameters, placement, Feed, FEED_GAP, STACK_GAP, STANDARD_EXTENT};
use super::graph_tools::output_port;
use super::project_tools::{last_message, refuse_unsaved, save_path, save_to};
use super::tools::MODIFIERS;
use super::{values, Step, ToolContext, ToolResult};
use crate::document::{Document, BUILD_DEFINITION_MEMBERS};
use crate::presets::PresetLibrary;

/// Where a graph planned without a view (a check of a proposed graph)
/// starts.
pub(crate) const SCRATCH_ORIGIN: [f32; 2] = [26000.0, 26000.0];

/// A node a plan makes.
pub(crate) struct PlannedNode {
    /// What connections call it: its `id` in the plan, else its number
    /// in the list (from 1).
    pub key: String,
    pub spec: &'static NodeSpec,
    pub name: Option<String>,
    pub position: Option<[f32; 2]>,
    pub parameters: Vec<(String, Value)>,
    pub modifiers: Vec<(&'static str, Vec<(String, Value)>)>,
    pub export: Option<PlannedExport>,
    pub bypassed: bool,
}

/// A planned node's mark for export.
pub(crate) struct PlannedExport {
    pub file_name: Option<String>,
    pub format: Option<FileFormat>,
}

/// One end of a planned connection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum End {
    /// The plan's node at this index.
    New(usize),
    /// A node the project has already.
    Existing(i32),
}

pub(crate) struct PlannedLink {
    pub from: End,
    pub from_port: Option<String>,
    pub to: End,
    pub to_port: Option<String>,
}

/// A graph to make, checked.
#[derive(Default)]
pub(crate) struct GraphPlan {
    pub nodes: Vec<PlannedNode>,
    pub links: Vec<PlannedLink>,
    /// What changed on the way (values clamped or rounded).
    pub notes: Vec<String>,
}

/// What making a plan came to.
pub(crate) struct Applied {
    /// The new nodes' ids, in the plan's order.
    pub ids: Vec<i32>,
    pub links: Vec<Value>,
}

/// A file format by the name tools use (`png16`) or the name projects
/// save (`PNG16`, `UshortRaw16`).
pub(crate) fn parse_format(name: &str) -> Result<FileFormat, String> {
    format_named(name).or_else(|problem| FileFormat::from_name(name.trim()).ok_or(problem))
}

/// A key as connections give it: text, or a number as text.
fn key_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.trim().to_string()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// The first of `names` the object has.
fn member<'a>(object: &'a Map<String, Value>, names: &[&str]) -> Option<&'a Value> {
    names.iter().find_map(|name| object.get(*name)).filter(|value| !value.is_null())
}

/// A planned node's modifiers: `["Autolevel", {"modifier": "Height",
/// "parameters": {…}}]`, or `{"Height": {…}}`.
fn planned_modifiers(given: &Value, problems: &mut Vec<String>, label: &str) -> Vec<(&'static str, Vec<(String, Value)>)> {
    let mut entries: Vec<(String, Option<&Map<String, Value>>)> = Vec::new();
    match given {
        Value::Array(items) => {
            for item in items {
                match item {
                    Value::String(name) => entries.push((name.clone(), None)),
                    Value::Object(object) => {
                        let name = member(object, &["modifier", "type", "name"]).and_then(Value::as_str).unwrap_or_default().to_string();
                        let parameters = member(object, &["parameters", "properties"]).and_then(Value::as_object);
                        entries.push((name, parameters));
                    }
                    _ => problems.push(format!("{label}: a modifier is a name or {{\"modifier\": …, \"parameters\": {{…}}}}.")),
                }
            }
        }
        Value::Object(object) => {
            for (name, parameters) in object {
                entries.push((name.clone(), parameters.as_object()));
            }
        }
        _ => problems.push(format!("{label}: modifiers is a list or an object.")),
    }
    let catalogue = nerthus_engine::Catalogue::get();
    let mut planned = Vec::new();
    for (name, parameters) in entries {
        let Some(type_name) = MODIFIERS.iter().find(|candidate| candidate.eq_ignore_ascii_case(name.trim())) else {
            problems.push(format!("{label}: there is no {name} modifier; the modifiers are {}.", MODIFIERS.join(", ")));
            continue;
        };
        let Some(spec) = catalogue.modifier(type_name) else {
            continue;
        };
        let mut values_set = Vec::new();
        for (parameter_name, value) in parameters.into_iter().flatten() {
            let converted = values::find_modifier_parameter(spec, parameter_name).and_then(|parameter| values::convert(parameter, value, spec.defaults.get(&parameter.name)).map(|stored| (parameter.name.clone(), stored.value)));
            match converted {
                Ok(pair) => values_set.push(pair),
                Err(problem) => problems.push(format!("{label}: {problem}")),
            }
        }
        planned.push((*type_name, values_set));
    }
    planned
}

/// A planned node's export: `true`, or `{"file_name", "format",
/// "enabled"}` (`save_definition`'s `filename` too).
fn planned_export(given: &Value, problems: &mut Vec<String>, label: &str) -> Option<PlannedExport> {
    match given {
        Value::Bool(true) => Some(PlannedExport { file_name: None, format: None }),
        Value::Bool(false) | Value::Null => None,
        Value::Object(object) => {
            let enabled = member(object, &["enabled", "IsEnabled"]).and_then(super::arguments::boolean_of).unwrap_or(true);
            if !enabled {
                return None;
            }
            let file_name = member(object, &["file_name", "filename", "Filename"]).and_then(Value::as_str).map(str::trim).filter(|name| !name.is_empty()).map(str::to_string);
            let format = match member(object, &["format", "Format"]).and_then(Value::as_str) {
                Some(name) => match parse_format(name) {
                    Ok(format) => Some(format),
                    Err(problem) => {
                        problems.push(format!("{label}: {problem}"));
                        None
                    }
                },
                None => None,
            };
            Some(PlannedExport { file_name, format })
        }
        _ => {
            problems.push(format!("{label}: export is true or {{\"file_name\", \"format\"}}."));
            None
        }
    }
}

/// Plans the nodes and connections given, checking every one; the
/// problems all together when there are any. Connections may name the
/// project's own nodes when `existing` is given.
pub(crate) fn plan_graph(nodes: &[Value], connections: &[Value], existing: Option<&Document>) -> Result<GraphPlan, Vec<String>> {
    let mut plan = GraphPlan::default();
    let mut problems = Vec::new();
    for (index, item) in nodes.iter().enumerate() {
        let Some(object) = item.as_object() else {
            problems.push(format!("Node {}: a node is an object with a type.", index + 1));
            continue;
        };
        let key = member(object, &["id", "key"]).and_then(key_text).unwrap_or_else(|| (index + 1).to_string());
        let label = format!("Node {key}");
        let Some(type_name) = member(object, &["type", "node_type"]).and_then(Value::as_str) else {
            problems.push(format!("{label}: type is required."));
            continue;
        };
        let spec = match find_type(type_name) {
            Ok(spec) => spec,
            Err(problem) => {
                problems.push(format!("{label}: {problem}"));
                continue;
            }
        };
        let name = member(object, &["name"]).and_then(Value::as_str).map(str::trim).filter(|name| !name.is_empty()).map(str::to_string);
        let position = member(object, &["position"]).and_then(pair_of).map(|[x, y]| [x as f32, y as f32]);
        let mut parameters = Vec::new();
        if let Some(given) = member(object, &["parameters", "properties"]) {
            match given.as_object() {
                Some(given) => match converted_parameters(spec, given, None) {
                    Ok(converted) => {
                        parameters = converted.values;
                        plan.notes.extend(converted.notes.into_iter().map(|note| format!("{label}: {note}")));
                    }
                    Err(problem) => problems.push(format!("{label}: {problem}")),
                },
                None => problems.push(format!("{label}: parameters is an object of values by name.")),
            }
        }
        let modifiers = member(object, &["modifiers"]).map(|given| planned_modifiers(given, &mut problems, &label)).unwrap_or_default();
        let export = member(object, &["export", "save_definition"]).and_then(|given| planned_export(given, &mut problems, &label));
        let bypassed = member(object, &["bypassed"]).and_then(super::arguments::boolean_of).unwrap_or(false);
        if plan.nodes.iter().any(|node| node.key == key) {
            problems.push(format!("{label}: another node has the id {key}."));
        }
        plan.nodes.push(PlannedNode { key, spec, name, position, parameters, modifiers, export, bypassed });
    }

    let resolve = |value: &Value, plan: &GraphPlan| -> Result<End, String> {
        if let Some(text) = key_text(value)
            && let Some(index) = plan.nodes.iter().position(|node| node.key == text)
        {
            return Ok(End::New(index));
        }
        if let Some(text) = value.as_str() {
            let named: Vec<usize> = plan.nodes.iter().enumerate().filter(|(_, node)| node.name.as_deref().is_some_and(|name| name.eq_ignore_ascii_case(text.trim()))).map(|(index, _)| index).collect();
            if let [single] = named[..] {
                return Ok(End::New(single));
            }
        }
        match existing {
            Some(document) => find_in(document, value).map(End::Existing).map_err(|_| format!("{value} is neither a node of this graph nor of the project.")),
            None => Err(format!("{value} is not a node of this graph.")),
        }
    };
    for (index, item) in connections.iter().enumerate() {
        let label = format!("Connection {}", index + 1);
        let (from, from_port, to, to_port) = match item {
            Value::Object(object) => {
                let from = member(object, &["from", "from_node", "source"]);
                let to = member(object, &["to", "to_node", "target"]);
                let from_port = member(object, &["from_port", "source_port", "output"]).and_then(Value::as_str).map(str::to_string);
                let to_port = member(object, &["to_port", "target_port", "input"]).and_then(Value::as_str).map(str::to_string);
                (from, from_port, to, to_port)
            }
            Value::Array(items) if items.len() == 2 => (items.first(), None, items.get(1), None),
            _ => (None, None, None, None),
        };
        let (Some(from), Some(to)) = (from, to) else {
            problems.push(format!("{label}: a connection is {{\"from\", \"to\"}} (and from_port, to_port), or [from, to]."));
            continue;
        };
        match (resolve(from, &plan), resolve(to, &plan)) {
            (Ok(from), Ok(to)) if from == to => problems.push(format!("{label}: a node cannot feed itself.")),
            (Ok(from), Ok(to)) => plan.links.push(PlannedLink { from, from_port, to, to_port }),
            (Err(problem), _) | (_, Err(problem)) => problems.push(format!("{label}: {problem}")),
        }
    }
    if problems.is_empty() { Ok(plan) } else { Err(problems) }
}

/// Positions for the planned nodes without one: in columns by how far
/// down the plan's connections each is, from `origin`.
fn layout(plan: &GraphPlan, origin: [f32; 2]) -> Vec<[f32; 2]> {
    let count = plan.nodes.len();
    let mut depth = vec![0usize; count];
    // Longest path from the plan's own sources; a loop stops at `count`.
    for _ in 0..count {
        let mut changed = false;
        for link in &plan.links {
            if let (End::New(from), End::New(to)) = (link.from, link.to)
                && depth[to] < depth[from] + 1
                && depth[from] + 1 < count.max(1)
            {
                depth[to] = depth[from] + 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut rows: HashMap<usize, usize> = HashMap::new();
    let mut positions = Vec::with_capacity(count);
    for (index, node) in plan.nodes.iter().enumerate() {
        if let Some(position) = node.position {
            positions.push(position);
            continue;
        }
        let row = rows.entry(depth[index]).or_default();
        let x = origin[0] + depth[index] as f32 * (STANDARD_EXTENT[0] + FEED_GAP);
        let y = origin[1] + *row as f32 * (STANDARD_EXTENT[1] + STACK_GAP * 2.0);
        *row += 1;
        positions.push([x, y]);
    }
    positions
}

/// Makes the plan's nodes on `tab` and connects them, as part of an edit
/// already under way; what could not be made when anything fails (the
/// caller then takes the edit back).
pub(crate) fn apply_plan(document: &mut Document, presets: Option<&PresetLibrary>, plan: &GraphPlan, tab: usize, origin: [f32; 2]) -> Result<Applied, Vec<String>> {
    let positions = layout(plan, origin);
    let mut ids = Vec::with_capacity(plan.nodes.len());
    let mut problems = Vec::new();
    for (node, position) in plan.nodes.iter().zip(positions) {
        let Some(id) = document.place_node(node.spec.key(), position, tab) else {
            problems.push(format!("Node {}: {} could not be added.", node.key, display_name(node.spec)));
            continue;
        };
        ids.push(id);
        if let Some(preset) = presets.and_then(|library| library.default_for(&node.spec.name)) {
            let preset_values = crate::presets::preset_values(preset, node.spec);
            document.set_values_quietly(id, &preset_values);
        }
        if !node.parameters.is_empty() {
            document.set_values_quietly(id, &node.parameters);
        }
        if let Some(data) = document.project.terrain_mut().nodes.get_mut(&id) {
            if let Some(name) = &node.name {
                data.name = name.clone();
            }
            if node.bypassed {
                data.set_member("StateOverride", json!("Bypassed"));
            }
        }
        for (modifier, modifier_values) in &node.modifiers {
            document.toggle_modifier(id, modifier);
            let index = document.node(id).and_then(|data| data.modifiers.iter().position(|added| nerthus_engine::editing::modifier_name(added) == *modifier));
            if let Some(index) = index {
                for (name, value) in modifier_values {
                    document.set_modifier_value(id, index, name, value.clone());
                }
            }
        }
        if let Some(export) = &node.export {
            let format = export.format.unwrap_or(FileFormat::Png16);
            document.toggle_export(&[id], format);
            if let Some(file_name) = &export.file_name {
                document.edit_export(id, "Export settings", |settings| settings.filename = file_name.clone());
            }
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }

    let id_of = |end: End| match end {
        End::New(index) => ids[index],
        End::Existing(id) => id,
    };
    let mut links = Vec::new();
    for (index, link) in plan.links.iter().enumerate() {
        let label = format!("Connection {}", index + 1);
        let (from, to) = (id_of(link.from), id_of(link.to));
        let (Some(from_node), Some(to_node)) = (document.node(from), document.node(to)) else {
            problems.push(format!("{label}: a node it joins is gone."));
            continue;
        };
        let from_port = match output_port(from_node, link.from_port.as_deref()) {
            Ok(port) => port,
            Err(problem) => {
                problems.push(format!("{label}: {problem}"));
                continue;
            }
        };
        if document.would_cycle(from, to) {
            problems.push(format!("{label}: {} feeding {} would make a loop.", from_node.name, to_node.name));
            continue;
        }
        let to_name = to_node.name.clone();
        let to_port = match choose_input(to_node, link.to_port.as_deref()) {
            Ok(Feed::Existing(port)) => port,
            Ok(Feed::AddedMask) => {
                document.add_mask_port(to);
                "Mask".to_string()
            }
            Ok(Feed::AddedPort) => match document.add_new_input(to) {
                Some(port) => port,
                None => {
                    problems.push(format!("{label}: no input could be added to {to_name}."));
                    continue;
                }
            },
            Err(problem) => {
                problems.push(format!("{label}: {problem}"));
                continue;
            }
        };
        document.link(from, &from_port, to, &to_port);
        links.push(json!({"from": {"node": from, "port": from_port}, "to": {"node": to, "port": to_port}}));
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    Ok(Applied { ids, links })
}

/// The node a new graph ends in, to show: the last marked for export,
/// else the last that feeds none of the others.
fn result_node(plan: &GraphPlan, ids: &[i32]) -> Option<i32> {
    let exported = plan.nodes.iter().zip(ids).filter(|(node, _)| node.export.is_some()).map(|(_, id)| *id).next_back();
    let feeds_nothing = |index: usize| !plan.links.iter().any(|link| link.from == End::New(index));
    exported.or_else(|| (0..ids.len()).rev().find(|index| feeds_nothing(*index)).map(|index| ids[index]))
}

/// Build settings by member name, checked: `(member, value)`.
pub(crate) fn build_members(given: Option<&Map<String, Value>>) -> Result<Vec<(&'static str, Value)>, String> {
    let mut members = Vec::new();
    for (name, value) in given.into_iter().flatten() {
        let member = BUILD_DEFINITION_MEMBERS.iter().find(|member| member.eq_ignore_ascii_case(name));
        let member = member.ok_or_else(|| format!("There is no build setting {name}; they are {}.", BUILD_DEFINITION_MEMBERS.join(", ")))?;
        members.push((*member, value.clone()));
    }
    Ok(members)
}

fn problem_list(problems: Vec<String>) -> String {
    format!("Nothing was made:\n- {}", problems.join("\n- "))
}

pub(crate) fn create_graph(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let nodes = arguments.array("nodes")?.ok_or("nodes is required.")?;
    if nodes.is_empty() {
        return Err("nodes names no node.".to_string());
    }
    let empty = Vec::new();
    let connections = arguments.array("connections")?.unwrap_or(&empty);
    let new_project = arguments.boolean("new_project")?.unwrap_or(false);
    let settings = build_members(arguments.object("build_settings")?)?;
    let save = arguments.string("save_path")?;
    let existing = (!new_project).then_some(&context.editor.document);
    let plan = plan_graph(nodes, connections, existing).map_err(problem_list)?;
    if new_project {
        refuse_unsaved(context.editor, arguments)?;
    }
    let save = match save {
        Some(text) => Some(save_path(context.editor, Some(text))?),
        None => None,
    };
    if new_project {
        crate::files::new_project(context.editor);
        context.ui_state.graph.tab = 0;
    }
    let editor = &mut *context.editor;
    let tab = match arguments.integer("tab")? {
        Some(tab) if !new_project => tab.max(0) as usize,
        _ => context.ui_state.graph.tab,
    };
    if !editor.document.graph_tabs().iter().any(|info| info.index == tab) {
        return Err(format!("There is no graph tab {tab}; get_project lists the tabs."));
    }
    let origin = placement(editor, context.ui_state, tab, None, i32::MIN);

    let mark = editor.document.undo_mark();
    let label = format!("Create {} nodes", plan.nodes.len());
    editor.document.checkpoint(label.clone());
    let applied = apply_plan(&mut editor.document, Some(&editor.presets), &plan, tab, origin);
    let applied = match applied {
        Ok(applied) => applied,
        Err(problems) => {
            editor.document.abandon_edits(mark);
            return Err(problem_list(problems));
        }
    };
    if !settings.is_empty() {
        editor.document.set_build_settings(&settings);
    }
    editor.document.squash_undo(mark, label);
    editor.document.apply_node_display(&applied.ids, &editor.user.options);
    let placed: Vec<(i32, [f32; 2], usize)> = applied.ids.iter().filter_map(|id| editor.document.node(*id).map(|node| (*id, node.position, tab))).collect();
    editor.document.settle_in_groups(&placed);
    context.ui_state.graph.clear_element_selection();
    if let Some(shown) = result_node(&plan, &applied.ids) {
        editor.select(Some(shown));
    }
    editor.changed();

    let created: Vec<Value> = plan
        .nodes
        .iter()
        .zip(&applied.ids)
        .map(|(node, id)| {
            let name = editor.document.node(*id).map(|data| data.name.clone());
            json!({"key": node.key, "id": id, "name": name, "type": node.spec.key()})
        })
        .collect();
    let mut outcome = json!({"created": created, "connections": applied.links});
    if !plan.notes.is_empty() {
        outcome["notes"] = json!(plan.notes);
    }
    if let Some(path) = save {
        save_to(editor, &path)?;
        outcome["saved"] = json!(path);
    }
    Ok(Step::Done(ToolResult::json(&outcome)))
}

/// The example projects: the files in the examples folder (the ones
/// Nerthus comes with, written there for the Examples window), by name.
pub(crate) fn example_files() -> Vec<PathBuf> {
    let folder = crate::ui::file_windows::examples_folder().or_else(|| {
        // Without a data folder, a copy in the temporary folder.
        let folder = std::env::temp_dir().join("nerthus-examples");
        std::fs::create_dir_all(&folder).ok()?;
        for (name, bytes) in crate::ui::file_windows::BUNDLED_EXAMPLES {
            let _ = std::fs::write(folder.join(name), bytes);
        }
        Some(folder)
    });
    let Some(entries) = folder.and_then(|folder| std::fs::read_dir(folder).ok()) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).filter(|path| path.extension().is_some_and(|extension| extension == "terrain")).collect();
    files.sort();
    files
}

fn example_name(path: &Path) -> String {
    path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default()
}

/// An example by name: exactly (any case), else the one whose name holds
/// the words.
fn find_example(wanted: &str) -> Result<PathBuf, String> {
    let files = example_files();
    let squeeze = |text: &str| text.chars().filter(|character| character.is_alphanumeric()).collect::<String>().to_lowercase();
    let wanted_text = squeeze(wanted.trim().trim_end_matches(".terrain"));
    let exact = files.iter().find(|path| squeeze(&example_name(path)) == wanted_text);
    let partial = || {
        let matching: Vec<&PathBuf> = files.iter().filter(|path| !wanted_text.is_empty() && squeeze(&example_name(path)).contains(&wanted_text)).collect();
        if let [single] = matching[..] { Some(single) } else { None }
    };
    exact.or_else(partial).cloned().ok_or_else(|| {
        let names: Vec<String> = files.iter().map(|path| example_name(path)).collect();
        format!("There is no example \"{wanted}\"; the examples are {}.", names.join(", "))
    })
}

/// A project's node types in the order its builds take them: from what
/// starts the graph to what it ends in.
pub(crate) fn types_in_build_order(project: &Project) -> Vec<String> {
    let terrain = project.terrain();
    let mut fed: Vec<i32> = Vec::new();
    for node in terrain.nodes.values() {
        for port in &node.ports {
            if let Some(record) = &port.record {
                fed.push(record.from);
            }
        }
    }
    let mut types = Vec::new();
    let ends = terrain.nodes.values().filter(|node| !fed.contains(&node.id));
    for end in ends {
        for id in Engine::build_order(terrain, end.id) {
            if let Some(node) = terrain.nodes.get(&id)
                && !types.contains(&node.type_name)
            {
                types.push(node.type_name.clone());
            }
        }
    }
    types
}

pub(crate) fn list_examples(_arguments: &Arguments) -> Result<Step, String> {
    let mut listed = Vec::new();
    for path in example_files() {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(project) = Project::parse(text.trim_start_matches('\u{feff}')) else {
            continue;
        };
        let terrain = project.terrain();
        let exported: Vec<&str> = nerthus_export::build::exporting_nodes(terrain).iter().filter_map(|id| terrain.nodes.get(id)).map(|node| node.name.as_str()).collect();
        let connections: usize = terrain.nodes.values().map(|node| node.ports.iter().filter(|port| port.record.is_some()).count()).sum();
        listed.push(json!({
            "name": example_name(&path),
            "nodes": terrain.nodes.len(),
            "connections": connections,
            "types": types_in_build_order(&project),
            "exports": exported,
        }));
    }
    let listing = json!({"examples": listed, "note": "open_example opens one as an untitled copy to start from."});
    Ok(Step::Done(ToolResult::json(&listing)))
}

/// Each node's new values, and notes on what changed on the way.
type NodeChanges = Vec<(i32, Vec<(String, Value)>, Vec<String>)>;

/// Parameter changes by node (`{"node id or name": {"Parameter": value}}`)
/// in `document`, each checked.
pub(crate) fn node_overrides(document: &Document, given: &Map<String, Value>) -> Result<NodeChanges, String> {
    let mut changes = Vec::new();
    for (node, parameters) in given {
        let id = find_in(document, &Value::String(node.clone()))?;
        let data = document.node(id).ok_or("The node is gone.")?;
        let spec = nerthus_engine::Catalogue::get().node(&data.type_name).ok_or("The node's type is unknown.")?;
        let parameters = parameters.as_object().ok_or_else(|| format!("The parameters for {node} are an object of values by name."))?;
        let converted = converted_parameters(spec, parameters, Some(data))?;
        changes.push((id, converted.values, converted.notes));
    }
    Ok(changes)
}

pub(crate) fn open_example(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let path = find_example(arguments.required_string("name")?)?;
    let overrides = arguments.object("parameters")?;
    refuse_unsaved(context.editor, arguments)?;
    let save = match arguments.string("save_path")? {
        Some(text) => Some(save_path(context.editor, Some(text))?),
        None => None,
    };
    let editor = &mut *context.editor;
    if !crate::files::open_example(editor, &path) {
        return Err(last_message(editor));
    }
    let mut notes = Vec::new();
    if let Some(given) = overrides {
        // The example stays open whether or not its changes are good.
        let changes = node_overrides(&editor.document, given)?;
        editor.document.checkpoint("Example settings");
        for (id, values, change_notes) in changes {
            editor.document.set_values_quietly(id, &values);
            notes.extend(change_notes);
        }
    }
    editor.changed();
    let terrain = editor.document.terrain();
    let nodes: Vec<Value> = terrain.nodes.values().map(|node| json!({"id": node.id, "name": node.name, "type": node.type_name})).collect();
    let exports: Vec<Value> = nerthus_export::build::exporting_nodes(terrain)
        .iter()
        .filter_map(|id| editor.document.export_settings(*id).map(|settings| json!({"node": id, "file_name": settings.filename, "format": format_key(settings.format)})))
        .collect();
    let mut outcome = json!({"opened": example_name(&path), "untitled": true, "nodes": nodes, "exports": exports});
    if !notes.is_empty() {
        outcome["notes"] = json!(notes);
    }
    if let Some(path) = save {
        save_to(editor, &path)?;
        outcome["saved"] = json!(path);
        outcome["untitled"] = json!(false);
    }
    Ok(Step::Done(ToolResult::json(&outcome)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(nodes: Value, connections: Value) -> Result<GraphPlan, Vec<String>> {
        plan_graph(nodes.as_array().unwrap(), connections.as_array().unwrap(), None)
    }

    #[test]
    fn a_plan_checks_everything_first() {
        let planned = plan(
            json!([{"id": 1, "type": "Mountain", "parameters": {"Style": "Nonsense"}}, {"id": 2, "type": "NoSuchNode"}, {"id": 3, "type": "ErosionPro", "modifiers": ["Unheard"]}]),
            json!([{"from": 1, "to": 9}]),
        );
        let Err(problems) = planned else {
            panic!("the plan should not pass");
        };
        assert_eq!(problems.len(), 4, "{problems:?}");
    }

    #[test]
    fn a_plan_becomes_one_edit() {
        let Ok(plan) = plan(
            json!([
                {"id": 100, "type": "Mountain", "name": "Base", "properties": {"Scale": 0.6}},
                {"id": 101, "type": "Erosion2", "modifiers": {"Height": {"Falloff": 0.3}}},
                {"id": 102, "type": "SatMap", "export": {"filename": "colour", "format": "PNG8"}}
            ]),
            json!([{"from_node": 100, "to_node": 101}, [101, 102]]),
        ) else {
            panic!("the plan should pass");
        };
        let mut document = Document::new();
        let mark = document.undo_mark();
        document.checkpoint("Create");
        let Ok(applied) = apply_plan(&mut document, None, &plan, 0, SCRATCH_ORIGIN) else {
            panic!("the plan should apply");
        };
        assert!(document.squash_undo(mark, "Create 3 nodes"));
        assert_eq!(document.undo_labels(), ["Create 3 nodes"]);
        assert_eq!(applied.ids.len(), 3);
        assert_eq!(applied.links.len(), 2);
        let erosion = document.node(applied.ids[1]).unwrap();
        assert_eq!(erosion.port("In").unwrap().record.as_ref().unwrap().from, applied.ids[0]);
        assert_eq!(erosion.modifiers.len(), 1);
        let colour = document.export_settings(applied.ids[2]).unwrap();
        assert_eq!(colour.filename, "colour");
        assert_eq!(colour.format, FileFormat::Png8);
        assert_eq!(document.node(applied.ids[0]).unwrap().name, "Base");
        // Laid out left to right along the connections.
        assert!(document.node(applied.ids[2]).unwrap().position[0] > document.node(applied.ids[0]).unwrap().position[0]);
        // Undone, the document is empty again.
        assert!(document.undo());
        assert!(document.terrain().nodes.is_empty());
    }

    #[test]
    fn a_failed_plan_leaves_nothing() {
        // Perlin has no output named Nope.
        let Ok(plan) = plan(json!([{"type": "Perlin"}, {"type": "Blur"}]), json!([{"from": 1, "from_port": "Nope", "to": 2}])) else {
            panic!("the plan should pass");
        };
        let mut document = Document::new();
        let mark = document.undo_mark();
        document.checkpoint("Create");
        let Err(problems) = apply_plan(&mut document, None, &plan, 0, SCRATCH_ORIGIN) else {
            panic!("the plan should fail");
        };
        assert_eq!(problems.len(), 1);
        document.abandon_edits(mark);
        assert!(document.terrain().nodes.is_empty());
        assert!(!document.can_undo() && !document.can_redo());
    }

    #[test]
    fn the_examples_are_listed_in_build_order() {
        let (_, project) = &super::super::knowledge::bundled_projects()[0];
        let types = types_in_build_order(project);
        assert!(!types.is_empty());
        assert!(parse_format("UshortRaw16").is_ok());
        assert!(parse_format("png16").is_ok());
        assert!(parse_format("bmp").is_err());
    }
}
