//! The steps `Importer::ReadFile(path, 0x800a)` runs after the importer:
//! `ScenePreprocessor`, then (in registry order) Triangulate, SortByPType
//! and JoinIdenticalVertices.

mod join_vertices;
mod preprocess;
mod sort_by_primitive_type;
mod triangulate;

use crate::scene::{ImportError, Scene};

/// `Exporter::Export`'s steps for an exporter that enforces
/// JoinIdenticalVertices, Triangulate and SortByPType (glTF 2): the same
/// steps in registry order, without the importer's `ScenePreprocessor`.
pub fn run_export(scene: &mut Scene) -> Result<(), ImportError> {
    for mesh in &mut scene.meshes {
        triangulate::triangulate_mesh(mesh);
    }
    sort_by_primitive_type::execute(scene)?;
    for mesh in &mut scene.meshes {
        join_vertices::process_mesh(mesh);
    }
    Ok(())
}

pub fn run(scene: &mut Scene) -> Result<(), ImportError> {
    preprocess::process_scene(scene);
    for mesh in &mut scene.meshes {
        triangulate::triangulate_mesh(mesh);
    }
    sort_by_primitive_type::execute(scene)?;
    for mesh in &mut scene.meshes {
        join_vertices::process_mesh(mesh);
    }
    Ok(())
}
