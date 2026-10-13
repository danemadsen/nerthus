//! The tools the server offers: their names, descriptions and input
//! schemas (`tools/list`), and which code runs each (`tools/call`).

use std::sync::OnceLock;

use serde_json::{json, Value};

use super::arguments::Arguments;
use super::{authoring_tools, build_tools, catalogue_tools, edit_tools, file_tools, graph_tools, project_tools, review_tools, run_tools};
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
        "type": {"type": "string", "description": "The node type, e.g. \"Mountain\" or \"ErosionPro\" (the names .terrain files use, such as Erosion2 or SatMap, work too)."},
    });
    let describe_node_type = tool(
        "describe_node_type",
        "Describe Node Type",
        "A node type's description, keywords and short code, its input and output ports, whether it is a generator or needs baking, and its parameters with their types, ranges, curves, options, defaults and descriptions. Also how the example projects Nerthus comes with use it (the values they chose, what usually feeds it and what it usually feeds), and what calibrate_nodes measured it to make.",
        describe_properties,
        &["type"],
        Effect::Reads,
    );

    let modifier_properties = json!({
        "modifier": {"type": "string", "description": "The modifier, e.g. \"Height\"; left out (or \"*\"), every modifier."},
        "label": {"type": "string", "description": "A parameter's label as the property panel shows it, e.g. \"Height Remap\", to find the modifier it belongs to."},
    });
    let describe_modifier = tool(
        "describe_modifier",
        "Describe Modifier",
        "Node modifiers (the adjustments set_modifier adds to a node's output): what each does, whether it works off the node's input (and so makes nothing on a node nothing feeds), and its parameters with ranges and defaults. Give modifier for one, label to find one by a parameter's label, or neither for all.",
        modifier_properties,
        &[],
        Effect::Reads,
    );

    let suggest_properties = json!({
        "node": node_property("A node in the graph: what usually follows it in the example projects."),
        "context": {"type": "string", "description": "What the terrain is meant to be, e.g. \"desert canyon\" or \"alpine glacier\": node types that fit it."},
        "current_nodes": {"type": "array", "items": {"type": "string"}, "description": "Node types to judge instead of the open project's (for a graph being planned)."},
        "limit": {"type": "integer", "minimum": 1, "maximum": 30, "description": "How many types each list names at most (8 when left out)."},
    });
    let suggest_nodes = tool(
        "suggest_nodes",
        "Suggest Nodes",
        "Node types to add next: the stages the open project (or current_nodes) lacks (a generator, erosion, colour, an output), what usually follows a given node in the example projects, and types that fit a description of the terrain.",
        suggest_properties,
        &[],
        Effect::Reads,
    );

    vec![search_node_types, describe_node_type, describe_modifier, suggest_nodes]
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

    let camera_properties = json!({
        "look_at": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3, "description": "The point the camera looks at, [x, y, z]: x east and z north, the terrain spanning -10 to 10; y up (0 to 20)."},
        "inclination": {"type": "number", "minimum": 0.001, "maximum": 1.0, "description": "From looking straight down (0) to looking at the horizon (1)."},
        "zoom": {"type": "number", "minimum": 0.4, "maximum": 250.0, "description": "The camera's distance from the point it looks at (the terrain is 20 across; the default view is 50)."},
        "rotation": {"type": "number", "minimum": -1.0, "maximum": 1.0, "description": "The heading, in turns clockwise from north."},
        "reset": {"type": "boolean", "description": "Start from the default view instead of the current one."},
    });
    let set_camera = tool(
        "set_camera",
        "Set Camera",
        "Moves the 3D viewport's camera; values left out keep the current view's. The project keeps the view as its camera (Reset to Loaded returns to it), and saving draws its thumbnail from it. Returns the camera.",
        camera_properties,
        &[],
        Effect::Edits,
    );

    vec![list_nodes, get_node, select_node, set_camera]
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

fn graph_spec_properties() -> (Value, Value) {
    let node = json!({
        "type": "object",
        "properties": {
            "id": {"type": ["integer", "string"], "description": "What connections call the node (its number in the list when left out)."},
            "type": {"type": "string", "description": "The node type."},
            "name": {"type": "string"},
            "position": {"description": "[x, y] on the graph's canvas; laid out along the connections when left out."},
            "parameters": {"type": "object", "description": "Parameter values by name, as set_parameters takes them."},
            "modifiers": {"description": "Modifiers to add: [\"Autolevel\", {\"modifier\": \"Height\", \"parameters\": {…}}] or {\"Height\": {…}}."},
            "export": {"description": "true, or {\"file_name\", \"format\"}: mark the node for Build and Export."},
            "bypassed": {"type": "boolean"}
        },
        "required": ["type"]
    });
    let connection = json!({
        "type": "object",
        "properties": {
            "from": {"type": ["integer", "string"], "description": "A node of the list by its id, or a node of the project."},
            "from_port": {"type": "string", "description": "Its output; Out when left out."},
            "to": {"type": ["integer", "string"]},
            "to_port": {"type": "string", "description": "The input; the first free one when left out (\"Mask\" adds a mask input)."}
        },
        "required": ["from", "to"]
    });
    let nodes = json!({"type": "array", "items": node, "minItems": 1, "description": "The nodes to make."});
    let connections = json!({"type": "array", "items": connection, "description": "Connections between them (or to the project's nodes); [from, to] pairs work too."});
    (nodes, connections)
}

fn authoring_tools() -> Vec<Value> {
    let (nodes, connections) = graph_spec_properties();
    let create_properties = json!({
        "nodes": nodes,
        "connections": connections,
        "new_project": {"type": "boolean", "description": "Start a new project for it (as new_project) instead of adding to the open one."},
        "discard_unsaved_changes": discard_property(),
        "tab": {"type": "integer", "description": "The graph tab to add to; the one shown when left out."},
        "build_settings": {"type": "object", "description": "Build settings by member name, e.g. {\"Resolution\": 2048, \"ColorSpace\": \"sRGB\"}."},
        "save_path": {"type": "string", "description": "Save the project here (absolute) once made."},
    });
    let create_graph = tool(
        "create_graph",
        "Create Graph",
        "Makes a whole graph in one undoable edit: many nodes with their parameters, modifiers and export marks, and the connections between them (and to nodes already there). Everything is checked first, so a mistake changes nothing. Returns each node's id. The quickest way to build a project from a plan.",
        create_properties,
        &["nodes"],
        Effect::Edits,
    );
    let list_examples = tool(
        "list_examples",
        "List Examples",
        "The example projects Nerthus comes with (alpine valley, volcanic isle, desert dunes, rivers, glaciers…): each one's node types in build order and what it exports. open_example starts from one.",
        json!({}),
        &[],
        Effect::Reads,
    );
    let open_properties = json!({
        "name": {"type": "string", "description": "The example's name, e.g. \"Volcanic Isle\" (list_examples lists them)."},
        "parameters": {"type": "object", "description": "Changes to make once it is open: {\"node name or id\": {\"Parameter\": value}}."},
        "discard_unsaved_changes": discard_property(),
        "save_path": {"type": "string", "description": "Save it here (absolute) once open."},
    });
    let open_example = tool(
        "open_example",
        "Open Example",
        "Opens an example project as an untitled copy to start from (File › Examples), optionally changing parameters and saving it. Refuses while the open project has unsaved changes, unless discard_unsaved_changes is true.",
        open_properties,
        &["name"],
        Effect::Destroys,
    );
    vec![create_graph, list_examples, open_example]
}

fn review_tools() -> Vec<Value> {
    let (nodes, connections) = graph_spec_properties();
    let validate_properties = json!({
        "path": {"type": "string", "description": "Check this .terrain file (absolute) instead of the open project."},
        "nodes": nodes,
        "connections": connections,
        "strict": {"type": "boolean", "description": "Also count nodes connected to nothing and a graph that exports nothing as errors."},
        "fix": {"type": "boolean", "description": "Fix what is safe to fix in the open project, as one undoable edit: values out of range or of the wrong form, missing ports, duplicate export names, an unnamed build type."},
        "runtime_check": {"type": "boolean", "description": "Also build it at 512 into a scratch folder (what feeds nothing, when nothing is marked for export) and report what fails."},
    });
    let validate_graph = tool(
        "validate_graph",
        "Validate Graph",
        "Looks the open project over (or a .terrain file, or a graph given as nodes and connections as create_graph takes them, without making it) for what stops it building or makes it build nothing useful: unknown types, required inputs not connected, bad connections, loops, values out of range, modifiers with no input to work off, settings the panel hides, nothing to export, what Build and Export would refuse. Can fix what is safe to fix, and build it to be sure.",
        validate_properties,
        &[],
        Effect::Edits,
    );
    let analyze_properties = json!({
        "terrain_type": {"type": "string", "enum": crate::mcp::review_tools::terrain_kinds(), "description": "The kind of terrain meant, for suggestions that fit it (general when left out)."},
    });
    let analyze_graph = tool(
        "analyze_graph",
        "Analyze Graph",
        "How the open project's graph is put together: nodes by category, generators, ends, exports, nodes no export uses, the longest chain, patterns (erosion stages, masks, data maps, combining, simulations, variables), the slowest nodes built so far, a complexity score, and what a terrain of the kind asked for usually adds.",
        analyze_properties,
        &[],
        Effect::Reads,
    );
    let optimize_properties = json!({
        "mode": {"type": "string", "enum": ["performance", "quality", "balanced"], "description": "performance: fastest processing; quality: finest; balanced (when left out): each type's default."},
        "nodes": {"type": "array", "items": {"type": ["integer", "string"]}, "description": "Only these nodes; all when left out."},
        "apply": {"type": "boolean", "description": "Make the changes (when left out), or only list them."},
    });
    let optimize_parameters = tool(
        "optimize_parameters",
        "Optimize Parameters",
        "Sets how finely nodes process (each node's Quality and HighQuality settings) for speed or for quality, as one undoable edit, leaving what they make alone; suggests a preview resolution to match. Lists what it changes.",
        optimize_properties,
        &[],
        Effect::Edits,
    );
    vec![validate_graph, analyze_graph, optimize_parameters]
}

fn run_tools() -> Vec<Value> {
    let run_properties = json!({
        "path": {"type": "string", "description": "The .terrain project to build (absolute); the open project's saved file when left out."},
        "resolution": {"type": "integer", "description": "The build resolution (the project's when left out)."},
        "build_path": {"type": "string", "description": "The folder to build into (absolute), in place of the project's destination."},
        "profile": {"type": "string", "description": "Build with this build profile (Build Settings › Profiles)."},
        "region": {"type": "string", "description": "Build this region of the terrain."},
        "seed": {"type": "integer", "minimum": 0, "description": "Build a mutation: every node's seed moved on by this (0, none)."},
        "variables": {"type": "object", "description": "Variables set for this build, by name, e.g. {\"Height\": 0.6}."},
        "target_node": {"type": ["integer", "string"], "description": "Export only this node (by id or name), marked for it when it is not."},
        "build_settings": {"type": "object", "description": "Build settings changed for this build, by member name, e.g. {\"Type\": \"Hybrid\"}."},
        "ignore_cache": {"type": "boolean", "description": "Build baked nodes instead of reading their bakes."},
        "silent": {"type": "boolean", "description": "Leave no report in the build folder."},
        "use_unsaved_changes": {"type": "boolean", "description": "For the open project: build it as it is in the editor, unsaved changes and all."},
        "timeout": {"type": "integer", "description": "Seconds to wait at most (4 hours when left out); the build stops then."},
    });
    let run_project = tool(
        "run_project",
        "Run Project",
        "Builds a saved project and writes its exports as nerthus-build does, on a thread of its own without touching the editor, with the command line's options: resolution, build folder, profile, region, mutation seed, variables, one node only, settings. Returns the folder, the files written with their sizes, the nodes that failed, and what the first heightfield written looks like. Waits for the build.",
        run_properties,
        &[],
        Effect::Edits,
    );
    let calibrate_properties = json!({
        "nodes": {"type": "array", "items": {"type": "string"}, "description": "Node types to measure; every generator of the Primitive and Terrain categories when left out."},
        "property": {"type": "string", "description": "A parameter to sweep across its range, e.g. \"Height\"; the defaults only when left out."},
        "steps": {"type": "integer", "minimum": 2, "maximum": 9, "description": "Points along the sweep, both ends included (3 when left out)."},
        "seeds": {"type": "array", "items": {"type": "integer"}, "description": "Seeds to build each point with ([1001, 2002, 3003] when left out); their spread shows how much the seed alone moves the result."},
        "resolution": {"type": "integer", "enum": [256, 512, 1024, 2048], "description": "512 when left out."},
    });
    let calibrate_nodes = tool(
        "calibrate_nodes",
        "Calibrate Nodes",
        "Builds generator types on their own and measures what they make: the share of the height range each output spans and its mean height, at the defaults or across a parameter's range, over several seeds. Answers \"how tall does this come out\", which no parameter range can. Keeps the measurements in the data folder; describe_node_type reports them. Waits for the builds.",
        calibrate_properties,
        &[],
        Effect::Edits,
    );
    let history_properties = json!({"limit": {"type": "integer", "minimum": 1, "maximum": 200, "description": "How many of the latest builds to list (10 when left out)."}});
    let build_history = tool(
        "build_history",
        "Build History",
        "The builds these tools ran this session (build_node, get_node_image, build_and_export, run_project, runtime checks, calibrations): when, what, resolution, time, files, problems; with counts of successes and failures and the average time.",
        history_properties,
        &[],
        Effect::Reads,
    );
    let analyze_build_properties = json!({
        "build_dir": {"type": "string", "description": "The build folder (absolute); the latest build's when left out."},
        "expected_files": {"type": "array", "items": {"type": "string"}, "description": "File names that should be there; each is reported present or missing."},
    });
    let analyze_build = tool(
        "analyze_build",
        "Analyze Build",
        "Reads a build folder: the files with their sizes, the build report (result, time, slowest nodes), the nodes that failed and the first fault in CRASH_LOG.txt (everything after it fails only for want of input), expected files, and what the first heightfield looks like.",
        analyze_build_properties,
        &[],
        Effect::Reads,
    );
    let analyze_terrain_properties = json!({
        "node": node_property("A node to build (as build_node does) and read."),
        "port": {"type": "string", "description": "The node's output to read; its main one when left out."},
        "path": {"type": "string", "description": "A heightfield file to read instead (absolute): .r16, .raw, .r32, .graw, PNG, TIFF, EXR…"},
        "encoding": {"type": "string", "enum": ["ushort", "half", "float"], "description": "How a .raw or .r16 file stores its heights, when not 16-bit whole numbers (a .raw of floats is told by its size)."},
        "metres_across": {"type": "number", "description": "The terrain's width in metres (the project's when left out)."},
        "metres_up": {"type": "number", "description": "The height a 1 stands for, in metres (the project's when left out)."},
        "gentle_degrees": {"type": "number", "description": "The steepest slope counted as gentle (5 when left out)."},
        "centre_fraction": {"type": "number", "minimum": 0.1, "maximum": 1, "description": "The share of each side taken as the middle; the rest is the edge (0.6 when left out)."},
    });
    let analyze_terrain = tool(
        "analyze_terrain",
        "Analyze Terrain",
        "Reads a heightfield as land: a node's output, or a file a build wrote. Relief and heights in metres, how much of the range it fills, how much of the middle and of the whole is gentle, the mean slope, whether the edges stand above the middle (a basin) or below it (a mound), the split into low, middle and high ground, and a height profile along each axis. Says outright when it is flat.",
        analyze_terrain_properties,
        &[],
        Effect::Reads,
    );
    vec![run_project, calibrate_nodes, build_history, analyze_build, analyze_terrain]
}

fn file_tools() -> Vec<Value> {
    let get_properties = json!({
        "path": {"type": "string", "description": "A .terrain file (absolute); the open project as it would be saved now when left out."},
        "encoding": {"type": "string", "enum": ["raw", "base64"], "description": "raw (the JSON text, when left out) or base64."},
    });
    let get_project_file = tool(
        "get_project_file",
        "Get Project File",
        "A project's .terrain text: the open project's as saving would write it now, or a file's.",
        get_properties,
        &[],
        Effect::Reads,
    );
    let list_properties = json!({
        "directory": {"type": "string", "description": "List the projects in this folder (absolute)."},
        "recursive": {"type": "boolean", "description": "Look in its folders too."},
    });
    let list_projects = tool(
        "list_projects",
        "List Projects",
        "Project files, newest first, with their sizes and when they changed: those in a folder, or (with no folder) the recent projects, the projects beside the open one, and the examples.",
        list_properties,
        &[],
        Effect::Reads,
    );
    let repair_properties = json!({
        "path": {"type": "string", "description": "The .terrain file (absolute); not the open project's."},
        "create_backup": {"type": "boolean", "description": "Copy it to .terrain.backup first (when left out)."},
        "write": {"type": "boolean", "description": "Write the repairs (when left out), or only list them."},
        "drop_unknown_nodes": {"type": "boolean", "description": "Write it even though it has nodes of types Nerthus does not know, leaving them out."},
    });
    let repair_project_file = tool(
        "repair_project_file",
        "Repair Project File",
        "Repairs a .terrain file without opening it in the editor, as opening and saving it would plus the safe fixes validate_graph makes: retired names and old node versions migrated, export marks given their node, missing ports added, values brought into range, duplicate export names renamed, the build type named. Leaves the file alone when there is nothing to repair; reports what is still wrong.",
        repair_properties,
        &["path"],
        Effect::Edits,
    );
    let patch_properties = json!({
        "path": {"type": "string", "description": "The .terrain file (absolute); not the open project's."},
        "patches": {"type": "array", "description": "One entry per node: {\"node\": id or name, \"parameters\": {…}, \"export\": true, false or {\"file_name\", \"format\", \"enabled\"}, \"bypassed\": bool, \"name\": text}.", "items": {"type": "object"}},
        "build_settings": {"type": "object", "description": "Build settings by member name."},
        "create_backup": {"type": "boolean", "description": "Copy it to .terrain.backup first (when left out)."},
    });
    let patch_project_file = tool(
        "patch_project_file",
        "Patch Project File",
        "Changes a .terrain file in place without opening it in the editor: node parameters (checked and stored as the property panel stores them), export marks, names, bypassing and build settings. Changes that are wrong are rejected and the rest made; the file is written only when something changed.",
        patch_properties,
        &["path"],
        Effect::Edits,
    );
    vec![get_project_file, list_projects, repair_project_file, patch_project_file]
}

/// Every tool, as `tools/list` gives them.
pub(crate) fn definitions() -> &'static [Value] {
    static DEFINITIONS: OnceLock<Vec<Value>> = OnceLock::new();
    DEFINITIONS.get_or_init(|| {
        let mut all = project_tools();
        all.extend(catalogue_tools());
        all.extend(graph_tools());
        all.extend(edit_tools());
        all.extend(authoring_tools());
        all.extend(review_tools());
        all.extend(variable_tools());
        all.extend(build_tools());
        all.extend(run_tools());
        all.extend(file_tools());
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
        "describe_modifier" => catalogue_tools::describe_modifier(arguments),
        "suggest_nodes" => catalogue_tools::suggest_nodes(arguments, context),
        "list_nodes" => graph_tools::list_nodes(arguments, context),
        "get_node" => graph_tools::get_node(arguments, context),
        "select_node" => graph_tools::select_node(arguments, context),
        "set_camera" => graph_tools::set_camera(arguments, context),
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
        "create_graph" => authoring_tools::create_graph(arguments, context),
        "list_examples" => authoring_tools::list_examples(arguments),
        "open_example" => authoring_tools::open_example(arguments, context),
        "validate_graph" => review_tools::validate_graph(arguments, context),
        "analyze_graph" => review_tools::analyze_graph(arguments, context),
        "optimize_parameters" => review_tools::optimize_parameters(arguments, context),
        "run_project" => run_tools::run_project(arguments, context),
        "calibrate_nodes" => run_tools::calibrate_nodes(arguments, context),
        "build_history" => run_tools::build_history(arguments, context),
        "analyze_build" => run_tools::analyze_build(arguments, context),
        "analyze_terrain" => run_tools::analyze_terrain(arguments, context),
        "get_project_file" => file_tools::get_project_file(arguments, context),
        "list_projects" => file_tools::list_projects(arguments, context),
        "repair_project_file" => file_tools::repair_project_file(arguments, context),
        "patch_project_file" => file_tools::patch_project_file(arguments, context),
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

    /// An editor and what tools work with, driven through `start` as the
    /// server drives them.
    struct Harness {
        editor: crate::editor::Editor,
        ui_state: crate::ui::UiState,
        screenshots: nerthus_render::Screenshots,
        next_capture: u64,
        history: super::super::jobs::BuildHistory,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                editor: crate::editor::Editor::new(crate::document::Document::new()),
                ui_state: crate::ui::UiState::default(),
                screenshots: nerthus_render::Screenshots::default(),
                next_capture: 0,
                history: super::super::jobs::BuildHistory::default(),
            }
        }

        /// The call's answer, waited for; its JSON, or the error's text.
        fn call(&mut self, name: &str, arguments: Value) -> Result<Value, String> {
            let mut context = ToolContext {
                editor: &mut self.editor,
                ui_state: &mut self.ui_state,
                screenshots: &mut self.screenshots,
                next_capture: &mut self.next_capture,
                history: &mut self.history,
            };
            let mut step = start(name, &arguments, &mut context);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
            let result = loop {
                match step {
                    Step::Done(result) => break result,
                    Step::Wait(mut wait) => loop {
                        assert!(std::time::Instant::now() < deadline, "{name} took too long");
                        if let Some(next) = wait.poll(&mut context) {
                            step = next;
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    },
                }
            };
            let text = result.content[0]["text"].as_str().unwrap_or_default().to_string();
            if result.is_error {
                return Err(text);
            }
            Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
        }
    }

    #[test]
    fn the_new_tools_work_end_to_end() {
        let folder = std::env::temp_dir().join(format!("nerthus-mcp-tools-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let mut harness = Harness::new();

        // A graph in one call, named as .terrain files name the types.
        let created = harness
            .call(
                "create_graph",
                json!({
                    "nodes": [
                        {"id": "base", "type": "Mountain", "parameters": {"Scale": 0.7}},
                        {"id": "erode", "type": "Erosion2", "modifiers": ["Autolevel"]},
                        {"id": "colour", "type": "SatMap"}
                    ],
                    "connections": [{"from": "base", "to": "erode"}, ["erode", "colour"]],
                    "build_settings": {"Resolution": 512}
                }),
            )
            .unwrap();
        let ids: Vec<i64> = created["created"].as_array().unwrap().iter().map(|node| node["id"].as_i64().unwrap()).collect();
        assert_eq!(ids.len(), 3);
        assert_eq!(harness.editor.document.undo_labels()[0], "Create 3 nodes");
        // A bad graph changes nothing.
        let nodes_before = harness.editor.document.terrain().nodes.len();
        let refused = harness.call("create_graph", json!({"nodes": [{"type": "Mountain"}], "connections": [{"from": 1, "to": 77777}]}));
        assert!(refused.is_err());
        assert_eq!(harness.editor.document.terrain().nodes.len(), nodes_before);

        // Nothing marked for export: a warning; a proposed graph is checked
        // without being made.
        let checked = harness.call("validate_graph", json!({})).unwrap();
        assert_eq!(checked["valid"], json!(true), "{checked}");
        assert!(checked["warnings"].to_string().contains("export"));
        let proposed = harness.call("validate_graph", json!({"nodes": [{"type": "Blur"}]})).unwrap();
        assert_eq!(proposed["valid"], json!(false), "{proposed}");
        assert_eq!(harness.editor.document.terrain().nodes.len(), nodes_before);

        let analysed = harness.call("analyze_graph", json!({"terrain_type": "mountain"})).unwrap();
        assert_eq!(analysed["nodes"], json!(3));
        assert!(!analysed["patterns"].as_array().unwrap().is_empty(), "{analysed}");
        let optimized = harness.call("optimize_parameters", json!({"mode": "quality", "apply": false})).unwrap();
        assert_eq!(optimized["applied"], json!(false));
        let suggested = harness.call("suggest_nodes", json!({"node": ids[0], "context": "snowy alpine peaks"})).unwrap();
        assert!(suggested["for_context"].as_array().is_some_and(|types| !types.is_empty()), "{suggested}");
        let modifier = harness.call("describe_modifier", json!({"label": "Height Remap"})).unwrap();
        assert_eq!(modifier["modifier"], json!("Multiplier"));
        let described = harness.call("describe_node_type", json!({"type": "Erosion2"})).unwrap();
        assert_eq!(described["type"], json!("ErosionPro"));
        assert_eq!(described["file_type_name"], json!("Erosion2"));

        // Marked and saved, the project builds on its own thread.
        harness.call("mark_for_export", json!({"node": ids[1], "format": "ushort_r16"})).unwrap();
        let path = folder.join("Tools.terrain");
        harness.call("save_project", json!({"path": path})).unwrap();
        let text = harness.call("get_project_file", json!({})).unwrap();
        assert!(text["content"].as_str().unwrap().contains("Erosion2"));
        let ran = harness.call("run_project", json!({"build_path": folder.join("build"), "resolution": 256, "seed": 2})).unwrap();
        let files = ran["files"].as_array().unwrap();
        // ErosionPro writes each of its outputs: Out, Flow, Wear, Deposits.
        assert_eq!(files.len(), 4, "{ran}");
        assert!(ran["terrain"]["file"].as_str().unwrap().ends_with("_Out.r16"), "{ran}");
        assert!(ran["terrain"]["relief_metres"].as_f64().unwrap() > 0.0, "{ran}");
        let history = harness.call("build_history", json!({})).unwrap();
        assert_eq!(history["total"], json!(1));
        let build = harness.call("analyze_build", json!({"expected_files": ["nothing.png"]})).unwrap();
        assert_eq!(build["file_count"].as_u64().unwrap() as usize, std::fs::read_dir(folder.join("build")).unwrap().count());
        assert_eq!(build["expected_files"][0]["present"], json!(false));
        let written = files[0]["path"].as_str().unwrap();
        let terrain = harness.call("analyze_terrain", json!({"path": written})).unwrap();
        assert_eq!(terrain["resolution"], json!([256, 256]));

        // Another file, changed and repaired without opening it.
        let other = folder.join("Other.terrain");
        std::fs::copy(&path, &other).unwrap();
        let patched = harness
            .call("patch_project_file", json!({"path": other, "patches": [{"node": ids[0], "parameters": {"Scale": 0.2, "Nonsense": 1}}, {"node": ids[2], "export": {"file_name": "colour", "format": "PNG8"}}], "create_backup": false}))
            .unwrap();
        assert_eq!(patched["changed"], json!(true));
        assert_eq!(patched["rejected"].as_array().unwrap().len(), 1, "{patched}");
        let reopened = crate::document::Document::open(&other).unwrap();
        assert_eq!(reopened.node(ids[0] as i32).unwrap().values["Scale"], json!(0.2));
        assert!(reopened.export_settings(ids[2] as i32).is_some());
        let repaired = harness.call("repair_project_file", json!({"path": other, "write": false})).unwrap();
        assert_eq!(repaired["changed"], json!(false), "{repaired}");
        // The open project's own file is left to the editor's tools.
        assert!(harness.call("patch_project_file", json!({"path": path, "patches": [{"node": ids[0], "parameters": {"Scale": 0.3}}]})).is_err());
        let listed = harness.call("list_projects", json!({"directory": folder})).unwrap();
        assert_eq!(listed["count"], json!(2));
        std::fs::remove_dir_all(&folder).ok();
    }

    #[test]
    fn modifiers_offered_are_in_the_catalogue() {
        for name in MODIFIERS {
            assert!(nerthus_engine::Catalogue::get().modifier(name).is_some(), "{name}");
        }
    }
}
