//! Assimp's AC3D importer (`ACLoader.cpp`): `MATERIAL` lines and an
//! `OBJECT` tree (vertices, surfaces of polygons, closed and open lines
//! or triangle strips with UVs, `kids`). Each object becomes a mesh per
//! material used (polygons and strips moved by the object's location,
//! lines not), or a point mesh when it has no references; objects with
//! `subdiv` are Catmull-Clark subdivided.
//!
//! Kept quirks: a line can only be "processed again" when it isn't
//! indented (else it is skipped); the line after an object tree is
//! skipped, and more than one root object loses every mesh; `name` reads
//! from where the token left off; a surface's out-of-range material
//! counts as material 0 but is then matched against the original number,
//! so its faces are never filled (empty faces, zero vertices). Writes
//! outside the counted arrays and reads outside the entries are errors
//! here. A node's mesh list can be counted longer than it is filled; the
//! uninitialised numbers are only read when SortByPType rebuilds node
//! lists, which doesn't reach the meshes (unless the read crashes), so
//! that loads here.

use crate::fast_atof::{fast_atoreal_move, strtoul10, strtoul_cppstyle};
use crate::parsing::{at, is_line_end, is_space, skip_line, skip_spaces, text_file_to_buffer, token_match};
use crate::scene::{ImportError, Mesh, Scene, check_allocation, fail};
use crate::subdivision::catmull_clark;

/// `AI_MAX_ALLOC(aiVector3D)` and `AI_MAX_ALLOC(aiFace)`.
const MAX_VERTICES: u32 = 256 * 1024 * 1024 / 12;
const MAX_FACES: u32 = 256 * 1024 * 1024 / 16;

/// Object nesting the C++ recursion would only end with a stack overflow.
const MAX_DEPTH: usize = 1024;

const POLYGON: u32 = 0;
const CLOSED_LINE: u32 = 1;
const OPEN_LINE: u32 = 2;
const TRIANGLE_STRIP: u32 = 4;

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

struct Surface {
    material: u32,
    flags: u32,
    entries: Vec<(u32, [f32; 2])>,
}

impl Surface {
    fn kind(&self) -> u32 {
        self.flags & 0xf
    }
}

#[derive(Default)]
struct Object {
    texture_count: usize,
    translation: [f32; 3],
    vertices: Vec<[f32; 3]>,
    surfaces: Vec<Surface>,
    reference_count: u32,
    subdivision: u32,
    children: Vec<Object>,
}

struct Parser {
    buffer: Vec<u8>,
    position: usize,
    object_count: u32,
}

impl Parser {
    fn end(&self) -> usize {
        self.buffer.len()
    }

    fn current(&self) -> u8 {
        at(&self.buffer, self.position)
    }

    /// `GetNextLine`: the next line's first non-blank; whether there is one.
    fn next_line(&mut self) -> bool {
        self.position = skip_line(&self.buffer, self.position, self.end()).0;
        let (position, goes_on) = skip_spaces(&self.buffer, self.position, self.end());
        self.position = position;
        goes_on
    }

    fn skip_spaces(&mut self) {
        self.position = skip_spaces(&self.buffer, self.position, self.end()).0;
    }

    fn token(&mut self, token: &[u8]) -> bool {
        token_match(&self.buffer, &mut self.position, token)
    }

    /// `--mBuffer.data`: one byte back, so the line is read again (or, when
    /// it was indented, skipped).
    fn step_back(&mut self) {
        self.position = self.position.saturating_sub(1);
    }

    /// `AcGetString`: a quoted string; only whether it reads matters.
    fn read_string(&mut self) -> Result<(), ImportError> {
        if self.current() == 0 {
            return fail("AC3D: Unexpected EOF in string");
        }
        self.position += 1;
        while self.current() != b'"' && self.position != self.end() {
            if is_line_end(self.current()) {
                break;
            }
            self.position += 1;
        }
        if is_line_end(self.current()) {
            return Ok(());
        }
        self.position += 1;
        Ok(())
    }

    /// `TAcCheckedLoadFloatArray`: an optional name, then `count` numbers.
    fn read_floats(&mut self, name: &[u8], count: usize) -> Result<Vec<f32>, ImportError> {
        self.skip_spaces();
        let mut values = vec![0.0f32; count];
        if !name.is_empty() {
            let named = self.buffer[self.position.min(self.end())..].starts_with(name) && is_space(at(&self.buffer, self.position + name.len()));
            if !named {
                // "AC3D: Unexpected token": the values stay unread.
                return Ok(Vec::new());
            }
            self.position += name.len() + 1;
        }
        for value in &mut values {
            self.skip_spaces();
            let (number, next) = fast_atoreal_move(&self.buffer, self.position, true)?;
            *value = number;
            self.position = next;
        }
        Ok(values)
    }

    fn read_material(&mut self) -> Result<(), ImportError> {
        self.skip_spaces();
        if self.current() == b'"' {
            self.read_string()?;
            self.skip_spaces();
        }
        self.read_floats(b"rgb", 3)?;
        self.read_floats(b"amb", 3)?;
        self.read_floats(b"emis", 3)?;
        self.read_floats(b"spec", 3)?;
        self.read_floats(b"shi", 1)?;
        self.read_floats(b"trans", 1)?;
        Ok(())
    }

    fn read_surface(&mut self, object: &mut Object, workaround: &mut bool) -> Result<(), ImportError> {
        self.next_line();
        if !self.token(b"SURF") {
            self.step_back();
            *workaround = true;
        }
        self.skip_spaces();
        let flags = strtoul_cppstyle(&self.buffer, self.position).0;
        let mut surface = Surface { material: 0, flags, entries: Vec::new() };
        loop {
            if !self.next_line() {
                return fail("AC3D: Unexpected EOF: surface is incomplete");
            }
            if self.token(b"mat") {
                self.skip_spaces();
                surface.material = strtoul10(&self.buffer, self.position).0;
            } else if self.token(b"refs") {
                if *workaround && !surface.entries.is_empty() {
                    self.position = self.position.saturating_sub(6);
                    break;
                }
                self.skip_spaces();
                let count = strtoul10(&self.buffer, self.position).0;
                check_allocation(count as u64, 12)?;
                object.reference_count = object.reference_count.wrapping_add(count);
                for _ in 0..count {
                    if !self.next_line() {
                        // "AC3D: Unexpected EOF: surface references are incomplete"
                        break;
                    }
                    let (vertex, next) = strtoul10(&self.buffer, self.position);
                    self.position = next;
                    self.skip_spaces();
                    let uv = self.read_floats(b"", 2)?;
                    surface.entries.push((vertex, [uv[0], uv[1]]));
                }
            } else {
                self.step_back();
                break;
            }
        }
        object.surfaces.push(surface);
        Ok(())
    }

    /// `LoadObjectSection`: false when the line isn't an `OBJECT`, or the
    /// file ended before its `kids`.
    fn load_object(&mut self, objects: &mut Vec<Object>, depth: usize) -> Result<bool, ImportError> {
        if !self.token(b"OBJECT") {
            return Ok(false);
        }
        if depth > MAX_DEPTH {
            return fail("AC3D: objects nested too deep");
        }
        self.skip_spaces();
        self.object_count += 1;
        objects.push(Object::default());
        let index = objects.len() - 1;
        while self.next_line() {
            if self.token(b"kids") {
                self.skip_spaces();
                let (count, next) = strtoul10(&self.buffer, self.position);
                self.position = next;
                self.next_line();
                let mut children = Vec::new();
                for _ in 0..count {
                    if !self.load_object(&mut children, depth + 1)? {
                        // "AC3D: wrong number of kids"
                        break;
                    }
                }
                objects[index].children = children;
                return Ok(true);
            }
            let object = &mut objects[index];
            if self.token(b"name") {
                // `SkipSpaces` with the end at the cursor skips nothing.
                self.read_string()?;
            } else if self.token(b"texture") {
                self.skip_spaces();
                self.read_string()?;
                object.texture_count += 1;
            } else if self.token(b"texrep") || self.token(b"texoff") {
                self.skip_spaces();
                self.read_floats(b"", 2)?;
            } else if self.token(b"rot") {
                self.skip_spaces();
                self.read_floats(b"", 9)?;
            } else if self.token(b"loc") {
                self.skip_spaces();
                let values = self.read_floats(b"", 3)?;
                object.translation = [values[0], values[1], values[2]];
            } else if self.token(b"subdiv") {
                self.skip_spaces();
                object.subdivision = strtoul10(&self.buffer, self.position).0;
            } else if self.token(b"crease") {
                self.skip_spaces();
                fast_atoreal_move(&self.buffer, self.position, true)?;
            } else if self.token(b"numvert") {
                self.skip_spaces();
                let (count, next) = strtoul10(&self.buffer, self.position);
                self.position = next;
                if count >= MAX_VERTICES {
                    return fail("AC3D: Too many vertices, would run out of memory");
                }
                for _ in 0..count {
                    if !self.next_line() {
                        // "AC3D: Unexpected EOF: not all vertices have been parsed yet"
                        break;
                    }
                    if !matches!(self.current(), b'0'..=b'9' | b'-' | b'+') {
                        self.step_back();
                        break;
                    }
                    let values = self.read_floats(b"", 3)?;
                    objects[index].vertices.push([values[0], values[1], values[2]]);
                }
            } else if self.token(b"numsurf") {
                self.skip_spaces();
                let (count, next) = strtoul10(&self.buffer, self.position);
                self.position = next;
                check_allocation(count as u64, 32)?;
                let mut workaround = false;
                for _ in 0..count {
                    self.read_surface(&mut objects[index], &mut workaround)?;
                }
            }
        }
        // "AC3D: Unexpected EOF: 'kids' line was expected"
        Ok(false)
    }
}

/// The mesh of one material: faces and vertices as counted, filled in.
struct MeshBuilder {
    faces: Vec<Vec<u32>>,
    face_limit: usize,
    vertices: Vec<[f32; 3]>,
    uvs: Option<Vec<[f32; 3]>>,
    vertex_limit: usize,
}

impl MeshBuilder {
    fn push_face(&mut self, indices: Vec<u32>) -> Result<(), ImportError> {
        if self.faces.len() >= self.face_limit {
            return undefined("a face written past the faces");
        }
        self.faces.push(indices);
        Ok(())
    }

    /// A vertex without the C++'s count check.
    fn push_vertex(&mut self, position: [f32; 3], uv: [f32; 2]) -> Result<(), ImportError> {
        if self.vertices.len() >= self.vertex_limit {
            return undefined("a vertex written past the vertices");
        }
        self.vertices.push(position);
        if let Some(uvs) = self.uvs.as_mut() {
            uvs.push([uv[0], uv[1], 0.0]);
        }
        Ok(())
    }

    /// A vertex after the C++'s `Invalid number of vertices` check.
    fn push_checked_vertex(&mut self, position: [f32; 3], uv: [f32; 2]) -> Result<(), ImportError> {
        if self.vertices.len() >= self.vertex_limit {
            return fail("AC3D: Invalid number of vertices");
        }
        self.push_vertex(position, uv)
    }
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// The meshes of one material's surfaces.
fn build_material_mesh(object: &Object, material: u32, face_count: u32, vertex_count: u32) -> Result<Mesh, ImportError> {
    if face_count > MAX_FACES {
        return fail("AC3D: Too many faces, would run out of memory");
    }
    if vertex_count == 0 {
        return fail("AC3D: No vertices");
    }
    if vertex_count > MAX_VERTICES {
        return fail("AC3D: Too many vertices, would run out of memory");
    }
    let mut builder = MeshBuilder {
        faces: Vec::with_capacity(face_count as usize),
        face_limit: face_count as usize,
        vertices: Vec::with_capacity(vertex_count as usize),
        uvs: if object.texture_count > 0 { Some(Vec::with_capacity(vertex_count as usize)) } else { None },
        vertex_limit: vertex_count as usize,
    };
    let vertex = |entry: &(u32, [f32; 2])| -> Result<[f32; 3], ImportError> {
        match object.vertices.get(entry.0 as usize) {
            Some(&position) => Ok(position),
            None => undefined("a read past the vertices"),
        }
    };
    let mut next = 0u32;
    for surface in object.surfaces.iter().filter(|surface| surface.material == material) {
        match surface.kind() {
            POLYGON => {
                let mut indices = Vec::with_capacity(surface.entries.len());
                for entry in &surface.entries {
                    indices.push(next);
                    next += 1;
                    builder.push_checked_vertex(add(vertex(entry)?, object.translation), entry.1)?;
                }
                builder.push_face(indices)?;
            }
            TRIANGLE_STRIP => {
                let strip_faces = (surface.entries.len() as u32).wrapping_sub(2);
                for index in 0..strip_faces as usize {
                    let (Some(first), Some(second), Some(third)) = (surface.entries.get(index), surface.entries.get(index + 1), surface.entries.get(index + 2)) else {
                        return undefined("a strip read past its entries");
                    };
                    builder.push_face(vec![next, next + 1, next + 2])?;
                    next += 3;
                    let (a, b) = if index & 1 == 0 { (first, second) } else { (second, first) };
                    builder.push_vertex(add(vertex(a)?, object.translation), a.1)?;
                    builder.push_vertex(add(vertex(b)?, object.translation), b.1)?;
                    builder.push_checked_vertex(add(vertex(third)?, object.translation), third.1)?;
                }
            }
            kind => {
                let mut line_count = surface.entries.len() as u32;
                if kind == OPEN_LINE {
                    line_count = line_count.wrapping_sub(1);
                }
                let mut cursor = 0usize;
                for line in 0..line_count {
                    builder.push_face(vec![next, next + 1])?;
                    next += 2;
                    let Some(start) = surface.entries.get(cursor) else {
                        return fail("AC3D: Bad line");
                    };
                    builder.push_vertex(vertex(start)?, start.1)?;
                    if kind == CLOSED_LINE && line == line_count - 1 {
                        cursor = 0;
                    } else {
                        cursor += 1;
                    }
                    let Some(end) = surface.entries.get(cursor) else {
                        return undefined("a line read past its entries");
                    };
                    builder.push_vertex(vertex(end)?, end.1)?;
                }
            }
        }
    }
    // Faces and vertices counted but never filled stay empty and zero.
    builder.faces.resize(face_count as usize, Vec::new());
    builder.vertices.resize(vertex_count as usize, [0.0; 3]);
    let mut mesh = Mesh { vertex_count: vertex_count as usize, vertices: Some(builder.vertices), faces: Some(builder.faces), ..Mesh::default() };
    if let Some(mut uvs) = builder.uvs {
        uvs.resize(vertex_count as usize, [0.0; 3]);
        mesh.uv_components[0] = 2;
        mesh.texture_coords[0] = Some(uvs);
    }
    Ok(mesh)
}

/// `ConvertObjectSection`: the object's meshes, then its children's.
fn convert_object(object: &mut Object, material_count: usize, meshes: &mut Vec<Mesh>, depth: usize) -> Result<(), ImportError> {
    if depth > MAX_DEPTH {
        return fail("AC3D: objects nested too deep");
    }
    if !object.vertices.is_empty() {
        if object.surfaces.is_empty() || object.reference_count == 0 {
            // A point per vertex, unmoved.
            let count = object.vertices.len();
            let faces = (0..count as u32).map(|index| vec![index]).collect();
            meshes.push(Mesh { vertex_count: count, vertices: Some(object.vertices.clone()), faces: Some(faces), ..Mesh::default() });
        } else {
            let mut needed = vec![(0u32, 0u32); material_count];
            for surface in &mut object.surfaces {
                let mut index = surface.material as usize;
                if index >= needed.len() {
                    // "AC3D: material index is out of range"
                    index = 0;
                }
                for entry in &mut surface.entries {
                    if entry.0 as usize >= object.vertices.len() {
                        // "AC3D: Invalid vertex reference"
                        entry.0 = 0;
                    }
                }
                let size = surface.entries.len() as u64;
                let counts = &mut needed[index];
                match surface.kind() {
                    CLOSED_LINE => {
                        counts.0 = counts.0.wrapping_add(size as u32);
                        counts.1 = counts.1.wrapping_add((size << 1) as u32);
                    }
                    OPEN_LINE => {
                        counts.0 = counts.0.wrapping_add(size.wrapping_sub(1) as u32);
                        counts.1 = counts.1.wrapping_add((size.wrapping_sub(1) << 1) as u32);
                    }
                    TRIANGLE_STRIP => {
                        let strip_faces = (size as u32).wrapping_sub(2);
                        counts.0 = counts.0.wrapping_add(strip_faces);
                        counts.1 = counts.1.wrapping_add(strip_faces.wrapping_mul(3));
                    }
                    kind => {
                        if kind != POLYGON {
                            // "AC3D: The type flag of a surface is unknown"
                            surface.flags &= !0xf;
                        }
                        counts.0 = counts.0.wrapping_add(1);
                        counts.1 = counts.1.wrapping_add(size as u32);
                    }
                }
            }
            let first_mesh = meshes.len();
            for (material, &(face_count, vertex_count)) in needed.iter().enumerate() {
                if face_count == 0 {
                    continue;
                }
                meshes.push(build_material_mesh(object, material as u32, face_count, vertex_count)?);
            }
            if object.subdivision != 0 {
                let own = meshes.split_off(first_mesh);
                meshes.extend(catmull_clark(own, object.subdivision)?);
            }
        }
    }
    for child in &mut object.children {
        convert_object(child, material_count, meshes, depth + 1)?;
    }
    Ok(())
}

pub fn read(bytes: &[u8]) -> Result<Scene, ImportError> {
    let buffer = text_file_to_buffer(bytes, false)?;
    if !buffer.starts_with(b"AC3D") {
        return fail("AC3D: No valid AC3D file, magic sequence not found");
    }
    let mut parser = Parser { buffer, position: 0, object_count: 0 };
    let mut material_count = 0usize;
    let mut roots: Vec<Object> = Vec::new();
    while parser.next_line() {
        if parser.token(b"MATERIAL") {
            parser.read_material()?;
            material_count += 1;
        } else {
            parser.load_object(&mut roots, 0)?;
        }
    }
    if roots.is_empty() || parser.object_count == 0 {
        return fail("AC3D: No meshes have been loaded");
    }
    // No materials: a default one.
    let material_count = material_count.max(1);

    let mut meshes = Vec::new();
    if roots.len() == 1 {
        convert_object(&mut roots[0], material_count, &mut meshes, 0)?;
    }
    // Several roots: an empty stand-in root, which has no meshes.
    if meshes.is_empty() {
        return fail("An unknown error occurred during converting");
    }
    Ok(Scene { meshes })
}
