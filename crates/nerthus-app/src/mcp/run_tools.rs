//! Builds on threads of their own, as `nerthus-build` builds: any saved
//! project built with the command line's options (`run_project`), a
//! project built small to see whether it builds (validate_graph's
//! runtime check), and generators built to measure what they make
//! (`calibrate_nodes`). Then reading builds back: the builds the server
//! ran (`build_history`), a build folder (`analyze_build`) and a
//! heightfield as land (`analyze_terrain`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nerthus_engine::macros::load_project_macros_from;
use nerthus_engine::{BuildResult, Catalogue, Engine, NodeData, NodeSpec, Project, Resolution};
use nerthus_export::build::{self, BuildDefinition, BuildEvent, ExportPreferences, RunOptions};
use nerthus_export::swarm::{self, VariableAssignment};
use serde_json::{json, Map, Value};

use super::arguments::{find_in_terrain, Arguments};
use super::build_tools::{start_build, AfterBuild, TerrainSettings};
use super::catalogue_tools::find_type;
use super::graph_tools::output_port;
use super::jobs::{self, HistoryEntry, Reporter};
use super::knowledge::{self, Calibration, Measurement};
use super::terrain_report::{is_heightfield_file, Heightfield, RawEncoding};
use super::{values, Step, ToolContext, ToolResult};
use crate::document::ExportSettings;

/// How long a build of a project file may take unless told otherwise.
const RUN_LIMIT: Duration = Duration::from_secs(4 * 60 * 60);
/// The runtime check's build.
const CHECK_RESOLUTION: usize = 512;
const CHECK_LIMIT: Duration = Duration::from_secs(30 * 60);
const CALIBRATION_LIMIT: Duration = Duration::from_secs(4 * 60 * 60);
/// The share of the height range under which calibrate_nodes calls an
/// output quiet.
const QUIET_AMPLITUDE: f64 = 0.2;
/// How much of a crash log analyze_build returns.
const CRASH_LOG_SHOWN: usize = 4000;

/// A build of a project file and what it is built with.
pub(crate) struct RunRequest {
    /// The project's file: where its bakes and macros are found.
    pub path: PathBuf,
    /// The project's text when it is not the file's (the open project's
    /// unsaved state, or a graph only proposed).
    pub text: Option<String>,
    pub resolution: Option<usize>,
    pub destination: Option<PathBuf>,
    pub profile: Option<String>,
    pub region: Option<String>,
    pub seed: i32,
    pub variables: Vec<(String, String)>,
    pub settings: Vec<(String, Value)>,
    /// Only this node is exported.
    pub target: Option<Value>,
    /// With nothing marked for export, the nodes that feed nothing are.
    pub export_ends: bool,
    pub ignore_cache: bool,
    pub silent: bool,
    pub preferences: ExportPreferences,
}

/// What came of a build.
pub(crate) struct RunOutcome {
    pub report: Value,
    pub record: HistoryEntry,
}

/// Leaves only `target` exporting: its own export mark (one made for it
/// when it has none), the other nodes' turned off and other Export nodes
/// out of the build.
fn export_only(project: &mut Project, target: i32, default_format: nerthus_export::FileFormat) {
    let terrain = project.terrain_mut();
    let upstream = Engine::build_order(terrain, target);
    let others: Vec<i32> = terrain.nodes.values().filter(|node| node.id != target && node.type_name == "Export" && !upstream.contains(&node.id)).map(|node| node.id).collect();
    for id in others {
        terrain.nodes.shift_remove(&id);
    }
    for node in terrain.nodes.values_mut() {
        if node.id == target {
            continue;
        }
        if let Some(Value::Object(definition)) = node.base.get_mut("SaveDefinition") {
            definition.insert("IsEnabled".to_string(), json!(false));
        }
    }
    if let Some(node) = terrain.nodes.get_mut(&target)
        && node.type_name != "Export"
    {
        let settings = ExportSettings::of(node).unwrap_or_else(|| ExportSettings { node: target, filename: node.name.clone(), format: default_format, enabled: true, disabled_in_profiles: Vec::new() });
        let settings = ExportSettings { enabled: true, node: target, ..settings };
        node.base.insert("SaveDefinition".to_string(), settings.to_json());
    }
}

/// Marks the nodes that feed nothing for export (in PNG 16-bit), for a
/// build of a project that exports nothing.
fn export_ends(project: &mut Project) {
    let terrain = project.terrain_mut();
    let mut fed = Vec::new();
    for node in terrain.nodes.values() {
        fed.extend(node.ports.iter().filter_map(|port| port.record.as_ref()).map(|record| record.from));
    }
    for node in terrain.nodes.values_mut() {
        if fed.contains(&node.id) {
            continue;
        }
        let settings = ExportSettings { node: node.id, filename: format!("{}-{:03}", node.name, node.id), format: nerthus_export::FileFormat::Png16, enabled: true, disabled_in_profiles: Vec::new() };
        node.base.insert("SaveDefinition".to_string(), settings.to_json());
    }
}

/// `MEMBER=VALUE` settings applied to a saved build definition.
fn apply_settings(saved: Option<&Value>, settings: &[(String, Value)]) -> Value {
    let mut definition = saved.cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
    if let Some(members) = definition.as_object_mut() {
        for (member, value) in settings {
            members.insert(member.clone(), value.clone());
        }
    }
    definition
}

/// Builds a project as `nerthus-build build` does, with the request's
/// options; progress goes to `reporter`.
pub(crate) fn run_build(request: &RunRequest, reporter: &Reporter) -> Result<RunOutcome, String> {
    let started = Instant::now();
    let text = match &request.text {
        Some(text) => text.clone(),
        None => std::fs::read_to_string(&request.path).map_err(|error| format!("{} cannot be read: {error}", request.path.display()))?,
    };
    let mut project = Project::parse(text.trim_start_matches('\u{feff}')).map_err(|error| format!("{} cannot be opened: {error}", request.path.display()))?;
    let unknown = project.unknown_types.clone();

    let target = match &request.target {
        Some(value) => {
            let id = find_in_terrain(project.terrain(), value)?;
            export_only(&mut project, id, nerthus_export::FileFormat::Png16);
            Some(id)
        }
        None => None,
    };
    if request.export_ends && build::exporting_nodes(project.terrain()).is_empty() {
        export_ends(&mut project);
    }

    let registry = Arc::new(nerthus_nodes::registry());
    let mut engine = Engine::new(registry);
    let asset = &project.assets[0];
    engine.automation = asset.automation.clone();
    let macro_ids = project.macro_ids();
    if !macro_ids.is_empty() {
        let folders = crate::files::macro_folders(Some(&request.path));
        let (library, _problems) = load_project_macros_from(&macro_ids, &folders);
        engine.macros = library;
    }
    let mut printed = Vec::new();
    if !request.variables.is_empty() {
        let assignment = VariableAssignment::Pairs(request.variables.clone());
        swarm::assign_variables(&mut engine.automation, &assignment, &mut |line| printed.push(line))?;
    }
    let chosen = match &request.profile {
        Some(name) => Some(swarm::profile_json(asset, name).ok_or_else(|| format!("The project has no build profile named {name}; its profiles are {}.", swarm::profile_names(asset).join(", ")))?),
        None => asset.extra.get("BuildDefinition"),
    };
    let saved = apply_settings(chosen, &request.settings);
    let mut definition = BuildDefinition::from_json(Some(&saved));
    let region = build::set_up_region(&asset.terrain, &mut definition, request.region.as_deref());
    if request.region.is_some() && region.is_none() {
        return Err(format!("The terrain has no region named {}.", request.region.as_deref().unwrap_or_default()));
    }
    if let Some(resolution) = request.resolution {
        definition.resolution = resolution;
    }
    if let Some(folder) = &request.destination {
        std::fs::create_dir_all(folder).map_err(|error| format!("{} cannot be made: {error}", folder.display()))?;
        let absolute = folder.canonicalize().map_err(|error| error.to_string())?;
        definition.destination = absolute.to_string_lossy().into_owned();
    }
    if request.seed > 0 {
        nerthus_engine::mutation::mutate_terrain(project.terrain_mut(), request.seed);
        engine.mutation = request.seed;
    }
    let asset = &project.assets[0];
    let project_id = project.extra.get("Id").and_then(Value::as_str).unwrap_or_default().to_string();
    let bakes = request.preferences.bake_store(&request.path, &project_id);
    let names: HashMap<i32, String> = asset.terrain.nodes.values().map(|node| (node.id, node.name.clone())).collect();
    let mut total = 0usize;
    let mut built = 0usize;
    let mut on_event = |event: BuildEvent| match event {
        BuildEvent::Started { nodes, .. } => {
            total = nodes;
            reporter.progress(0.0, Some(total as f64), Some("Building".to_string()));
        }
        BuildEvent::NodeStarted { node } => reporter.progress(built as f64, Some(total.max(1) as f64), names.get(&node).cloned()),
        BuildEvent::NodeFinished { .. } => built += 1,
        BuildEvent::Stage(stage) => reporter.progress(built as f64, Some(total.max(1) as f64), Some(stage)),
        _ => {}
    };
    let options = RunOptions { silent: request.silent, profile: request.profile.clone(), ignore_cache: request.ignore_cache };
    let report = build::run_with(&asset.terrain, &mut engine, &request.path, &definition, &request.preferences, Some(&bakes), &options, reporter.cancel_flag(), &mut on_event).map_err(|refusal| format!("The build did not start: {refusal}"))?;

    let failures: Vec<Value> = report.failures.iter().map(|(node, error)| json!({"node": node, "name": names.get(node), "error": error})).collect();
    let files: Vec<Value> = report.files.iter().map(|path| json!({"path": path, "bytes": std::fs::metadata(path).map(|data| data.len()).unwrap_or(0)})).collect();
    let terrain = report.files.iter().filter(|path| is_heightfield_file(path)).find_map(|path| {
        let field = Heightfield::load(path, None).ok()?;
        let mut analysed = field.report(asset.terrain.width.max(1.0) as f64, asset.terrain.height.max(1.0) as f64, 5.0, 0.6);
        analysed["file"] = json!(path);
        Some(analysed)
    });
    let result = report.swarm.as_ref().map(|swarm| swarm.result.clone());
    let seconds = started.elapsed().as_secs_f64();
    let mut outcome = json!({
        "project": request.path,
        "folder": report.folder,
        "resolution": definition.resolution,
        "result": result,
        "seconds": (seconds * 10.0).round() / 10.0,
        "files": files,
    });
    if !failures.is_empty() {
        outcome["failures"] = json!(failures);
    }
    if report.cancelled {
        outcome["cancelled"] = json!(true);
    }
    if let Some(found) = region {
        outcome["region"] = json!(found.name);
    }
    if let Some(target) = target {
        outcome["only_node"] = json!(target);
    }
    if request.seed > 0 {
        outcome["mutation"] = json!(request.seed);
    }
    if !printed.is_empty() {
        outcome["variables"] = json!(printed);
    }
    if !unknown.is_empty() {
        outcome["unknown_node_types"] = json!(unknown);
    }
    if let Some(terrain) = terrain {
        outcome["terrain"] = terrain;
    }
    if report.files.is_empty() && !report.cancelled {
        outcome["note"] = json!("The build wrote no files.");
    }

    let mut record = HistoryEntry::new("run_project");
    record.project = Some(request.path.display().to_string());
    record.target = target.and_then(|id| names.get(&id).map(|name| format!("{name} ({id})")));
    record.resolution = definition.resolution;
    record.seconds = seconds;
    record.files = report.files.len();
    record.folder = Some(report.folder.clone());
    record.problems = report.failures.iter().map(|(node, error)| format!("{} ({node}): {error}", names.get(node).cloned().unwrap_or_default())).collect();
    record.success = report.failures.is_empty() && !report.cancelled;
    Ok(RunOutcome { report: outcome, record })
}

/// Variables as `name=value` text, from an object of values.
fn variable_pairs(given: Option<&Map<String, Value>>) -> Vec<(String, String)> {
    let text = |value: &Value| match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    given.into_iter().flatten().map(|(name, value)| (name.clone(), text(value))).collect()
}

pub(crate) fn run_project(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let editor = &*context.editor;
    let path = match arguments.string("path")? {
        Some(text) => PathBuf::from(text.trim()),
        None => editor.document.path.clone().ok_or("The open project has never been saved; save it (save_project) or give path.")?,
    };
    if path.is_relative() || !path.is_file() {
        return Err(format!("There is no project at {} (give an absolute path).", path.display()));
    }
    let is_open = editor.document.path.as_deref() == Some(path.as_path());
    let unsaved = is_open && editor.document.dirty;
    let resolution = match arguments.integer("resolution")? {
        Some(resolution) if !(64..=262_144).contains(&resolution) => return Err("resolution runs from 64 to 262144.".to_string()),
        Some(resolution) => Some(resolution as usize),
        None => None,
    };
    let destination = arguments.string("build_path")?.map(|text| PathBuf::from(text.trim()));
    if destination.as_ref().is_some_and(|folder| folder.is_relative()) {
        return Err("build_path must be absolute.".to_string());
    }
    let seed = arguments.integer("seed")?.unwrap_or(0);
    if !(0..=i32::MAX as i64).contains(&seed) {
        return Err("seed is a whole number from 0 up (0 builds no mutation).".to_string());
    }
    let settings = super::authoring_tools::build_members(arguments.object("build_settings")?)?.into_iter().map(|(member, value)| (member.to_string(), value)).collect();
    let timeout = arguments.integer("timeout")?.map(|seconds| Duration::from_secs(seconds.clamp(10, 24 * 60 * 60) as u64)).unwrap_or(RUN_LIMIT);
    let use_unsaved = arguments.boolean("use_unsaved_changes")?.unwrap_or(false);
    let request = RunRequest {
        text: (unsaved && use_unsaved).then(|| nerthus_engine::to_terrain_json(&editor.document.project)),
        path: path.clone(),
        resolution,
        destination,
        profile: arguments.string("profile")?.map(str::to_string),
        region: arguments.string("region")?.map(str::to_string),
        seed: seed as i32,
        variables: variable_pairs(arguments.object("variables")?),
        settings,
        target: arguments.value("target_node").cloned(),
        export_ends: false,
        ignore_cache: arguments.boolean("ignore_cache")?.unwrap_or(false),
        silent: arguments.boolean("silent")?.unwrap_or(false),
        preferences: editor.user.preferences.export_preferences(),
    };
    let note = (unsaved && !use_unsaved).then_some("The open project has unsaved changes; this built the saved file (use_unsaved_changes builds them too).");
    Ok(jobs::spawn("mcp-run-project", timeout, move |reporter| match run_build(&request, reporter) {
        Ok(mut outcome) => {
            if let Some(note) = note {
                outcome.report["unsaved_changes"] = json!(note);
            }
            (ToolResult::json(&outcome.report), Some(outcome.record))
        }
        Err(problem) => {
            let mut record = HistoryEntry::new("run_project");
            record.project = Some(request.path.display().to_string());
            record.success = false;
            record.problems.push(problem.clone());
            (ToolResult::error(problem), Some(record))
        }
    }))
}

/// validate_graph's runtime check: `project` built at 512 into a folder
/// of its own (what feeds nothing exported when nothing is), the result
/// added to `outcome` and the folder deleted.
pub(crate) fn runtime_check(project: String, path: Option<PathBuf>, outcome: Value, context: &mut ToolContext) -> Result<Step, String> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|since| since.as_nanos()).unwrap_or(0);
    let folder = std::env::temp_dir().join(format!("nerthus-runtime-check-{}-{nanos}", std::process::id()));
    let project_path = path.unwrap_or_else(|| folder.join("Check.terrain"));
    let request = RunRequest {
        path: project_path,
        text: Some(project),
        resolution: Some(CHECK_RESOLUTION),
        destination: Some(folder.join("build")),
        profile: None,
        region: None,
        seed: 0,
        variables: Vec::new(),
        // A Standard build, whatever the project's: buckets and tiles are
        // not what is being checked.
        settings: vec![
            ("Type".to_string(), json!("Standard")),
            ("Portion".to_string(), Value::Null),
            ("OutputType".to_string(), json!("SingleImage")),
            ("PostBuildScript".to_string(), json!("")),
            ("OpenFolder".to_string(), json!(false)),
            ("MaintainPrimeFolder".to_string(), json!(false)),
        ],
        target: None,
        export_ends: true,
        ignore_cache: false,
        silent: true,
        preferences: context.editor.user.preferences.export_preferences(),
    };
    Ok(jobs::spawn("mcp-runtime-check", CHECK_LIMIT, move |reporter| {
        let mut outcome = outcome;
        let built = run_build(&request, reporter);
        let check = match &built {
            Ok(run) => {
                let failures = run.report.get("failures").cloned().unwrap_or(json!([]));
                let ok = failures.as_array().is_none_or(Vec::is_empty) && run.report.get("cancelled").is_none();
                json!({"built": ok, "resolution": CHECK_RESOLUTION, "seconds": run.report["seconds"], "files": run.report["files"].as_array().map_or(0, Vec::len), "failures": failures})
            }
            Err(problem) => json!({"built": false, "resolution": CHECK_RESOLUTION, "problem": problem}),
        };
        if check["built"] == json!(false) {
            outcome["valid"] = json!(false);
        }
        outcome["runtime_check"] = check;
        let _ = std::fs::remove_dir_all(&folder);
        let record = built.ok().map(|run| {
            let mut record = run.record;
            record.tool = "validate_graph".to_string();
            record.folder = None;
            record
        });
        (ToolResult::json(&outcome), record)
    }))
}

/// The generators calibrate_nodes measures by default: those the toolbox
/// lists under Primitive and Terrain.
fn measurable_types() -> Vec<&'static NodeSpec> {
    let mut types: Vec<&'static NodeSpec> = Catalogue::get()
        .nodes()
        .filter(|spec| crate::ui::toolbox::listed(spec) && matches!(spec.toolbox.as_deref(), Some("Primitive" | "Terrain")) && knowledge::is_generator(spec))
        .collect();
    types.sort_by(|a, b| a.key().cmp(b.key()));
    types
}

/// What calibrate_nodes builds: each type at each point (the defaults, or
/// fractions along the swept parameter's range) with each seed.
struct Calibrate {
    types: Vec<&'static NodeSpec>,
    property: Option<String>,
    points: Vec<Option<f64>>,
    seeds: Vec<i64>,
    resolution: usize,
}

/// The span and mean of a built height map, both as shares of the full
/// height range (heights past 0 and 1 count as 0 and 1, as exports clip
/// them).
fn measure(values: &[f32]) -> (f64, f64) {
    let mut low = f64::INFINITY;
    let mut high = f64::NEG_INFINITY;
    let mut sum = 0.0;
    let mut count = 0usize;
    for value in values.iter().filter(|value| value.is_finite()) {
        let value = (*value as f64).clamp(0.0, 1.0);
        low = low.min(value);
        high = high.max(value);
        sum += value;
        count += 1;
    }
    if count == 0 {
        return (0.0, 0.0);
    }
    (high - low, sum / count as f64)
}

fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

/// Builds one type at one point with one seed: its output's span and mean.
fn build_one(spec: &'static NodeSpec, property: Option<(&str, f64)>, seed: i64, resolution: usize, blank: &nerthus_engine::Terrain, registry: &Arc<nerthus_engine::Registry>) -> Result<(f64, f64), String> {
    let mut node = NodeData::new(spec, 1, seed as i32);
    if let Some(seed_parameter) = spec.parameters.iter().find(|parameter| parameter.is_seed()) {
        node.values.insert(seed_parameter.name.clone(), json!(seed));
    }
    if let Some((name, value)) = property {
        let parameter = spec.parameter(name).ok_or_else(|| format!("{} has no parameter {name}.", spec.key()))?;
        let stored = values::convert(parameter, &json!(value), node.values.get(name))?;
        node.values.insert(parameter.name.clone(), stored.value);
    }
    let mut terrain = blank.clone();
    terrain.nodes.insert(1, node);
    let mut engine = Engine::new(registry.clone());
    if let BuildResult::Failed(error) = engine.build(&terrain, 1, Resolution::square(resolution)) {
        return Err(error.to_string());
    }
    let map = engine.cache.get(1, "Out").ok_or("The build made no Out.")?;
    Ok(measure(map.channel(0)))
}

fn run_calibration(plan: &Calibrate, reporter: &Reporter) -> ToolResult {
    let registry = Arc::new(nerthus_nodes::registry());
    let blank = crate::document::Document::new().terrain().clone();
    let mut blank = blank;
    blank.nodes.clear();
    let total = plan.types.len() * plan.points.len() * plan.seeds.len();
    let mut done = 0usize;
    let mut measurements = Vec::new();
    let mut skipped = Vec::new();
    'types: for spec in &plan.types {
        for point in &plan.points {
            // The swept parameter's value at this point of its range.
            let property = match (point, &plan.property) {
                (Some(fraction), Some(name)) => {
                    let Some(parameter) = spec.parameter(name) else {
                        skipped.push(json!({"node": spec.key(), "reason": format!("it has no parameter {name}")}));
                        done += plan.points.len() * plan.seeds.len();
                        continue 'types;
                    };
                    let (Some(min), Some(max)) = (parameter.min, parameter.max) else {
                        skipped.push(json!({"node": spec.key(), "reason": format!("{name} has no range to sweep")}));
                        done += plan.points.len() * plan.seeds.len();
                        continue 'types;
                    };
                    Some((parameter.name.as_str(), ((min + (max - min) * fraction) * 10_000.0).round() / 10_000.0))
                }
                _ => None,
            };
            let mut spans = Vec::new();
            let mut means = Vec::new();
            for seed in &plan.seeds {
                if reporter.cancelled() {
                    return ToolResult::error("Calibration was stopped.");
                }
                reporter.progress(done as f64, Some(total as f64), Some(spec.key().to_string()));
                done += 1;
                match build_one(spec, property, *seed, plan.resolution, &blank, &registry) {
                    Ok((span, mean)) => {
                        spans.push(span);
                        means.push(mean);
                    }
                    Err(problem) => {
                        skipped.push(json!({"node": spec.key(), "seed": seed, "at": property.map(|(name, value)| format!("{name} = {value}")), "reason": problem}));
                    }
                }
            }
            if spans.is_empty() {
                continue;
            }
            let low = spans.iter().copied().fold(f64::INFINITY, f64::min);
            let high = spans.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            measurements.push(Measurement {
                node_type: spec.key().to_string(),
                property: property.map(|(name, _)| name.to_string()),
                value: property.map(|(_, value)| value),
                seeds: plan.seeds.clone(),
                amplitude: median(&mut spans),
                amplitude_low: low,
                amplitude_high: high,
                mean_level: median(&mut means),
            });
        }
    }
    if measurements.is_empty() {
        return ToolResult::error(format!("Nothing was measured: {}", serde_json::to_string(&skipped).unwrap_or_default()));
    }
    let taken = measurements.len();
    let fresh = Calibration { nerthus_version: env!("CARGO_PKG_VERSION").to_string(), measured_at: chrono::Local::now().to_rfc3339(), resolution: plan.resolution, measurements };
    let merged = Calibration::merge(knowledge::load_calibration(), fresh);
    let saved = knowledge::save_calibration(&merged);
    let mut quiet: Vec<Value> = merged
        .measurements
        .iter()
        .filter(|measurement| measurement.amplitude < QUIET_AMPLITUDE)
        .map(|measurement| json!({"node": measurement.node_type, "at": measurement.property.as_ref().map(|name| format!("{name} = {}", measurement.value.unwrap_or_default())), "fills_percent": (measurement.amplitude * 1000.0).round() / 10.0}))
        .collect();
    quiet.truncate(15);
    let measured_types: std::collections::BTreeSet<&str> = merged.measurements.iter().map(|measurement| measurement.node_type.as_str()).collect();
    let mut outcome = json!({
        "resolution": plan.resolution,
        "sweep": plan.property,
        "points": plan.points.len(),
        "seeds": plan.seeds,
        "measurements_taken": taken,
        "measurements_on_file": merged.measurements.len(),
        "types_on_file": measured_types.len(),
        "fills_under_a_fifth_of_the_range": quiet,
        "skipped": skipped,
        "note": "describe_node_type reports these measurements.",
    });
    match saved {
        Ok(path) => outcome["calibration_file"] = json!(path),
        Err(problem) => outcome["not_saved"] = json!(problem),
    }
    ToolResult::json(&outcome)
}

pub(crate) fn calibrate_nodes(arguments: &Arguments, _context: &mut ToolContext) -> Result<Step, String> {
    let types = match arguments.array("nodes")? {
        Some(items) => {
            let mut types = Vec::new();
            for item in items {
                let name = item.as_str().ok_or("nodes lists node types by name.")?;
                types.push(find_type(name)?);
            }
            types
        }
        None => measurable_types(),
    };
    if types.is_empty() {
        return Err("nodes names no node type.".to_string());
    }
    let property = arguments.string("property")?.map(|name| name.trim().to_string()).filter(|name| !name.is_empty());
    let steps = arguments.integer("steps")?.unwrap_or(3).clamp(2, 9) as usize;
    let points: Vec<Option<f64>> = match &property {
        None => vec![None],
        Some(name) => {
            let any = types.iter().any(|spec| spec.parameter(name).is_some_and(|parameter| parameter.min.is_some() && parameter.max.is_some()));
            if !any {
                return Err(format!("None of the types has a parameter {name} with a range to sweep."));
            }
            (0..steps).map(|index| Some(index as f64 / (steps - 1) as f64)).collect()
        }
    };
    let seeds: Vec<i64> = match arguments.array("seeds")? {
        Some(items) => items.iter().filter_map(super::arguments::number_of).map(|seed| seed.round() as i64).collect(),
        None => vec![1001, 2002, 3003],
    };
    if seeds.is_empty() {
        return Err("seeds names no seed.".to_string());
    }
    let resolution = arguments.integer("resolution")?.unwrap_or(512);
    if ![256, 512, 1024, 2048].contains(&resolution) {
        return Err("resolution is 256, 512, 1024 or 2048.".to_string());
    }
    let plan = Calibrate { types, property, points, seeds, resolution: resolution as usize };
    Ok(jobs::spawn("mcp-calibrate", CALIBRATION_LIMIT, move |reporter| {
        let started = Instant::now();
        let result = run_calibration(&plan, reporter);
        let mut record = HistoryEntry::new("calibrate_nodes");
        record.resolution = plan.resolution;
        record.seconds = started.elapsed().as_secs_f64();
        record.success = !result.is_error;
        record.target = Some(format!("{} types", plan.types.len()));
        (result, Some(record))
    }))
}

pub(crate) fn build_history(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let limit = arguments.integer("limit")?.unwrap_or(10).clamp(1, 200) as usize;
    let entries = context.history.entries();
    let recent: Vec<Value> = entries.iter().rev().take(limit).map(|entry| entry.to_json()).collect();
    let successes = entries.iter().filter(|entry| entry.success).count();
    let timed: Vec<f64> = entries.iter().map(|entry| entry.seconds).filter(|seconds| *seconds > 0.0).collect();
    let average = if timed.is_empty() { 0.0 } else { timed.iter().sum::<f64>() / timed.len() as f64 };
    let mut by_tool: HashMap<&str, usize> = HashMap::new();
    for entry in entries {
        *by_tool.entry(entry.tool.as_str()).or_default() += 1;
    }
    let outcome = json!({
        "recent": recent,
        "total": entries.len(),
        "succeeded": successes,
        "failed": entries.len() - successes,
        "average_seconds": (average * 100.0).round() / 100.0,
        "by_tool": by_tool,
    });
    Ok(Step::Done(ToolResult::json(&outcome)))
}

/// The files under `folder`, by their path within it.
fn files_in(folder: &Path) -> Vec<(String, PathBuf, u64)> {
    let mut found = Vec::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(current) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                pending.push(path);
            } else {
                let relative = path.strip_prefix(folder).unwrap_or(&path).to_string_lossy().replace('\\', "/");
                found.push((relative, path, metadata.len()));
            }
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// A .NET `TimeSpan` (`00:00:01.2345678`) in seconds.
fn time_span_seconds(text: &str) -> Option<f64> {
    let (days, rest) = match text.split_once('.') {
        Some((days, rest)) if rest.contains(':') => (days.parse::<f64>().ok()?, rest),
        _ => (0.0, text),
    };
    let parts: Vec<&str> = rest.split(':').collect();
    let [hours, minutes, seconds] = parts[..] else {
        return None;
    };
    Some(days * 86400.0 + hours.parse::<f64>().ok()? * 3600.0 + minutes.parse::<f64>().ok()? * 60.0 + seconds.parse::<f64>().ok()?)
}

/// What report.json says of a build: its result, and its slowest nodes.
fn read_report(folder: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(folder.join("report.json")).ok()?;
    let report: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    let mut timed: Vec<(String, f64)> = report
        .get("SecondsTaken")
        .and_then(|taken| taken.get("$values"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| Some((item.get("Item1")?.as_str()?.to_string(), item.get("Item2").and_then(Value::as_str).and_then(time_span_seconds).unwrap_or(0.0))))
        .collect();
    timed.sort_by(|a, b| b.1.total_cmp(&a.1));
    let slowest: Vec<Value> = timed.iter().take(5).filter(|(_, seconds)| *seconds > 0.0).map(|(name, seconds)| json!({"node": name, "seconds": (seconds * 100.0).round() / 100.0})).collect();
    Some(json!({
        "result": report.get("Result"),
        "resolution": report.get("Resolution"),
        "node_count": report.get("NodeCount"),
        "seconds": report.get("Duration").and_then(Value::as_str).and_then(time_span_seconds).map(|seconds| (seconds * 100.0).round() / 100.0),
        "slowest_nodes": slowest,
    }))
}

/// The nodes report.txt says failed (`007 - Name - failed!`).
fn failed_in_report(folder: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(folder.join("report.txt")) else {
        return Vec::new();
    };
    text.lines().map(str::trim).filter(|line| line.ends_with("- failed!")).map(|line| line.trim_end_matches("- failed!").trim().to_string()).collect()
}

/// The crash log, and its first fault: everything downstream of that one
/// fails only because its input came back empty.
fn read_crash_log(folder: &Path) -> Option<(String, Option<String>)> {
    let text = std::fs::read_to_string(folder.join("CRASH_LOG.txt")).ok()?;
    let is_fault = |line: &&str| {
        let line = line.trim();
        let numbered = line.split_once(':').is_some_and(|(number, _)| !number.is_empty() && number.chars().all(|character| character.is_ascii_digit()));
        numbered || line.contains("failed:") || line.ends_with("- failed!")
    };
    let first = text.lines().find(is_fault).map(|line| line.trim().to_string());
    let shown: String = text.chars().take(CRASH_LOG_SHOWN).collect();
    Some((shown, first))
}

pub(crate) fn analyze_build(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let folder = match arguments.string("build_dir")? {
        Some(text) => PathBuf::from(text.trim()),
        None => {
            let latest = context.history.latest_folder().cloned();
            let exported = context.editor.export_build.as_ref().and_then(|build| build.folder.clone());
            latest.or(exported).ok_or("No build has run yet; give build_dir.")?
        }
    };
    if !folder.is_dir() {
        return Err(format!("There is no folder at {}.", folder.display()));
    }
    let files = files_in(&folder);
    let total: u64 = files.iter().map(|(_, _, bytes)| bytes).sum();
    let listed: Vec<Value> = files.iter().map(|(name, _, bytes)| json!({"name": name, "bytes": bytes})).collect();
    let expected: Vec<Value> = arguments
        .array("expected_files")?
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|wanted| {
            let present = files.iter().any(|(name, _, _)| name == wanted || name.rsplit('/').next() == Some(wanted));
            json!({"name": wanted, "present": present})
        })
        .collect();
    let crash = read_crash_log(&folder);
    let failed = failed_in_report(&folder);
    let (metres_across, metres_up) = {
        let terrain = context.editor.document.terrain();
        let width = if terrain.width > 0.0 { terrain.width as f64 } else { 5000.0 };
        let height = if terrain.height > 0.0 { terrain.height as f64 } else { 2500.0 };
        (width, height)
    };
    let terrain = files.iter().filter(|(_, path, _)| is_heightfield_file(path)).find_map(|(name, path, _)| {
        let field = Heightfield::load(path, None).ok()?;
        let mut analysed = field.report(metres_across, metres_up, 5.0, 0.6);
        analysed["file"] = json!(name);
        Some(analysed)
    });
    let mut outcome = json!({
        "build_dir": folder,
        "file_count": files.len(),
        "total_bytes": total,
        "files": listed,
        "crashed": crash.is_some(),
    });
    if let Some(report) = read_report(&folder) {
        outcome["report"] = report;
    }
    if !failed.is_empty() {
        outcome["failed_nodes"] = json!(failed);
    }
    if let Some((log, first)) = crash {
        outcome["first_fault"] = json!(first);
        outcome["crash_log"] = json!(log);
    }
    if !expected.is_empty() {
        outcome["expected_files"] = json!(expected);
    }
    if let Some(terrain) = terrain {
        outcome["terrain"] = terrain;
    }
    if files.is_empty() {
        outcome["note"] = json!("The folder is empty: the build wrote nothing.");
    }
    Ok(Step::Done(ToolResult::json(&outcome)))
}

pub(crate) fn analyze_terrain(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let editor = &mut *context.editor;
    let terrain = editor.document.terrain();
    let width = if terrain.width > 0.0 { terrain.width as f64 } else { 5000.0 };
    let height = if terrain.height > 0.0 { terrain.height as f64 } else { 2500.0 };
    let settings = TerrainSettings {
        metres_across: arguments.number("metres_across")?.unwrap_or(width).max(1.0),
        metres_up: arguments.number("metres_up")?.unwrap_or(height).max(1.0),
        gentle_degrees: arguments.number("gentle_degrees")?.unwrap_or(5.0).clamp(0.0, 90.0),
        centre_fraction: arguments.number("centre_fraction")?.unwrap_or(0.6).clamp(0.1, 1.0),
    };
    let path = arguments.string("path")?.map(|text| PathBuf::from(text.trim()));
    let node = arguments.optional_node("node", editor)?;
    match (path, node) {
        (Some(_), Some(_)) => Err("Give node or path, not both.".to_string()),
        (None, None) => Err("Give a node to build and read, or the path of a heightfield file.".to_string()),
        (Some(path), None) => {
            if path.is_relative() || !path.is_file() {
                return Err(format!("There is no file at {} (give an absolute path).", path.display()));
            }
            let encoding = match arguments.string("encoding")? {
                Some(name) => Some(RawEncoding::from_name(name).ok_or("encoding is ushort, half or float.")?),
                None => None,
            };
            let field = Heightfield::load(&path, encoding)?;
            let mut analysed = field.report(settings.metres_across, settings.metres_up, settings.gentle_degrees, settings.centre_fraction);
            analysed["path"] = json!(path);
            Ok(Step::Done(ToolResult::json(&analysed)))
        }
        (None, Some(node)) => {
            let port = match arguments.string("port")? {
                Some(port) => Some(output_port(editor.document.node(node).ok_or("The node is gone.")?, Some(port))?),
                None => None,
            };
            let shown = port.clone();
            start_build(editor, node, shown.as_deref(), AfterBuild::Analyze { port, settings })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_spans_read_as_seconds() {
        assert_eq!(time_span_seconds("00:00:01.5"), Some(1.5));
        assert_eq!(time_span_seconds("01:02:03"), Some(3723.0));
        assert_eq!(time_span_seconds("1.00:00:00"), Some(86400.0));
        assert_eq!(time_span_seconds("nonsense"), None);
    }

    #[test]
    fn measurements_clip_to_the_height_range() {
        assert_eq!(measure(&[0.25, 0.75]), (0.5, 0.5));
        assert_eq!(measure(&[-1.0, 2.0]), (1.0, 0.5));
        assert_eq!(measure(&[]), (0.0, 0.0));
    }

    #[test]
    fn a_generator_is_measured() {
        let registry = Arc::new(nerthus_nodes::registry());
        let mut blank = crate::document::Document::new().terrain().clone();
        blank.nodes.clear();
        let spec = Catalogue::get().node("Mountain").unwrap();
        let (span, mean) = build_one(spec, None, 1001, 256, &blank, &registry).unwrap();
        assert!(span > 0.0 && span <= 1.0, "{span}");
        assert!(mean > 0.0 && mean < 1.0, "{mean}");
        assert!(measurable_types().iter().any(|spec| spec.key() == "Mountain"));
    }

    #[test]
    fn a_project_file_builds() {
        let folder = std::env::temp_dir().join(format!("nerthus-run-project-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let mut document = crate::document::Document::new();
        let perlin = document.add_node("Perlin", [26000.0, 26000.0]).unwrap();
        let blur = document.add_node("Blur", [26300.0, 26000.0]).unwrap();
        document.connect(perlin, "Out", blur, "In");
        let path = folder.join("Run.terrain");
        document.save(&path).unwrap();
        let (sender, _messages) = crossbeam_channel::unbounded();
        let reporter = jobs::test_reporter(sender);
        let request = RunRequest {
            path: path.clone(),
            text: None,
            resolution: Some(256),
            destination: Some(folder.join("build")),
            profile: None,
            region: None,
            seed: 3,
            variables: Vec::new(),
            settings: Vec::new(),
            target: Some(json!(blur)),
            export_ends: false,
            ignore_cache: false,
            silent: true,
            preferences: ExportPreferences::default(),
        };
        let outcome = run_build(&request, &reporter).unwrap();
        assert_eq!(outcome.report["files"].as_array().unwrap().len(), 1, "{}", outcome.report);
        assert!(outcome.record.success);
        assert!(outcome.report.get("terrain").is_some());
        // Nothing marked and no target: refused, unless the ends are exported.
        let request = RunRequest { target: None, ..request };
        assert!(run_build(&request, &reporter).is_err());
        let request = RunRequest { export_ends: true, ..request };
        assert!(run_build(&request, &reporter).is_ok());
        std::fs::remove_dir_all(&folder).ok();
    }
}
