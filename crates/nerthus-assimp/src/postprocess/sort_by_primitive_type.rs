//! `SortByPTypeProcess::Execute` (with `AI_CONFIG_PP_SBP_REMOVE` at 0): a
//! mesh with several primitive types is split into one mesh per type,
//! every face corner getting its own vertex.

use crate::scene::{ImportError, MAX_COLOR_SETS, MAX_TEXTURE_COORDS, Mesh, PRIMITIVE_LINE, PRIMITIVE_POINT, PRIMITIVE_POLYGON, PRIMITIVE_TRIANGLE, Scene, check_allocation, fail};

/// `VertexWeightTable` (a `std::vector`, MSVC x64).
const WEIGHT_TABLE_ENTRY_SIZE: u64 = 24;

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
        let weight_table = vertex_bone_table(&mesh)?;
        for (kind, &face_count) in per_type.iter().enumerate() {
            if face_count == 0 {
                continue;
            }
            let vertex_count = if kind == 3 { polygon_corners } else { face_count * (kind + 1) };
            let part = split_out(&mesh, faces, kind, vertex_count, &weight_table)?;
            output.push(part);
        }
    }
    if output.is_empty() {
        return fail("No meshes remaining");
    }
    scene.meshes = output;
    Ok(())
}

/// `ComputeVertexBoneWeightTable`: the bones weighing each vertex, in bone
/// order (none without vertices or bones). A weight past the vertices is
/// written past the table.
fn vertex_bone_table(mesh: &Mesh) -> Result<Vec<Vec<usize>>, ImportError> {
    if mesh.vertex_count == 0 || mesh.bones.is_empty() {
        return Ok(Vec::new());
    }
    check_allocation(mesh.vertex_count as u64, WEIGHT_TABLE_ENTRY_SIZE)?;
    let mut table: Vec<Vec<usize>> = vec![Vec::new(); mesh.vertex_count];
    for (bone_index, bone) in mesh.bones.iter().enumerate() {
        for &vertex in bone {
            let Some(entry) = table.get_mut(vertex as usize) else {
                return fail("undefined behaviour: a bone weight past the vertices");
            };
            entry.push(bone_index);
        }
    }
    Ok(table)
}

/// The faces of one primitive kind (0 points … 3 polygons) as a new mesh
/// with a vertex per face corner, and the bones that weigh its vertices.
fn split_out(mesh: &Mesh, faces: &[Vec<u32>], kind: usize, vertex_count: usize, weight_table: &[Vec<usize>]) -> Result<Mesh, ImportError> {
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
    // The streams copied by these corners (sets up to the first missing).
    let mut shortest = usize::MAX;
    let streams_3 = [&mesh.vertices, &mesh.normals, &mesh.tangents, &mesh.bitangents];
    for values in streams_3.into_iter().flatten() {
        shortest = shortest.min(values.len());
    }
    for values in mesh.texture_coords.iter().map_while(Option::as_ref) {
        shortest = shortest.min(values.len());
    }
    for values in mesh.colors.iter().map_while(Option::as_ref) {
        shortest = shortest.min(values.len());
    }
    if indices.iter().any(|&index| index >= shortest) {
        return fail("undefined behaviour: a face corner past a vertex stream");
    }
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

    if !weight_table.is_empty() {
        let mut bones: Vec<Vec<u32>> = vec![Vec::new(); mesh.bones.len()];
        for (corner, &index) in indices.iter().enumerate() {
            for &bone in &weight_table[index] {
                bones[bone].push(corner as u32);
            }
        }
        bones.retain(|weights| !weights.is_empty());
        out.bones = bones;
    }
    Ok(out)
}
