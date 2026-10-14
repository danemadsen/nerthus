//! Assimp's Silo importer (`SIBImporter.cpp`): nested chunks (`HEAD`,
//! `SHAP`, `MATR`, `INST`, `LGHT`). A shape's faces get vertex normals
//! from walks around each corner's vertex across uncreased edges, are
//! moved into the shape's local frame, and become a mesh per material
//! (counting the materials read before the shape). Instances and lights
//! add no meshes.
//!
//! Kept quirks: the second normal pass starts where the first stopped;
//! each `FACS` chunk numbers its corners from 0 again and resizes the
//! normal and UV arrays to its own count; a `FTVS` face reads as many
//! corners as it says, on through the following faces; a chunk's end is
//! its position plus size in 32 bits. Reads and writes outside the
//! arrays, faces without corners and walks that never end (the C++ hangs)
//! are errors here.

use std::collections::{HashMap, HashSet};

use crate::matrix::Matrix4;
use crate::scene::{ImportError, Mesh, Scene, check_allocation, fail};
use crate::stream_reader::StreamReader;
use crate::vector::{Vector3, cross, square_length};

const POS: usize = 0;
const NRM: usize = 1;
const UV: usize = 2;
const N: usize = 3;
const NO_FACE: u32 = 0xffff_ffff;

/// `TAG(A, B, C, D)`.
const fn tag(name: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*name)
}

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

#[derive(Clone, Copy)]
struct Edge {
    face_a: u32,
    face_b: u32,
    creased: bool,
}

/// `SIBMesh`.
struct SibMesh {
    axis: Matrix4,
    positions: Vec<Vector3>,
    normals: Vec<Vector3>,
    uvs: Vec<Vector3>,
    idx: Vec<u32>,
    face_start: Vec<u32>,
    materials: Vec<u32>,
    edges: Vec<Edge>,
    edge_map: HashMap<(u32, u32), u32>,
}

impl SibMesh {
    /// `GetEdge`: the edge between two positions, created when new.
    fn edge(&mut self, a: u32, b: u32) -> usize {
        let pair = if a < b { (a, b) } else { (b, a) };
        if let Some(&index) = self.edge_map.get(&pair) {
            return index as usize;
        }
        let index = self.edges.len();
        self.edge_map.insert(pair, index as u32);
        self.edges.push(Edge { face_a: NO_FACE, face_b: NO_FACE, creased: false });
        index
    }

    fn idx_at(&self, index: usize) -> Result<u32, ImportError> {
        match self.idx.get(index) {
            Some(&value) => Ok(value),
            None => undefined("a read past the index list"),
        }
    }

    /// A face's corner count and the start of its corners.
    fn face(&self, face: usize) -> Result<(u32, usize), ImportError> {
        let start = self.face_start[face] as usize;
        let count = self.idx_at(start)?;
        if count == 0 {
            return undefined("a face without corners");
        }
        Ok((count, start + 1))
    }
}

/// The reader's chunk loop: chunks while 8 bytes are left to the limit,
/// each read with its end as the limit, then the reader at that end.
/// Endless when a wrapping chunk end steps back (the C++ hangs): counted.
struct Chunks {
    steps: usize,
    limit: usize,
}

impl Chunks {
    fn new(file_size: usize) -> Self {
        Chunks { steps: 0, limit: file_size * 4 + 1024 }
    }

    /// `ReadChunk` and the read limit: the tag and size, and the limit to
    /// restore.
    fn next(&mut self, stream: &mut StreamReader) -> Result<Option<(u32, u32, u32)>, ImportError> {
        if stream.remaining_size_to_limit() < 8 {
            return Ok(None);
        }
        self.steps += 1;
        if self.steps > self.limit {
            return fail("SIB: chunks that never end");
        }
        let tag = stream.get_u4()?.swap_bytes();
        let size = stream.get_u4()?;
        let end = (stream.current_pos() as u32).wrapping_add(size);
        let old_limit = stream.set_read_limit(end)?;
        Ok(Some((tag, size, old_limit)))
    }

    fn finish(stream: &mut StreamReader, old_limit: u32) -> Result<(), ImportError> {
        let end = stream.read_limit();
        stream.set_current_pos(end as usize)?;
        stream.set_read_limit(old_limit)?;
        Ok(())
    }
}

/// `ReadString`: UTF-16 code units, failing on unpaired surrogates.
fn read_string(stream: &mut StreamReader, units: u32) -> Result<(), ImportError> {
    if units == 0 {
        return Ok(());
    }
    check_allocation(units as u64, 2)?;
    let mut values = Vec::with_capacity(units as usize);
    for _ in 0..units {
        values.push(stream.get_u2()?);
    }
    let mut index = 0;
    while index < values.len() {
        let unit = values[index];
        index += 1;
        if (0xd800..=0xdbff).contains(&unit) {
            if index == values.len() {
                return fail("Invalid UTF-16");
            }
            let trail = values[index];
            index += 1;
            if !(0xdc00..=0xdfff).contains(&trail) {
                return fail("Invalid UTF-16");
            }
        } else if (0xdc00..=0xdfff).contains(&unit) {
            return fail("Invalid UTF-16");
        }
    }
    Ok(())
}

fn read_faces(mesh: &mut SibMesh, stream: &mut StreamReader) -> Result<(), ImportError> {
    let mut corner_index = 0u32;
    while stream.remaining_size_to_limit() > 0 {
        let count = stream.get_u4()?;
        let position = mesh.idx.len() + 1;
        let added = count.wrapping_mul(N as u32) as usize;
        check_allocation((position + added) as u64, 4)?;
        mesh.idx.resize(position + added, 0);
        mesh.idx[position - 1] = count;
        mesh.face_start.push((position - 1) as u32);
        mesh.materials.push(0);
        for corner in 0..count as usize {
            let point = stream.get_u4()?;
            if point as usize >= mesh.positions.len() {
                return fail("Vertex index is out of range.");
            }
            let slot = position + corner * N;
            if slot + UV >= mesh.idx.len() {
                return undefined("a write past the index list");
            }
            mesh.idx[slot + POS] = point;
            mesh.idx[slot + NRM] = corner_index;
            mesh.idx[slot + UV] = corner_index;
            corner_index += 1;
        }
    }
    mesh.normals.resize(corner_index as usize, [0.0; 3]);
    mesh.uvs.resize(corner_index as usize, [0.0; 3]);
    Ok(())
}

fn read_uvs(mesh: &mut SibMesh, stream: &mut StreamReader) -> Result<(), ImportError> {
    while stream.remaining_size_to_limit() > 0 {
        let face = stream.get_u4()?;
        let count = stream.get_u4()?;
        if face as usize >= mesh.face_start.len() {
            return fail("Invalid face index.");
        }
        let start = mesh.face_start[face as usize] as usize + 1;
        for corner in 0..count as usize {
            let id = mesh.idx_at(start + corner * N + UV)? as usize;
            let u = stream.get_f4()?;
            if id >= mesh.uvs.len() {
                return undefined("a write past the UVs");
            }
            mesh.uvs[id][0] = u;
            let v = stream.get_f4()?;
            mesh.uvs[id][1] = v;
        }
    }
    Ok(())
}

fn read_materials(mesh: &mut SibMesh, stream: &mut StreamReader) -> Result<(), ImportError> {
    let mut previous_face = stream.get_u4()?;
    let mut previous_material = stream.get_u4()?.wrapping_add(1);
    while stream.remaining_size_to_limit() > 0 {
        let face = stream.get_u4()?;
        let material = stream.get_u4()?.wrapping_add(1);
        while previous_face < face {
            if previous_face as usize >= mesh.materials.len() {
                return fail("Invalid face index.");
            }
            mesh.materials[previous_face as usize] = previous_material;
            previous_face += 1;
        }
        previous_face = face;
        previous_material = material;
    }
    while (previous_face as usize) < mesh.materials.len() {
        mesh.materials[previous_face as usize] = previous_material;
        previous_face += 1;
    }
    Ok(())
}

/// `ReadAxis`: the translation, then the three axes as columns.
fn read_axis(stream: &mut StreamReader) -> Result<Matrix4, ImportError> {
    let mut axis = Matrix4::IDENTITY;
    for slot in [3, 7, 11] {
        axis.0[slot] = stream.get_f4()?;
    }
    axis.0[15] = 1.0;
    for column in 0..3 {
        for row in 0..3 {
            axis.0[row * 4 + column] = stream.get_f4()?;
        }
        axis.0[12 + column] = 0.0;
    }
    Ok(axis)
}

fn connect_faces(mesh: &mut SibMesh) -> Result<(), ImportError> {
    for face in 0..mesh.face_start.len() {
        let (count, start) = mesh.face(face)?;
        let mut previous = mesh.idx_at(start + (count as usize - 1) * N + POS)?;
        for corner in 0..count as usize {
            let next = mesh.idx_at(start + corner * N + POS)?;
            let edge = mesh.edge(previous, next);
            let edge = &mut mesh.edges[edge];
            if edge.face_a == NO_FACE {
                edge.face_a = face as u32;
            } else if edge.face_b == NO_FACE {
                edge.face_b = face as u32;
            }
            previous = next;
        }
    }
    Ok(())
}

/// `CalculateVertexNormal`: two walks around `position` across uncreased
/// edges, the second from where the first stopped.
fn vertex_normal(mesh: &mut SibMesh, mut face: u32, position: u32, face_normals: &[Vector3]) -> Result<Vector3, ImportError> {
    let mut normal = [0.0f32; 3];
    for _pass in 0..2 {
        normal = [0.0; 3];
        let start_face = face;
        let mut previous_face = face;
        let mut seen = HashSet::new();
        loop {
            if !seen.insert((face, previous_face)) {
                return fail("SIB: a vertex normal walk that never ends");
            }
            let face_normal = face_normals[face as usize];
            normal = [normal[0] + face_normal[0], normal[1] + face_normal[1], normal[2] + face_normal[2]];
            let mut next_face = NO_FACE;
            let (count, start) = mesh.face(face as usize)?;
            let mut a = mesh.idx_at(start + (count as usize - 1) * N + POS)?;
            for corner in 0..count as usize {
                let b = mesh.idx_at(start + corner * N + POS)?;
                if a == position || b == position {
                    let edge = mesh.edge(a, b);
                    let edge = mesh.edges[edge];
                    if (edge.face_a == face || edge.face_b == face) && !edge.creased {
                        if edge.face_a != previous_face && edge.face_a != face && edge.face_a != NO_FACE {
                            next_face = edge.face_a;
                        } else if edge.face_b != previous_face && edge.face_b != face && edge.face_b != NO_FACE {
                            next_face = edge.face_b;
                        }
                    }
                }
                a = b;
            }
            if next_face == NO_FACE || next_face == start_face {
                break;
            }
            previous_face = face;
            face = next_face;
        }
    }
    let length = square_length(normal).sqrt();
    if length > 0.000_000_001 {
        let inverse = 1.0 / length;
        normal = [normal[0] * inverse, normal[1] * inverse, normal[2] * inverse];
    }
    Ok(normal)
}

fn calculate_normals(mesh: &mut SibMesh) -> Result<(), ImportError> {
    let face_count = mesh.face_start.len();
    let mut face_normals = vec![[0.0f32; 3]; face_count];
    for (face, slot) in face_normals.iter_mut().enumerate() {
        let (count, start) = mesh.face(face)?;
        let mut sum = [0.0f32; 3];
        let mut previous = mesh.idx_at(start + (count as usize - 1) * N + POS)? as usize;
        for corner in 0..count as usize {
            let next = mesh.idx_at(start + corner * N + POS)? as usize;
            let (Some(&a), Some(&b)) = (mesh.positions.get(previous), mesh.positions.get(next)) else {
                return undefined("a read past the positions");
            };
            let product = cross(a, b);
            sum = [sum[0] + product[0], sum[1] + product[1], sum[2] + product[2]];
            previous = next;
        }
        *slot = sum;
    }
    for face in 0..face_count {
        let (count, start) = mesh.face(face)?;
        for corner in 0..count as usize {
            let position = mesh.idx_at(start + corner * N + POS)?;
            let normal_index = mesh.idx_at(start + corner * N + NRM)? as usize;
            let normal = vertex_normal(mesh, face as u32, position, &face_normals)?;
            if normal_index >= mesh.normals.len() {
                return undefined("a write past the normals");
            }
            mesh.normals[normal_index] = normal;
        }
    }
    Ok(())
}

/// `ReadShape`: its meshes, a mesh per material with faces.
fn read_shape(stream: &mut StreamReader, chunks: &mut Chunks, material_count: usize) -> Result<Vec<Mesh>, ImportError> {
    let mut mesh = SibMesh {
        axis: Matrix4::IDENTITY,
        positions: Vec::new(),
        normals: Vec::new(),
        uvs: Vec::new(),
        idx: Vec::new(),
        face_start: Vec::new(),
        materials: Vec::new(),
        edges: Vec::new(),
        edge_map: HashMap::new(),
    };
    while let Some((chunk, size, old_limit)) = chunks.next(stream)? {
        match chunk {
            value if value == tag(b"VRTS") => {
                let count = size / 12;
                check_allocation(count as u64, 12)?;
                mesh.positions.resize(count as usize, [0.0; 3]);
                for position in &mut mesh.positions {
                    *position = [stream.get_f4()?, stream.get_f4()?, stream.get_f4()?];
                }
            }
            value if value == tag(b"FACS") => read_faces(&mut mesh, stream)?,
            value if value == tag(b"FTVS") => read_uvs(&mut mesh, stream)?,
            value if value == tag(b"SNAM") => read_string(stream, size / 2)?,
            value if value == tag(b"FAMA") => read_materials(&mut mesh, stream)?,
            value if value == tag(b"AXIS") => mesh.axis = read_axis(stream)?,
            value if value == tag(b"EDGS") => {
                while stream.remaining_size_to_limit() > 0 {
                    let a = stream.get_u4()?;
                    let b = stream.get_u4()?;
                    mesh.edge(a, b);
                }
            }
            value if value == tag(b"ECRS") => {
                while stream.remaining_size_to_limit() > 0 {
                    let edge = stream.get_u4()? as usize;
                    if edge >= mesh.edges.len() {
                        return fail("SIB: Invalid edge index.");
                    }
                    mesh.edges[edge].creased = true;
                }
            }
            // Mirror planes, display information and unknown chunks.
            _ => {}
        }
        Chunks::finish(stream, old_limit)?;
    }

    connect_faces(&mut mesh)?;
    calculate_normals(&mut mesh)?;
    let world_to_local = mesh.axis.inverse();
    let mut normal_matrix = world_to_local;
    normal_matrix.0[3] = 0.0;
    normal_matrix.0[7] = 0.0;
    normal_matrix.0[11] = 0.0;
    let normal_matrix = normal_matrix.inverse().transpose();

    struct TempMesh {
        vertices: Vec<Vector3>,
        normals: Vec<Vector3>,
        uvs: Vec<Vector3>,
        faces: Vec<Vec<u32>>,
    }
    let mut temp: Vec<TempMesh> = (0..material_count).map(|_| TempMesh { vertices: Vec::new(), normals: Vec::new(), uvs: Vec::new(), faces: Vec::new() }).collect();
    for face in 0..mesh.face_start.len() {
        let mut material = mesh.materials[face] as usize;
        if material >= temp.len() {
            // "SIB: Face material index is invalid."
            material = 0;
        }
        let start = mesh.face_start[face] as usize;
        let count = mesh.idx_at(start)?;
        let destination = &mut temp[material];
        let mut indices = Vec::with_capacity(count as usize);
        for corner in 0..count as usize {
            let slot = start + 1 + corner * N;
            let (Some(&position), Some(&normal), Some(&uv)) = (mesh.idx.get(slot + POS), mesh.idx.get(slot + NRM), mesh.idx.get(slot + UV)) else {
                return undefined("a read past the index list");
            };
            let (Some(&position), Some(&normal), Some(&uv)) = (mesh.positions.get(position as usize), mesh.normals.get(normal as usize), mesh.uvs.get(uv as usize)) else {
                return undefined("a read past a vertex array");
            };
            indices.push(destination.vertices.len() as u32);
            destination.vertices.push(world_to_local.transform(position));
            destination.normals.push(normal_matrix.transform(normal));
            destination.uvs.push(uv);
        }
        destination.faces.push(indices);
    }

    let mut meshes = Vec::new();
    for source in temp {
        if source.faces.is_empty() {
            continue;
        }
        let mut output = Mesh {
            vertex_count: source.vertices.len(),
            vertices: Some(source.vertices),
            normals: Some(source.normals),
            faces: Some(source.faces),
            ..Mesh::default()
        };
        output.uv_components[0] = 2;
        output.texture_coords[0] = Some(source.uvs);
        meshes.push(output);
    }
    Ok(meshes)
}

fn read_material(stream: &mut StreamReader) -> Result<(), ImportError> {
    // Diffuse, ambient, specular and emissive colours, four words each,
    // and the shininess.
    for _ in 0..17 {
        stream.get_u4()?;
    }
    let name_length = stream.get_u4()?;
    read_string(stream, name_length / 2)?;
    let texture_length = stream.get_u4()?;
    read_string(stream, texture_length / 2)
}

fn read_light(stream: &mut StreamReader, chunks: &mut Chunks) -> Result<(), ImportError> {
    while let Some((chunk, size, old_limit)) = chunks.next(stream)? {
        if chunk == tag(b"LNFO") {
            // Type, position, direction, three colours of four words,
            // spot exponent and cutoff, three attenuations.
            for _ in 0..24 {
                stream.get_u4()?;
            }
        } else if chunk == tag(b"SNAM") {
            read_string(stream, size / 2)?;
        }
        Chunks::finish(stream, old_limit)?;
    }
    Ok(())
}

fn read_instance(stream: &mut StreamReader, chunks: &mut Chunks, shape_count: usize) -> Result<(), ImportError> {
    let mut shape_index = 0u32;
    while let Some((chunk, size, old_limit)) = chunks.next(stream)? {
        match chunk {
            value if value == tag(b"AXIS") => {
                read_axis(stream)?;
            }
            value if value == tag(b"INSI") => shape_index = stream.get_u4()?,
            value if value == tag(b"SMTX") => {
                for _ in 0..16 {
                    stream.get_f4()?;
                }
            }
            value if value == tag(b"SNAM") => read_string(stream, size / 2)?,
            _ => {}
        }
        Chunks::finish(stream, old_limit)?;
    }
    if shape_index as usize >= shape_count {
        return fail("SIB: Invalid shape index.");
    }
    Ok(())
}

pub fn read(bytes: &[u8]) -> Result<Scene, ImportError> {
    let mut stream = StreamReader::new(bytes, false)?;
    if stream.remaining_size() < 16 {
        return fail("SIB file is either empty or corrupt");
    }
    let mut chunks = Chunks::new(bytes.len());
    // The default material comes first.
    let mut material_count = 1;
    let mut shape_count = 0;
    let mut meshes = Vec::new();
    while let Some((chunk, _size, old_limit)) = chunks.next(&mut stream)? {
        match chunk {
            value if value == tag(b"HEAD") => {
                let version = stream.get_u4()?;
                if !(1..=2).contains(&version) {
                    return fail("SIB: Unsupported file version.");
                }
            }
            value if value == tag(b"SHAP") => {
                meshes.extend(read_shape(&mut stream, &mut chunks, material_count)?);
                shape_count += 1;
            }
            value if value == tag(b"INST") => read_instance(&mut stream, &mut chunks, shape_count)?,
            value if value == tag(b"MATR") => {
                read_material(&mut stream)?;
                material_count += 1;
            }
            value if value == tag(b"LGHT") => read_light(&mut stream, &mut chunks)?,
            // Groups, textures and unknown chunks.
            _ => {}
        }
        Chunks::finish(&mut stream, old_limit)?;
    }
    Ok(Scene { meshes })
}
