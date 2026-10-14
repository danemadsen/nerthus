//! `MeshPort_Load`: `.obj` files (the text
//! after the last dot, case-sensitive) through tinyobjloader, everything
//! else through Assimp (`ReadFile` with JoinIdenticalVertices, Triangulate
//! and SortByPType).

use std::path::Path;

use crate::mesh_rasterizer::Mesh;
use crate::obj_loader;

/// The mesh for a file, `None` where there is no mesh (an unreadable
/// `.obj`, or a failed Assimp import).
pub fn load(path: &Path) -> Option<Mesh> {
    let text = path.to_string_lossy();
    let extension = text.rfind('.').map(|dot| &text[dot + 1..]).unwrap_or("");
    if extension == "obj" {
        return obj_loader::load_obj(path);
    }
    load_assimp(&text)
}

/// The Assimp path: every mesh's vertices as (x, z, y) and its
/// three-index faces, offset by the vertices before it (node transforms
/// are not applied). The positions are read for the mesh's vertex count
/// unchecked: a mesh with vertices but no (or too few) positions crashes
/// it, no mesh here.
pub fn load_assimp(path: &str) -> Option<Mesh> {
    let scene = nerthus_assimp::read_file(path).ok()?;
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    let mut base = 0u32;
    for mesh in &scene.meshes {
        if mesh.vertex_count > 0 {
            let positions = mesh.vertices.as_ref()?;
            if positions.len() < mesh.vertex_count {
                return None;
            }
            for position in &positions[..mesh.vertex_count] {
                vertices.push([position[0], position[2], position[1]]);
            }
        }
        if let Some(faces) = &mesh.faces {
            for face in faces {
                if face.len() != 3 {
                    continue;
                }
                let triangle = [face[0].wrapping_add(base), face[1].wrapping_add(base), face[2].wrapping_add(base)];
                triangles.push(triangle);
            }
        }
        base = base.wrapping_add(mesh.vertex_count as u32);
    }
    Some(Mesh { vertices, triangles, normals: Vec::new() })
}
