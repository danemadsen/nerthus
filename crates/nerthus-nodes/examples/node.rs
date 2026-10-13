//! Builds one node the way the oracle's `build` command builds it, so the
//! outputs can be compared file by file.
//!
//! usage: node <Type> <resolution> <output directory> [Property=Value…] [in:Port=file.raw…] [up:Port=file.raw…]
//!
//! Values: numbers, `true`/`false`, enum member names (or indices),
//! `x,y` for Float2 and `r,g,b[,a]` for colours. Inputs are raw
//! little-endian f32 maps at the given resolution. `up:Port` feeds the `In`
//! port of the stand-in node connected to `Port`, for nodes that look one
//! step further upstream. `out:Port:Output` gives that stand-in another
//! output, and `acc:Type:Output` adds a built node of `Type` holding only
//! `Output` (for nodes that gather accumulation). `PortCount=N` adds ports
//! the way the editor's "Add Input" command does. `Owner.Member=value`
//! sets one member of an object-valued property (a Mixer layer).
//! `src:Port=Type` connects a real node of `Type` to `Port` (for nodes that
//! rebuild their source, like Reseed) and `src:Port.Member=value` sets one
//! of its parameters. `var:Name=Type:Value` adds a project variable,
//! `bind:Property=Variable` binds a parameter to it and
//! `expr:Property=formula` gives a parameter an expression; each variable
//! is then written out as `Variable.<name>` (value, boxed-type code).
//! `own:Port=file.raw` stores one of the node's own outputs, and `builds=0`
//! skips the build and takes the node as built from those. For a type with
//! its own viewport colours, `Node.GetCustomColorMap()` is written out as
//! `CustomColorMap`.

use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use nerthus_engine::automation::{Automation, VariableValue};
use nerthus_engine::json::References;
use nerthus_engine::project::{ConnectionRecord, ModifierData, NodeData, PortData, Terrain};
use nerthus_engine::{BuildResult, Catalogue, Engine, Map, ParameterSpec, Resolution, ValueType};
use serde_json::{json, Map as JsonObject, Value};

fn parse_value(parameter: Option<&ParameterSpec>, text: &str) -> Value {
    let numbers: Vec<f64> = text.split(',').filter_map(|part| part.trim().parse().ok()).collect();
    let Some(parameter) = parameter else {
        return serde_json::from_str(text).unwrap_or_else(|_| Value::from(text));
    };
    match parameter.value_type {
        ValueType::Float | ValueType::Double => json!(numbers.first().copied().unwrap_or(0.0)),
        ValueType::Int | ValueType::Byte => json!(numbers.first().copied().unwrap_or(0.0) as i64),
        ValueType::Bool => json!(text == "true" || text == "True" || text == "1"),
        ValueType::Enum => match text.parse::<i64>() {
            Ok(index) => json!(parameter.enum_name(index).unwrap_or(text)),
            Err(_) => json!(text),
        },
        ValueType::Float2 => json!({"X": numbers[0], "Y": numbers[1]}),
        ValueType::Float2List => {
            let points: Vec<Value> = text
                .split(';')
                .filter(|point| !point.is_empty())
                .map(|point| {
                    let coordinates: Vec<f64> = point.split(',').filter_map(|part| part.trim().parse().ok()).collect();
                    json!({"X": coordinates[0], "Y": coordinates[1]})
                })
                .collect();
            Value::Array(points)
        }
        ValueType::Float4List => {
            let points: Vec<Value> = text
                .split(';')
                .filter(|point| !point.is_empty())
                .map(|point| {
                    let components: Vec<f64> = point.split(',').filter_map(|part| part.trim().parse().ok()).collect();
                    json!({"X": components[0], "Y": components[1], "Z": components[2], "W": components[3]})
                })
                .collect();
            Value::Array(points)
        }
        ValueType::Color => {
            let alpha = numbers.get(3).copied().unwrap_or(0.0);
            json!({"R": numbers[0], "G": numbers[1], "B": numbers[2], "A": alpha})
        }
        _ => serde_json::from_str(text).unwrap_or_else(|_| Value::from(text)),
    }
}

/// A nested member's value (`Owner.Member=…`), whose type the catalogue
/// does not record: integers, numbers, booleans, `x,y` pairs and `r,g,b`
/// colours, else the text itself (an enum member name).
fn parse_loose(text: &str) -> Value {
    if let Ok(flag) = text.parse::<bool>() {
        return Value::from(flag);
    }
    if let Ok(integer) = text.parse::<i64>() {
        return Value::from(integer);
    }
    if let Ok(number) = text.parse::<f64>() {
        return json!(number);
    }
    let parts: Vec<&str> = text.split(',').collect();
    let numbers: Vec<f64> = parts.iter().filter_map(|part| part.parse().ok()).collect();
    if numbers.len() == parts.len() && numbers.len() == 2 {
        return json!({"X": numbers[0], "Y": numbers[1]});
    }
    if numbers.len() == parts.len() && numbers.len() == 3 {
        return json!({"R": numbers[0], "G": numbers[1], "B": numbers[2]});
    }
    Value::from(text)
}

fn read_raw(path: &str) -> Vec<f32> {
    let bytes = fs::read(path).expect("input file should be readable");
    bytes.as_chunks::<4>().0.iter().map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])).collect()
}

fn write_raw(path: &Path, values: &[f32]) {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    fs::write(path, bytes).expect("output file should be writable");
}

/// A built stand-in node holding `data`, shaped like a bare `Node` (an
/// `In` and an `Out` port).
fn add_source(terrain: &mut Terrain, engine: &mut Engine, id: i32, resolution: Resolution, data: Vec<f32>) {
    let port = |name: &str, port_type: &str| PortData {
        name: name.to_string(),
        port_type: port_type.to_string(),
        record: None,
        is_exporting: false,
        has_parent: true,
        extra: JsonObject::new(),
        declared_intent: None,
    };
    let source = NodeData {
        id,
        type_name: "Constant".to_string(),
        name: format!("Input{id}"),
        position: [0.0, 0.0],
        values: Default::default(),
        base: Default::default(),
        ports: vec![port("In", "PrimaryIn"), port("Out", "PrimaryOut")],
        modifiers: Vec::new(),
    };
    terrain.nodes.insert(id, source);
    engine.cache.store(id, "Out", Arc::new(raw_map(resolution, data)));
    engine.mark_built(id);
}

/// A map from raw planes, one after another.
fn raw_map(resolution: Resolution, data: Vec<f32>) -> Map {
    let plane = resolution.area();
    let channels: Vec<Vec<f32>> = data.chunks(plane).map(<[f32]>::to_vec).collect();
    Map::from_channels(resolution, channels)
}

/// A built node of `type_name` holding `data` as its `output`: a stand-in
/// for nodes that gather accumulation.
fn add_accumulator(terrain: &mut Terrain, engine: &mut Engine, id: i32, type_name: &str, output: &str, resolution: Resolution, data: Vec<f32>) -> Result<(), String> {
    let spec = Catalogue::get().node(type_name).ok_or_else(|| format!("unknown node type {type_name}"))?;
    terrain.nodes.insert(id, NodeData::new(spec, id, 0));
    engine.cache.store(id, output, Arc::new(raw_map(resolution, data)));
    engine.mark_built(id);
    Ok(())
}

/// What Newtonsoft reads into an `object` member: an integer, a fraction,
/// a flag or text.
fn json_scalar(text: &str) -> Value {
    if let Ok(integer) = text.parse::<i64>() {
        return json!(integer);
    }
    if let Ok(number) = text.parse::<f64>() {
        return json!(number);
    }
    if let Ok(flag) = text.to_ascii_lowercase().parse::<bool>() {
        return json!(flag);
    }
    json!(text)
}

/// A variable's value as a float and a code for its boxed type (as the
/// oracle writes them).
fn variable_output(value: &VariableValue) -> [f32; 2] {
    match value {
        VariableValue::Float(number) => [*number, 0.0],
        VariableValue::Int(integer) => [*integer as f32, 1.0],
        VariableValue::Bool(flag) => [if *flag { 1.0 } else { 0.0 }, 2.0],
        VariableValue::Text(_) => [f32::NAN, 3.0],
        VariableValue::Long(integer) => [*integer as f32, 4.0],
        VariableValue::Double(number) => [*number as f32, 5.0],
        VariableValue::Null => [f32::NAN, 7.0],
        VariableValue::Float2(_) | VariableValue::Other(_) => [f32::NAN, 8.0],
    }
}

fn connection(from: i32, to: i32, to_port: &str) -> ConnectionRecord {
    ConnectionRecord {
        from,
        to,
        from_port: "Out".to_string(),
        to_port: to_port.to_string(),
        is_valid: true,
    }
}

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments.len() < 3 {
        eprintln!("usage: node <Type> <resolution> <output directory> [Property=Value…] [in:Port=file.raw…]");
        return ExitCode::FAILURE;
    }
    let type_name = &arguments[0];
    let size: usize = arguments[1].parse().expect("resolution should be a number");
    let output_directory = Path::new(&arguments[2]);
    fs::create_dir_all(output_directory).expect("output directory should be creatable");

    let catalogue = Catalogue::get();
    let Some(spec) = catalogue.node(type_name) else {
        eprintln!("unknown node type {type_name}");
        return ExitCode::FAILURE;
    };

    const NODE_ID: i32 = 1;
    let mut node = NodeData::new(spec, NODE_ID, 0);
    let resolution = Resolution::square(size);
    let mut terrain = Terrain {
        id: String::new(),
        nodes: Default::default(),
        width: 5000.0,
        height: 2500.0,
        extra: JsonObject::new(),
    };
    let registry = Arc::new(nerthus_nodes::registry());
    let mut engine = Engine::new(registry.clone());

    let settings: Vec<(&str, &str)> = arguments[3..].iter().filter_map(|setting| setting.split_once('=')).collect();

    // Extra ports first, so inputs can be connected to them.
    for &(key, value) in &settings {
        if key != "PortCount" {
            continue;
        }
        let wanted: i64 = value.parse().expect("PortCount should be a number");
        let builder = registry.node(type_name).expect("the node is implemented");
        loop {
            let count = node.values.get("PortCount").and_then(Value::as_i64).unwrap_or(0);
            if count >= wanted || builder.add_new_port(&mut node).is_none() {
                break;
            }
        }
    }

    let mut source_id = 1000;
    let mut variables = JsonObject::new();
    let mut bindings = Vec::new();
    let mut expressions = JsonObject::new();
    let mut has_automation = false;
    for &(key, value) in &settings {
        if key == "PortCount" || key == "builds" {
            continue;
        }
        if let Some(name) = key.strip_prefix("var:") {
            let (variable_type, text) = value.split_once(':').expect("var: needs Type:Value");
            let variable = json!({"Name": name, "Type": variable_type, "Value": json_scalar(text)});
            variables.insert(name.to_string(), variable);
            has_automation = true;
            continue;
        }
        if let Some(property) = key.strip_prefix("bind:") {
            bindings.push(json!({"Node": NODE_ID, "Property": property, "Variable": value}));
            has_automation = true;
            continue;
        }
        if let Some(property) = key.strip_prefix("expr:") {
            expressions.insert(format!("{NODE_ID}_{property}"), json!(value));
            has_automation = true;
            continue;
        }
        if let Some(port) = key.strip_prefix("in:") {
            add_source(&mut terrain, &mut engine, source_id, resolution, read_raw(value));
            match node.port_mut(port) {
                Some(target) => target.record = Some(connection(source_id, NODE_ID, port)),
                None => {
                    eprintln!("{type_name} has no port {port}");
                    return ExitCode::FAILURE;
                }
            }
            source_id += 1;
            continue;
        }
        if let Some(port) = key.strip_prefix("own:") {
            engine.cache.store(NODE_ID, port, Arc::new(raw_map(resolution, read_raw(value))));
            continue;
        }
        if let Some(rest) = key.strip_prefix("out:") {
            // Another output of the stand-in feeding a port: out:Port:Output=file.
            let Some((port, output)) = rest.split_once(':') else {
                eprintln!("out: needs Port:Output");
                return ExitCode::FAILURE;
            };
            let Some(feeding) = node.port(port).and_then(|port| port.record.as_ref()).map(|record| record.from) else {
                eprintln!("out:{port} needs in:{port} first");
                return ExitCode::FAILURE;
            };
            engine.cache.store(feeding, output, Arc::new(raw_map(resolution, read_raw(value))));
            continue;
        }
        if let Some(rest) = key.strip_prefix("acc:") {
            // acc:Type:Output=file.
            let Some((accumulator_type, output)) = rest.split_once(':') else {
                eprintln!("acc: needs Type:Output");
                return ExitCode::FAILURE;
            };
            if let Err(problem) = add_accumulator(&mut terrain, &mut engine, source_id, accumulator_type, output, resolution, read_raw(value)) {
                eprintln!("{problem}");
                return ExitCode::FAILURE;
            }
            source_id += 1;
            continue;
        }
        if let Some(target) = key.strip_prefix("src:") {
            // src:Port=Type connects a real node of Type to Port (for nodes
            // that rebuild their source, like Reseed); src:Port.Member=value
            // sets one of its members.
            let Some((port, member)) = target.split_once('.') else {
                let Some(source_spec) = catalogue.node(value) else {
                    eprintln!("unknown node type {value}");
                    return ExitCode::FAILURE;
                };
                terrain.nodes.insert(source_id, NodeData::new(source_spec, source_id, 0));
                match node.port_mut(target) {
                    Some(input) => input.record = Some(connection(source_id, NODE_ID, target)),
                    None => {
                        eprintln!("{type_name} has no port {target}");
                        return ExitCode::FAILURE;
                    }
                }
                source_id += 1;
                continue;
            };
            let Some(feeding) = node.port(port).and_then(|port| port.record.as_ref()).map(|record| record.from) else {
                eprintln!("src:{port}.{member} needs src:{port} first");
                return ExitCode::FAILURE;
            };
            let source_node = terrain.nodes.get_mut(&feeding).expect("the source exists");
            let source_spec = catalogue.node(&source_node.type_name).expect("the source type is known");
            let parsed = parse_value(source_spec.parameter(member), value);
            source_node.values.insert(member.to_string(), parsed);
            continue;
        }
        if let Some(port) = key.strip_prefix("up:") {
            let Some(feeding) = node.port(port).and_then(|port| port.record.as_ref()).map(|record| record.from) else {
                eprintln!("up:{port} needs in:{port} first");
                return ExitCode::FAILURE;
            };
            add_source(&mut terrain, &mut engine, source_id, resolution, read_raw(value));
            let feeding_node = terrain.nodes.get_mut(&feeding).expect("the stand-in exists");
            feeding_node.ports[0].record = Some(connection(source_id, feeding, "In"));
            source_id += 1;
            continue;
        }
        if let Some(spec) = key.strip_prefix("mod:") {
            // mod:Type= adds a modifier; mod:Type.Member=value sets a member
            // (a Float2 as `x,y`).
            let (modifier_type, member) = match spec.split_once('.') {
                Some((modifier_type, member)) => (modifier_type, Some(member)),
                None => (spec, None),
            };
            let existing = node.modifiers.iter().position(|modifier| modifier.type_name == modifier_type);
            let position = match existing {
                Some(position) => position,
                None => {
                    let order = if matches!(modifier_type, "Max" | "Min") { 66 } else { 0 };
                    let mut modifier = ModifierData { type_name: modifier_type.to_string(), values: Default::default() };
                    modifier.values.insert("Order".to_string(), json!(order));
                    node.modifiers.push(modifier);
                    node.modifiers.len() - 1
                }
            };
            if let Some(member) = member {
                let parsed = match value.split_once(',') {
                    Some((x, y)) => json!({"X": x.parse::<f64>().unwrap_or(0.0), "Y": y.parse::<f64>().unwrap_or(0.0)}),
                    None => parse_loose(value),
                };
                node.modifiers[position].values.insert(member.to_string(), parsed);
            }
            continue;
        }
        if let Some((owner, path)) = key.split_once('.') {
            let mut target = node.values.get_mut(owner);
            let steps: Vec<&str> = path.split('.').collect();
            let (member, parents) = steps.split_last().expect("the path has a member");
            for step in parents {
                target = match step.parse::<usize>() {
                    Ok(position) => target.and_then(|value| value.get_mut(position)),
                    Err(_) => target.and_then(|value| value.get_mut(*step)),
                };
            }
            let Some(object) = target.and_then(Value::as_object_mut) else {
                eprintln!("{type_name} has no object at {key}");
                return ExitCode::FAILURE;
            };
            object.insert(member.to_string(), parse_loose(value));
            continue;
        }
        let parsed = parse_value(spec.parameter(key), value);
        node.values.insert(key.to_string(), parsed);
    }
    terrain.nodes.insert(NODE_ID, node);
    if has_automation {
        let automation_json = json!({"Variables": variables, "Bindings": bindings, "Expressions": expressions});
        let references = References::collect(&automation_json);
        engine.automation = Automation::from_json(&automation_json, &references);
    }

    // builds=N builds the node N times (for nodes that keep state between
    // builds); only the last build may not fail.
    let builds: usize = settings.iter().find(|(key, _)| *key == "builds").map(|(_, value)| value.parse().expect("builds should be a number")).unwrap_or(1);
    for earlier in 1..builds {
        if let BuildResult::Failed(error) = engine.build(&terrain, NODE_ID, resolution) {
            println!("build {earlier} failed: {error}");
        }
        engine.invalidate(&terrain, NODE_ID);
    }
    if builds == 0 {
        engine.mark_built(NODE_ID);
    } else {
        let started = std::time::Instant::now();
        let result = engine.build(&terrain, NODE_ID, resolution);
        if let BuildResult::Failed(error) = result {
            eprintln!("build failed: {error}");
            return ExitCode::from(2);
        }
        println!("built {type_name} at {size} in {} ms", started.elapsed().as_millis());
    }

    let mut outputs = Vec::new();
    for (port, map) in engine.cache.ports(NODE_ID) {
        let channel_count = map.channel_count();
        for channel in 0..channel_count {
            let file = if channel_count == 1 { format!("{port}.raw") } else { format!("{port}.c{channel}.raw") };
            write_raw(&output_directory.join(file), map.channel(channel));
        }
        outputs.push(json!({"port": port, "channels": channel_count, "intent": map.info.render_intent.name()}));
    }
    // What the viewport shows for the built node (`RendererBase.ResolveColorfield`).
    let custom_colours = if spec.has_flag("CustomColorization") { engine.custom_colour_map(&terrain, NODE_ID, resolution) } else { None };
    if let Some(colours) = custom_colours {
        let channel_count = colours.channel_count();
        for channel in 0..channel_count {
            let file = if channel_count == 1 { "CustomColorMap.raw".to_string() } else { format!("CustomColorMap.c{channel}.raw") };
            write_raw(&output_directory.join(file), colours.channel(channel));
        }
        outputs.push(json!({"port": "CustomColorMap", "channels": channel_count}));
    }
    if has_automation {
        for (name, variable) in &engine.automation.variables {
            let port = format!("Variable.{name}");
            write_raw(&output_directory.join(format!("{port}.raw")), &variable_output(&variable.value));
            outputs.push(json!({"port": port, "channels": 1}));
        }
    }
    let manifest = json!({"node": type_name, "resolution": size, "outputs": outputs});
    let manifest_text = serde_json::to_string_pretty(&manifest).expect("manifest serializes");
    fs::write(output_directory.join("manifest.json"), manifest_text).expect("manifest should be writable");
    println!("{}", serde_json::to_string(&manifest["outputs"]).expect("outputs serialize"));
    ExitCode::SUCCESS
}
