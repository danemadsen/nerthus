//! The parts of Assimp that Nerthus's mesh loading uses, ported from
//! Assimp "5.4.3.0 (Commit 97257a3b)":
//! `Importer::ReadFile(path, JoinIdenticalVertices | Triangulate |
//! SortByPType)` for the formats the Object node offers.

mod base64;
mod collada;
pub mod export;
mod fast_atof;
mod fbx;
mod gltf1;
pub mod gltf2;
mod io;
pub mod json;
mod matrix;
mod obj;
mod postprocess;
pub mod scene;
mod vector;
mod xml;

pub use scene::{ImportError, Mesh, Scene};

/// `BaseImporter::HasExtension`: a case-insensitive suffix match.
fn has_extension(path: &str, extension: &str) -> bool {
    let suffix = format!(".{extension}");
    path.len() >= suffix.len() && path.as_bytes()[path.len() - suffix.len()..].eq_ignore_ascii_case(suffix.as_bytes())
}

/// `Importer::ReadFile(path, 0x800a)`: the importer chosen by extension
/// (by signature when several claim it), then the post-processing.
pub fn read_file(path: &str) -> Result<Scene, ImportError> {
    let Ok(bytes) = std::fs::read(path) else {
        return scene::fail(format!("Unable to open file \"{path}\"."));
    };
    let mut scene = if has_extension(path, "fbx") {
        fbx::read(&bytes)?
    } else if has_extension(path, "dae") {
        collada::read(&bytes)?
    } else if has_extension(path, "obj") {
        obj::read(path, &bytes)?
    } else if has_extension(path, "gltf") || has_extension(path, "glb") {
        if gltf1::can_read(&bytes) {
            gltf1::read(path, &bytes)?
        } else if gltf2::can_read(&bytes) {
            gltf2::read(path, &bytes)?
        } else {
            return scene::fail("No suitable reader found for the file format of file");
        }
    } else {
        return scene::fail("No suitable reader found for the file format of file");
    };
    postprocess::run(&mut scene)?;
    Ok(scene)
}
