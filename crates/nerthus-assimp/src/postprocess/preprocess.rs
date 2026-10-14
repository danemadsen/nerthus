//! `ScenePreprocessor::ProcessMesh` (the material and animation fix-ups do
//! not touch meshes' geometry).

use crate::scene::{
    MAX_TEXTURE_COORDS, Mesh, PRIMITIVE_LINE, PRIMITIVE_POINT, PRIMITIVE_POLYGON, PRIMITIVE_TRIANGLE, Scene,
};
use crate::vector::cross;

pub fn process_scene(scene: &mut Scene) {
    for mesh in &mut scene.meshes {
        process_mesh(mesh);
    }
}

/// Texture coordinate sets get their component count (2 when unset) and
/// their unused components zeroed (a 3D set whose third components are all
/// zero becomes 2D); a mesh without primitive types gets them from its
/// faces; bitangents are derived when only normals and tangents exist.
pub(super) fn process_mesh(mesh: &mut Mesh) {
    for set in 0..MAX_TEXTURE_COORDS {
        let Some(coordinates) = mesh.texture_coords[set].as_mut() else {
            mesh.uv_components[set] = 0;
            continue;
        };
        if mesh.uv_components[set] == 0 {
            mesh.uv_components[set] = 2;
        }
        let count = mesh.vertex_count.min(coordinates.len());
        match mesh.uv_components[set] {
            2 => {
                for coordinate in &mut coordinates[..count] {
                    coordinate[2] = 0.0;
                }
            }
            1 => {
                for coordinate in &mut coordinates[..count] {
                    coordinate[2] = 0.0;
                    coordinate[1] = 0.0;
                }
            }
            3 => {
                let flat = coordinates[..count].iter().all(|coordinate| !(coordinate[2] != 0.0));
                if flat {
                    mesh.uv_components[set] = 2;
                }
            }
            _ => {}
        }
    }

    if mesh.primitive_types == 0
        && let Some(faces) = &mesh.faces {
            for face in faces {
                mesh.primitive_types |= match face.len() {
                    3 => PRIMITIVE_TRIANGLE,
                    2 => PRIMITIVE_LINE,
                    1 => PRIMITIVE_POINT,
                    _ => PRIMITIVE_POLYGON,
                };
            }
        }

    if let (Some(tangents), Some(normals), None) = (&mesh.tangents, &mesh.normals, &mesh.bitangents) {
        let bitangents = (0..mesh.vertex_count).map(|vertex| cross(normals[vertex], tangents[vertex])).collect();
        mesh.bitangents = Some(bitangents);
    }
}
