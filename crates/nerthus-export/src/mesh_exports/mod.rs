//! Build and Export for the mesh nodes. Mesher has its own
//! `PostBuild` (one mesh per level of detail, named after the node);
//! PointCloud, TextureBaker and MeshWarp go through `Node.PostBuild` with
//! their own `GetExportFilename` (PointCloud) and `Export` overrides.
//!
//! Single-image builds only: tiled outputs would call these overrides per
//! tile in ways that make no sense for meshes (a Mesher bucket saved under
//! an image extension, a PointCloud keeping the first tile's points), so
//! Nerthus reports them as unsupported instead.

mod assimp_export;
pub mod mesh_port;
mod mesher;
pub mod numbers;
mod point_cloud;
pub mod sophia;
mod texture_baker;

pub use mesher::{export as export_mesher_map, export_adaptive, export_grid, MeshExportOptions, TerrainSize};
pub use point_cloud::{map_points, write_points};

use std::path::{Path, PathBuf};
use std::sync::Arc;

use nerthus_engine::{Engine, Map, NodeData, NodeState, StateOverride, Terrain};
use serde_json::Value;

use crate::build::{BuildDefinition, BuildType, ExportPreferences, OutputType};

/// What a mesh export needs from the build.
pub struct PostBuildContext<'a> {
    pub terrain: &'a Terrain,
    pub engine: &'a Engine,
    /// The build folder.
    pub folder: &'a Path,
    pub definition: &'a BuildDefinition,
    pub preferences: &'a ExportPreferences,
    /// The bucket a bucketed build is on (`BuildContext.Tile`), if any.
    pub tile: Option<(i32, i32)>,
}

/// One file a mesh export wrote, or failed to.
#[derive(Clone, Debug)]
pub enum FileResult {
    Written(PathBuf),
    Failed { path: PathBuf, message: String },
}

/// The node types this module exports.
pub const MESH_EXPORT_TYPES: [&str; 4] = ["Mesher", "PointCloud", "TextureBaker", "MeshWarp"];

/// Whether `node` exports through this module.
pub fn handles(node: &NodeData) -> bool {
    MESH_EXPORT_TYPES.contains(&node.type_name.as_str())
}

/// `Node.PostBuild` for the mesh nodes: false for any other node type
/// (nothing done), true once the node's files are written or have
/// failed, each reported through `on_file`.
pub fn post_build(context: &PostBuildContext<'_>, node: &NodeData, on_file: &mut dyn FnMut(FileResult)) -> bool {
    if !handles(node) {
        return false;
    }
    if node.type_name == "Mesher" {
        mesher::post_build(context, node, on_file);
        return true;
    }
    let single = context.definition.output_type == OutputType::SingleImage || context.definition.build_type == BuildType::Region;
    if !single {
        let message = format!("{} exports are only supported for single-image builds", node.type_name);
        on_file(FileResult::Failed { path: context.folder.to_path_buf(), message });
        return true;
    }
    for port in node.ports.iter().filter(|port| port.is_output() && port.is_exporting) {
        let port_name = port_file_name(&port.name, context);
        let map = context.engine.cache.get(node.id, &port.name);
        match node.type_name.as_str() {
            "PointCloud" => point_cloud::export_port(context, node, &port_name, map, on_file),
            _ => texture_baker::export_port(context, node, &port.name, map, on_file),
        }
    }
    true
}

/// `Node.PostBuild`'s port part of a file name: the delimiter and the
/// port's name, nothing for the primary port when the build says so.
fn port_file_name(port: &str, context: &PostBuildContext<'_>) -> String {
    if port == "Out" && context.definition.remove_primary_port_name {
        return String::new();
    }
    format!("{}{port}", context.preferences.filename_delimiter)
}

/// The map an input port reads — through portals'
/// inputs and bypassed nodes, nothing from a node that failed.
pub(crate) fn input_map(context: &PostBuildContext<'_>, node: &NodeData, port: &str) -> Option<Arc<Map>> {
    let record = node.port(port)?.record.as_ref()?;
    read_output(context, record.from, &record.from_port)
}

fn read_output(context: &PostBuildContext<'_>, node: i32, port: &str) -> Option<Arc<Map>> {
    let source = context.terrain.nodes.get(&node)?;
    if let Some(own_input) = source.port(port).filter(|candidate| candidate.is_input()) {
        let record = own_input.record.as_ref()?;
        return read_output(context, record.from, &record.from_port);
    }
    if StateOverride::of(source) == StateOverride::Bypassed {
        let record = source.port("In")?.record.as_ref()?;
        return read_output(context, record.from, "Out");
    }
    if context.engine.state(node) == NodeState::Error {
        return None;
    }
    context.engine.cache.get(node, port)
}

/// The node feeding one of `node`'s ports.
pub(crate) fn source_node<'a>(context: &PostBuildContext<'a>, node: &NodeData, port: &str) -> Option<&'a NodeData> {
    let record = node.port(port)?.record.as_ref()?;
    context.terrain.nodes.get(&record.from)
}

/// A node property as text (an enum's member name).
pub(crate) fn text<'a>(node: &'a NodeData, name: &str, default: &'a str) -> &'a str {
    node.values.get(name).and_then(Value::as_str).unwrap_or(default)
}

pub(crate) fn flag(node: &NodeData, name: &str, default: bool) -> bool {
    node.values.get(name).and_then(Value::as_bool).unwrap_or(default)
}

pub(crate) fn integer(node: &NodeData, name: &str, default: i64) -> i64 {
    node.values.get(name).and_then(Value::as_i64).unwrap_or(default)
}

/// `MeshScale` as units per terrain width: 1 for Normalized, metres or
/// kilometres otherwise.
pub(crate) fn scale_factor(scale: &str, terrain_width: f32) -> f32 {
    match scale {
        "Meter" => terrain_width,
        "Kilometer" => terrain_width * 0.001,
        _ => 1.0,
    }
}

/// .NET's `Path.GetExtension` on a file name: from the last dot, empty
/// when there is none or it ends the name.
pub(crate) fn dotnet_extension(file_name: &str) -> &str {
    match file_name.rfind('.') {
        Some(position) if position + 1 < file_name.len() => &file_name[position..],
        _ => "",
    }
}

/// .NET's `Path.GetFileNameWithoutExtension` on a file name.
pub(crate) fn dotnet_stem(file_name: &str) -> &str {
    match file_name.rfind('.') {
        Some(position) => &file_name[..position],
        None => file_name,
    }
}
