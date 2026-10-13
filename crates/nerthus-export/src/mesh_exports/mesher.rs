//! Mesher's export: `Mesher.PostBuild` and `Mesher.Export`, then the
//! topology's mesher (Tris and Quads to the grid mesher, Adaptive to
//! Sophia).

use std::path::{Path, PathBuf};

use nerthus_engine::region::terrain_ratio;
use nerthus_engine::{Map, NodeData, Resolution};
use nerthus_nodes::mesh_rasterizer::heightmap_to_mesh;
use nerthus_nodes::transformer::{self, ResizeMethod};

use super::mesh_port::{save_mesh, SaveParameters, UvOrigin};
use super::sophia::Sophia;
use super::{dotnet_extension, dotnet_stem, flag, input_map, integer, scale_factor, text, FileResult, PostBuildContext};
use crate::build::BuildType;
use crate::format::clamp_for_export;

/// `MeshExportOptions` as `Mesher.Export` fills it.
#[derive(Clone, Debug)]
pub struct MeshExportOptions {
    /// `MeshScale`: "Normalized", "Meter" or "Kilometer".
    pub scale: String,
    pub centered: bool,
    pub uvs: bool,
    pub walls: bool,
    /// The base mesh and one per level of detail.
    pub lod_count: i32,
    pub vertices: i32,
    pub uv_origin: UvOrigin,
    pub normals: bool,
    /// Stays (0, 0) from Mesher, which never passes a tile.
    pub top_left: [f32; 2],
    pub bottom_right: [f32; 2],
}

impl Default for MeshExportOptions {
    fn default() -> Self {
        Self {
            scale: "Normalized".to_string(),
            centered: true,
            uvs: false,
            walls: false,
            lod_count: 1,
            vertices: 2048,
            uv_origin: UvOrigin::BottomLeft,
            normals: false,
            top_left: [0.0, 0.0],
            bottom_right: [1.0, 1.0],
        }
    }
}

/// The terrain's size in metres.
#[derive(Clone, Copy, Debug)]
pub struct TerrainSize {
    pub width: f32,
    pub height: f32,
    /// `Terrain.Ratio`: height over width, made as if a Region build's
    /// region were the whole terrain. Sophia scales heights by it; the
    /// grid mesh takes the width and height themselves.
    pub ratio: f32,
}

impl TerrainSize {
    /// A terrain outside a Region build.
    pub fn new(width: f32, height: f32) -> TerrainSize {
        let ratio = terrain_ratio(width, height, None);
        TerrainSize { width, height, ratio }
    }
}

/// The file extension of a `MesherFormats` member.
pub fn extension(format: &str) -> &'static str {
    match format {
        "FBX" => ".fbx",
        "DAE" => ".dae",
        "GLTF" => ".gltf",
        "GLB" => ".glb",
        _ => ".obj",
    }
}

/// `LOD` as a count of meshes: the base mesh and one per level.
pub fn lod_count(level: &str) -> i32 {
    match level {
        "LOD1" => 2,
        "LOD2" => 3,
        "LOD3" => 4,
        "LOD4" => 5,
        "LOD5" => 6,
        "LOD6" => 7,
        _ => 1,
    }
}

/// `Mesher.PostBuild`: `<build folder>\<node name><tile suffix><ext>`,
/// made from the map feeding the Mesher — not the Mesher's own output, so
/// Artifact Reduction never reaches the file.
pub(super) fn post_build(context: &PostBuildContext<'_>, node: &NodeData, on_file: &mut dyn FnMut(FileResult)) {
    let format = text(node, "Format", "OBJ");
    let suffix = match context.tile {
        Some((x, y)) => format!("_x{x}_y{y}"),
        None => String::new(),
    };
    let path = context.folder.join(format!("{}{suffix}{}", node.name, extension(format)));
    let Some(map) = input_map(context, node, "In") else {
        on_file(FileResult::Failed { path, message: format!("{} has no input map to export", node.name) });
        return;
    };
    let options = MeshExportOptions {
        scale: text(node, "Scale", "Normalized").to_string(),
        centered: flag(node, "CenterPivot", true),
        uvs: flag(node, "CreateUVs", false),
        walls: flag(node, "CreateWalls", false),
        lod_count: lod_count(text(node, "LevelOfDetail", "None")),
        vertices: integer(node, "VerticesPerSide", 2048) as i32,
        uv_origin: UvOrigin::from_name(text(node, "UVOrigin", "BottomLeft")),
        normals: flag(node, "CreateNormals", false),
        top_left: [0.0, 0.0],
        bottom_right: [1.0, 1.0],
    };
    let region = match context.definition.build_type {
        BuildType::Region => context.definition.portion.as_ref(),
        _ => None,
    };
    let width = context.terrain.width;
    let height = context.terrain.height;
    let ratio = terrain_ratio(width, height, region);
    let terrain = TerrainSize { width, height, ratio };
    let topology = text(node, "Topology", "AdaptiveTris");
    export(&path, &map, topology, &options, terrain, on_file);
}

/// `Mesher.Export`: the map clamped, enlarged to the vertex count when
/// smaller (there is no edition resolution cap), then
/// meshed by the topology's method.
pub fn export(path: &Path, map: &Map, topology: &str, options: &MeshExportOptions, terrain: TerrainSize, on_file: &mut dyn FnMut(FileResult)) {
    let mut prepared = clamp_for_export(map);
    if (prepared.width() as i32) < options.vertices {
        prepared = transformer::resize(&prepared, Resolution::square(options.vertices.max(0) as usize));
    }
    match topology {
        "Tris" => export_grid(path, &prepared, false, options, terrain, on_file),
        "Quads" => export_grid(path, &prepared, true, options, terrain, on_file),
        _ => export_adaptive(path, &prepared, options, terrain, on_file),
    }
}

/// Create Walls: the outermost rows and columns set
/// to 0 so the mesh drops to the floor at its edges.
fn add_walls(map: &mut Map) {
    let size = map.width();
    if size == 0 {
        return;
    }
    for index in 0..size {
        map.set(0, index, 0.0);
        map.set(size - 1, index, 0.0);
        map.set(index, 0, 0.0);
        map.set(index, size - 1, 0.0);
    }
}

fn report(on_file: &mut dyn FnMut(FileResult), path: PathBuf, outcome: std::io::Result<bool>) {
    match outcome {
        Ok(true) => on_file(FileResult::Written(path)),
        Ok(false) => on_file(FileResult::Failed { path, message: "The mesh could not be exported in this format".to_string() }),
        Err(error) => on_file(FileResult::Failed { path, message: error.to_string() }),
    }
}

/// `path` with `suffix` before its extension (`$"{dir}\\{name}{suffix}{ext}"`).
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let file_name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let renamed = format!("{}{suffix}{}", dotnet_stem(&file_name), dotnet_extension(&file_name));
    path.with_file_name(renamed)
}

/// For each level of detail, the map resized to the
/// vertex count (halved per level), made a grid mesh
/// (`MeshPort_HeightmapToMesh`) and saved (`MeshPort_Save`). FBX files are
/// scaled ×100 (centimetres).
pub fn export_grid(path: &Path, map: &Map, quads: bool, options: &MeshExportOptions, terrain: TerrainSize, on_file: &mut dyn FnMut(FileResult)) {
    let file_name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let mut scale = scale_factor(&options.scale, terrain.width);
    if dotnet_extension(&file_name) == ".fbx" {
        scale *= 100.0;
    }
    let top_left = [options.top_left[0] * scale, options.top_left[1] * scale];

    let mut vertices = options.vertices;
    let mut lod_suffix = String::new();
    for level in 0..options.lod_count {
        let size = vertices.max(0) as usize;
        let mut resized = transformer::resize_with(map, Resolution::square(size), ResizeMethod::HighQuality);
        if options.walls {
            add_walls(&mut resized);
        }
        let lod_path = with_suffix(path, &lod_suffix);
        let mut mesh = heightmap_to_mesh(resized.data(), size, size, terrain.width, terrain.height);

        let mut parameters = SaveParameters::new(scale, top_left, options.bottom_right);
        parameters.centered = options.centered;
        parameters.normals = options.normals;
        parameters.quads = quads;
        parameters.uvs = options.uvs;
        parameters.uv_origin = options.uv_origin;
        let outcome = save_mesh(&lod_path, &mut mesh, Some((vertices, vertices)), &parameters);
        report(on_file, lod_path, outcome);

        vertices /= 2;
        lod_suffix = format!("_LOD{}", level + 1);
    }
}

/// Sophia's adaptive triangulation of the whole map,
/// aiming at half the vertex count squared per level of detail; the
/// coarsest level is processed and saved first, the target doubling
/// after each.
pub fn export_adaptive(path: &Path, map: &Map, options: &MeshExportOptions, terrain: TerrainSize, on_file: &mut dyn FnMut(FileResult)) {
    let area = (map.width() * map.height()) as i32;
    let mut polygons = options.vertices.wrapping_mul(options.vertices).min(area);
    for _ in 0..options.lod_count {
        polygons /= 2;
    }
    let mut sophia = Sophia::new(map.data(), polygons);
    sophia.calculate_errors();
    sophia.target_polycount = polygons;
    let scale = scale_factor(&options.scale, terrain.width);
    let ratio = terrain.ratio;
    for level in (0..options.lod_count).rev() {
        let lod_suffix = if level == 0 { String::new() } else { format!("_LOD{level}") };
        let lod_path = with_suffix(path, &lod_suffix);
        sophia.process_mesh();
        let (mut mesh, resolution) = sophia.mesh(ratio);
        let mut parameters = SaveParameters::new(scale, options.top_left, options.bottom_right);
        parameters.centered = options.centered;
        parameters.normals = options.normals;
        parameters.uvs = options.uvs;
        parameters.uv_origin = options.uv_origin;
        let outcome = save_mesh(&lod_path, &mut mesh, Some(resolution), &parameters);
        report(on_file, lod_path, outcome);
        sophia.target_polycount = sophia.target_polycount.wrapping_mul(2);
    }
}
