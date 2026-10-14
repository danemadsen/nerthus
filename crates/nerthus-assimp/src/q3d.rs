//! Assimp's Quick3D importer (`Q3DLoader.cpp`): after the `quick3Do` or
//! `quick3Ds` signature and the mesh, material and texture counts, chunks
//! `m` (meshes), `c` (materials), `t` (textures) and `s` (the scene,
//! which ends the reading). An output mesh per used material, a vertex
//! per face corner, with the source normal or a face normal and, while
//! the material index is below the texture count, texture coordinates.
//!
//! Kept quirks: the face normal's third point is the vertex numbered
//! (index count - 1), its second corner is read before indices are
//! clamped (out of range, undefined behaviour, an error here, as is a
//! source normal past the normals); the texture coordinate cursor only
//! moves for source meshes with coordinates; a material name stops at
//! 1024 bytes, the rest read as its colours.

use crate::scene::{ImportError, Mesh, Scene, check_allocation, fail};
use crate::stream_reader::StreamReader;
use crate::vector::{Vector3, cross, normalize, sub};

/// `AI_MAXLEN`.
const MAX_NAME_LENGTH: usize = 1024;

/// `sizeof(Q3DImporter::Material)`, `Mesh` and `Face` (MSVC x64).
const MATERIAL_SIZE: u64 = 1072;
const MESH_SIZE: u64 = 104;
const FACE_SIZE: u64 = 56;

struct Face {
    indices: Vec<u32>,
    uv_indices: Vec<u32>,
    material: u32,
}

#[derive(Default)]
struct SourceMesh {
    vertices: Vec<Vector3>,
    normals: Vec<Vector3>,
    uv: Vec<Vector3>,
    faces: Vec<Face>,
    previous_uv_index: u32,
}

fn undefined_read<T>() -> Result<T, ImportError> {
    fail("undefined behaviour: a read past a vector")
}

/// `count` triples of floats, failing (at the stream's end) before
/// allocating more than the file holds.
fn read_vectors(stream: &mut StreamReader, count: u32, components: usize) -> Result<Vec<Vector3>, ImportError> {
    check_allocation(count as u64, 12)?;
    if stream.remaining_size_to_limit() < count as usize * components * 4 {
        return fail("End of file or stream limit was reached");
    }
    let mut vectors = vec![[0.0f32; 3]; count as usize];
    for vector in &mut vectors {
        for component in vector.iter_mut().take(components) {
            *component = stream.get_f4()?;
        }
    }
    Ok(vectors)
}

fn read_mesh(stream: &mut StreamReader, texture_count: u32, major: u8, minor: u8) -> Result<SourceMesh, ImportError> {
    let mut mesh = SourceMesh::default();
    let vertex_count = stream.get_i4()? as u32;
    if vertex_count == 0 {
        return fail("Quick3D: Found mesh with zero vertices");
    }
    mesh.vertices = read_vectors(stream, vertex_count, 3)?;

    let face_count = stream.get_i4()? as u32;
    if face_count == 0 {
        return fail("Quick3D: Found mesh with zero faces");
    }
    check_allocation(face_count as u64, FACE_SIZE)?;
    if stream.remaining_size_to_limit() < face_count as usize * 2 {
        return fail("End of file or stream limit was reached");
    }
    for _ in 0..face_count {
        // A negative count becomes billions (a failed allocation).
        let size = stream.get_i2()? as i32 as u32;
        if size == 0 {
            return fail("Quick3D: Found face with zero indices");
        }
        check_allocation(size as u64, 4)?;
        let face = Face { indices: vec![0; size as usize], uv_indices: vec![0; size as usize], material: 0 };
        mesh.faces.push(face);
    }
    for face in &mut mesh.faces {
        for index in &mut face.indices {
            *index = stream.get_i4()? as u32;
        }
    }
    for face in &mut mesh.faces {
        face.material = stream.get_i4()? as u32;
    }

    let normal_count = stream.get_i4()? as u32;
    mesh.normals = read_vectors(stream, normal_count, 3)?;

    let uv_count = stream.get_i4()? as u32;
    if texture_count != 0 && uv_count != 0 {
        mesh.uv = read_vectors(stream, uv_count, 2)?;
        for (face_index, face) in mesh.faces.iter_mut().enumerate() {
            for corner in 0..face.indices.len() {
                let value = stream.get_i4()? as u32;
                face.uv_indices[corner] = value;
                if face_index == 0 && corner == 0 {
                    mesh.previous_uv_index = value;
                } else if value != mesh.previous_uv_index {
                    mesh.previous_uv_index = u32::MAX;
                }
            }
        }
    }

    stream.inc_ptr(36)?;
    if minor > b'0' && major == b'3' {
        stream.inc_ptr(mesh.faces.len() as i64)?;
    }
    Ok(mesh)
}

/// A material's name (to a NUL, at most 1024 bytes) and the fields after
/// it; only how many bytes they take matters here.
fn skip_material(stream: &mut StreamReader) -> Result<(), ImportError> {
    let mut length = 0;
    let mut byte = stream.get_i1()?;
    while byte != 0 {
        length += 1;
        if length == MAX_NAME_LENGTH {
            // "String ouverflow detected, skipped material name parsing."
            break;
        }
        byte = stream.get_i1()?;
    }
    // Ambient, diffuse and specular colours, transparency, texture index.
    for _ in 0..11 {
        stream.get_f4()?;
    }
    Ok(())
}

/// The textures: only their size and the zero-size check matter.
fn skip_textures(stream: &mut StreamReader, texture_count: u32) -> Result<(), ImportError> {
    for _ in 0..texture_count {
        while stream.get_i1()? != 0 {}
        let width = stream.get_i4()? as u32;
        let height = stream.get_i4()? as u32;
        if width == 0 || height == 0 {
            return fail("Quick3D: Invalid texture. Width or height is zero");
        }
        let texels = width.wrapping_mul(height);
        check_allocation(texels as u64, 4)?;
        let size = texels as usize * 3;
        if stream.remaining_size_to_limit() < size {
            return fail("End of file or stream limit was reached");
        }
        stream.inc_ptr(size as i64)?;
    }
    Ok(())
}

/// The scene chunk: camera, light and background, all skipped.
fn skip_scene(stream: &mut StreamReader) -> Result<(), ImportError> {
    stream.inc_ptr(12)?;
    for _ in 0..16 {
        stream.get_f4()?;
    }
    stream.inc_ptr(16)?;
    for _ in 0..3 {
        stream.get_f4()?;
    }
    stream.inc_ptr(12)?;
    for _ in 0..3 {
        stream.get_f4()?;
    }
    stream.inc_ptr(29)?;
    for _ in 0..3 {
        stream.get_f4()?;
    }
    let width = stream.get_i4()?;
    let height = stream.get_i4()?;
    let texels = width.wrapping_mul(height) as u32;
    while stream.get_i1()? != 0 {}
    let skipped = texels.wrapping_mul(3).wrapping_add(20);
    stream.inc_ptr(skipped as i64)
}

/// The face normal: `(v1 - v0) ^ normalize(v[count - 1] - v0)`.
fn face_normal(source: &SourceMesh, face: &Face) -> Result<Vector3, ImportError> {
    let corner = |index: usize| source.vertices.get(index).copied();
    let first = corner(face.indices[0] as usize);
    let second = corner(face.indices[1] as usize);
    let third = corner(face.indices.len() - 1);
    let (Some(first), Some(second), Some(third)) = (first, second, third) else {
        return undefined_read();
    };
    Ok(cross(sub(second, first), normalize(sub(third, first))))
}

pub fn read(bytes: &[u8]) -> Result<Scene, ImportError> {
    let mut stream = StreamReader::new(bytes, false)?;
    if stream.remaining_size() < 22 {
        return fail("File is either empty or corrupt");
    }
    let signature = &bytes[..8];
    if !signature.eq_ignore_ascii_case(b"quick3Do") && !signature.eq_ignore_ascii_case(b"quick3Ds") {
        return fail("Not a Quick3D file.");
    }
    let major = bytes[8];
    let minor = bytes[9];
    stream.inc_ptr(10)?;
    let mesh_count = stream.get_i4()? as u32;
    let material_count = stream.get_i4()? as u32;
    let texture_count = stream.get_i4()? as u32;
    check_allocation(material_count as u64, MATERIAL_SIZE)?;
    check_allocation(mesh_count as u64, MESH_SIZE)?;

    let mut meshes: Vec<SourceMesh> = Vec::new();
    let mut materials_read = 0usize;
    let mut scene_texture_count = 0u32;
    while stream.remaining_size() >= 1 {
        let chunk = stream.get_i1()? as u8;
        match chunk {
            b'm' => {
                for _ in 0..mesh_count {
                    meshes.push(read_mesh(&mut stream, texture_count, major, minor)?);
                }
            }
            b'c' => {
                for _ in 0..material_count {
                    materials_read += 1;
                    skip_material(&mut stream)?;
                }
            }
            b't' => {
                scene_texture_count = texture_count;
                skip_textures(&mut stream, texture_count)?;
            }
            b's' => {
                skip_scene(&mut stream)?;
                break;
            }
            _ => return fail("Quick3D: Unknown chunk"),
        }
    }

    if meshes.is_empty() {
        return fail("Quick3D: No meshes loaded");
    }
    // No materials: a default one.
    let material_total = materials_read.max(1);

    // The faces of each material, in mesh and face order.
    let mut groups: Vec<Vec<(usize, usize)>> = vec![Vec::new(); material_total];
    for (mesh_index, mesh) in meshes.iter_mut().enumerate() {
        for (face_index, face) in mesh.faces.iter_mut().enumerate() {
            if face.material as usize >= material_total {
                // "Quick3D: Material index overflow"
                face.material = 0;
            }
            groups[face.material as usize].push((mesh_index, face_index));
        }
    }

    let mut output = Vec::new();
    for group in groups.iter().filter(|group| !group.is_empty()) {
        let real = output.len() as u32;
        let vertex_count: usize = group.iter().map(|&(mesh, face)| meshes[mesh].faces[face].indices.len()).sum();
        let with_uv = real < scene_texture_count;
        let mut vertices = Vec::with_capacity(vertex_count);
        let mut normals = Vec::with_capacity(vertex_count);
        let mut uv = vec![[0.0f32; 3]; if with_uv { vertex_count } else { 0 }];
        let mut uv_cursor = 0;
        let mut faces = Vec::with_capacity(group.len());
        for &(mesh_index, face_index) in group {
            let source = &mut meshes[mesh_index];
            let index_count = source.faces[face_index].indices.len();
            let mut face_indices = Vec::with_capacity(index_count);
            let mut normal: Option<Vector3> = None;
            for corner in 0..index_count {
                let face = &mut source.faces[face_index];
                if face.indices[corner] as usize >= source.vertices.len() {
                    // "Quick3D: Vertex index overflow"
                    face.indices[corner] = 0;
                }
                let face = &source.faces[face_index];
                let index = face.indices[corner] as usize;
                vertices.push(source.vertices[index]);
                if index >= source.normals.len() && index_count >= 3 {
                    if normal.is_none() {
                        normal = Some(face_normal(source, face)?);
                    }
                    normals.push(normal.unwrap_or_default());
                } else {
                    let Some(&source_normal) = source.normals.get(index) else {
                        return undefined_read();
                    };
                    normals.push(source_normal);
                }
                if with_uv && !source.uv.is_empty() {
                    let mut coords = if source.previous_uv_index != u32::MAX && source.uv.len() >= source.vertices.len() {
                        source.uv[index]
                    } else {
                        let face = &mut source.faces[face_index];
                        if face.uv_indices[corner] as usize >= source.uv.len() {
                            // "Quick3D: Texture coordinate index overflow"
                            face.uv_indices[corner] = 0;
                        }
                        source.uv[source.faces[face_index].uv_indices[corner] as usize]
                    };
                    coords[1] = 1.0 - coords[1];
                    uv[uv_cursor] = coords;
                    uv_cursor += 1;
                }
                face_indices.push((vertices.len() - 1) as u32);
            }
            faces.push(face_indices);
        }
        let mut mesh = Mesh { vertex_count, vertices: Some(vertices), normals: Some(normals), faces: Some(faces), ..Mesh::default() };
        if with_uv {
            mesh.uv_components[0] = 2;
            mesh.texture_coords[0] = Some(uv);
        }
        output.push(mesh);
    }
    Ok(Scene { meshes: output })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(meshes: i32, materials: i32, textures: i32) -> Vec<u8> {
        let mut bytes = b"quick3Do30".to_vec();
        for value in [meshes, materials, textures] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    fn push_floats(bytes: &mut Vec<u8>, values: &[f32]) {
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }

    #[test]
    fn a_triangle_without_normals_gets_the_face_normal() {
        let mut bytes = header(1, 0, 0);
        bytes.push(b'm');
        bytes.extend_from_slice(&3i32.to_le_bytes());
        push_floats(&mut bytes, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 2.0, 0.0]);
        bytes.extend_from_slice(&1i32.to_le_bytes());
        bytes.extend_from_slice(&3i16.to_le_bytes());
        for index in [0i32, 1, 2] {
            bytes.extend_from_slice(&index.to_le_bytes());
        }
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&[0; 36]);
        let scene = read(&bytes).unwrap();
        let normals = scene.meshes[0].normals.as_ref().unwrap();
        // (1, 0, 0) ^ normalize(0, 2, 0).
        assert_eq!(normals[0], [0.0, 0.0, 1.0]);
    }
}
