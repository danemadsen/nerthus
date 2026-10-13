//! TextureBaker's and MeshWarp's exports (the two nodes are the same code;
//! MeshWarp is hidden). For each level of detail of the Mesher
//! feeding `In`, the Mesher's map is resized to that level's vertex count
//! and made a grid mesh, then baked against the full map: the primary
//! port writes the tangent-space
//! normal maps (`<node>_Normals`), every other port the map connected to
//! its input warped onto the mesh (`<source node>`), `_LOD<n>` before the
//! extension from the second level on. Files go to the build folder in the
//! node's save format, unclamped.

use std::path::PathBuf;
use std::sync::Arc;

use nerthus_engine::{Map, NodeData, Resolution};
use nerthus_nodes::mesh_rasterizer::heightmap_to_mesh;
use nerthus_nodes::tangent_bake::{compute_tangent_space_normals, warp_channel, TangentBake};
use nerthus_nodes::transformer::{self, ResizeMethod};
use serde_json::Value;

use super::mesher::lod_count;
use super::{dotnet_extension, flag, input_map, integer, source_node, text, FileResult, PostBuildContext};
use crate::build::OverwriteMode;
use crate::destination::place_in_folder;
use crate::format::{write_map, FileFormat};

/// `SaveDefinition.Format` (Pure Raw when there is none).
fn save_format(node: &NodeData) -> FileFormat {
    let definition = node.base.get("SaveDefinition").filter(|value| !value.is_null());
    let name = definition.and_then(|definition| definition.get("Format")).and_then(Value::as_str);
    name.and_then(FileFormat::from_name).unwrap_or(FileFormat::PureRaw)
}

/// The bake for each level of detail: the Mesher's vertex count, halved per
/// level, as a grid mesh over `heightmap` (`TextureBaker.GetNormals` and
/// `GetLODs`).
fn bake_levels(context: &PostBuildContext<'_>, heightmap: &Map, mesher: &NodeData) -> Vec<TangentBake> {
    let levels = lod_count(text(mesher, "LevelOfDetail", "None"));
    let mut vertices = integer(mesher, "VerticesPerSide", 2048) as i32;
    let width = context.terrain.width;
    let height = context.terrain.height;
    let ratio = height / width;
    let mut bakes = Vec::with_capacity(levels.max(0) as usize);
    for _ in 0..levels {
        let size = vertices.max(0) as usize;
        let resized = transformer::resize_with(heightmap, Resolution::square(size), ResizeMethod::HighQuality);
        let mesh = heightmap_to_mesh(resized.data(), size, size, width, height);
        let bake = compute_tangent_space_normals(&mesh, heightmap.channel(0), heightmap.width(), heightmap.height(), ratio);
        bakes.push(bake);
        vertices /= 2;
    }
    bakes
}

/// Every channel of `map` warped by the bake.
fn warped(map: &Map, bake: &TangentBake, warp_width: usize) -> Map {
    let width = map.width();
    let height = map.height();
    let channels = (0..map.channel_count())
        .map(|channel| warp_channel(map.channel(channel), width, height, &bake.warp, warp_width))
        .collect();
    Map::from_channels(map.resolution(), channels)
}

/// Writes one level's map as `TextureBaker.Export` names it: `base`, then
/// `_LOD<level>` before its extension (every occurrence, as .NET's
/// `string.Replace` does), placed in the build folder.
fn write_level(context: &PostBuildContext<'_>, base: &str, level: usize, map: &Map, format: FileFormat, on_file: &mut dyn FnMut(FileResult)) {
    let name = if level > 0 {
        let extension = dotnet_extension(base);
        base.replace(extension, &format!("_LOD{level}{extension}"))
    } else {
        base.to_string()
    };
    let increment = context.definition.overwrite_mode == OverwriteMode::Increment;
    let path: PathBuf = place_in_folder(&name, context.folder, increment);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut options = context.preferences.write.clone();
    options.color_space = context.definition.color_space.clone();
    match write_map(format, &path, map, &options) {
        Ok(()) => on_file(FileResult::Written(path)),
        Err(error) => on_file(FileResult::Failed { path, message: error.to_string() }),
    }
}

/// `Node.PostBuild` for one of TextureBaker's or MeshWarp's ports, which
/// `Export` overrides: the normal maps for `Out` (when Generate Normals is
/// on), the warped input for `Out1`…`Out9`.
pub(super) fn export_port(context: &PostBuildContext<'_>, node: &NodeData, port: &str, map: Option<Arc<Map>>, on_file: &mut dyn FnMut(FileResult)) {
    // Build has already failed without a Mesher on `In`.
    let Some(mesher) = source_node(context, node, "In").filter(|source| source.type_name == "Mesher") else {
        return;
    };
    let format = save_format(node);
    let extension = format.extension();
    let Some(map) = map else {
        let message = format!("{} has no map to export on {port}", node.name);
        on_file(FileResult::Failed { path: context.folder.to_path_buf(), message });
        return;
    };

    if port == "Out" {
        if !flag(node, "GenerateNormals", true) {
            return;
        }
        let base = format!("{}_Normals{extension}", node.name);
        for (level, bake) in bake_levels(context, &map, mesher).into_iter().enumerate() {
            let normal_map = Map::from_channels(map.resolution(), vec![bake.red, bake.green, bake.blue]);
            write_level(context, &base, level, &normal_map, format, on_file);
        }
        return;
    }

    let input_port = port.replace("Out", "In");
    let Some(heightmap) = input_map(context, node, "In") else {
        let message = format!("{} has no Mesher map to bake against", node.name);
        on_file(FileResult::Failed { path: context.folder.to_path_buf(), message });
        return;
    };
    let source_name = source_node(context, node, &input_port).map(|source| source.name.clone()).unwrap_or_default();
    let base = format!("{source_name}{extension}");
    for (level, bake) in bake_levels(context, &heightmap, mesher).into_iter().enumerate() {
        let level_map = warped(&map, &bake, heightmap.width());
        write_level(context, &base, level, &level_map, format, on_file);
    }
}
