//! `ColladaExporter` for the scenes the mesh exports make: the asset header (with
//! the local time as created and modified), materials as Phong effects
//! from their colours, each mesh's sources (positions, normals, texture
//! coordinates) and a `polylist`, and a visual scene "Scene" around the
//! root node (Collada scenes cannot hold meshes, so Assimp always adds
//! one here).
//!
//! Numbers print as C++ streams at Assimp's text precision (`%.9g`). The
//! file is opened in text mode ("wt"): CRLF on Windows, so here too.

use std::collections::HashSet;
use std::path::Path;

use super::printf::c_general;
use super::{export_time, ExportError, ExportScene};
use crate::scene::{Mesh, MAX_COLOR_SETS, MAX_TEXTURE_COORDS};

/// `ASSIMP_AI_REAL_TEXT_PRECISION` for single-precision builds.
const TEXT_PRECISION: usize = 9;

fn number(value: f32) -> String {
    c_general(value as f64, TEXT_PRECISION)
}

/// `XMLEscape`.
fn xml_escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len() + text.len() / 8);
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// `XMLIDEncode`: a valid xsd:ID (a leading underscore unless it starts
/// with a letter or one, other characters replaced by
/// `XML_ID_CHARS[c % 65]`).
fn xml_id_encode(name: &str) -> String {
    const XML_ID_CHARS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_-.";
    if name.is_empty() {
        return String::new();
    }
    let bytes = name.as_bytes();
    let mut encoded = Vec::with_capacity(bytes.len() + 1);
    let first = bytes[0];
    if !((b'A'..=b'z').contains(&first) || first == b'_') {
        encoded.push(b'_');
    }
    for &byte in bytes {
        if byte != 0 && XML_ID_CHARS.contains(&byte) {
            encoded.push(byte);
        } else {
            // `(*it) % count` on a signed char: negative for bytes ≥ 0x80.
            let signed = byte as i8 as i32;
            let index = signed % XML_ID_CHARS.len() as i32;
            let index = if index < 0 { index + XML_ID_CHARS.len() as i32 } else { index };
            encoded.push(XML_ID_CHARS[index as usize]);
        }
    }
    String::from_utf8_lossy(&encoded).into_owned()
}

/// `MakeUniqueId`.
fn make_unique_id(ids: &HashSet<String>, prefix: &str, postfix: &str) -> String {
    let mut result = format!("{prefix}{postfix}");
    let mut number = 1usize;
    while ids.contains(&result) {
        result = format!("{prefix}_{number}{postfix}");
        number += 1;
    }
    result
}

/// The output stream with Assimp's two-space indentation.
struct Writer {
    output: String,
    indent: String,
}

impl Writer {
    fn line(&mut self, text: &str) {
        self.output.push_str(&self.indent);
        self.output.push_str(text);
        self.output.push('\n');
    }

    fn push(&mut self) {
        self.indent.push_str("  ");
    }

    fn pop(&mut self) {
        self.indent.truncate(self.indent.len().saturating_sub(2));
    }
}

/// The names and ids `AddObjectIndexToMaps` hands out.
struct Ids {
    used: HashSet<String>,
}

impl Ids {
    /// An object's (escaped name, unique id).
    fn object(&mut self, name: &str, default_prefix: &str, index: usize) -> (String, String) {
        let id = if name.is_empty() { format!("{default_prefix}{index}") } else { xml_id_encode(name) };
        let escaped = if name.is_empty() { String::new() } else { xml_escape(name) };
        let id = make_unique_id(&self.used, &id, "");
        self.used.insert(id.clone());
        (escaped, id)
    }
}

/// `WriteFloatArray`: a source with its float array and accessor.
fn write_float_array(writer: &mut Writer, id: &str, values: &[[f32; 3]], per_element: usize, parameters: &[&str]) {
    let array_id = format!("{}-array", xml_id_encode(id));
    writer.line(&format!("<source id=\"{}\" name=\"{}\">", xml_id_encode(id), xml_escape(id)));
    writer.push();
    let mut text = format!("{}<float_array id=\"{array_id}\" count=\"{}\"> ", writer.indent, values.len() * per_element);
    for value in values {
        for component in value.iter().take(per_element) {
            text.push_str(&number(*component));
            text.push(' ');
        }
    }
    text.push_str("</float_array>\n");
    writer.output.push_str(&text);
    writer.line("<technique_common>");
    writer.push();
    writer.line(&format!("<accessor count=\"{}\" offset=\"0\" source=\"#{array_id}\" stride=\"{per_element}\">", values.len()));
    writer.push();
    for parameter in parameters {
        writer.line(&format!("<param name=\"{parameter}\" type=\"float\" />"));
    }
    writer.pop();
    writer.line("</accessor>");
    writer.pop();
    writer.line("</technique_common>");
    writer.pop();
    writer.line("</source>");
}

/// `WriteGeometry`: nothing for a mesh without faces or vertices.
fn write_geometry(writer: &mut Writer, mesh: &Mesh, id: &str, name: &str) {
    let faces = mesh.faces.as_deref().unwrap_or(&[]);
    if faces.is_empty() || mesh.vertex_count == 0 {
        return;
    }
    writer.line(&format!("<geometry id=\"{id}\" name=\"{name}\" >"));
    writer.push();
    writer.line("<mesh>");
    writer.push();
    if let Some(vertices) = &mesh.vertices {
        write_float_array(writer, &format!("{id}-positions"), vertices, 3, &["X", "Y", "Z"]);
    }
    if mesh.has_normals() {
        write_float_array(writer, &format!("{id}-normals"), mesh.normals.as_ref().unwrap(), 3, &["X", "Y", "Z"]);
    }
    for channel in 0..MAX_TEXTURE_COORDS {
        if mesh.has_texture_coords(channel) {
            let coordinates = mesh.texture_coords[channel].as_ref().unwrap();
            if mesh.uv_components[channel] == 3 {
                write_float_array(writer, &format!("{id}-tex{channel}"), coordinates, 3, &["S", "T", "P"]);
            } else {
                write_float_array(writer, &format!("{id}-tex{channel}"), coordinates, 2, &["S", "T"]);
            }
        }
    }
    writer.line(&format!("<vertices id=\"{id}-vertices\">"));
    writer.push();
    writer.line(&format!("<input semantic=\"POSITION\" source=\"#{id}-positions\" />"));
    writer.pop();
    writer.line("</vertices>");

    let line_count = faces.iter().filter(|face| face.len() == 2).count();
    let polygon_count = faces.iter().filter(|face| face.len() >= 3).count();
    let inputs = |writer: &mut Writer, offset: &str| {
        if mesh.has_normals() {
            writer.line(&format!("<input {offset}semantic=\"NORMAL\" source=\"#{id}-normals\" />"));
        }
        for channel in 0..MAX_TEXTURE_COORDS {
            if mesh.has_texture_coords(channel) {
                writer.line(&format!("<input {offset}semantic=\"TEXCOORD\" source=\"#{id}-tex{channel}\" set=\"{channel}\" />"));
            }
        }
        for channel in 0..MAX_COLOR_SETS {
            if mesh.has_vertex_colors(channel) {
                writer.line(&format!("<input {offset}semantic=\"COLOR\" source=\"#{id}-color{channel}\" set=\"{channel}\" />"));
            }
        }
    };
    if line_count > 0 {
        writer.line(&format!("<lines count=\"{line_count}\" material=\"defaultMaterial\">"));
        writer.push();
        writer.line(&format!("<input offset=\"0\" semantic=\"VERTEX\" source=\"#{id}-vertices\" />"));
        inputs(writer, "");
        let mut text = format!("{}<p>", writer.indent);
        for face in faces.iter().filter(|face| face.len() == 2) {
            for index in face {
                text.push_str(&format!("{index} "));
            }
        }
        text.push_str("</p>\n");
        writer.output.push_str(&text);
        writer.pop();
        writer.line("</lines>");
    }
    if polygon_count > 0 {
        writer.line(&format!("<polylist count=\"{polygon_count}\" material=\"defaultMaterial\">"));
        writer.push();
        writer.line(&format!("<input offset=\"0\" semantic=\"VERTEX\" source=\"#{id}-vertices\" />"));
        inputs(writer, "offset=\"0\" ");
        let mut counts = format!("{}<vcount>", writer.indent);
        for face in faces.iter().filter(|face| face.len() >= 3) {
            counts.push_str(&format!("{} ", face.len()));
        }
        counts.push_str("</vcount>\n");
        writer.output.push_str(&counts);
        let mut text = format!("{}<p>", writer.indent);
        for face in faces.iter().filter(|face| face.len() >= 3) {
            for index in face {
                text.push_str(&format!("{index} "));
            }
        }
        text.push_str("</p>\n");
        writer.output.push_str(&text);
        writer.pop();
        writer.line("</polylist>");
    }
    writer.pop();
    writer.line("</mesh>");
    writer.pop();
    writer.line("</geometry>");
}

/// `ExportSceneCollada`.
pub fn write(scene: &ExportScene, path: &Path) -> Result<(), ExportError> {
    let mut writer = Writer { output: String::new(), indent: String::new() };
    let mut ids = Ids { used: HashSet::new() };
    writer.line("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\" ?>");
    writer.line("<COLLADA xmlns=\"http://www.collada.org/2005/11/COLLADASchema\" version=\"1.4.1\">");
    writer.push();

    // `WriteHeader`: the root node holds meshes, so Assimp adds its own
    // root, which means Y up and a unit scale.
    let time = export_time().format("%Y-%m-%dT%H:%M:%S").to_string();
    writer.line("<asset>");
    writer.push();
    writer.line("<contributor>");
    writer.push();
    writer.line("<author>Assimp</author>");
    writer.line("<authoring_tool>Assimp Exporter</authoring_tool>");
    writer.pop();
    writer.line("</contributor>");
    writer.line(&format!("<created>{time}</created>"));
    writer.line(&format!("<modified>{time}</modified>"));
    writer.line("<unit name=\"meter\" meter=\"1\" />");
    writer.line("<up_axis>Y_UP</up_axis>");
    writer.pop();
    writer.line("</asset>");

    // `CreateNodeIds`: the root node's id first.
    let root_id = if scene.root_name.is_empty() { "node".to_string() } else { xml_id_encode(&scene.root_name) };
    let root_id = make_unique_id(&ids.used, &root_id, "");
    ids.used.insert(root_id.clone());

    // `WriteMaterials`: each material's existing colours (only diffuse in
    // the exported scenes) as Phong colours; `library_images` appears
    // (empty) once a colour exists.
    let material_ids: Vec<(String, String)> = scene.materials.iter().enumerate().map(|(index, material)| ids.object(&material.name, "material_", index)).collect();
    if !scene.materials.is_empty() {
        writer.line("<library_images>");
        writer.line("</library_images>");
        writer.line("<library_effects>");
        writer.push();
        for (material, (name, id)) in scene.materials.iter().zip(&material_ids) {
            writer.line(&format!("<effect id=\"{id}-fx\" name=\"{name}\">"));
            writer.push();
            writer.line("<profile_COMMON>");
            writer.push();
            writer.line("<technique sid=\"standard\">");
            writer.push();
            writer.line("<phong>");
            writer.push();
            writer.line("<diffuse>");
            writer.push();
            let [red, green, blue, alpha] = material.diffuse;
            writer.line(&format!("<color sid=\"diffuse\">{}   {}   {}   {}</color>", number(red), number(green), number(blue), number(alpha)));
            writer.pop();
            writer.line("</diffuse>");
            writer.pop();
            writer.line("</phong>");
            writer.pop();
            writer.line("</technique>");
            writer.pop();
            writer.line("</profile_COMMON>");
            writer.pop();
            writer.line("</effect>");
        }
        writer.pop();
        writer.line("</library_effects>");
        writer.line("<library_materials>");
        writer.push();
        for (name, id) in &material_ids {
            writer.line(&format!("<material id=\"{id}\" name=\"{name}\">"));
            writer.push();
            writer.line(&format!("<instance_effect url=\"#{id}-fx\"/>"));
            writer.pop();
            writer.line("</material>");
        }
        writer.pop();
        writer.line("</library_materials>");
    }

    // `WriteGeometryLibrary`.
    let mut mesh_ids = Vec::with_capacity(scene.meshes.len());
    writer.line("<library_geometries>");
    writer.push();
    for (index, mesh) in scene.meshes.iter().enumerate() {
        let (name, id) = ids.object(&mesh.name, "mesh_", index);
        write_geometry(&mut writer, mesh, &id, &name);
        mesh_ids.push(id);
    }
    writer.pop();
    writer.line("</library_geometries>");
    writer.line("<library_controllers>");
    writer.line("</library_controllers>");

    // `WriteSceneLibrary` with Assimp's own root.
    let scene_id = make_unique_id(&ids.used, "Scene", "");
    ids.used.insert(scene_id.clone());
    writer.line("<library_visual_scenes>");
    writer.push();
    writer.line(&format!("<visual_scene id=\"{scene_id}\" name=\"Scene\">"));
    writer.push();
    writer.line(&format!("<node id=\"{root_id}\" name=\"{}\" type=\"NODE\">", xml_escape(&scene.root_name)));
    writer.push();
    writer.line("<matrix sid=\"matrix\">1 0 0 0 0 1 0 0 0 0 1 0 0 0 0 1</matrix>");
    for &mesh_index in &scene.root_meshes {
        let mesh = &scene.meshes[mesh_index as usize];
        let faces = mesh.faces.as_deref().unwrap_or(&[]);
        if faces.is_empty() || mesh.vertex_count == 0 {
            continue;
        }
        writer.line(&format!("<instance_geometry url=\"#{}\">", mesh_ids[mesh_index as usize]));
        writer.push();
        writer.line("<bind_material>");
        writer.push();
        writer.line("<technique_common>");
        writer.push();
        let material_id = &material_ids[mesh.material_index as usize].1;
        writer.line(&format!("<instance_material symbol=\"defaultMaterial\" target=\"#{material_id}\">"));
        writer.push();
        for channel in 0..MAX_TEXTURE_COORDS {
            if mesh.has_texture_coords(channel) {
                writer.line(&format!("<bind_vertex_input semantic=\"CHANNEL{channel}\" input_semantic=\"TEXCOORD\" input_set=\"{channel}\"/>"));
            }
        }
        writer.pop();
        writer.line("</instance_material>");
        writer.pop();
        writer.line("</technique_common>");
        writer.pop();
        writer.line("</bind_material>");
        writer.pop();
        writer.line("</instance_geometry>");
    }
    writer.pop();
    writer.line("</node>");
    writer.pop();
    writer.line("</visual_scene>");
    writer.pop();
    writer.line("</library_visual_scenes>");

    writer.line("<scene>");
    writer.push();
    writer.line(&format!("<instance_visual_scene url=\"#{scene_id}\" />"));
    writer.pop();
    writer.line("</scene>");
    writer.pop();
    writer.line("</COLLADA>");

    let text = writer.output.replace('\n', "\r\n");
    std::fs::write(path, text.as_bytes()).map_err(|error| ExportError(error.to_string()))
}
