//! Assimp's OBJ importer (`ObjFileImporter`), which mesh loading reaches
//! for the extensions MeshPort's case-sensitive ".obj" check lets through
//! (".OBJ").

mod mtl;
mod parser;
mod tools;

use parser::{Files, Model, ObjMesh, ObjParser};

use crate::io::FileSystem;
use crate::scene::{self, ImportError, PRIMITIVE_LINE, PRIMITIVE_POINT, PRIMITIVE_POLYGON, PRIMITIVE_TRIANGLE, Scene, fail};

/// `ObjMinSize`.
const MIN_FILE_SIZE: usize = 16;

/// `AI_MAX_VERTICES`.
const MAX_VERTICES: u32 = 0x7fff_ffff;

/// `ObjFileImporter::InternReadFile`.
pub fn read(path: &str, bytes: &[u8], system: &FileSystem) -> Result<Scene, ImportError> {
    if bytes.len() < MIN_FILE_SIZE {
        return fail("OBJ-file is too small.");
    }
    // The file's folder (when not empty) goes on the directory stack the
    // material library is looked up through, and comes off it (or what is
    // on top) after an import that does not fail.
    let path_bytes = path.as_bytes();
    if let Some(position) = path_bytes.iter().rposition(|&byte| byte == b'\\' || byte == b'/') {
        system.push_directory(&path_bytes[..position]);
    }
    let files = Files { system, original_file: path_bytes.to_vec() };
    let model = ObjParser::parse(bytes, files)?;
    let scene = create_data(&model)?;
    if system.directory_count() > 0 {
        system.pop_directory();
    }
    Ok(scene)
}

/// `CreateDataFromImport`: a mesh per object mesh with faces, or one
/// point mesh of every vertex when there are no objects.
fn create_data(model: &Model) -> Result<Scene, ImportError> {
    let mut meshes = Vec::new();
    if !model.objects.is_empty() {
        for object in &model.objects {
            for &mesh_index in &object.meshes {
                if let Some(mesh) = create_topology(model, &model.meshes[mesh_index])?
                    && mesh.face_count() > 0
                {
                    meshes.push(mesh);
                }
            }
        }
        return Ok(Scene { meshes });
    }
    if model.vertices.is_empty() {
        return Ok(Scene { meshes });
    }
    let vertex_count = model.vertices.len();
    let mut mesh = scene::Mesh {
        primitive_types: PRIMITIVE_POINT,
        vertex_count,
        vertices: Some(model.vertices.clone()),
        ..scene::Mesh::default()
    };
    if !model.normals.is_empty() {
        if model.normals.len() < vertex_count {
            return fail("OBJ: vertex normal index out of range");
        }
        mesh.normals = Some(model.normals[..vertex_count].to_vec());
    }
    if !model.vertex_colors.is_empty() {
        if model.vertex_colors.len() < vertex_count {
            return fail("OBJ: vertex color index out of range");
        }
        let colors = model.vertex_colors[..vertex_count].iter().map(|color| [color[0], color[1], color[2], 1.0]).collect();
        mesh.colors[0] = Some(colors);
    }
    meshes.push(mesh);
    Ok(Scene { meshes })
}

/// `createTopology`: faces (a line of n points is n − 1 segments, a point
/// statement one face per point), then the vertex array.
fn create_topology(model: &Model, source: &ObjMesh) -> Result<Option<scene::Mesh>, ImportError> {
    if source.faces.is_empty() {
        return Ok(None);
    }
    let mut mesh = scene::Mesh { name: String::from_utf8_lossy(&source.name).into_owned(), ..scene::Mesh::default() };
    let mut face_count = 0u32;
    for face in &source.faces {
        let point_count = face.vertices.len() as u32;
        if face.kind == PRIMITIVE_LINE {
            face_count = face_count.wrapping_add(point_count.wrapping_sub(1));
            mesh.primitive_types |= PRIMITIVE_LINE;
        } else if face.kind == PRIMITIVE_POINT {
            face_count = face_count.wrapping_add(point_count);
            mesh.primitive_types |= PRIMITIVE_POINT;
        } else {
            face_count += 1;
            mesh.primitive_types |= if point_count > 3 { PRIMITIVE_POLYGON } else { PRIMITIVE_TRIANGLE };
        }
    }
    let mut faces: Vec<Vec<u32>> = Vec::new();
    let mut index_count = 0u32;
    if face_count > 0 {
        for face in &source.faces {
            let point_count = face.vertices.len();
            if face.kind == PRIMITIVE_LINE {
                for _ in 1..point_count {
                    faces.push(vec![0; 2]);
                    index_count += 2;
                }
            } else if face.kind == PRIMITIVE_POINT {
                for _ in 0..point_count {
                    faces.push(vec![0; 1]);
                    index_count += 1;
                }
            } else {
                faces.push(vec![0; point_count]);
                index_count += point_count as u32;
            }
        }
    }
    create_vertex_array(model, source, &mut mesh, &mut faces, index_count)?;
    if face_count > 0 {
        mesh.faces = Some(faces);
    }
    Ok(Some(mesh))
}

/// `createVertexArray`: one vertex per face corner; a line's inner points
/// are duplicated so each segment has its own pair.
fn create_vertex_array(model: &Model, source: &ObjMesh, mesh: &mut scene::Mesh, faces: &mut [Vec<u32>], index_count: u32) -> Result<(), ImportError> {
    if source.index_count < 1 {
        return Ok(());
    }
    let vertex_count = index_count;
    if vertex_count == 0 {
        return fail("OBJ: no vertices");
    }
    if vertex_count > MAX_VERTICES {
        return fail("OBJ: Too many vertices");
    }
    let count = vertex_count as usize;
    let mut vertices = vec![[0.0f32; 3]; count];
    let mut normals = (!model.normals.is_empty() && source.has_normals).then(|| vec![[0.0f32; 3]; count]);
    let mut colors = (!model.vertex_colors.is_empty()).then(|| vec![[0.0f32; 4]; count]);
    let mut coordinates = None;
    if !model.texture_coords.is_empty() && source.uv_coordinates != 0 {
        mesh.uv_components[0] = model.texture_coord_dimension;
        coordinates = Some(vec![[0.0f32; 3]; count]);
    }

    let mut normals_valid = true;
    let mut coordinates_valid = true;
    let mut new_index = 0u32;
    let mut out_index = 0usize;
    for face in &source.faces {
        let mut out_vertex = 0usize;
        let point_count = face.vertices.len();
        for corner in 0..point_count {
            let vertex = face.vertices[corner] as usize;
            if vertex >= model.vertices.len() {
                return fail("OBJ: vertex index out of range");
            }
            if vertex_count <= new_index {
                return fail("OBJ: bad vertex index");
            }
            let slot = new_index as usize;
            vertices[slot] = model.vertices[vertex];
            if normals_valid && !model.normals.is_empty() && corner < face.normals.len() {
                let normal = face.normals[corner] as usize;
                if normal >= model.normals.len() {
                    normals_valid = false;
                } else if let Some(normals) = normals.as_mut() {
                    normals[slot] = model.normals[normal];
                }
            }
            if let Some(color) = model.vertex_colors.get(vertex)
                && let Some(colors) = colors.as_mut()
            {
                colors[slot] = [color[0], color[1], color[2], 1.0];
            }
            if coordinates_valid && !model.texture_coords.is_empty() && corner < face.texture_coords.len() {
                let coordinate = face.texture_coords[corner] as usize;
                if coordinate >= model.texture_coords.len() {
                    coordinates_valid = false;
                } else if let Some(coordinates) = coordinates.as_mut() {
                    coordinates[slot] = model.texture_coords[coordinate];
                }
            }

            let destination = out_index;
            let last = corner == point_count - 1;
            if face.kind != PRIMITIVE_LINE || !last {
                faces[destination][out_vertex] = new_index;
                out_vertex += 1;
            }
            if face.kind == PRIMITIVE_POINT {
                out_index += 1;
                out_vertex = 0;
            } else if face.kind == PRIMITIVE_LINE {
                out_vertex = 0;
                if !last {
                    out_index += 1;
                }
                if corner != 0 {
                    if !last {
                        if vertex_count <= new_index + 1 {
                            return fail("OBJ: bad vertex index");
                        }
                        let next = slot + 1;
                        vertices[next] = vertices[slot];
                        if !face.normals.is_empty() && !model.normals.is_empty() && let Some(normals) = normals.as_mut() {
                            normals[next] = normals[slot];
                        }
                        if !model.texture_coords.is_empty() && let Some(coordinates) = coordinates.as_mut() {
                            coordinates[next] = coordinates[slot];
                        }
                        new_index += 1;
                    }
                    faces[destination - 1][1] = new_index;
                }
            } else if last {
                out_index += 1;
            }
            new_index += 1;
        }
    }

    mesh.vertex_count = count;
    mesh.vertices = Some(vertices);
    mesh.normals = if normals_valid { normals } else { None };
    mesh.colors[0] = colors;
    mesh.texture_coords[0] = if coordinates_valid { coordinates } else { None };
    Ok(())
}
