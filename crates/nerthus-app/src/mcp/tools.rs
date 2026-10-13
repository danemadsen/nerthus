//! The tools the server offers: their names, descriptions and input
//! schemas (`tools/list`), and which code runs each (`tools/call`).

use std::sync::OnceLock;

use serde_json::{json, Value};

use super::arguments::Arguments;
use super::{build_tools, catalogue_tools, edit_tools, graph_tools, project_tools};
use super::{Step, ToolContext, ToolResult};

/// What a tool does to the project, as the client is told.
#[derive(Clone, Copy)]
enum Effect {
    /// Changes nothing.
    Reads,
    /// Edits the project (undoable) or writes files.
    Edits,
    /// Can lose work: deletes nodes, or replaces the open project.
    Destroys,
}

/// The modifiers `set_modifier` takes.
pub(crate) const MODIFIERS: [&str; 19] = [
    "Autolevel", "Blur", "Clamp", "Clip", "Difference", "Drop", "Equalize", "Height", "Influence", "Invert", "MatchHeight", "Max", "Min", "Multiplier", "Shaper", "Slope", "Strong", "Threshold", "Warp",
];

fn tool(name: &str, title: &str, description: &str, properties: Value, required: &[&str], effect: Effect) -> Value {
    let mut schema = json!({"type": "object", "properties": properties, "additionalProperties": false});
    if !required.is_empty() {
        schema["required"] = json!(required);
    }
    let annotations = match effect {
        Effect::Reads => json!({"title": title, "readOnlyHint": true, "openWorldHint": false}),
        Effect::Edits => json!({"title": title, "readOnlyHint": false, "destructiveHint": false, "openWorldHint": false}),
        Effect::Destroys => json!({"title": title, "readOnlyHint": false, "destructiveHint": true, "openWorldHint": false}),
    };
    json!({"name": name, "title": title, "description": description, "inputSchema": schema, "annotations": annotations})
}

fn node_property(description: &str) -> Value {
    json!({"type": ["integer", "string"], "description": description})
}

fn point_property(description: &str) -> Value {
    json!({"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2, "description": description})
}

fn discard_property() -> Value {
    json!({"type": "boolean", "description": "Go ahead even though the open project has unsaved changes, losing them. Ask the user first."})
}

fn image_size_property(default: u32) -> Value {
    let description = format!("The picture's longer side in pixels at most (64 to 2048; {default} when left out).");
    json!({"type": "integer", "minimum": 64, "maximum": 2048, "description": description})
}

fn image_format_property(default: &str) -> Value {
    let description = format!("png or jpeg ({default} when left out).");
    json!({"type": "string", "enum": ["png", "jpeg"], "description": description})
}

fn project_tools() -> Vec<Value> {
    let none = json!({});
    let get_project = tool(
        "get_project",
        "Project Overview",
        "The open project: its file and whether it has unsaved changes, the terrain's size, graph tabs, node count, the selected and previewed node, the preview build, build settings, nodes marked for export, a Build and Export under way, undo history and the editor's latest status messages. Start here.",
        none,
        &[],
        Effect::Reads,
    );

    let open_properties = json!({
        "path": {"type": "string", "description": "The .terrain file's absolute path."},
        "discard_unsaved_changes": discard_property(),
    });
    let open_project = tool(
        "open_project",
        "Open Project",
        "Opens a .terrain project in the editor in place of the open one. Refuses while the open project has unsaved changes, unless discard_unsaved_changes is true.",
        open_properties,
        &["path"],
        Effect::Destroys,
    );

    let new_properties = json!({"discard_unsaved_changes": discard_property()});
    let new_project = tool(
        "new_project",
        "New Project",
        "Starts a new, empty project in the editor (File › New). Refuses while the open project has unsaved changes, unless discard_unsaved_changes is true.",
        new_properties,
        &[],
        Effect::Destroys,
    );

    let save_properties = json!({
        "path": {"type": "string", "description": "Where to save it (Save As); .terrain is added when missing. Leave out to save to the project's own file."},
    });
    let save_project = tool(
        "save_project",
        "Save Project",
        "Saves the project to its file, or to path. A project that was never saved needs a path.",
        save_properties,
        &[],
        Effect::Edits,
    );

    let settings_properties = json!({
        "terrain_width": {"type": "number", "description": "The terrain's real width in metres (100 to 2,400,000). Erosion and the other simulations are scaled to it."},
        "terrain_height": {"type": "number", "description": "The real height in metres that a height of 1 stands for (100 to 10,000)."},
        "preview_resolution": {"type": "integer", "enum": [512, 1024, 2048, 4096], "description": "The resolution nodes build at in the editor."},
        "build_resolution": {"type": "integer", "description": "The resolution Build and Export builds at (a power of two from 512 to 16384)."},
        "build_destination": {"type": "string", "description": "Where builds are written: a folder, or a template with tokens such as <Builds>, <FileLocation>, [Filename], [Date] and [+++] (a new numbered folder for each build). The default is <Builds>\\[Filename]\\[+++]."},
        "build_settings": {"type": "object", "description": "Other build settings by member name, e.g. {\"OrganizeFiles\": \"NodeSubFolder\", \"OverwriteMode\": \"Increment\"}; get_project shows them."},
    });
    let set_project_settings = tool(
        "set_project_settings",
        "Project Settings",
        "Changes the project's settings, each an undoable edit: the terrain's real-world size, the preview resolution, and Build and Export's resolution, destination and other settings.",
        settings_properties,
        &[],
        Effect::Edits,
    );

    vec![get_project, open_project, new_project, save_project, set_project_settings]
}

fn catalogue_tools() -> Vec<Value> {
    let categories: Vec<&str> = crate::ui::toolbox::CATEGORIES.into_iter().filter(|category| *category != "Macro").collect();
    let search_properties = json!({
        "query": {"type": "string", "description": "Words to look for in the types' names, keywords and descriptions, e.g. \"river erosion\"."},
        "category": {"type": "string", "enum": categories, "description": "Only the types in this toolbox category."},
    });
    let search_node_types = tool(
        "search_node_types",
        "Search Node Types",
        "Finds node types to add, by words in their names, keywords and descriptions, or the types in a toolbox category. With neither, lists every category with its types.",
        search_properties,
        &[],
        Effect::Reads,
    );

    let describe_properties = json!({
        "type": {"type": "string", "description": "The node type, e.g. \"Mountain\" or \"Erosion2\"."},
    });
    let describe_node_type = tool(
        "describe_node_type",
        "Describe Node Type",
        "A node type's description, its input and output ports, and its parameters with their types, ranges, options, defaults and descriptions.",
        describe_properties,
        &["type"],
        Effect::Reads,
    );

    vec![search_node_types, describe_node_type]
}

fn graph_tools() -> Vec<Value> {
    let list_properties = json!({
        "tab": {"type": "integer", "description": "Only the nodes on this graph tab (get_project lists the tabs)."},
    });
    let list_nodes = tool(
        "list_nodes",
        "List Nodes",
        "The nodes in the graph: id, name, type, graph tab, position, what feeds each input, outputs, state (bypassed, baked), whether it is marked for export, and its last build.",
        list_properties,
        &[],
        Effect::Reads,
    );

    let get_properties = json!({"node": node_property("The node's id, or its name when no other node has it.")});
    let get_node = tool(
        "get_node",
        "Get Node",
        "One node in full: its parameters (value, default, range, options, description; whether the property panel shows them as things stand; bound variables and expressions), its inputs and what feeds them, its outputs and what they feed, modifiers, export settings and last build.",
        get_properties,
        &["node"],
        Effect::Reads,
    );

    let select_properties = json!({
        "node": node_property("The node's id or unique name."),
        "port": {"type": "string", "description": "An output to show other than the main one (Out)."},
    });
    let select_node = tool(
        "select_node",
        "Select Node",
        "Selects a node in the editor, centres it in the graph and shows it in the viewport (one of its other outputs when port is given), so the user sees it.",
        select_properties,
        &["node"],
        Effect::Reads,
    );

    vec![list_nodes, get_node, select_node]
}

fn edit_tools() -> Vec<Value> {
    let add_properties = json!({
        "type": {"type": "string", "description": "The node type (search_node_types finds them)."},
        "name": {"type": "string", "description": "A name for the node; the type's name when left out."},
        "input_from": node_property("A node to feed the new node's main input from; the new node goes to its right."),
        "input_from_port": {"type": "string", "description": "The output of input_from to use; its main output when left out."},
        "parameters": {"type": "object", "description": "Parameter values to start with, by name (see set_parameters for the forms values take)."},
        "position": point_property("[x, y] on the graph's canvas; worked out when left out."),
        "tab": {"type": "integer", "description": "The graph tab to add it to; the one shown when left out."},
    });
    let add_node = tool(
        "add_node",
        "Add Node",
        "Adds a node as one undoable edit, with the type's default preset, then selects and previews it. Returns its id and ports.",
        add_properties,
        &["type"],
        Effect::Edits,
    );

    let set_properties = json!({
        "node": node_property("The node's id or unique name."),
        "parameters": {"type": "object", "description": "Values by parameter name, e.g. {\"Scale\": 0.6, \"Style\": \"Alpine\", \"Seed\": 1234}."},
    });
    let set_parameters = tool(
        "set_parameters",
        "Set Parameters",
        "Sets node parameters as one undoable edit, stored as the property panel stores them: numbers are clamped to the slider's range, enums take a member name, colours take \"#rrggbb\" (or scRGB [r, g, b]), ranges take [low, high], curves take [[x, y], …], gradients take [{\"color\": \"#rrggbb\", \"position\": 0 to 1}, …]. Returns the values as stored.",
        set_properties,
        &["node", "parameters"],
        Effect::Edits,
    );

    let update_properties = json!({
        "node": node_property("The node's id or unique name."),
        "name": {"type": "string", "description": "Its new name."},
        "position": point_property("Its new [x, y] on the graph's canvas."),
        "bypassed": {"type": "boolean", "description": "Bypass it (its input passes through unchanged), or stop bypassing it."},
    });
    let update_node = tool(
        "update_node",
        "Update Node",
        "Renames, moves or bypasses a node; each change is an undoable edit.",
        update_properties,
        &["node"],
        Effect::Edits,
    );

    let connect_properties = json!({
        "from": node_property("The node whose output to take."),
        "from_port": {"type": "string", "description": "That node's output; its main output (Out) when left out."},
        "to": node_property("The node to feed."),
        "to_port": {"type": "string", "description": "The input to feed; the first free input when left out (nodes such as Combine get another). \"Mask\" adds a mask input to nodes that take one."},
    });
    let connect_nodes = tool(
        "connect_nodes",
        "Connect Nodes",
        "Connects an output to an input as an undoable edit, in place of what fed that input. Feeding a node's main input passes the source's render intent on, as connecting in the editor does: fed a mask, the node renders as a mask. Refuses connections that would make a loop.",
        connect_properties,
        &["from", "to"],
        Effect::Edits,
    );

    let disconnect_properties = json!({
        "node": node_property("The node whose input to disconnect."),
        "port": {"type": "string", "description": "The input."},
    });
    let disconnect_input = tool(
        "disconnect_input",
        "Disconnect Input",
        "Removes the connection feeding one of a node's inputs, as an undoable edit.",
        disconnect_properties,
        &["node", "port"],
        Effect::Edits,
    );

    let delete_properties = json!({
        "nodes": {"type": "array", "items": {"type": ["integer", "string"]}, "minItems": 1, "description": "The nodes' ids or unique names."},
    });
    let delete_nodes = tool(
        "delete_nodes",
        "Delete Nodes",
        "Deletes nodes as one undoable edit. A single node's input source is joined to whatever it fed, as the editor's Delete does; several nodes go with their connections.",
        delete_properties,
        &["nodes"],
        Effect::Destroys,
    );

    let modifier_properties = json!({
        "node": node_property("The node's id or unique name."),
        "modifier": {"type": "string", "enum": MODIFIERS, "description": "The modifier: Height and Slope mask by height or slope, Multiplier remaps heights, Min and Max exclude each other."},
        "enabled": {"type": "boolean", "description": "false removes the modifier; true (or left out) adds it when the node lacks it."},
        "parameters": {"type": "object", "description": "The modifier's parameters by name (describe in get_node once added), e.g. {\"Factor\": 0.5}."},
    });
    let set_modifier = tool(
        "set_modifier",
        "Set Modifier",
        "Adds, configures or removes one of a node's modifiers (adjustments applied to its output), each change an undoable edit.",
        modifier_properties,
        &["node", "modifier"],
        Effect::Edits,
    );

    let steps = json!({"steps": {"type": "integer", "minimum": 1, "maximum": 100, "description": "How many edits (1 when left out)."}});
    let undo = tool("undo", "Undo", "Undoes the latest edits, the user's included (Ctrl Z).", steps.clone(), &[], Effect::Edits);
    let redo = tool("redo", "Redo", "Redoes edits that were undone.", steps, &[], Effect::Edits);

    vec![add_node, set_parameters, update_node, connect_nodes, disconnect_input, delete_nodes, set_modifier, undo, redo]
}

fn variable_tools() -> Vec<Value> {
    let list_variables = tool(
        "list_variables",
        "List Variables",
        "The project's variables (the Data View's Vars): key, name, type, value, limits or options, and the node parameters bound to each.",
        json!({}),
        &[],
        Effect::Reads,
    );

    let set_properties = json!({
        "variable": {"type": "string", "description": "The variable's key or name."},
        "value": {"description": "Its value: a number (Float, Int), [low, high] (Range), an option's name or index (Choice), true or false (Bool), \"#rrggbb\" (Color), or text."},
    });
    let set_variable = tool(
        "set_variable",
        "Set Variable",
        "Sets a variable's value as an undoable edit; the parameters bound to it follow on the next build.",
        set_properties,
        &["variable", "value"],
        Effect::Edits,
    );

    vec![list_variables, set_variable]
}

fn build_tools() -> Vec<Value> {
    let build_properties = json!({
        "node": node_property("The node's id or unique name."),
        "resolution": {"type": "integer", "enum": [512, 1024, 2048, 4096], "description": "The preview resolution to build at (it stays the editor's preview resolution); the current one when left out."},
    });
    let build_node = tool(
        "build_node",
        "Build Node",
        "Builds a node and everything feeding it at the preview resolution, previewing it in the editor, and reports each output's size and value range (min, max, mean) and any errors on the way. Waits for the build.",
        build_properties,
        &["node"],
        Effect::Reads,
    );

    let image_properties = json!({
        "node": node_property("The node's id or unique name."),
        "port": {"type": "string", "description": "The output to show; the main one (Out) when left out."},
        "style": {"type": "string", "enum": ["rendered", "data"], "description": "rendered (when left out): as the 2D viewport shows it, heightfields shaded by the viewport's render mode and lighting, masks and colours as they are. data: the values themselves, one channel in grey from its lowest value to its highest, colour maps as colours."},
        "size": image_size_property(512),
        "format": image_format_property("jpeg"),
    });
    let get_node_image = tool(
        "get_node_image",
        "Node Image",
        "Builds a node as build_node does and returns a top-down picture of one of its outputs, with the output's value range.",
        image_properties,
        &["node"],
        Effect::Reads,
    );

    let capture_properties = json!({
        "view": {"type": "string", "enum": ["3d", "window"], "description": "3d (when left out): the 3D viewport, drawn again without the interface over it once the preview is up to date. window: the whole editor window."},
        "size": image_size_property(1024),
        "format": image_format_property("jpeg for 3d, png for window"),
    });
    let capture_view = tool(
        "capture_view",
        "Capture View",
        "A picture of what the user sees: the previewed node in the 3D viewport (select_node chooses it), or the whole editor window.",
        capture_properties,
        &[],
        Effect::Reads,
    );

    let formats: Vec<&str> = super::build_tools::FORMATS.iter().map(|(name, _)| *name).collect();
    let export_properties = json!({
        "node": node_property("The node's id or unique name."),
        "port": {"type": "string", "description": "The output to write; the main one (Out) when left out."},
        "path": {"type": "string", "description": "The file to write (absolute); the format's extension is added when missing."},
        "format": {"type": "string", "enum": formats, "description": "The file format."},
    });
    let export_node_map = tool(
        "export_node_map",
        "Export Node Map",
        "Builds a node at the preview resolution and writes one of its outputs to a file, as Build and Export would write it, without saving or building the whole project.",
        export_properties,
        &["node", "path", "format"],
        Effect::Edits,
    );

    let mark_properties = json!({
        "node": node_property("The node's id or unique name."),
        "enabled": {"type": "boolean", "description": "false unmarks it; true (or left out) marks it."},
        "format": {"type": "string", "enum": formats, "description": "The file format its outputs are written in; the preferences' heightfield format when first marked."},
        "file_name": {"type": "string", "description": "The file name its outputs are written under (without extension); the node's name when first marked."},
    });
    let mark_for_export = tool(
        "mark_for_export",
        "Mark for Export",
        "Marks a node for Build and Export (or unmarks it), with its file format and file name, as an undoable edit.",
        mark_properties,
        &["node"],
        Effect::Edits,
    );

    let export_build_properties = json!({
        "wait": {"type": "boolean", "description": "Wait for the build to finish (when left out), or return once it has started (get_project follows it)."},
    });
    let build_and_export = tool(
        "build_and_export",
        "Build and Export",
        "Saves the project, then builds every node marked for export at the build resolution and writes their files into a new build folder (Project › Build and Export). Needs a saved project. Returns the folder and the files written.",
        export_build_properties,
        &[],
        Effect::Edits,
    );

    vec![build_node, get_node_image, capture_view, export_node_map, mark_for_export, build_and_export]
}

/// Every tool, as `tools/list` gives them.
pub(crate) fn definitions() -> &'static [Value] {
    static DEFINITIONS: OnceLock<Vec<Value>> = OnceLock::new();
    DEFINITIONS.get_or_init(|| {
        let mut all = project_tools();
        all.extend(catalogue_tools());
        all.extend(graph_tools());
        all.extend(edit_tools());
        all.extend(variable_tools());
        all.extend(build_tools());
        all
    })
}

pub(crate) fn exists(name: &str) -> bool {
    definitions().iter().any(|tool| tool["name"] == name)
}

/// Starts a call: its answer, or what it waits for. Problems with it
/// become tool errors, for the model to read.
pub(crate) fn start(name: &str, arguments: &Value, context: &mut ToolContext) -> Step {
    let outcome = Arguments::of(arguments).and_then(|arguments| run(name, &arguments, context));
    match outcome {
        Ok(step) => step,
        Err(problem) => Step::Done(ToolResult::error(problem)),
    }
}

fn run(name: &str, arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    match name {
        "get_project" => project_tools::get_project(context),
        "open_project" => project_tools::open_project(arguments, context),
        "new_project" => project_tools::new_project(arguments, context),
        "save_project" => project_tools::save_project(arguments, context),
        "set_project_settings" => project_tools::set_project_settings(arguments, context),
        "search_node_types" => catalogue_tools::search_node_types(arguments),
        "describe_node_type" => catalogue_tools::describe_node_type(arguments),
        "list_nodes" => graph_tools::list_nodes(arguments, context),
        "get_node" => graph_tools::get_node(arguments, context),
        "select_node" => graph_tools::select_node(arguments, context),
        "add_node" => edit_tools::add_node(arguments, context),
        "set_parameters" => edit_tools::set_parameters(arguments, context),
        "update_node" => edit_tools::update_node(arguments, context),
        "connect_nodes" => edit_tools::connect_nodes(arguments, context),
        "disconnect_input" => edit_tools::disconnect_input(arguments, context),
        "delete_nodes" => edit_tools::delete_nodes(arguments, context),
        "set_modifier" => edit_tools::set_modifier(arguments, context),
        "undo" => edit_tools::undo(arguments, context, false),
        "redo" => edit_tools::undo(arguments, context, true),
        "list_variables" => project_tools::list_variables(context),
        "set_variable" => project_tools::set_variable(arguments, context),
        "build_node" => build_tools::build_node(arguments, context),
        "get_node_image" => build_tools::get_node_image(arguments, context),
        "capture_view" => build_tools::capture_view(arguments, context),
        "export_node_map" => build_tools::export_node_map(arguments, context),
        "mark_for_export" => build_tools::mark_for_export(arguments, context),
        "build_and_export" => build_tools::build_and_export(arguments, context),
        _ => Err(format!("Unknown tool: {name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_runs_somewhere_and_has_a_schema() {
        let names: Vec<&str> = definitions().iter().filter_map(|tool| tool["name"].as_str()).collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len());
        for tool in definitions() {
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert!(tool["description"].as_str().is_some_and(|text| !text.is_empty()));
            // No schema names a property it does not describe.
            let properties = tool["inputSchema"]["properties"].as_object().unwrap();
            for required in tool["inputSchema"]["required"].as_array().into_iter().flatten() {
                assert!(properties.contains_key(required.as_str().unwrap()), "{}", tool["name"]);
            }
        }
        assert!(exists("add_node"));
        assert!(!exists("nonsense"));
    }

    #[test]
    fn modifiers_offered_are_in_the_catalogue() {
        for name in MODIFIERS {
            assert!(nerthus_engine::Catalogue::get().modifier(name).is_some(), "{name}");
        }
    }
}
