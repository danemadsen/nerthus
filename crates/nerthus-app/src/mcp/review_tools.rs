//! Looking a graph over: what stops it building or makes it build nothing
//! useful (`validate_graph`, which can fix what is safe to fix), how it
//! is put together and what a terrain of its kind usually adds
//! (`analyze_graph`), and its processing quality set for speed or for
//! quality (`optimize_parameters`).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use nerthus_engine::{Catalogue, Engine, NodeData, NodeSpec, StateOverride, Terrain, ValueType};
use serde_json::{json, Value};

use super::arguments::Arguments;
use super::authoring_tools::{apply_plan, plan_graph, SCRATCH_ORIGIN};
use super::knowledge::{self, READS_NODE_INPUT};
use super::{values, Step, ToolContext, ToolResult};
use crate::document::Document;
use crate::editor::Editor;

/// What a look over a project found.
#[derive(Default)]
pub(crate) struct Findings {
    /// What stops nodes building, or the build starting.
    pub errors: Vec<String>,
    /// What builds, but likely not as meant.
    pub warnings: Vec<String>,
    /// Worth knowing.
    pub notes: Vec<String>,
    /// What `fix` would change.
    pub fixable: Vec<String>,
}

impl Findings {
    pub fn to_json(&self) -> Value {
        json!({
            "valid": self.errors.is_empty(),
            "errors": self.errors,
            "warnings": self.warnings,
            "notes": self.notes,
        })
    }
}

/// A node as findings name it.
fn called(node: &NodeData) -> String {
    format!("{} ({})", node.name, node.id)
}

/// Whether two numbers are the same value, single precision aside.
fn same_number(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * a.abs().max(b.abs()).max(1.0)
}

/// A stored value as the property panel would store it, when that
/// differs (clamped into range, an enum's number made its name); an
/// error when it cannot be stored at all.
pub(crate) fn corrected_value(parameter: &nerthus_engine::ParameterSpec, value: &Value) -> Result<Option<Value>, String> {
    if values::settable(parameter).is_err() {
        return Ok(None);
    }
    let checked = matches!(parameter.value_type, ValueType::Float | ValueType::Double | ValueType::Int | ValueType::Enum | ValueType::Bool | ValueType::Float2 | ValueType::Int2);
    if !checked || value.is_null() {
        return Ok(None);
    }
    // A pair's members at zero are left out of files (`{"Y": 0.25}`).
    let filled;
    let value = match (parameter.value_type, value) {
        (ValueType::Float2 | ValueType::Int2, Value::Object(members)) => {
            let member = |name: &str| members.get(name).cloned().unwrap_or(json!(0));
            filled = json!({"X": member("X"), "Y": member("Y")});
            &filled
        }
        _ => value,
    };
    let converted = values::convert(parameter, value, Some(value))?;
    let same = match (&converted.value, value) {
        (Value::Number(a), Value::Number(b)) => same_number(a.as_f64().unwrap_or(0.0), b.as_f64().unwrap_or(0.0)),
        (Value::Object(a), Value::Object(b)) => ["X", "Y"].iter().all(|member| match (a.get(*member).and_then(Value::as_f64), b.get(*member).and_then(Value::as_f64)) {
            (Some(a), Some(b)) => same_number(a, b),
            _ => true,
        }),
        (a, b) => a == b,
    };
    Ok((!same).then_some(converted.value))
}

/// Whether following inputs from any node comes back to it.
fn has_cycle(terrain: &Terrain) -> Option<i32> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Visiting,
        Done,
    }
    fn visit(id: i32, terrain: &Terrain, marks: &mut HashMap<i32, Mark>) -> Option<i32> {
        match marks.get(&id) {
            Some(Mark::Visiting) => return Some(id),
            Some(Mark::Done) => return None,
            None => {}
        }
        marks.insert(id, Mark::Visiting);
        if let Some(node) = terrain.nodes.get(&id) {
            for record in node.ports.iter().filter_map(|port| port.record.as_ref()) {
                if let Some(found) = visit(record.from, terrain, marks) {
                    return Some(found);
                }
            }
        }
        marks.insert(id, Mark::Done);
        None
    }
    let mut marks = HashMap::new();
    terrain.nodes.keys().find_map(|id| visit(*id, terrain, &mut marks))
}

/// The nodes each node's outputs feed.
fn fed_nodes(terrain: &Terrain) -> HashMap<i32, Vec<i32>> {
    let mut feeds: HashMap<i32, Vec<i32>> = HashMap::new();
    for node in terrain.nodes.values() {
        for record in node.ports.iter().filter_map(|port| port.record.as_ref()) {
            feeds.entry(record.from).or_default().push(node.id);
        }
    }
    feeds
}

/// The ports a node of its type has that it lacks (a file from another
/// version, or written by hand).
pub(crate) fn missing_ports(node: &NodeData, spec: &NodeSpec) -> Vec<(String, String)> {
    let fresh = NodeData::new(spec, 0, 0);
    fresh.ports.iter().filter(|port| node.port(&port.name).is_none()).map(|port| (port.name.clone(), port.port_type.clone())).collect()
}

/// Looks a project over.
pub(crate) fn check_document(document: &Document, strict: bool) -> Findings {
    let mut findings = Findings::default();
    let catalogue = Catalogue::get();
    let terrain = document.terrain();
    let asset = &document.project.assets[0];
    if !document.project.unknown_types.is_empty() {
        let types: Vec<&str> = document.project.unknown_types.iter().map(|name| name.split(',').next().unwrap_or(name).rsplit('.').next().unwrap_or(name)).collect();
        findings.errors.push(format!("Nodes of types Nerthus does not know were left out when the file was read: {}.", types.join(", ")));
    }
    let feeds = fed_nodes(terrain);
    let exporting = nerthus_export::build::exporting_nodes(terrain);

    for node in terrain.nodes.values() {
        let who = called(node);
        let Some(spec) = catalogue.node(&node.type_name) else {
            findings.errors.push(format!("{who} is of the unknown type {}.", node.type_name));
            continue;
        };
        // Its inputs.
        for port in node.ports.iter().filter(|port| port.is_input()) {
            match &port.record {
                None if port.is_required() => findings.errors.push(format!("{who}: its input {} is required and not connected, so the node cannot build.", port.name)),
                None => {}
                Some(record) => match terrain.nodes.get(&record.from) {
                    None => findings.errors.push(format!("{who}: its input {} is fed from node {}, which is not in the project.", port.name, record.from)),
                    Some(source) => {
                        let has_output = source.ports.iter().any(|candidate| candidate.is_output() && candidate.name == record.from_port);
                        if !has_output {
                            findings.errors.push(format!("{who}: its input {} is fed from {}'s output {}, which that node does not have.", port.name, source.name, record.from_port));
                        }
                    }
                },
            }
        }
        let missing = missing_ports(node, spec);
        if !missing.is_empty() {
            let names: Vec<&str> = missing.iter().map(|(name, _)| name.as_str()).collect();
            findings.warnings.push(format!("{who} lacks the ports its type has: {}.", names.join(", ")));
            findings.fixable.push(format!("{who}: add the ports {}", names.join(", ")));
        }

        // Its parameters.
        let rules = crate::node_ui::node_ui(node, &terrain.nodes, &asset.automation);
        for parameter in &spec.parameters {
            let Some(value) = node.values.get(&parameter.name) else {
                continue;
            };
            // A type's own default stands, whatever the slider's range.
            let is_default = spec.defaults.get(&parameter.name) == Some(value);
            let checked = if is_default { Ok(None) } else { corrected_value(parameter, value) };
            match checked {
                Err(problem) => findings.errors.push(format!("{who}: {problem} (it has {value}).")),
                Ok(Some(corrected)) => {
                    findings.warnings.push(format!("{who}: {} is {value}, which the property panel would store as {corrected}.", parameter.name));
                    findings.fixable.push(format!("{who}: {} = {corrected}", parameter.name));
                }
                Ok(None) => {}
            }
            let changed = spec.defaults.get(&parameter.name).is_some_and(|default| default != value);
            let hidden = !(parameter.visible && rules.visible(&parameter.name)) && !parameter.hidden;
            if changed && hidden && values::settable(parameter).is_ok() && !parameter.is_seed() {
                findings.notes.push(format!("{who}: {} is set but the property panel hides it as the node is set up, so it likely has no effect.", parameter.name));
            }
        }

        // Its modifiers.
        let fed = node.port("In").is_some_and(|port| port.record.is_some());
        for modifier in &node.modifiers {
            let name = nerthus_engine::editing::modifier_name(modifier);
            if catalogue.modifier(&modifier.type_name).is_none() {
                findings.warnings.push(format!("{who}: the {name} modifier is not one Nerthus knows; it does nothing."));
                continue;
            }
            if READS_NODE_INPUT.contains(&modifier.type_name.as_str()) && !fed {
                let fails = matches!(modifier.type_name.as_str(), "Difference" | "MatchHeight");
                let text = format!("{who}: the {name} modifier works off the node's input, but nothing feeds it.");
                if fails { findings.errors.push(format!("{text} The node fails to build.")) } else { findings.warnings.push(format!("{text} It makes an empty or unchanged result.")) }
            }
        }

        match StateOverride::of(node) {
            StateOverride::Bypassed => findings.notes.push(format!("{who} is bypassed: its input passes through unchanged.")),
            StateOverride::Baked => findings.notes.push(format!("{who} is baked: builds read its baked maps.")),
            StateOverride::None => {}
        }
        if spec.has_flag("RequiresBaking") {
            findings.notes.push(format!("{who} is a {}, which must be baked before a Hybrid or Tiled build.", node.type_name));
        }
        let feeds_nothing = feeds.get(&node.id).is_none_or(Vec::is_empty);
        let takes_nothing = !node.ports.iter().any(|port| port.record.is_some());
        if strict && terrain.nodes.len() > 1 && feeds_nothing && takes_nothing {
            findings.errors.push(format!("{who} is connected to nothing."));
        } else if strict && feeds_nothing && !exporting.contains(&node.id) && !exporting.is_empty() {
            findings.warnings.push(format!("{who} feeds nothing and is not marked for export, so builds leave it out."));
        }
    }

    if let Some(node) = has_cycle(terrain) {
        findings.errors.push(format!("The graph loops back on itself through node {node}; it cannot build."));
    }
    if exporting.is_empty() {
        let text = "No node is marked for export, so Build and Export has nothing to write (mark_for_export marks nodes).".to_string();
        if strict { findings.errors.push(text) } else { findings.warnings.push(text) }
    } else {
        let definition = document.build_definition();
        if let Err(refusal) = nerthus_export::build::check(terrain, definition.build_type) {
            findings.errors.push(format!("Build and Export would refuse: {refusal}"));
        }
        if document.has_duplicate_export_names() {
            findings.warnings.push("Two nodes marked for export write the same file name; one overwrites the other.".to_string());
            findings.fixable.push("rename duplicate export file names".to_string());
        }
        // Exports that a bypassed node stands in front of.
        for id in &exporting {
            let bypassed: Vec<String> = Engine::build_order(terrain, *id).into_iter().filter_map(|upstream| terrain.nodes.get(&upstream)).filter(|upstream| StateOverride::of(upstream) == StateOverride::Bypassed).map(called).collect();
            if !bypassed.is_empty() && let Some(node) = terrain.nodes.get(id) {
                findings.warnings.push(format!("{} is exported with bypassed nodes upstream: {}.", called(node), bypassed.join(", ")));
            }
        }
    }
    let has_type = asset.extra.get("BuildDefinition").and_then(|definition| definition.get("Type")).is_some();
    if !has_type {
        findings.notes.push("The build settings name no build type; Nerthus builds Standard, but Gaea's command-line build writes nothing for such a file.".to_string());
        findings.fixable.push("set the build type to Standard".to_string());
    }
    for variable in document.orphan_variables() {
        findings.notes.push(format!("No parameter is bound to the variable {variable}."));
    }
    findings
}

/// Fixes what is safe to fix, in the edit under way: values stored as the
/// property panel stores them, missing ports added, duplicate export
/// names renamed, the build type named. What it changed.
pub(crate) fn fix_document(document: &mut Document) -> Vec<String> {
    let catalogue = Catalogue::get();
    let mut fixed = Vec::new();
    let ids: Vec<i32> = document.terrain().nodes.keys().copied().collect();
    for id in ids {
        let Some(node) = document.node(id) else {
            continue;
        };
        let Some(spec) = catalogue.node(&node.type_name) else {
            continue;
        };
        let who = called(node);
        let mut corrected = Vec::new();
        for parameter in &spec.parameters {
            if let Some(value) = node.values.get(&parameter.name)
                && spec.defaults.get(&parameter.name) != Some(value)
                && let Ok(Some(better)) = corrected_value(parameter, value)
            {
                fixed.push(format!("{who}: {} {value} → {better}", parameter.name));
                corrected.push((parameter.name.clone(), better));
            }
        }
        let missing = missing_ports(node, spec);
        if !corrected.is_empty() {
            document.set_values_quietly(id, &corrected);
        }
        if !missing.is_empty() {
            let names: Vec<&str> = missing.iter().map(|(name, _)| name.as_str()).collect();
            fixed.push(format!("{who}: added the ports {}", names.join(", ")));
            if let Some(data) = document.project.terrain_mut().nodes.get_mut(&id) {
                for (name, port_type) in &missing {
                    data.add_port(name, port_type);
                }
            }
            document.mark_stale(id);
        }
    }
    if document.has_duplicate_export_names() {
        document.fix_duplicate_export_names();
        fixed.push("renamed duplicate export file names".to_string());
    }
    let asset = &mut document.project.assets[0];
    let definition = asset.extra.entry("BuildDefinition").or_insert_with(|| json!({}));
    if let Some(members) = definition.as_object_mut()
        && !members.contains_key("Type")
    {
        members.insert("Type".to_string(), json!("Standard"));
        document.dirty = true;
        fixed.push("named the build type: Standard".to_string());
    }
    fixed
}

/// What to look over.
enum Source {
    Open,
    File(PathBuf),
    Proposed,
}

pub(crate) fn validate_graph(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let strict = arguments.boolean("strict")?.unwrap_or(false);
    let fix = arguments.boolean("fix")?.unwrap_or(false);
    let runtime = arguments.boolean("runtime_check")?.unwrap_or(false);
    let path = arguments.string("path")?.map(|text| PathBuf::from(text.trim()));
    let nodes = arguments.array("nodes")?;
    let source = match (&path, nodes) {
        (Some(_), Some(_)) => return Err("Give path or nodes, not both.".to_string()),
        (Some(path), None) => Source::File(path.clone()),
        (None, Some(_)) => Source::Proposed,
        (None, None) => Source::Open,
    };
    if fix && !matches!(source, Source::Open) {
        return Err("fix works on the open project; repair_project_file fixes a file.".to_string());
    }

    let scratch;
    let (document, label): (&Document, String) = match &source {
        Source::Open => (&context.editor.document, "the open project".to_string()),
        Source::File(path) => {
            if path.is_relative() || !path.is_file() {
                return Err(format!("There is no file at {} (give an absolute path).", path.display()));
            }
            scratch = Document::open(path).map_err(|error| format!("{error:#}"))?;
            (&scratch, path.display().to_string())
        }
        Source::Proposed => {
            let empty = Vec::new();
            let connections = arguments.array("connections")?.unwrap_or(&empty);
            let proposed = plan_graph(nodes.unwrap_or(&empty), connections, None);
            let mut document = Document::new();
            let applied = proposed.and_then(|plan| apply_plan(&mut document, None, &plan, 0, SCRATCH_ORIGIN).map(|_| plan.notes));
            match applied {
                Ok(_) => {
                    scratch = document;
                    (&scratch, "the proposed graph".to_string())
                }
                Err(problems) => {
                    let outcome = json!({"checked": "the proposed graph", "valid": false, "errors": problems, "warnings": [], "notes": []});
                    return Ok(Step::Done(ToolResult::json(&outcome)));
                }
            }
        }
    };
    let findings = check_document(document, strict);
    let mut outcome = findings.to_json();
    outcome["checked"] = json!(label);
    outcome["nodes"] = json!(document.terrain().nodes.len());
    if !findings.fixable.is_empty() && !fix {
        outcome["fixable"] = json!(findings.fixable);
    }
    // A file or proposed graph is built as it is; the open project once fixed.
    let mut runtime_project = (runtime && !matches!(source, Source::Open)).then(|| nerthus_engine::to_terrain_json(&document.project));
    let runtime_path = match &source {
        Source::Open => context.editor.document.path.clone(),
        Source::File(path) => Some(path.clone()),
        Source::Proposed => None,
    };

    if fix {
        let editor = &mut *context.editor;
        let mark = editor.document.undo_mark();
        let mut fixed = Vec::new();
        if !findings.fixable.is_empty() {
            editor.document.checkpoint("Fix project");
            fixed = fix_document(&mut editor.document);
        }
        if fixed.is_empty() {
            editor.document.abandon_edits(mark);
        } else {
            editor.document.squash_undo(mark, "Fix project");
            editor.changed();
            let after = check_document(&editor.document, strict);
            outcome = after.to_json();
            outcome["checked"] = json!(label);
            outcome["nodes"] = json!(editor.document.terrain().nodes.len());
        }
        outcome["fixed"] = json!(fixed);
    }
    if runtime && matches!(source, Source::Open) {
        runtime_project = Some(nerthus_engine::to_terrain_json(&context.editor.document.project));
    }
    match runtime_project {
        Some(project) => super::run_tools::runtime_check(project, runtime_path, outcome, context),
        None => Ok(Step::Done(ToolResult::json(&outcome))),
    }
}

/// A kind of terrain and what it usually has: (what it adds, the types
/// any of which give it).
type TerrainKind = (&'static str, &'static [(&'static str, &'static [&'static str])]);

/// Node types a terrain of each kind usually has.
const TERRAIN_KINDS: [TerrainKind; 7] = [
    ("mountain", &[("snow on the peaks", &["Snow", "Snowfield", "Dusting"]), ("rock detail", &["Stones", "Rockscape", "RockNoise", "Craggy"])]),
    ("volcanic", &[("a volcano", &["Volcano"]), ("thermal weathering", &["Thermal", "Thermal2"]), ("rock strata", &["Stratify"])]),
    ("canyon", &[("a canyon", &["Canyon"]), ("rock strata", &["Stratify"]), ("terraced shelves", &["Terraces", "FractalTerraces"])]),
    ("coastal", &[("a coastline", &["Sea", "Lake"]), ("an island shape", &["Island"])]),
    ("arctic", &[("ice", &["Glacier", "IceFloe"]), ("snow", &["Snow", "Snowfield", "Dusting"])]),
    ("desert", &[("sand", &["DuneSea", "Sand"]), ("desert erosion", &["Sandstone"])]),
    ("river", &[("rivers", &["Rivers"]), ("deposits and water", &["Sediments", "Lake"])]),
];

/// The kinds of terrain analyze_graph knows.
pub(crate) fn terrain_kinds() -> Vec<&'static str> {
    let mut kinds: Vec<&str> = TERRAIN_KINDS.iter().map(|(kind, _)| *kind).collect();
    kinds.push("general");
    kinds
}

pub(crate) fn analyze_graph(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let kind = arguments.string("terrain_type")?.map(|kind| kind.trim().to_lowercase()).unwrap_or_else(|| "general".to_string());
    if !terrain_kinds().contains(&kind.as_str()) {
        return Err(format!("terrain_type is one of {}.", terrain_kinds().join(", ")));
    }
    let editor = &*context.editor;
    let document = &editor.document;
    let terrain = document.terrain();
    let catalogue = Catalogue::get();
    let spec_of = |node: &NodeData| catalogue.node(&node.type_name);
    let types: HashSet<&str> = terrain.nodes.values().map(|node| node.type_name.as_str()).collect();
    let in_category = |category: &str| terrain.nodes.values().filter(|node| spec_of(node).is_some_and(|spec| spec.toolbox.as_deref() == Some(category))).count();
    let in_family = |family: &str| terrain.nodes.values().filter(|node| spec_of(node).is_some_and(|spec| spec.family.as_deref() == Some(family))).count();

    let mut categories = serde_json::Map::new();
    for category in crate::ui::toolbox::CATEGORIES {
        let count = in_category(category);
        if count > 0 {
            categories.insert(category.to_string(), json!(count));
        }
    }
    let connections: usize = terrain.nodes.values().map(|node| node.ports.iter().filter(|port| port.record.is_some()).count()).sum();
    let feeds = fed_nodes(terrain);
    let ends: Vec<&NodeData> = terrain.nodes.values().filter(|node| feeds.get(&node.id).is_none_or(Vec::is_empty)).collect();
    let depth = ends.iter().map(|node| Engine::build_order(terrain, node.id).len()).max().unwrap_or(0);
    let exporting = nerthus_export::build::exporting_nodes(terrain);
    let mut used: HashSet<i32> = HashSet::new();
    for id in &exporting {
        used.extend(Engine::build_order(terrain, *id));
    }
    let unused: Vec<String> = if exporting.is_empty() { Vec::new() } else { terrain.nodes.values().filter(|node| !used.contains(&node.id)).map(called).collect() };
    let generators: Vec<String> = terrain.nodes.values().filter(|node| spec_of(node).is_some_and(knowledge::is_generator)).map(called).collect();

    // How it is put together.
    let erosion = in_family("Erosion");
    let colour = in_category("Colorize");
    let mut patterns = Vec::new();
    if !generators.is_empty() && erosion > 0 && colour > 0 {
        patterns.push("The classic pipeline: a generator, erosion, then colour.".to_string());
    } else if !generators.is_empty() && erosion > 0 {
        patterns.push("A generator shaped by erosion.".to_string());
    }
    if erosion > 1 {
        patterns.push(format!("{erosion} erosion stages, for layered detail."));
    }
    let data_maps = terrain.nodes.values().flat_map(|node| node.ports.iter()).filter_map(|port| port.record.as_ref()).filter(|record| record.from_port != "Out" && terrain.nodes.get(&record.from).and_then(spec_of).is_some_and(|spec| matches!(spec.toolbox.as_deref(), Some("Simulate")))).count();
    if data_maps > 0 {
        patterns.push(format!("{data_maps} connections take a simulation's data maps (flow, wear, deposits…)."));
    }
    let masked = terrain.nodes.values().filter(|node| node.port("Mask").is_some_and(|port| port.record.is_some())).count();
    let mask_modifiers = terrain.nodes.values().flat_map(|node| node.modifiers.iter()).filter(|modifier| matches!(modifier.type_name.as_str(), "Height" | "Slope")).count();
    if masked + mask_modifiers > 0 {
        patterns.push(format!("Masking: {masked} masked nodes, {mask_modifiers} height or slope modifiers."));
    }
    let combining = terrain.nodes.values().filter(|node| matches!(node.type_name.as_str(), "Combine" | "Mixer" | "Layers")).count();
    if combining > 0 {
        patterns.push(format!("{combining} nodes combine branches."));
    }
    if types.iter().any(|name| matches!(*name, "Snow" | "Snowfield" | "Glacier" | "Rivers" | "Lake" | "Sea" | "Sediments")) {
        patterns.push("Water, snow or ice simulation.".to_string());
    }
    let bindings = document.automation().bindings.len();
    if bindings > 0 {
        patterns.push(format!("{bindings} parameters are bound to variables."));
    }
    if types.contains("Macro") {
        patterns.push("Macros.".to_string());
    }

    // What it could use.
    let mut suggestions = Vec::new();
    if generators.is_empty() {
        suggestions.push("Start from a generator: Mountain, MountainRange, Perlin or another of the Terrain and Primitive types.".to_string());
    }
    if erosion == 0 && !generators.is_empty() {
        suggestions.push("Add erosion (ErosionPro or Erosion) for natural detail.".to_string());
    }
    if colour == 0 {
        suggestions.push("Add a colour node (SatelliteMap, CLUTer or SuperColor) for a texture.".to_string());
    }
    if exporting.is_empty() {
        suggestions.push("Mark the nodes to write for Build and Export (mark_for_export).".to_string());
    }
    if !unused.is_empty() {
        suggestions.push("Some nodes feed no export, so builds leave them out (see unused).".to_string());
    }
    if let Some((_, wanted)) = TERRAIN_KINDS.iter().find(|(name, _)| *name == kind) {
        for (adds, options) in *wanted {
            if !options.iter().any(|option| types.contains(option)) {
                suggestions.push(format!("For a {kind} terrain, add {adds}: {}.", options.join(" or ")));
            }
        }
    }

    // What it costs to build.
    let mut timed: Vec<(f64, &NodeData)> = terrain.nodes.values().filter_map(|node| {
        let status = editor.status(node.id);
        (status.milliseconds > 0.0).then_some((status.milliseconds as f64, node))
    }).collect();
    timed.sort_by(|a, b| b.0.total_cmp(&a.0));
    let slowest: Vec<Value> = timed.iter().take(5).map(|(milliseconds, node)| json!({"node": called(node), "milliseconds": milliseconds.round()})).collect();
    let heavy: Vec<String> = terrain.nodes.values().filter(|node| spec_of(node).is_some_and(|spec| spec.toolbox.as_deref() == Some("Simulate") || spec.has_flag("RequiresBaking"))).map(called).collect();

    let complexity = ((terrain.nodes.len() as f64 * 0.3 + connections as f64 * 0.2) * 10.0).round() / 10.0;
    let outcome = json!({
        "terrain_type": kind,
        "nodes": terrain.nodes.len(),
        "connections": connections,
        "by_category": categories,
        "generators": generators,
        "ends": ends.iter().map(|node| called(node)).collect::<Vec<_>>(),
        "exported": exporting.iter().filter_map(|id| terrain.nodes.get(id)).map(called).collect::<Vec<_>>(),
        "unused": unused,
        "longest_chain": depth,
        "patterns": patterns,
        "suggestions": suggestions,
        "simulations": heavy,
        "slowest_built": slowest,
        "complexity_score": complexity,
    });
    Ok(Step::Done(ToolResult::json(&outcome)))
}

/// The processing-quality settings optimize_parameters moves: each
/// node's `Quality` (an enum from fastest to finest) and `HighQuality`.
/// They change how finely a node works, not what it makes.
fn quality_value(spec: &NodeSpec, parameter: &nerthus_engine::ParameterSpec, mode: &str) -> Option<Value> {
    match (parameter.name.as_str(), parameter.value_type) {
        ("Quality", ValueType::Enum) => {
            let options = &parameter.enum_names;
            let chosen = match mode {
                "performance" => options.first().cloned(),
                "quality" => options.last().cloned(),
                _ => spec.defaults.get(&parameter.name).and_then(Value::as_str).map(str::to_string),
            };
            chosen.map(Value::String)
        }
        ("HighQuality", ValueType::Bool) => match mode {
            "performance" => Some(json!(false)),
            "quality" => Some(json!(true)),
            _ => spec.defaults.get(&parameter.name).cloned(),
        },
        _ => None,
    }
}

/// The preview resolution each mode suggests.
fn suggested_preview(mode: &str) -> usize {
    match mode {
        "performance" => 512,
        "quality" => 2048,
        _ => 1024,
    }
}

pub(crate) fn optimize_parameters(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let mode = arguments.string("mode")?.map(|mode| mode.trim().to_lowercase()).unwrap_or_else(|| "balanced".to_string());
    if !matches!(mode.as_str(), "performance" | "quality" | "balanced") {
        return Err("mode is performance, quality or balanced.".to_string());
    }
    let apply = arguments.boolean("apply")?.unwrap_or(true);
    let editor: &mut Editor = context.editor;
    let chosen: Option<Vec<i32>> = match arguments.array("nodes")? {
        Some(items) => Some(items.iter().map(|item| super::arguments::find_node(editor, item)).collect::<Result<_, _>>()?),
        None => None,
    };
    let catalogue = Catalogue::get();
    let mut changes: Vec<(i32, String, Value, Value)> = Vec::new();
    for node in editor.document.terrain().nodes.values() {
        if chosen.as_ref().is_some_and(|ids| !ids.contains(&node.id)) {
            continue;
        }
        let Some(spec) = catalogue.node(&node.type_name) else {
            continue;
        };
        for parameter in &spec.parameters {
            let Some(wanted) = quality_value(spec, parameter, &mode) else {
                continue;
            };
            let current = node.values.get(&parameter.name).cloned().unwrap_or(Value::Null);
            if current != wanted {
                changes.push((node.id, parameter.name.clone(), current, wanted));
            }
        }
    }
    let listed: Vec<Value> = changes
        .iter()
        .map(|(id, name, from, to)| {
            let node = editor.document.node(*id).map(|node| node.name.clone());
            json!({"node": id, "name": node, "parameter": name, "from": from, "to": to})
        })
        .collect();
    if apply && !changes.is_empty() {
        let mark = editor.document.undo_mark();
        editor.document.checkpoint(format!("Optimize for {mode}"));
        let mut by_node: HashMap<i32, Vec<(String, Value)>> = HashMap::new();
        for (id, name, _, to) in &changes {
            by_node.entry(*id).or_default().push((name.clone(), to.clone()));
        }
        for (id, values) in by_node {
            editor.document.set_values_quietly(id, &values);
        }
        editor.document.squash_undo(mark, format!("Optimize for {mode}"));
        editor.changed();
    }
    let preview = suggested_preview(&mode);
    let mut notes = vec!["Only processing quality changes (each node's Quality and HighQuality); what a node makes, such as erosion's Duration, stays as it is.".to_string()];
    if editor.preview_resolution != preview {
        notes.push(format!("For {mode}, preview at {preview} (set_project_settings preview_resolution); it is {} now.", editor.preview_resolution));
    }
    let outcome = json!({"mode": mode, "applied": apply && !changes.is_empty(), "changes": listed, "notes": notes});
    Ok(Step::Done(ToolResult::json(&outcome)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::catalogue_tools::find_type;

    #[test]
    fn a_sound_graph_passes_and_a_broken_one_does_not() {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [0.0, 0.0]).unwrap();
        let erosion = document.add_node("ErosionPro", [300.0, 0.0]).unwrap();
        document.connect(mountain, "Out", erosion, "In");
        document.toggle_export(&[erosion], nerthus_export::FileFormat::Png16);
        let findings = check_document(&document, true);
        assert!(findings.errors.is_empty(), "{:?}", findings.errors);

        // An erosion fed by nothing cannot build; a Height modifier on a
        // generator has nothing to work off.
        let lonely = document.add_node("ErosionPro", [0.0, 300.0]).unwrap();
        document.toggle_modifier(mountain, "Height");
        let findings = check_document(&document, false);
        assert!(findings.errors.iter().any(|error| error.contains(&format!("({lonely})"))), "{:?}", findings.errors);
        assert!(findings.warnings.iter().any(|warning| warning.contains("Height modifier")), "{:?}", findings.warnings);
    }

    #[test]
    fn values_out_of_range_are_found_and_fixed() {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [0.0, 0.0]).unwrap();
        document.set_values_quietly(mountain, &[("Scale".to_string(), json!(7.0)), ("Style".to_string(), json!(1))]);
        let findings = check_document(&document, false);
        assert_eq!(findings.fixable.len(), 2, "{:?}", findings.fixable);
        let fixed = fix_document(&mut document);
        assert_eq!(fixed.len(), 2, "{fixed:?}");
        let node = document.node(mountain).unwrap();
        assert_eq!(node.values["Scale"], json!(1.0));
        assert!(node.values["Style"].is_string());
        assert!(check_document(&document, false).fixable.is_empty());
    }

    #[test]
    fn the_examples_are_sound() {
        for (name, project) in knowledge::bundled_projects() {
            let mut document = Document::new();
            document.project = project.clone();
            let findings = check_document(&document, false);
            assert!(findings.errors.is_empty(), "{name}: {:?}", findings.errors);
            assert!(findings.fixable.is_empty(), "{name}: {:?}", findings.fixable);
        }
    }

    #[test]
    fn defaults_stand_whatever_the_slider() {
        // Trees' Spread defaults to 0, below its slider's 0.0001.
        let mut document = Document::new();
        document.add_node("Trees", [0.0, 0.0]).unwrap();
        let findings = check_document(&document, false);
        assert!(findings.fixable.is_empty(), "{:?}", findings.fixable);
    }

    #[test]
    fn a_loop_is_found() {
        let mut document = Document::new();
        let first = document.add_node("Blur", [0.0, 0.0]).unwrap();
        let second = document.add_node("Blur", [300.0, 0.0]).unwrap();
        document.connect(first, "Out", second, "In");
        // Written straight in: the editor never makes one.
        let record = nerthus_engine::ConnectionRecord { from: second, to: first, from_port: "Out".to_string(), to_port: "In".to_string(), is_valid: true };
        document.project.terrain_mut().nodes.get_mut(&first).unwrap().port_mut("In").unwrap().record = Some(record);
        assert!(has_cycle(document.terrain()).is_some());
    }

    #[test]
    fn every_suggested_type_exists() {
        for (_, wanted) in TERRAIN_KINDS {
            for (_, options) in wanted {
                for option in *options {
                    assert_eq!(find_type(option).unwrap().key(), *option);
                }
            }
        }
    }

    #[test]
    fn quality_moves_between_the_ends() {
        let spec = Catalogue::get().node("SlopeBlur").unwrap();
        let quality = spec.parameter("Quality").unwrap();
        assert_eq!(quality_value(spec, quality, "performance"), Some(json!("Low")));
        assert_eq!(quality_value(spec, quality, "quality"), Some(json!("Ultra")));
        assert_eq!(quality_value(spec, quality, "balanced"), Some(json!("Medium")));
        let blur = spec.parameter("Iterations").unwrap();
        assert_eq!(quality_value(spec, blur, "quality"), None);
    }
}
