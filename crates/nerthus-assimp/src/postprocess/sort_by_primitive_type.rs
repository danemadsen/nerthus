//! `SortByPTypeProcess::Execute` (with `AI_CONFIG_PP_SBP_REMOVE` at 0): a
//! mesh with several primitive types is split into one mesh per type,
//! every face corner getting its own vertex.

use crate::scene::{ImportError, MAX_COLOR_SETS, MAX_TEXTURE_COORDS, Mesh, PRIMITIVE_LINE, PRIMITIVE_POINT, PRIMITIVE_POLYGON, PRIMITIVE_TRIANGLE, Scene, fail};

pub fn execute(scene: &mut Scene) -> Result<(), ImportError> {
    if scene.meshes.is_empty() {
        return Ok(());
    }
    let mut output = Vec::with_capacity(scene.meshes.len() * 2);
    for mesh in std::mem::take(&mut scene.meshes) {
        if mesh.primitive_types == 0 {
            return fail(format!("Mesh with invalid primitive type: {}", mesh.name));
        }
        let type_bits = [PRIMITIVE_POINT, PRIMITIVE_LINE, PRIMITIVE_TRIANGLE, PRIMITIVE_POLYGON];
        let type_count = type_bits.iter().filter(|&&bit| mesh.primitive_types & bit != 0).count();
        if type_count == 1 {
            output.push(mesh);
            continue;
        }
        let Some(faces) = mesh.faces.as_ref() else {
            continue;
        };

        let mut per_type = [0usize; 4];
        let mut polygon_corners = 0usize;
        for face in faces {
            // A face without indices increments aiNumPerPType[-1], past
            // the stack array (undefined; crashes the native build).
            if face.is_empty() {
                return fail("SortByPType: face without indices in a mixed mesh");
            }
            if face.len() <= 3 {
                per_type[face.len() - 1] += 1;
            } else {
                per_type[3] += 1;
                polygon_corners += face.len();
            }
        }
        for (kind, &face_count) in per_type.iter().enumerate() {
            if face_count == 0 {
                continue;
            }
            let vertex_count = if kind == 3 { polygon_corners } else { face_count * (kind + 1) };
            output.push(split_out(&mesh, faces, kind, vertex_count));
        }
    }
    if output.is_empty() {
        return fail("No meshes remaining");
    }
    scene.meshes = output;
    Ok(())
}

/// The faces of one primitive kind (0 points … 3 polygons) as a new mesh
/// with a vertex per face corner.
fn split_out(mesh: &Mesh, faces: &[Vec<u32>], kind: usize, vertex_count: usize) -> Mesh {
    let mut out = Mesh {
        name: mesh.name.clone(),
        primitive_types: 1 << kind,
        material_index: mesh.material_index,
        vertex_count,
        uv_components: mesh.uv_components,
        ..Mesh::default()
    };
    let corners = |face: &&Vec<u32>| if kind == 3 { face.len() > 3 } else { face.len() == kind + 1 };
    let selected: Vec<&Vec<u32>> = faces.iter().filter(corners).collect();
    let indices: Vec<usize> = selected.iter().flat_map(|face| face.iter().map(|&index| index as usize)).collect();
    let gather_3 = |stream: &Option<Vec<[f32; 3]>>| stream.as_ref().map(|values| indices.iter().map(|&index| values[index]).collect::<Vec<_>>());

    out.vertices = gather_3(&mesh.vertices);
    out.normals = gather_3(&mesh.normals);
    if mesh.tangents.is_some() {
        out.tangents = gather_3(&mesh.tangents);
        out.bitangents = gather_3(&mesh.bitangents);
    }
    // Sets are filled only up to the first missing one; later ones stay
    // zeroed.
    let mut filling = true;
    for set in 0..MAX_TEXTURE_COORDS {
        let Some(values) = mesh.texture_coords[set].as_ref() else {
            filling = false;
            continue;
        };
        let copied = if filling { indices.iter().map(|&index| values[index]).collect() } else { vec![[0.0; 3]; vertex_count] };
        out.texture_coords[set] = Some(copied);
    }
    let mut filling = true;
    for set in 0..MAX_COLOR_SETS {
        let Some(values) = mesh.colors[set].as_ref() else {
            filling = false;
            continue;
        };
        let copied = if filling { indices.iter().map(|&index| values[index]).collect() } else { vec![[0.0; 4]; vertex_count] };
        out.colors[set] = Some(copied);
    }

    let mut next = 0u32;
    let mut new_faces = Vec::with_capacity(selected.len());
    for face in selected {
        let renumbered: Vec<u32> = (0..face.len() as u32).map(|corner| next + corner).collect();
        next += face.len() as u32;
        new_faces.push(renumbered);
    }
    out.faces = Some(new_faces);
    out
}
