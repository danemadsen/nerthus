//! Files a final build writes: saving nodes' marked ports and the output
//! nodes (`SaveOutput`, `MeshOutput`). The command-line
//! build and the editor share these.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use nerthus_core::{file_safe_name, DisplayFormat, Field, NodeInstance, Port, TerrainDefinition};

use crate::build::{BuildWriter, FileEncoding};
use crate::heightmap::ExportFormat;
use crate::mesh::{export_mesh, MeshExport, MeshFormat, MeshStructure, MeshUnits};

/// Where and how a final build writes its files.
#[derive(Clone, Debug)]
pub struct BuildOutput {
    /// The build folder (`BuildService.CurrentPath`).
    pub directory: PathBuf,
    pub definition: TerrainDefinition,
    pub writer: BuildWriter,
    /// The project's file prefix (`Terrain.Prefix`).
    pub prefix: Option<String>,
}

/// Built outputs of one node, by port name.
pub type NodeOutputs = HashMap<String, Arc<Field>>;

fn is_bitmap(port: &Port) -> bool {
    port.display_format == DisplayFormat::Bitmap
}

/// A saved port's file name without the extension: `Prefix_FileName-Port` (no suffix for
/// the Output port).
fn marked_port_stem(node: &NodeInstance, port: &str, prefix: Option<&str>) -> String {
    let prefix_part = match prefix {
        Some(prefix) if !prefix.is_empty() => format!("{prefix}_"),
        _ => String::new(),
    };
    let suffix = if port == "Output" { String::new() } else { format!("-{port}") };
    let file_name = file_safe_name(node.file_label());
    format!("{prefix_part}{file_name}{suffix}")
}

/// Writes every port a saving node marked for saving, in the
/// node's save format.
pub fn save_marked_ports(node: &NodeInstance, outputs: &NodeOutputs, build: &BuildOutput) -> Result<Vec<PathBuf>> {
    let format = ExportFormat::from_save_format(node.save_format);
    let encoding = FileEncoding::Format(format);
    let mut written = Vec::new();
    for port in node.ports.iter().filter(|port| port.is_output() && port.save) {
        let Some(field) = outputs.get(&port.name) else { continue };
        let file_stem = marked_port_stem(node, &port.name, build.prefix.as_deref());
        let stem = build.directory.join(file_stem);
        let rgba = port.name == "RGBA";
        let files = build.writer.write(field, &stem, &encoding, is_bitmap(port), rgba)?;
        written.extend(files);
    }
    Ok(written)
}

/// `SaveOutput`'s files. `source` is the node wired into the Input port,
/// `source_port` the port it comes from and `source_outputs` everything that
/// node built.
pub fn save_output_node(node: &NodeInstance, source: &NodeInstance, source_port: &str, source_outputs: &NodeOutputs, build: &BuildOutput) -> Result<Vec<PathBuf>> {
    let params = &node.params;
    let path_param = params.string("Path");
    let to_build_folder = params.bool_or("Save to Build Folder", true);
    let folder = if to_build_folder { build.directory.clone() } else { PathBuf::from(&path_param) };
    let format = ExportFormat::from_save_format(params.choice_or("Format", 0));
    let encoding = FileEncoding::Format(format);
    let name = file_safe_name(node.label());

    if params.bool_or("Save all ports", false) {
        let mut written = Vec::new();
        for port in source.ports.iter().filter(|port| port.is_output()) {
            let Some(field) = source_outputs.get(&port.name) else { continue };
            let stem = folder.join(format!("{name}-{}", port.name));
            let rgba = port.name == "RGBA";
            let files = build.writer.write(field, &stem, &encoding, is_bitmap(port), rgba)?;
            written.extend(files);
        }
        return Ok(written);
    }

    let field = source_outputs.get(source_port).with_context(|| format!("{} did not build port {source_port}", source.label()))?;
    let bitmap = source.port(source_port).is_some_and(is_bitmap);
    let rgba = source_port == "RGBA";
    // A Path with an extension names the file exactly, whatever the folder
    // setting, and its extension picks the encoding.
    let explicit = Path::new(&path_param);
    if let Some(extension) = explicit.extension() {
        let extension = extension.to_string_lossy();
        let explicit_encoding = FileEncoding::from_extension(&extension);
        let stem = explicit.with_extension("");
        return build.writer.write(field, &stem, &explicit_encoding, bitmap, rgba);
    }
    let stem = folder.join(name);
    build.writer.write(field, &stem, &encoding, bitmap, rgba)
}

/// The MeshOutput node's settings from its parameters (the licence caps on
/// the vertex count are DRM and left out).
pub fn mesh_settings(node: &NodeInstance) -> MeshExport {
    let params = &node.params;
    let vertex_count = params.int_or("Vertex Count", 1024).max(16) as usize;
    let levels = (params.choice_or("LOD", 0) + 1).max(1) as usize;
    MeshExport {
        format: MeshFormat::from_index(params.choice_or("Format", 0)),
        structure: MeshStructure::from_index(params.choice_or("Structure", 2)),
        units: MeshUnits::from_index(params.choice_or("Scale", 0)),
        vertex_count,
        texture_coords: params.bool_or("Create Texture Coords", true),
        walls: params.bool_or("Create Walls", false),
        levels,
    }
}

/// `MeshOutput`: the mesh files for the heights the node received.
pub fn save_mesh_node(node: &NodeInstance, field: &Field, build: &BuildOutput) -> Result<Vec<PathBuf>> {
    let params = &node.params;
    let settings = mesh_settings(node);
    let adaptive = settings.structure == MeshStructure::Adaptive && settings.format == MeshFormat::Obj;
    let extension = match settings.format {
        MeshFormat::Xyz => ".xyz",
        MeshFormat::Fbx if !adaptive => ".fbx",
        _ => ".obj",
    };
    let to_build_folder = params.bool_or("Save to Build Folder", true);
    let folder = if to_build_folder {
        build.directory.clone()
    } else {
        // A File naming a file of this format means its folder.
        let file_param = params.string("File");
        let file = Path::new(&file_param);
        if file_param.contains(extension) { file.parent().map(Path::to_path_buf).unwrap_or_default() } else { file.to_path_buf() }
    };
    std::fs::create_dir_all(&folder).with_context(|| format!("cannot create {}", folder.display()))?;

    let size = field.size;
    let mut heights = field.to_height().data;
    build.writer.range.apply(&mut heights, size);
    let file_name = file_safe_name(node.file_label());
    let base = folder.join(file_name);
    export_mesh(&base, &heights, size, &settings, &build.definition)
}
