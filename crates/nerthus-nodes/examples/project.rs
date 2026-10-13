//! Builds one node of a `.terrain` project the way the oracle's `project`
//! command builds it (the file opened as File › Open opens it, the node
//! built with everything upstream), so the outputs can be compared file by
//! file.
//!
//! usage: project <file.terrain> <node id> <resolution> <output directory> [--community] [--macros <directory>]
//!
//! `--community` builds as a Community build does (as the oracle runs):
//! the build loop skips bindings and expressions, so
//! they are dropped here; variables still change.
//! `--macros` reads the project's macros from that directory.

use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use nerthus_engine::macros::load_project_macros;
use nerthus_engine::{BuildResult, Engine, Project, Resolution};
use serde_json::json;

fn write_raw(path: &Path, values: &[f32]) {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    fs::write(path, bytes).expect("output file should be writable");
}

fn main() -> ExitCode {
    let mut arguments: Vec<String> = env::args().skip(1).collect();
    let community = arguments.iter().any(|argument| argument == "--community");
    arguments.retain(|argument| argument != "--community");
    let mut macro_directory = None;
    if let Some(position) = arguments.iter().position(|argument| argument == "--macros") {
        macro_directory = arguments.get(position + 1).cloned();
        arguments.drain(position..(position + 2).min(arguments.len()));
    }
    if arguments.len() < 4 {
        eprintln!("usage: project <file.terrain> <node id> <resolution> <output directory>");
        return ExitCode::FAILURE;
    }
    let file = Path::new(&arguments[0]);
    let node_id: i32 = arguments[1].parse().expect("the node id should be a number");
    let size: usize = arguments[2].parse().expect("the resolution should be a number");
    let output_directory = Path::new(&arguments[3]);
    fs::create_dir_all(output_directory).expect("output directory should be creatable");

    let text = fs::read_to_string(file).expect("the project should be readable");
    let project = match Project::parse(&text) {
        Ok(project) => project,
        Err(error) => {
            eprintln!("build failed: {error}");
            return ExitCode::from(2);
        }
    };
    let asset = &project.assets[0];
    let terrain = &asset.terrain;
    if !terrain.nodes.contains_key(&node_id) {
        eprintln!("build failed: the project has no node {node_id}");
        return ExitCode::from(2);
    }

    let registry = Arc::new(nerthus_nodes::registry());
    let mut engine = Engine::new(registry);
    engine.automation = asset.automation.clone();
    if community {
        engine.automation.bindings.clear();
        engine.automation.expressions.clear();
    }
    if let Some(directory) = macro_directory {
        let (library, problems) = load_project_macros(&project.macro_ids(), Path::new(&directory));
        for (id, problem) in problems {
            println!("macro {id} error: {problem}");
        }
        engine.macros = library;
    }

    let resolution = Resolution::square(size);
    let started = std::time::Instant::now();
    let result = engine.build(terrain, node_id, resolution);
    println!("built node {node_id} at {size} in {} ms: {result:?}", started.elapsed().as_millis());
    for id in Engine::build_order(terrain, node_id) {
        if let Some(error) = engine.error(id) {
            println!("node {id} error: {error}");
        }
    }
    if let BuildResult::Failed(error) = result {
        eprintln!("build failed: {error}");
    }

    let mut outputs = Vec::new();
    for (port, map) in engine.cache.ports(node_id) {
        let channel_count = map.channel_count();
        for channel in 0..channel_count {
            let file = if channel_count == 1 { format!("{port}.raw") } else { format!("{port}.c{channel}.raw") };
            write_raw(&output_directory.join(file), map.channel(channel));
        }
        outputs.push(json!({"port": port, "channels": channel_count, "intent": map.info.render_intent.name()}));
    }
    let label = format!("{}#{node_id}", file.file_name().and_then(|name| name.to_str()).unwrap_or_default());
    let manifest = json!({"node": label, "resolution": size, "outputs": outputs});
    let manifest_text = serde_json::to_string_pretty(&manifest).expect("manifest serializes");
    fs::write(output_directory.join("manifest.json"), manifest_text).expect("manifest should be writable");
    println!("{}", serde_json::to_string(&manifest["outputs"]).expect("outputs serialize"));
    ExitCode::SUCCESS
}
