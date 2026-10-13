//! The parts of Assimp's export that `MeshPort_Save` uses: the scene the
//! mesh exports build (one root node, meshes, simple materials) written by
//! `Exporter::Export` with the Collada, binary FBX, glTF 2 and GLB
//! exporters of Assimp commit 97257a3b ("5.4.97257a3b").

pub mod collada;
pub mod fbx;
pub mod gltf2;
pub mod printf;
pub mod rapidjson;

use std::fmt;
use std::path::Path;

use crate::scene::{Mesh, Scene};

/// `aiGetVersionMajor/Minor/Revision` of that Assimp build.
pub const VERSION_MAJOR: u32 = 5;
pub const VERSION_MINOR: u32 = 4;
pub const VERSION_REVISION: u32 = 0x97257a3b;

/// A material as the mesh exports make one: a name and a diffuse colour.
#[derive(Clone, Debug)]
pub struct ExportMaterial {
    /// `?mat.name`.
    pub name: String,
    /// `$clr.diffuse`, RGBA.
    pub diffuse: [f32; 4],
}

/// An `aiScene` of the shape the mesh exports write: a root node with an
/// identity transform holding meshes, and the meshes' materials.
#[derive(Clone, Debug)]
pub struct ExportScene {
    pub root_name: String,
    /// The root node's `mMeshes`.
    pub root_meshes: Vec<u32>,
    pub meshes: Vec<Mesh>,
    pub materials: Vec<ExportMaterial>,
}

/// The local time exporters stamp files with (`std::localtime`), or
/// `NERTHUS_EXPORT_TIME` ("YYYY-MM-DDTHH:MM:SS[.mmm]") when set, so tests
/// can reproduce a file made at another moment.
pub fn export_time() -> chrono::NaiveDateTime {
    if let Ok(text) = std::env::var("NERTHUS_EXPORT_TIME") {
        for format in ["%Y-%m-%dT%H:%M:%S%.3f", "%Y-%m-%dT%H:%M:%S"] {
            if let Ok(time) = chrono::NaiveDateTime::parse_from_str(&text, format) {
                return time;
            }
        }
    }
    chrono::Local::now().naive_local()
}

/// A failed export (`DeadlyExportError`, or no exporter for the format).
#[derive(Clone, Debug)]
pub struct ExportError(pub String);

impl fmt::Display for ExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ExportError {}

/// `Exporter::GetExportFormatDescription` order: the id of the first
/// exporter (of those this build has) for a lowercase extension.
pub fn format_for_extension(extension: &str) -> Option<&'static str> {
    const EXPORTERS: [(&str, &str); 22] = [
        ("collada", "dae"),
        ("x", "x"),
        ("stp", "stp"),
        ("obj", "obj"),
        ("objnomtl", "obj"),
        ("stl", "stl"),
        ("stlb", "stl"),
        ("ply", "ply"),
        ("plyb", "ply"),
        ("3ds", "3ds"),
        ("gltf2", "gltf"),
        ("glb2", "glb"),
        ("gltf", "gltf"),
        ("glb", "glb"),
        ("assbin", "assbin"),
        ("assxml", "assxml"),
        ("x3d", "x3d"),
        ("fbx", "fbx"),
        ("fbxa", "fbx"),
        ("3mf", "3mf"),
        ("pbrt", "pbrt"),
        ("assjson", "json"),
    ];
    EXPORTERS.iter().find(|(_, candidate)| *candidate == extension).map(|(id, _)| *id)
}

/// `Exporter::Export(scene, format, path, 0, properties)`: a copy of the
/// scene, the exporter's enforced post-processing (glTF 2:
/// JoinIdenticalVertices, Triangulate, SortByPType), then the writer.
/// Only the formats the Mesher offers are implemented.
pub fn export(scene: &ExportScene, format: &str, path: &Path) -> Result<(), ExportError> {
    let mut copy = scene.clone();
    match format {
        "gltf2" | "glb2" => {
            let mut working = Scene { meshes: std::mem::take(&mut copy.meshes) };
            crate::postprocess::run_export(&mut working).map_err(|error| ExportError(error.0))?;
            copy.meshes = working.meshes;
            gltf2::write(&mut copy, path, format == "glb2")
        }
        "collada" => collada::write(&copy, path),
        "fbx" => fbx::write_binary(&copy, path),
        other => Err(ExportError(format!("The {other} exporter is not available in Nerthus"))),
    }
}
