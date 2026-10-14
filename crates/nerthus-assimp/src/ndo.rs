//! Assimp's Nendo importer (`NDOLoader.cpp`): big-endian objects of
//! winged edges, faces and vertices (16-bit counts and indices before
//! version 1.2). A mesh per object with vertices: each face key (an
//! edge's left or right face, the last edge naming it winning) walks its
//! edge ring from that edge, a vertex per step.
//!
//! An index past the edges or vertices reads past them, undefined
//! behaviour, an error here; so is a ring that never comes back to its
//! first edge, which the C++ walks until memory runs out.

use std::collections::BTreeMap;

use crate::scene::{ImportError, Mesh, Scene, check_allocation, fail};
use crate::stream_reader::StreamReader;

/// `NDOImporter::Edge`.
struct Edge {
    edge: [u32; 8],
}

/// `NDOImporter::Object`, as far as the mesh needs it.
#[derive(Default)]
struct Object {
    edges: Vec<Edge>,
    vertices: Vec<[f32; 3]>,
}

/// A count or index: 32-bit from version 1.2, else 16-bit.
fn read_index(reader: &mut StreamReader, file_format: u32) -> Result<u32, ImportError> {
    if file_format >= 12 {
        reader.get_u4()
    } else {
        Ok(reader.get_u2()? as u32)
    }
}

fn read_object(reader: &mut StreamReader, file_format: u32) -> Result<Object, ImportError> {
    let mut object = Object::default();
    let name_length = read_index(reader, file_format)?;
    if u32::MAX - 76 < name_length {
        return fail("Invalid name length");
    }
    // The name, then unknown data.
    reader.inc_ptr((name_length + 76) as i64)?;

    let edge_count = read_index(reader, file_format)?;
    check_allocation(edge_count as u64, 44)?;
    for _ in 0..edge_count {
        let mut edge = [0u32; 8];
        for value in &mut edge {
            *value = read_index(reader, file_format)?;
        }
        if file_format >= 11 {
            // Whether the edge is hard.
            reader.get_u1()?;
        }
        for _ in 0..8 {
            // The edge's colours.
            reader.get_u1()?;
        }
        object.edges.push(Edge { edge });
    }

    let face_count = read_index(reader, file_format)?;
    check_allocation(face_count as u64, 4)?;
    for _ in 0..face_count {
        read_index(reader, file_format)?;
    }

    let vertex_count = read_index(reader, file_format)?;
    check_allocation(vertex_count as u64, 16)?;
    for _ in 0..vertex_count {
        read_index(reader, file_format)?;
        let x = reader.get_f4()?;
        let y = reader.get_f4()?;
        let z = reader.get_f4()?;
        object.vertices.push([x, y, z]);
    }

    // Texture coordinates, skipped.
    for _ in 0..2 {
        let count = read_index(reader, file_format)?;
        for _ in 0..count {
            read_index(reader, file_format)?;
        }
    }

    if reader.get_u1()? != 0 {
        // A run-length coded texture, skipped.
        let width = reader.get_u2()? as u32;
        let height = reader.get_u2()? as u32;
        let mut filled = 0u32;
        while filled < width * height {
            let repeat = reader.get_u1()? as u32;
            reader.get_u1()?;
            reader.get_u1()?;
            reader.get_u1()?;
            filled = filled.wrapping_add(repeat);
        }
    }
    Ok(object)
}

/// The mesh of an object: a face per face key, walked around its ring.
fn build_mesh(object: &Object) -> Result<Mesh, ImportError> {
    let mut face_table = BTreeMap::new();
    for (index, edge) in object.edges.iter().enumerate() {
        face_table.insert(edge.edge[2], index as u32);
        face_table.insert(edge.edge[3], index as u32);
    }

    let mut vertices = Vec::new();
    let mut faces = Vec::with_capacity(face_table.len());
    for (&key, &first_edge) in &face_table {
        let mut indices = Vec::new();
        let mut current = first_edge;
        let mut steps = 0usize;
        loop {
            let Some(edge) = object.edges.get(current as usize) else {
                return fail("undefined behaviour: an edge index past the edges");
            };
            let (next_edge, next_vertex) = if key == edge.edge[3] { (edge.edge[5], edge.edge[1]) } else { (edge.edge[4], edge.edge[0]) };
            let Some(&vertex) = object.vertices.get(next_vertex as usize) else {
                return fail("undefined behaviour: a vertex index past the vertices");
            };
            indices.push(vertices.len() as u32);
            vertices.push(vertex);
            current = next_edge;
            if current == first_edge {
                break;
            }
            steps += 1;
            if steps > object.edges.len() {
                // The ring loops without its first edge: the C++ walks it
                // until memory runs out.
                return fail("Internal error");
            }
        }
        faces.push(indices);
    }
    Ok(Mesh { vertex_count: vertices.len(), vertices: Some(vertices), faces: Some(faces), ..Mesh::default() })
}

pub fn read(bytes: &[u8]) -> Result<Scene, ImportError> {
    let mut reader = StreamReader::new(bytes, true)?;
    reader.inc_ptr(9)?;
    if &bytes[..6] != b"nendo " {
        return fail("Not a Nendo file; magic signature missing");
    }
    let file_format = match &bytes[6..9] {
        b"1.0" => 10,
        b"1.1" => 11,
        // 1.2, and any other version read as one.
        _ => 12,
    };

    // Flags.
    reader.inc_ptr(2)?;
    if file_format >= 12 {
        reader.inc_ptr(2)?;
    }
    let object_count = reader.get_u1()? as usize;
    let mut objects = Vec::with_capacity(object_count);
    for _ in 0..object_count {
        if reader.get_i1()? == 0 {
            // An empty object.
            objects.push(Object::default());
            continue;
        }
        objects.push(read_object(&mut reader, file_format)?);
    }

    let mut meshes = Vec::new();
    for object in &objects {
        let mesh = build_mesh(object)?;
        if mesh.vertex_count > 0 {
            meshes.push(mesh);
        }
    }
    Ok(Scene { meshes })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A version 1.2 file with one object: a triangle's three edges.
    fn triangle() -> Vec<u8> {
        let mut bytes = b"nendo 1.2".to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 0, 1, 1]);
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(&[0; 76]);
        bytes.extend_from_slice(&3u32.to_be_bytes());
        // Edges: start vertex, end vertex, left face, right face, left
        // next, right next, then two unused.
        let edges = [[0, 1, 7, 9, 1, 2, 0, 0], [1, 2, 7, 9, 2, 0, 0, 0], [2, 0, 7, 9, 0, 1, 0, 0]];
        for edge in edges {
            for value in edge {
                bytes.extend_from_slice(&(value as u32).to_be_bytes());
            }
            bytes.extend_from_slice(&[0; 9]);
        }
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(&3u32.to_be_bytes());
        for vertex in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            bytes.extend_from_slice(&0u32.to_be_bytes());
            for value in vertex {
                bytes.extend_from_slice(&value.to_be_bytes());
            }
        }
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.push(0);
        bytes
    }

    #[test]
    fn rings_walk_to_faces() {
        let scene = read(&triangle()).unwrap();
        let mesh = &scene.meshes[0];
        assert_eq!(mesh.faces.as_ref().unwrap(), &vec![vec![0, 1, 2], vec![3, 4, 5]]);
        let vertices = mesh.vertices.as_ref().unwrap();
        // Face 7 from edge 2 along the left ring, face 9 along the right.
        assert_eq!(vertices[0], [0.0, 1.0, 0.0]);
        assert_eq!(vertices[3], [0.0, 0.0, 0.0]);
        assert_eq!(vertices[4], [0.0, 1.0, 0.0]);
    }

    #[test]
    fn a_ring_missing_its_first_edge_fails() {
        let mut bytes = triangle();
        // Edge 0's right next points at itself.
        let right_next = 9 + 6 + 4 + 76 + 4 + 5 * 4;
        bytes[right_next..right_next + 4].copy_from_slice(&0u32.to_be_bytes());
        assert!(read(&bytes).is_err());
    }
}
