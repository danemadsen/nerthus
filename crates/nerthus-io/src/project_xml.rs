//! Reading and writing the `Terrain` XML (the payload of `.tor` files).
//!
//! Element order and attribute spelling follow .NET's `XmlSerializer`, and
//! unknown content is preserved verbatim.

use std::fmt::Write as _;

use anyhow::{anyhow, Context, Result};
use nerthus_core::param::{CurvePoint, Param, ParamList, ParamType, ParamValue, Rgba};
use nerthus_core::port::{DisplayFormat, Port, PortSource, PortType, PortalState};
use nerthus_core::project::{BuildResolution, BuildSettings, GraphTab, GraphView, OrganizeFiles, OutputRange, ViewportPreferences};
use nerthus_core::{NodeClass, NodeInstance, Project, TerrainDefinition};
use roxmltree::{Document, Node};
use uuid::Uuid;

const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";

fn attr<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attribute(name)
}

fn xsi_type<'a>(node: Node<'a, '_>) -> Option<&'a str> {
    node.attribute((XSI_NS, "type"))
}

fn attr_bool(node: Node, name: &str, fallback: bool) -> bool {
    match attr(node, name) {
        Some(value) => value.eq_ignore_ascii_case("true"),
        None => fallback,
    }
}

fn attr_f64(node: Node, name: &str, fallback: f64) -> f64 {
    attr(node, name).and_then(|value| value.trim().parse::<f64>().ok()).unwrap_or(fallback)
}

fn attr_i32(node: Node, name: &str, fallback: i32) -> i32 {
    attr(node, name).and_then(|value| value.trim().parse::<i32>().ok()).unwrap_or(fallback)
}

fn attr_uuid(node: Node, name: &str) -> Option<Uuid> {
    attr(node, name).and_then(|value| Uuid::parse_str(value.trim()).ok())
}

fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children().find(|candidate| candidate.is_element() && candidate.tag_name().name() == name)
}

fn child_text(node: Node, name: &str) -> Option<String> {
    child(node, name).map(|element| element.text().unwrap_or("").to_string())
}

fn child_f64(node: Node, name: &str, fallback: f64) -> f64 {
    child_text(node, name).and_then(|text| text.trim().parse::<f64>().ok()).unwrap_or(fallback)
}

fn child_i32(node: Node, name: &str, fallback: i32) -> i32 {
    child_text(node, name).and_then(|text| text.trim().parse::<i32>().ok()).unwrap_or(fallback)
}

fn child_bool(node: Node, name: &str, fallback: bool) -> bool {
    match child_text(node, name) {
        Some(text) => text.trim().eq_ignore_ascii_case("true"),
        None => fallback,
    }
}

/// Raw source text of an element (kept verbatim for unknown content).
fn outer_xml(document: &str, node: Node) -> String {
    let range = node.range();
    document[range].to_string()
}

pub fn parse_param(document: &str, element: Node) -> Option<Param> {
    let type_name = xsi_type(element)?;
    let param_type = ParamType::from_xml_name(type_name)?;
    let name = attr(element, "Name").unwrap_or("").to_string();
    let mut param = match param_type {
        ParamType::Divider => Param::title(&name),
        ParamType::Boolean => Param::boolean(&name, attr_bool(element, "Value", false)),
        ParamType::Double => Param::double_plain(&name, attr_f64(element, "Value", 0.0), 0.0, 1.0),
        ParamType::Integer => Param::int(&name, attr_i32(element, "Value", 0), 0, 10),
        ParamType::Choice => Param::choice(&name, attr_i32(element, "Value", 0), &[]),
        ParamType::String => Param::string(&name, attr(element, "Value").unwrap_or("")),
        ParamType::Guid => {
            let mut param = Param::guid(&name);
            param.value = ParamValue::Guid(attr_uuid(element, "Value").unwrap_or_default());
            param
        }
        ParamType::Color => {
            let underlying = child(element, "Underlying");
            let color = match underlying {
                Some(node) => Rgba::new(attr_f64(node, "R", 0.0), attr_f64(node, "G", 0.0), attr_f64(node, "B", 0.0), attr_f64(node, "A", 1.0)),
                None => Rgba::BLACK,
            };
            Param::color(&name, color)
        }
        ParamType::Range => Param::range(&name, attr_f64(element, "ValueA", 0.0), attr_f64(element, "ValueB", 1.0), 0.0, 1.0),
        ParamType::Curve => {
            let mut points = Vec::new();
            if let Some(value) = child(element, "Value") {
                for point in value.children().filter(|node| node.is_element()) {
                    points.push(CurvePoint::new(attr_f64(point, "X", 0.0) as f32, attr_f64(point, "Y", 0.0) as f32));
                }
            }
            Param::curve(&name, points)
        }
        ParamType::Stroke => {
            let mut param = Param::strokes(&name);
            param.raw_xml = Some(outer_xml(document, element));
            param
        }
        ParamType::Object => return None,
    };
    param.auto = attr(element, "Auto").map(str::to_string);
    param.expr = attr(element, "Expr").map(str::to_string);
    param.owner = attr_uuid(element, "Owner").unwrap_or_default();
    Some(param)
}

pub fn parse_param_list(document: &str, element: Option<Node>) -> ParamList {
    let mut list = ParamList::new();
    let Some(element) = element else { return list };
    for child in element.children().filter(|node| node.is_element() && node.tag_name().name() == "Parameter") {
        if let Some(param) = parse_param(document, child) {
            list.push(param);
        }
    }
    list
}

fn parse_port(element: Node) -> Port {
    let name = attr(element, "Name").unwrap_or("Port");
    let port_type = PortType::from_xml_name(attr(element, "PortType").unwrap_or("In"));
    let mut port = Port::new(name, attr_i32(element, "Order", 0), port_type, attr_bool(element, "IsOptional", false));
    if let Some(display_name) = attr(element, "DisplayName")
        && display_name != name {
            port.display_name = Some(display_name.to_string());
        }
    port.note = attr(element, "Note").map(str::to_string);
    port.save = attr_bool(element, "Save", false);
    port.display_format = DisplayFormat::from_xml_name(attr(element, "DisplayFormat").unwrap_or("Terrain"));
    port.portal_state = PortalState::from_xml_name(attr(element, "PortalState").unwrap_or("None"));
    if let Some(source) = attr_uuid(element, "SourceLayer")
        && !source.is_nil() {
            let source_port = attr(element, "SourcePort").unwrap_or("Output").to_string();
            port.source = Some(PortSource { node: source, port: source_port });
        }
    port
}

fn parse_node(document: &str, element: Node) -> Result<NodeInstance> {
    let type_name = xsi_type(element).ok_or_else(|| anyhow!("node without xsi:type"))?.to_string();
    let id = attr_uuid(element, "Id").unwrap_or_else(Uuid::new_v4);
    let class = attr(element, "NodeType").and_then(NodeClass::from_xml_name).unwrap_or(NodeClass::Filter);
    let display_name = attr(element, "DisplayName").map(str::to_string);
    let file_name = attr(element, "FileName").map(str::to_string);
    let mut node = NodeInstance {
        id,
        type_name: type_name.clone(),
        name: type_name.clone(),
        class,
        display_name,
        file_name,
        params: parse_param_list(document, child(element, "Parameters")),
        mask_params: parse_param_list(document, child(element, "MaskParams")),
        ports: Vec::new(),
        display_override: DisplayFormat::from_xml_name(attr(element, "DisplayOverride").unwrap_or("Default")),
        version: attr_i32(element, "Version", 1),
        save_format: attr_i32(element, "SaveFormat", -1),
        collapsed: attr_bool(element, "Collapsed", false),
        bypassed: attr_bool(element, "IsBypassed", false),
        savable: attr_bool(element, "IsSavable", false),
        saving: attr_bool(element, "IsSaving", false),
        fave: attr_bool(element, "Fave", false),
        fave_index: attr_i32(element, "FaveIndex", 0),
        pinned: child_bool(element, "IsPinned", false),
        pinned_color: child_bool(element, "IsPinnedColor", false),
        note: child_text(element, "Note"),
        graph_index: 0,
        position: (0.0, 0.0),
        baked: false,
        extra_xml: Vec::new(),
    };
    if let Some(ports) = child(element, "Ports") {
        for port in ports.children().filter(|node| node.is_element() && node.tag_name().name() == "Port") {
            node.ports.push(parse_port(port));
        }
    }
    const KNOWN: [&str; 6] = ["MaskParams", "IsPinnedColor", "Note", "IsPinned", "Parameters", "Ports"];
    for extra in element.children().filter(|node| node.is_element()) {
        if !KNOWN.contains(&extra.tag_name().name()) {
            node.extra_xml.push(outer_xml(document, extra));
        }
    }
    Ok(node)
}

fn parse_build(element: Option<Node>) -> BuildSettings {
    let mut build = BuildSettings::default();
    let Some(element) = element else { return build };
    build.location = attr(element, "Location").unwrap_or(&build.location).to_string();
    build.filename_format = attr(element, "FilenameFormat").unwrap_or(&build.filename_format).to_string();
    build.mode = attr_i32(element, "Mode", 0);
    build.tile_count = attr_i32(element, "TileCount", 1);
    build.overlap = attr_i32(element, "Overlap", 0);
    build.is_selective_build = attr_bool(element, "IsSelectiveBuild", false);
    if let Some(tiles) = attr(element, "SelectedTiles") {
        build.selected_tiles = tiles.split_whitespace().map(str::to_string).collect();
    }
    build.flip_y = attr_bool(element, "FlipY", false);
    build.save_tor = attr_bool(element, "SaveTOR", false);
    build.use_bakes = attr_i32(element, "UseBakes", 1);
    build.organize_files = OrganizeFiles::from_xml_name(attr(element, "OrganizeFiles").unwrap_or("NodeSubfolder"));
    build.preserve_tiled_cache = attr_bool(element, "PreserveTiledCache", false);
    build.custom_reference_image = attr(element, "CustomReferenceImage").map(str::to_string);
    if let Some(resolution) = child(element, "Resolution") {
        build.resolution = BuildResolution {
            preview: attr_i32(resolution, "Preview", 512),
            build: attr_i32(resolution, "Build", 2048),
            resample: attr_i32(resolution, "Resample", 2048),
            tiled: attr_i32(resolution, "Tiled", 2048),
            tiled_resample: attr_i32(resolution, "TiledResample", 2048),
            bake: attr_i32(resolution, "Bake", 1024),
            blend: attr_i32(resolution, "Blend", 1),
        };
    }
    if let Some(range) = child(element, "Range") {
        build.range = OutputRange {
            range_type: attr_i32(range, "Type", 1),
            min: attr_f64(range, "Min", 0.0),
            max: attr_f64(range, "Max", 1.0),
            color_space: attr_i32(range, "ColorSpace", 1),
        };
    }
    build
}

fn parse_vector(element: Option<Node>) -> [f64; 3] {
    match element {
        Some(node) => [child_f64(node, "x", 0.0), child_f64(node, "y", 0.0), child_f64(node, "z", 0.0)],
        None => [0.0; 3],
    }
}

/// Parses project XML. Node behaviour is not attached here; callers rehydrate
/// nodes through a registry.
pub fn read_project(xml: &str) -> Result<Project> {
    let document = Document::parse(xml).context("project XML is malformed")?;
    let root = document.root_element();
    let mut project = Project {
        id: attr_uuid(root, "Id").unwrap_or_else(Uuid::new_v4),
        name: attr(root, "Name").unwrap_or("Untitled").to_string(),
        file_format_version: attr(root, "Def").unwrap_or("1.3.0.0").to_string(),
        workflow: attr(root, "Workflow").unwrap_or("Graph").to_string(),
        has_resource: attr_bool(root, "HasResource", false),
        prefix: attr(root, "Prefix").map(str::to_string),
        export_format: attr_i32(root, "ExportFormat", 0),
        format: attr_i32(root, "Format", 1),
        resample: attr_i32(root, "Resample", 0),
        baking_preference: attr_i32(root, "BakingPreference", 0),
        persist_cache: attr_bool(root, "PersistCache", false),
        use_central_cache: attr_bool(root, "UseCentralCache", true),
        infinity_tab_index: attr_i32(root, "InfinityTabIndex", 0),
        build: parse_build(child(root, "Build")),
        ..Project::default()
    };
    if let Some(definition) = child(root, "Definition") {
        project.definition = TerrainDefinition {
            height: child_f64(definition, "Height", 2600.0) as f32,
            scale: child_f64(definition, "Scale", 5000.0) as f32,
        };
    }
    if let Some(viewport) = child(root, "ViewportPreferences") {
        project.viewport = ViewportPreferences {
            azimuth: attr_f64(viewport, "Azimuth", 33.0),
            brightness: attr_f64(viewport, "Brightness", 1.33),
            heading: attr_f64(viewport, "Heading", 300.0),
            water_level: attr_f64(viewport, "WaterLevel", 0.0),
            water: attr_bool(viewport, "Water", false),
            material: attr_i32(viewport, "Material", 0),
            position: parse_vector(child(viewport, "Position")),
            rotation: parse_vector(child(viewport, "Rotation")),
        };
    }
    let lighting = parse_param_list(xml, child(root, "Lighting"));
    if !lighting.is_empty() {
        project.lighting = lighting;
    }
    project.nodes.clear();
    if let Some(layers) = child(root, "Layers") {
        for element in layers.children().filter(|node| node.is_element() && node.tag_name().name() == "Node") {
            project.nodes.push(parse_node(xml, element)?);
        }
    }
    if let Some(graph_data) = child(root, "GraphData") {
        parse_graph_data(graph_data, &mut project);
    }
    if let Some(graphs) = child(root, "Graphs") {
        project.graphs.clear();
        for definition in graphs.children().filter(|node| node.is_element() && node.tag_name().name() == "GraphDefinition") {
            project.graphs.push(GraphTab {
                name: child_text(definition, "Name").unwrap_or_else(|| "Graph".to_string()),
                index: child_i32(definition, "Index", 0),
                order: child_i32(definition, "Order", 1),
                zoom: attr_f64(definition, "Zoom", 1.0),
                pos_x: attr_f64(definition, "PosX", 0.0),
                pos_y: attr_f64(definition, "PosY", 0.0),
                selection_index: child_i32(definition, "SelectionIndex", -1),
                is_pinned: child_bool(definition, "isPinned", false),
                has_pinned: child_bool(definition, "hasPinned", false),
                has_pinned_color: child_bool(definition, "hasPinnedColor", false),
                color: child_i32(definition, "Color", 0),
            });
        }
        if project.graphs.is_empty() {
            project.graphs.push(GraphTab::default());
        }
    }
    Ok(project)
}

fn parse_graph_data(element: Node, project: &mut Project) {
    project.graph = GraphView {
        zoom: attr_f64(element, "Zoom", 1.0),
        pos_x: attr_f64(element, "PosX", 0.0),
        pos_y: attr_f64(element, "PosY", 0.0),
        ..GraphView::default()
    };
    if let Some(shapes) = child(element, "Shapes") {
        for shape in shapes.children().filter(|node| node.is_element()) {
            let text = shape.text().unwrap_or("");
            let parts: Vec<&str> = text.split('|').collect();
            if parts.len() < 3 {
                continue;
            }
            let Ok(id) = Uuid::parse_str(parts[0]) else { continue };
            let x = parts[1].parse::<f64>().unwrap_or(0.0);
            let y = parts[2].parse::<f64>().unwrap_or(0.0);
            let graph_index = parts.get(3).and_then(|value| value.parse::<i32>().ok()).unwrap_or(0);
            if let Some(node) = project.node_mut(id) {
                node.position = (x, y);
                node.graph_index = graph_index;
            }
        }
    }
    if let Some(notes) = child(element, "Notes") {
        project.graph.raw_notes = notes.children().filter(|node| node.is_element()).map(|node| node.text().unwrap_or("").to_string()).collect();
    }
    if let Some(containers) = child(element, "Containers") {
        project.graph.raw_containers = containers.children().filter(|node| node.is_element()).map(|node| node.text().unwrap_or("").to_string()).collect();
    }
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            '\r' => escaped.push_str("&#xD;"),
            '\n' => escaped.push_str("&#xA;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn escape_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn bool_text(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

/// .NET round-trippable number text.
fn number(value: f64) -> String {
    if value.is_finite() && value == value.trunc() && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    format!("{value}")
}

pub fn write_param(output: &mut String, param: &Param) {
    if let Some(raw) = &param.raw_xml {
        output.push_str(raw);
        return;
    }
    let type_name = param.param_type().xml_name();
    let name = escape(&param.name);
    let mut extra = String::new();
    if let Some(auto) = &param.auto {
        let _ = write!(extra, " Auto=\"{}\"", escape(auto));
    }
    if let Some(expr) = &param.expr {
        let _ = write!(extra, " Expr=\"{}\"", escape(expr));
    }
    match &param.value {
        ParamValue::Title => {
            let _ = write!(output, "<Parameter xsi:type=\"{type_name}\" Name=\"{name}\"{extra} />");
        }
        ParamValue::Bool(value) => {
            let _ = write!(output, "<Parameter xsi:type=\"{type_name}\" Name=\"{name}\"{extra} Value=\"{}\" />", bool_text(*value));
        }
        ParamValue::Double(value) => {
            let _ = write!(output, "<Parameter xsi:type=\"{type_name}\" Name=\"{name}\"{extra} Value=\"{}\" />", number(*value));
        }
        ParamValue::Int(value) | ParamValue::Choice(value) => {
            let _ = write!(output, "<Parameter xsi:type=\"{type_name}\" Name=\"{name}\"{extra} Value=\"{value}\" />");
        }
        ParamValue::String(value) => {
            let _ = write!(output, "<Parameter xsi:type=\"{type_name}\" Name=\"{name}\"{extra} Value=\"{}\" />", escape(value));
        }
        ParamValue::Guid(value) => {
            let _ = write!(output, "<Parameter xsi:type=\"{type_name}\" Name=\"{name}\"{extra} Value=\"{value}\" />");
        }
        ParamValue::Color(color) => {
            let _ = write!(
                output,
                "<Parameter xsi:type=\"{type_name}\" Name=\"{name}\"{extra}{}><Underlying R=\"{}\" G=\"{}\" B=\"{}\" A=\"{}\" /></Parameter>",
                if param.owner.is_nil() { String::new() } else { format!(" Owner=\"{}\"", param.owner) },
                number(color.r),
                number(color.g),
                number(color.b),
                number(color.a)
            );
        }
        ParamValue::Range(a, b) => {
            let _ = write!(output, "<Parameter xsi:type=\"{type_name}\" Name=\"{name}\"{extra} ValueA=\"{}\" ValueB=\"{}\" />", number(*a), number(*b));
        }
        ParamValue::Curve(points) => {
            let _ = write!(output, "<Parameter xsi:type=\"{type_name}\" Name=\"{name}\"{extra}><Value>");
            for point in points {
                let _ = write!(output, "<CurvePoint X=\"{}\" Y=\"{}\" />", number(point.x as f64), number(point.y as f64));
            }
            output.push_str("</Value></Parameter>");
        }
        ParamValue::Strokes(_) => {
            let _ = write!(output, "<Parameter xsi:type=\"{type_name}\" Name=\"{name}\"{extra}><Value /></Parameter>");
        }
    }
}

fn write_param_list(output: &mut String, tag: &str, params: &ParamList) {
    if params.is_empty() {
        let _ = write!(output, "<{tag} />");
        return;
    }
    let _ = write!(output, "<{tag}>");
    for param in params {
        write_param(output, param);
    }
    let _ = write!(output, "</{tag}>");
}

fn write_port(output: &mut String, port: &Port) {
    let _ = write!(
        output,
        "<Port Name=\"{}\" DisplayName=\"{}\"",
        escape(&port.name),
        escape(port.display_name.as_deref().unwrap_or(&port.name))
    );
    if let Some(note) = &port.note {
        let _ = write!(output, " Note=\"{}\"", escape(note));
    }
    let _ = write!(
        output,
        " Save=\"{}\" Order=\"{}\" PortType=\"{}\" DisplayFormat=\"{}\"",
        bool_text(port.save),
        port.order,
        port.port_type.xml_name(),
        port.display_format.xml_name()
    );
    match &port.source {
        Some(source) => {
            let _ = write!(output, " SourceLayer=\"{}\" SourcePort=\"{}\"", source.node, escape(&source.port));
        }
        None => output.push_str(" SourcePort=\"Output\""),
    }
    let _ = write!(output, " IsOptional=\"{}\" PortalState=\"{}\" />", bool_text(port.optional), port.portal_state.xml_name());
}

fn write_node(output: &mut String, node: &NodeInstance) {
    let _ = write!(
        output,
        "<Node xsi:type=\"{}\" DisplayOverride=\"{}\" Version=\"{}\" SaveFormat=\"{}\" Id=\"{}\" Collapsed=\"{}\" NodeType=\"{}\" DisplayName=\"{}\" FileName=\"{}\" IsBypassed=\"{}\" IsSavable=\"{}\" IsSaving=\"{}\" Fave=\"{}\" FaveIndex=\"{}\">",
        escape(&node.type_name),
        node.display_override.xml_name(),
        node.version,
        node.save_format,
        node.id,
        bool_text(node.collapsed),
        node.class.xml_name(),
        escape(node.label()),
        escape(node.file_label()),
        bool_text(node.bypassed),
        bool_text(node.savable),
        bool_text(node.saving),
        bool_text(node.fave),
        node.fave_index
    );
    write_param_list(output, "MaskParams", &node.mask_params);
    let _ = write!(output, "<IsPinnedColor>{}</IsPinnedColor>", bool_text(node.pinned_color));
    if let Some(note) = &node.note {
        let _ = write!(output, "<Note>{}</Note>", escape_text(note));
    }
    let _ = write!(output, "<IsPinned>{}</IsPinned>", bool_text(node.pinned));
    write_param_list(output, "Parameters", &node.params);
    output.push_str("<Ports>");
    for port in &node.ports {
        write_port(output, port);
    }
    output.push_str("</Ports>");
    for extra in &node.extra_xml {
        output.push_str(extra);
    }
    output.push_str("</Node>");
}

/// Serialises a project into `Terrain` XML.
pub fn write_project(project: &Project) -> String {
    let mut output = String::with_capacity(64 * 1024);
    output.push_str("<?xml version=\"1.0\"?>");
    let _ = write!(
        output,
        "<Terrain xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" Def=\"{}\" InfinityTabIndex=\"{}\" Id=\"{}\" Workflow=\"{}\" Name=\"{}\" HasResource=\"{}\"",
        escape(&project.file_format_version),
        project.infinity_tab_index,
        project.id,
        escape(&project.workflow),
        escape(&project.name),
        bool_text(project.has_resource)
    );
    if let Some(prefix) = &project.prefix {
        let _ = write!(output, " Prefix=\"{}\"", escape(prefix));
    }
    let _ = write!(
        output,
        " ExportFormat=\"{}\" Format=\"{}\" Resample=\"{}\" BakingPreference=\"{}\" PersistCache=\"{}\" UseCentralCache=\"{}\">",
        project.export_format,
        project.format,
        project.resample,
        project.baking_preference,
        bool_text(project.persist_cache),
        bool_text(project.use_central_cache)
    );
    output.push_str("<Def />");
    write_build(&mut output, &project.build);
    let _ = write!(
        output,
        "<Definition><Height>{}</Height><Scale>{}</Scale></Definition>",
        number(project.definition.height as f64),
        number(project.definition.scale as f64)
    );
    let viewport = &project.viewport;
    let _ = write!(
        output,
        "<ViewportPreferences Azimuth=\"{}\" Brightness=\"{}\" Heading=\"{}\" WaterLevel=\"{}\" Water=\"{}\" Material=\"{}\"><Position><x>{}</x><y>{}</y><z>{}</z></Position><Rotation><x>{}</x><y>{}</y><z>{}</z></Rotation></ViewportPreferences>",
        number(viewport.azimuth),
        number(viewport.brightness),
        number(viewport.heading),
        number(viewport.water_level),
        bool_text(viewport.water),
        viewport.material,
        number(viewport.position[0]),
        number(viewport.position[1]),
        number(viewport.position[2]),
        number(viewport.rotation[0]),
        number(viewport.rotation[1]),
        number(viewport.rotation[2])
    );
    output.push_str("<Layers>");
    for node in &project.nodes {
        write_node(&mut output, node);
    }
    output.push_str("</Layers>");
    write_param_list(&mut output, "Lighting", &project.lighting);
    output.push_str("<Internal />");
    write_graph_data(&mut output, project);
    output.push_str("<Graphs>");
    for graph in &project.graphs {
        let _ = write!(
            output,
            "<GraphDefinition Zoom=\"{}\" PosX=\"{}\" PosY=\"{}\"><Name>{}</Name><hasPinned>{}</hasPinned><hasPinnedColor>{}</hasPinnedColor><isPinned>{}</isPinned><Color>{}</Color><Index>{}</Index><Order>{}</Order><SelectionIndex>{}</SelectionIndex></GraphDefinition>",
            number(graph.zoom),
            number(graph.pos_x),
            number(graph.pos_y),
            escape_text(&graph.name),
            bool_text(graph.has_pinned),
            bool_text(graph.has_pinned_color),
            bool_text(graph.is_pinned),
            graph.color,
            graph.index,
            graph.order,
            graph.selection_index
        );
    }
    output.push_str("</Graphs></Terrain>");
    output
}

fn write_build(output: &mut String, build: &BuildSettings) {
    let _ = write!(
        output,
        "<Build Location=\"{}\" FilenameFormat=\"{}\" Mode=\"{}\" TileCount=\"{}\"",
        escape(&build.location),
        escape(&build.filename_format),
        build.mode,
        build.tile_count
    );
    if build.overlap != 0 {
        let _ = write!(output, " Overlap=\"{}\"", build.overlap);
    }
    if build.is_selective_build {
        output.push_str(" IsSelectiveBuild=\"true\"");
    }
    if !build.selected_tiles.is_empty() {
        let _ = write!(output, " SelectedTiles=\"{}\"", escape(&build.selected_tiles.join(" ")));
    }
    let _ = write!(
        output,
        " FlipY=\"{}\" SaveTOR=\"{}\" UseBakes=\"{}\" OrganizeFiles=\"{}\"",
        bool_text(build.flip_y),
        bool_text(build.save_tor),
        build.use_bakes,
        build.organize_files.xml_name()
    );
    if build.preserve_tiled_cache {
        output.push_str(" PreserveTiledCache=\"true\"");
    }
    if let Some(reference) = &build.custom_reference_image {
        let _ = write!(output, " CustomReferenceImage=\"{}\"", escape(reference));
    }
    let resolution = &build.resolution;
    let _ = write!(
        output,
        "><Resolution Preview=\"{}\" Build=\"{}\" Resample=\"{}\" Tiled=\"{}\" TiledResample=\"{}\" Bake=\"{}\" Blend=\"{}\" />",
        resolution.preview, resolution.build, resolution.resample, resolution.tiled, resolution.tiled_resample, resolution.bake, resolution.blend
    );
    let range = &build.range;
    let _ = write!(
        output,
        "<Range Type=\"{}\" Min=\"{}\" Max=\"{}\" ColorSpace=\"{}\" /></Build>",
        range.range_type,
        number(range.min),
        number(range.max),
        range.color_space
    );
}

fn write_graph_data(output: &mut String, project: &Project) {
    let graph = &project.graph;
    let _ = write!(
        output,
        "<GraphData Zoom=\"{}\" PosX=\"{}\" PosY=\"{}\"><LayerIDs>",
        number(graph.zoom),
        number(graph.pos_x),
        number(graph.pos_y)
    );
    for node in &project.nodes {
        let _ = write!(output, "<guid>{}</guid>", node.id);
    }
    output.push_str("</LayerIDs><Shapes>");
    for node in &project.nodes {
        let _ = write!(output, "<string>{}|{}|{}|{}</string>", node.id, number(node.position.0), number(node.position.1), node.graph_index);
    }
    output.push_str("</Shapes><Connections>");
    for node in &project.nodes {
        for port in node.ports.iter().filter(|port| port.is_input()) {
            if let Some(source) = &port.source {
                let _ = write!(
                    output,
                    "<string>{}|{}|{}|{}||{}</string>",
                    source.node,
                    node.id,
                    escape_text(&source.port),
                    escape_text(&port.name),
                    node.graph_index
                );
            }
        }
    }
    output.push_str("</Connections>");
    if graph.raw_notes.is_empty() {
        output.push_str("<Notes />");
    } else {
        output.push_str("<Notes>");
        for note in &graph.raw_notes {
            let _ = write!(output, "<string>{}</string>", escape_text(note));
        }
        output.push_str("</Notes>");
    }
    if graph.raw_containers.is_empty() {
        output.push_str("<Containers />");
    } else {
        output.push_str("<Containers>");
        for container in &graph.raw_containers {
            let _ = write!(output, "<string>{}</string>", escape_text(container));
        }
        output.push_str("</Containers>");
    }
    output.push_str("</GraphData>");
}
