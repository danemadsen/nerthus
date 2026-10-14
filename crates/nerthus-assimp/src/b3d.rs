//! Assimp's BlitzBasic 3D importer (`B3DImporter.cpp`): nested chunks
//! (`BB3D` holding `TEXS`, `BRUS` and `NODE`s, nodes holding `MESH`,
//! `BONE`, `ANIM`, `KEYS` and child `NODE`s, meshes holding `VRTS` and
//! `TRIS`). A mesh per `TRIS` chunk, three vertices per triangle from the
//! file's one vertex list, mirrored along z with the winding flipped.
//!
//! Kept quirks: the last `VRTS` chunk's flags decide for every mesh
//! whether it gets normals and texture coordinates; a chunk's end can lie
//! past its parent's or the file's (the size left wraps, so reading goes
//! on to the end of the file); a string the file ends inside reads as
//! empty. Bones, animation keys and node transforms don't reach the mesh;
//! only their reads and checks are kept.

use crate::scene::{ImportError, Mesh, PRIMITIVE_TRIANGLE, Scene, check_allocation, fail};

/// Node nesting the C++ recursion would only end with a stack overflow.
const MAX_DEPTH: usize = 2048;

/// `sizeof(B3DImporter::Vertex)` and `sizeof(aiFace)`.
const VERTEX_SIZE: u64 = 56;
const FACE_SIZE: u64 = 16;

fn data_error<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("B3D Importer - error in B3D file data: {what}"))
}

#[derive(Clone, Copy, Default)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    texcoords: [f32; 3],
}

struct Loader<'a> {
    buffer: &'a [u8],
    position: usize,
    stack: Vec<usize>,
    texture_count: usize,
    material_count: usize,
    vertices: Vec<Vertex>,
    /// Each `TRIS` chunk's triangles, as vertex list indices.
    meshes: Vec<Vec<[u32; 3]>>,
    node_count: usize,
    vertex_flags: i32,
    texcoord_sets: i32,
    texcoord_size: i32,
}

impl Loader<'_> {
    fn read_byte(&mut self) -> Result<u8, ImportError> {
        if self.position >= self.buffer.len() {
            return data_error("EOF");
        }
        self.position += 1;
        Ok(self.buffer[self.position - 1])
    }

    fn read_word(&mut self) -> Result<[u8; 4], ImportError> {
        if self.position + 4 > self.buffer.len() {
            return data_error("EOF");
        }
        let start = self.position;
        self.position += 4;
        Ok([self.buffer[start], self.buffer[start + 1], self.buffer[start + 2], self.buffer[start + 3]])
    }

    fn read_int(&mut self) -> Result<i32, ImportError> {
        Ok(i32::from_le_bytes(self.read_word()?))
    }

    fn read_float(&mut self) -> Result<f32, ImportError> {
        Ok(f32::from_le_bytes(self.read_word()?))
    }

    fn read_floats<const N: usize>(&mut self) -> Result<[f32; N], ImportError> {
        let mut values = [0.0f32; N];
        for value in &mut values {
            *value = self.read_float()?;
        }
        Ok(values)
    }

    /// `ReadString`: to a NUL; empty when the file ends first.
    fn read_string(&mut self) -> Result<(), ImportError> {
        if self.position > self.buffer.len() {
            return data_error("EOF");
        }
        while self.position < self.buffer.len() {
            if self.read_byte()? == 0 {
                break;
            }
        }
        Ok(())
    }

    /// `ReadChunk`: the tag, with the chunk's end pushed.
    fn read_chunk(&mut self) -> Result<[u8; 4], ImportError> {
        let mut tag = [0u8; 4];
        for byte in &mut tag {
            *byte = self.read_byte()?;
        }
        let size = self.read_int()? as u32;
        self.stack.push(self.position + size as usize);
        Ok(tag)
    }

    fn exit_chunk(&mut self) {
        self.position = self.stack.pop().unwrap_or(self.position);
    }

    /// `ChunkSize`: to the chunk's end, wrapping when past it.
    fn chunk_size(&self) -> usize {
        self.stack.last().copied().unwrap_or(0).wrapping_sub(self.position)
    }

    fn read_texs(&mut self) -> Result<(), ImportError> {
        while self.chunk_size() != 0 {
            self.read_string()?;
            // Flags, blend, position, scale, rotation.
            self.read_int()?;
            self.read_int()?;
            self.read_floats::<5>()?;
            self.texture_count += 1;
        }
        Ok(())
    }

    fn read_brus(&mut self) -> Result<(), ImportError> {
        let texture_slots = self.read_int()?;
        if !(0..=8).contains(&texture_slots) {
            return data_error("Bad texture count");
        }
        while self.chunk_size() != 0 {
            self.read_string()?;
            // Colour, alpha, shininess, blend, effects.
            self.read_floats::<5>()?;
            self.read_int()?;
            self.read_int()?;
            for _ in 0..texture_slots {
                let texture = self.read_int()?;
                if texture < -1 || (texture >= 0 && texture as usize >= self.texture_count) {
                    return data_error("Bad texture id");
                }
            }
            self.material_count += 1;
        }
        Ok(())
    }

    fn read_vrts(&mut self) -> Result<(), ImportError> {
        self.vertex_flags = self.read_int()?;
        self.texcoord_sets = self.read_int()?;
        self.texcoord_size = self.read_int()?;
        if !(0..=4).contains(&self.texcoord_sets) || !(0..=4).contains(&self.texcoord_size) {
            return data_error("Bad texcoord data");
        }
        let normal_size = if self.vertex_flags & 1 != 0 { 12 } else { 0 };
        let colour_size = if self.vertex_flags & 2 != 0 { 16 } else { 0 };
        let vertex_size = 12 + normal_size + colour_size + self.texcoord_sets * self.texcoord_size * 4;
        let count = self.chunk_size() / vertex_size as usize;
        let first = self.vertices.len();
        check_allocation((first + count) as u64, VERTEX_SIZE)?;
        // Reads past the file fail; no need to make room for them.
        if count.saturating_mul(vertex_size as usize) > self.buffer.len().saturating_sub(self.position) {
            return data_error("EOF");
        }
        for _ in 0..count {
            let mut vertex = Vertex { position: self.read_floats::<3>()?, ..Vertex::default() };
            if self.vertex_flags & 1 != 0 {
                vertex.normal = self.read_floats::<3>()?;
            }
            if self.vertex_flags & 2 != 0 {
                self.read_floats::<4>()?;
            }
            for set in 0..self.texcoord_sets {
                let mut coords = [0.0f32; 4];
                for value in coords.iter_mut().take(self.texcoord_size as usize) {
                    *value = self.read_float()?;
                }
                coords[1] = 1.0 - coords[1];
                if set == 0 {
                    vertex.texcoords = [coords[0], coords[1], coords[2]];
                }
            }
            self.vertices.push(vertex);
        }
        Ok(())
    }

    fn read_tris(&mut self, first_vertex: i32) -> Result<(), ImportError> {
        let material = self.read_int()?;
        if material != -1 && (material < 0 || material as usize >= self.material_count) {
            return data_error("Bad material id");
        }
        let count = self.chunk_size() / 12;
        check_allocation(count as u64, FACE_SIZE)?;
        if count.saturating_mul(12) > self.buffer.len().saturating_sub(self.position) {
            return data_error("EOF");
        }
        let vertex_count = self.vertices.len() as i32;
        let mut triangles = Vec::with_capacity(count);
        for _ in 0..count {
            let mut corners = [0u32; 3];
            for corner in &mut corners {
                let index = self.read_int()?.wrapping_add(first_vertex);
                if index < 0 || index >= vertex_count {
                    return data_error("Bad triangle index");
                }
                *corner = index as u32;
            }
            triangles.push(corners);
        }
        self.meshes.push(triangles);
        Ok(())
    }

    fn read_mesh(&mut self) -> Result<(), ImportError> {
        // The material id.
        self.read_int()?;
        let first_vertex = self.vertices.len() as i32;
        while self.chunk_size() != 0 {
            let tag = self.read_chunk()?;
            match &tag {
                b"VRTS" => self.read_vrts()?,
                b"TRIS" => self.read_tris(first_vertex)?,
                _ => {}
            }
            self.exit_chunk();
        }
        Ok(())
    }

    fn read_bone(&mut self) -> Result<(), ImportError> {
        while self.chunk_size() != 0 {
            let vertex = self.read_int()?;
            self.read_float()?;
            if vertex < 0 || vertex as usize >= self.vertices.len() {
                return data_error("Bad vertex index");
            }
        }
        Ok(())
    }

    fn read_keys(&mut self) -> Result<(), ImportError> {
        let flags = self.read_int()?;
        while self.chunk_size() != 0 {
            self.read_int()?;
            if flags & 1 != 0 {
                self.read_floats::<3>()?;
            }
            if flags & 2 != 0 {
                self.read_floats::<3>()?;
            }
            if flags & 4 != 0 {
                self.read_floats::<4>()?;
            }
        }
        Ok(())
    }

    fn read_node(&mut self, depth: usize) -> Result<(), ImportError> {
        if depth > MAX_DEPTH {
            return fail("B3D: nodes nested too deep");
        }
        self.read_string()?;
        // Translation, scale, rotation.
        self.read_floats::<10>()?;
        self.node_count += 1;
        while self.chunk_size() != 0 {
            let tag = self.read_chunk()?;
            match &tag {
                b"MESH" => self.read_mesh()?,
                b"BONE" => self.read_bone()?,
                b"ANIM" => {
                    self.read_int()?;
                    self.read_int()?;
                    self.read_float()?;
                }
                b"KEYS" => self.read_keys()?,
                b"NODE" => self.read_node(depth + 1)?,
                _ => {}
            }
            self.exit_chunk();
        }
        Ok(())
    }

    /// `ReadBB3D` up to building the meshes.
    fn read_bb3d(&mut self) -> Result<(), ImportError> {
        let tag = self.read_chunk()?;
        if &tag == b"BB3D" {
            // The version.
            self.read_int()?;
            while self.chunk_size() != 0 {
                let chunk = self.read_chunk()?;
                match &chunk {
                    b"TEXS" => self.read_texs()?,
                    b"BRUS" => self.read_brus()?,
                    b"NODE" => self.read_node(0)?,
                    _ => {}
                }
                self.exit_chunk();
            }
        }
        self.exit_chunk();
        if self.node_count == 0 {
            return data_error("No nodes");
        }
        if self.meshes.is_empty() {
            return data_error("No meshes");
        }
        Ok(())
    }
}

pub fn read(bytes: &[u8]) -> Result<Scene, ImportError> {
    if bytes.len() < 8 {
        return fail("B3D File is too small.");
    }
    let mut loader = Loader {
        buffer: bytes,
        position: 0,
        stack: Vec::new(),
        texture_count: 0,
        material_count: 0,
        vertices: Vec::new(),
        meshes: Vec::new(),
        node_count: 0,
        vertex_flags: 0,
        texcoord_sets: 0,
        texcoord_size: 0,
    };
    loader.read_bb3d()?;

    let with_normals = loader.vertex_flags & 1 != 0;
    let with_texcoords = loader.texcoord_sets != 0;
    let mut meshes = Vec::with_capacity(loader.meshes.len());
    for triangles in &loader.meshes {
        let vertex_count = triangles.len() * 3;
        let mut positions = Vec::with_capacity(vertex_count);
        let mut normals = Vec::new();
        let mut coords = Vec::new();
        let mut faces = Vec::with_capacity(triangles.len());
        for triangle in triangles {
            let first = positions.len() as u32;
            for &index in triangle {
                let vertex = loader.vertices[index as usize];
                // Mirrored along z.
                positions.push([vertex.position[0], vertex.position[1], -vertex.position[2]]);
                if with_normals {
                    normals.push([vertex.normal[0], vertex.normal[1], -vertex.normal[2]]);
                }
                if with_texcoords {
                    coords.push(vertex.texcoords);
                }
            }
            // Flipped from first, first + 1, first + 2.
            faces.push(vec![first + 2, first + 1, first]);
        }
        let mut mesh = Mesh { primitive_types: PRIMITIVE_TRIANGLE, vertex_count, vertices: Some(positions), faces: Some(faces), ..Mesh::default() };
        if with_normals {
            mesh.normals = Some(normals);
        }
        if with_texcoords {
            mesh.texture_coords[0] = Some(coords);
        }
        meshes.push(mesh);
    }
    Ok(Scene { meshes })
}
