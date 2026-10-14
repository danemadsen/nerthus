//! `ObjFileParser` over `IOStreamBuffer::getNextDataLine`: each line is
//! copied into a buffer as long as the read block (16 MiB at most) that
//! keeps whatever earlier, longer lines left after the line's '\n', and
//! the parser can read on into that.

use std::collections::BTreeMap;

use super::mtl::{self, MtlImporter};
use super::tools::{
    at, atoi, get_name, get_name_no_space, is_end_of_buffer, is_line_end, is_space_or_new_line, next_token, next_word, skip_line, trim,
};
use crate::fast_atof::fast_atof;
use crate::io::FileSystem;
use crate::scene::{ImportError, PRIMITIVE_LINE, PRIMITIVE_POINT, PRIMITIVE_POLYGON, fail};

/// `IOStreamBuffer`'s default cache.
const CACHE_SIZE: usize = 4096 * 4096;

/// `ObjFileParser::Buffersize`.
const WORD_BUFFER_SIZE: usize = 4096;

/// `ObjFile::Mesh::NoMaterial`.
pub const NO_MATERIAL: u32 = u32::MAX;

/// `AI_DEFAULT_MATERIAL_NAME`.
pub const DEFAULT_MATERIAL: &[u8] = b"DefaultMaterial";

const DEFAULT_OBJECT: &[u8] = b"defaultobject";

/// `AI_MAXLEN - 1`.
const MAX_AI_STRING: usize = 1023;

pub struct Face {
    pub kind: u32,
    pub vertices: Vec<u32>,
    pub normals: Vec<u32>,
    pub texture_coords: Vec<u32>,
}

pub struct Object {
    pub name: Vec<u8>,
    pub meshes: Vec<usize>,
}

pub struct ObjMesh {
    pub name: Vec<u8>,
    pub faces: Vec<Face>,
    pub index_count: u32,
    pub uv_coordinates: u32,
    pub material_index: u32,
    pub has_normals: bool,
}

/// `ObjFile::Model`; materials live in an arena and only their names
/// matter here.
pub struct Model {
    pub objects: Vec<Object>,
    pub current_object: Option<usize>,
    pub current_material: Option<usize>,
    pub material_names: Vec<Vec<u8>>,
    pub material_lib: Vec<Vec<u8>>,
    pub material_map: BTreeMap<Vec<u8>, usize>,
    pub vertices: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub vertex_colors: Vec<[f32; 3]>,
    pub active_group: Vec<u8>,
    pub texture_coords: Vec<[f32; 3]>,
    pub texture_coord_dimension: u32,
    pub current_mesh: Option<usize>,
    pub meshes: Vec<ObjMesh>,
}

/// `aiString::Set(std::string)` on a fresh string: empty when too long.
pub fn ai_string_set(value: &[u8]) -> Vec<u8> {
    if value.len() > MAX_AI_STRING { Vec::new() } else { value.to_vec() }
}

impl Model {
    /// A new `ObjFile::Material` named `name`.
    pub fn new_material(&mut self, name: &[u8]) -> usize {
        self.material_names.push(ai_string_set(name));
        self.material_names.len() - 1
    }

    /// `getMaterialIndex`.
    pub fn material_index(&self, name: &[u8]) -> i32 {
        if name.is_empty() {
            return -1;
        }
        match self.material_lib.iter().position(|entry| entry == name) {
            Some(index) => index as i32,
            None => -1,
        }
    }
}

/// `IOStreamBuffer<char>`: a cache filled with '\n' and read block by
/// block (reads past it give '\n').
struct StreamBuffer<'a> {
    data: &'a [u8],
    cache: Vec<u8>,
    cache_size: usize,
    cache_position: usize,
    file_position: usize,
}

impl<'a> StreamBuffer<'a> {
    fn open(data: &'a [u8]) -> Self {
        let cache_size = CACHE_SIZE.min(data.len());
        StreamBuffer { data, cache: vec![b'\n'; cache_size], cache_size, cache_position: 0, file_position: 0 }
    }

    fn cache_at(&self, index: usize) -> u8 {
        self.cache.get(index).copied().unwrap_or(b'\n')
    }

    /// `readNextBlock`.
    fn read_next_block(&mut self) -> bool {
        let remaining = self.data.len() - self.file_position;
        let read_length = self.cache_size.min(remaining);
        if read_length == 0 {
            return false;
        }
        let block = &self.data[self.file_position..self.file_position + read_length];
        self.cache[..read_length].copy_from_slice(block);
        if read_length < self.cache_size {
            self.cache_size = read_length;
        }
        self.file_position += self.cache_size;
        self.cache_position = 0;
        true
    }

    /// `getNextDataLine`: the line (continued after a '\\' before a line
    /// end) into `buffer`, ended by '\n'.
    fn next_data_line(&mut self, buffer: &mut Vec<u8>, continuation: u8) -> bool {
        buffer.resize(self.cache_size, 0);
        if (self.cache_position >= self.cache_size || self.file_position == 0) && !self.read_next_block() {
            return false;
        }
        let mut length = 0;
        loop {
            let byte = self.cache_at(self.cache_position);
            if byte == continuation && is_line_end(self.cache_at(self.cache_position + 1)) {
                self.cache_position += 1;
                while self.cache_at(self.cache_position) != b'\n' {
                    self.cache_position += 1;
                }
                self.cache_position += 1;
            } else if is_line_end(byte) {
                break;
            }
            buffer[length] = self.cache_at(self.cache_position);
            self.cache_position += 1;
            length += 1;
            if length == buffer.len() {
                let doubled = buffer.len() * 2;
                buffer.resize(doubled, 0);
            }
            if self.cache_position >= self.data.len() {
                break;
            }
            if self.cache_position >= self.cache_size && !self.read_next_block() {
                return false;
            }
        }
        buffer[length] = b'\n';
        self.cache_position += 1;
        true
    }
}

/// The file system an import sees (the filter, with the directory stack)
/// and the imported file's name.
pub struct Files<'a> {
    pub system: &'a FileSystem,
    pub original_file: Vec<u8>,
}

/// `ObjFileParser`.
pub struct ObjParser<'a> {
    pub model: Model,
    files: Files<'a>,
    line: Vec<u8>,
    it: usize,
}

impl<'a> ObjParser<'a> {
    /// The parser constructor: the default material, then the whole file.
    pub fn parse(data: &[u8], files: Files<'a>) -> Result<Model, ImportError> {
        let mut model = Model {
            objects: Vec::new(),
            current_object: None,
            current_material: None,
            material_names: Vec::new(),
            material_lib: Vec::new(),
            material_map: BTreeMap::new(),
            vertices: Vec::new(),
            normals: Vec::new(),
            vertex_colors: Vec::new(),
            active_group: Vec::new(),
            texture_coords: Vec::new(),
            texture_coord_dimension: 0,
            current_mesh: None,
            meshes: Vec::new(),
        };
        let default_material = model.new_material(DEFAULT_MATERIAL);
        model.material_lib.push(DEFAULT_MATERIAL.to_vec());
        model.material_map.insert(DEFAULT_MATERIAL.to_vec(), default_material);
        let mut parser = ObjParser { model, files, line: Vec::new(), it: 0 };
        parser.parse_file(data)?;
        Ok(parser.model)
    }

    fn end(&self) -> usize {
        self.line.len()
    }

    fn byte(&self) -> u8 {
        at(&self.line, self.it)
    }

    fn skip_line(&mut self) {
        self.it = skip_line(&self.line, self.it, self.end());
    }

    /// `parseFile`.
    fn parse_file(&mut self, data: &[u8]) -> Result<(), ImportError> {
        let mut stream = StreamBuffer::open(data);
        let mut inside_cstype = false;
        let mut line = std::mem::take(&mut self.line);
        while stream.next_data_line(&mut line, b'\\') {
            self.line = std::mem::take(&mut line);
            self.it = 0;
            self.parse_line(&mut inside_cstype)?;
            line = std::mem::take(&mut self.line);
        }
        Ok(())
    }

    fn parse_line(&mut self, inside_cstype: &mut bool) -> Result<(), ImportError> {
        let end = self.end();
        if *inside_cstype {
            if self.byte() == b'e' {
                let (_, name) = get_name_no_space(&self.line, self.it, end);
                *inside_cstype = name != b"end";
            }
            return Ok(());
        }
        match self.byte() {
            b'v' => {
                self.it += 1;
                let next = self.byte();
                if next == b' ' || next == b'\t' {
                    let component_count = self.component_count();
                    if component_count == 3 {
                        let vertex = self.get_vector3()?;
                        self.model.vertices.push(vertex);
                    } else if component_count == 4 {
                        self.get_homogeneous_vector3()?;
                    } else if component_count == 6 {
                        let vertex_count = self.model.vertices.len();
                        if self.model.vertex_colors.len() < vertex_count {
                            self.model.vertex_colors.resize(vertex_count, [0.0; 3]);
                        }
                        self.get_two_vectors3()?;
                    }
                    let vertex_count = self.model.vertices.len();
                    if !self.model.vertex_colors.is_empty() && self.model.vertex_colors.len() < vertex_count {
                        self.model.vertex_colors.resize(vertex_count, [0.0; 3]);
                    }
                } else if next == b't' {
                    self.it += 1;
                    let dimension = self.get_texture_coord_vector()?;
                    self.model.texture_coord_dimension = self.model.texture_coord_dimension.max(dimension as u32);
                } else if next == b'n' {
                    self.it += 1;
                    let normal = self.get_vector3()?;
                    self.model.normals.push(normal);
                }
            }
            b'p' => self.get_face(PRIMITIVE_POINT)?,
            b'l' => self.get_face(PRIMITIVE_LINE)?,
            b'f' => self.get_face(PRIMITIVE_POLYGON)?,
            b'u' => {
                let (_, name) = get_name_no_space(&self.line, self.it, end);
                if name == b"usemtl" {
                    self.get_material_description();
                }
            }
            b'm' => {
                let (_, name) = get_name_no_space(&self.line, self.it, end);
                if name == b"mtllib" {
                    self.get_material_library()?;
                }
            }
            b'g' => self.get_group_name(),
            b'o' => self.get_object_name(),
            b'c' => {
                let (_, name) = get_name_no_space(&self.line, self.it, end);
                *inside_cstype = name == b"cstype";
            }
            _ => {}
        }
        Ok(())
    }

    /// `copyNextWord`: the next word (a '\\' before it skips two bytes).
    fn copy_next_word(&mut self) -> Vec<u8> {
        let end = self.end();
        self.it = next_word(&self.line, self.it, end);
        if self.byte() == b'\\' {
            self.it += 2;
            self.it = next_word(&self.line, self.it, end);
        }
        let mut word = Vec::new();
        while self.it != end && !is_space_or_new_line(self.byte()) {
            word.push(self.byte());
            if word.len() == WORD_BUFFER_SIZE - 1 {
                break;
            }
            self.it += 1;
        }
        word
    }

    fn next_float(&mut self) -> Result<f32, ImportError> {
        let word = self.copy_next_word();
        fast_atof(&word)
    }

    /// `getNumComponentsInDataDefinition`: the tokens on the line that
    /// start with a digit or sign, or read "nan"/"inf".
    fn component_count(&self) -> usize {
        let line = &self.line;
        let end = self.end();
        let skip_spaces = |mut position: usize| {
            while (at(line, position) == b' ' || at(line, position) == b'\t') && position != end {
                position += 1;
            }
            position
        };
        let mut count = 0;
        let mut position = self.it;
        let mut end_of_definition = false;
        while !end_of_definition {
            if at(line, position) == b'\\' && is_line_end(at(line, position + 1)) {
                position += 2;
            } else if is_line_end(at(line, position)) {
                end_of_definition = true;
            }
            position = skip_spaces(position);
            if is_line_end(at(line, position)) {
                break;
            }
            let first = at(line, position);
            let numeric = first.is_ascii_digit() || first == b'-' || first == b'+';
            let lower = |offset: usize| at(line, position + offset).to_ascii_lowercase();
            let nan_or_inf = (lower(0) == b'n' && lower(1) == b'a' && lower(2) == b'n') || (lower(0) == b'i' && lower(1) == b'n' && lower(2) == b'f');
            position = skip_spaces(position);
            while !is_space_or_new_line(at(line, position)) {
                position += 1;
            }
            if numeric || nan_or_inf {
                count += 1;
            }
            position = skip_spaces(position);
            if is_line_end(at(line, position)) {
                break;
            }
        }
        count
    }

    /// `getTexCoordVector`: two or three components (non-finite ones read
    /// as 0).
    fn get_texture_coord_vector(&mut self) -> Result<usize, ImportError> {
        let component_count = self.component_count();
        let mut coordinate = [0.0f32; 3];
        if component_count == 2 {
            coordinate[0] = self.next_float()?;
            coordinate[1] = self.next_float()?;
        } else if component_count == 3 {
            coordinate[0] = self.next_float()?;
            coordinate[1] = self.next_float()?;
            coordinate[2] = self.next_float()?;
        } else {
            return fail("OBJ: Invalid number of components");
        }
        for value in coordinate.iter_mut() {
            if !value.is_finite() {
                *value = 0.0;
            }
        }
        self.model.texture_coords.push(coordinate);
        self.skip_line();
        Ok(component_count)
    }

    /// `getVector3`.
    fn get_vector3(&mut self) -> Result<[f32; 3], ImportError> {
        let x = self.next_float()?;
        let y = self.next_float()?;
        let z = self.next_float()?;
        self.skip_line();
        Ok([x, y, z])
    }

    /// `getHomogeneousVector3`.
    fn get_homogeneous_vector3(&mut self) -> Result<(), ImportError> {
        let x = self.next_float()?;
        let y = self.next_float()?;
        let z = self.next_float()?;
        let w = self.next_float()?;
        if w == 0.0 {
            return fail("OBJ: Invalid component in homogeneous vector (Division by zero)");
        }
        self.model.vertices.push([x / w, y / w, z / w]);
        self.skip_line();
        Ok(())
    }

    /// `getTwoVectors3`: a vertex and its colour.
    fn get_two_vectors3(&mut self) -> Result<(), ImportError> {
        let x = self.next_float()?;
        let y = self.next_float()?;
        let z = self.next_float()?;
        self.model.vertices.push([x, y, z]);
        let red = self.next_float()?;
        let green = self.next_float()?;
        let blue = self.next_float()?;
        self.model.vertex_colors.push([red, green, blue]);
        self.skip_line();
        Ok(())
    }

    /// `getFace`: indices are read with `atoi` and the cursor steps by the
    /// value's digit count, not by what was read.
    fn get_face(&mut self, kind: u32) -> Result<(), ImportError> {
        let end = self.end();
        self.it = next_token(&self.line, self.it, end);
        if self.it == end || self.byte() == 0 {
            return Ok(());
        }
        let mut face = Face { kind, vertices: Vec::new(), normals: Vec::new(), texture_coords: Vec::new() };
        let mut has_normal = false;
        let vertex_count = self.model.vertices.len() as u32 as i32;
        let coordinate_count = self.model.texture_coords.len() as u32 as i32;
        let normal_count = self.model.normals.len() as u32 as i32;
        let has_coordinates = !self.model.texture_coords.is_empty();
        let has_normals = !self.model.normals.is_empty();
        let mut slot = 0;
        while self.it < end {
            let mut step = 1;
            let byte = self.byte();
            if is_line_end(byte) {
                break;
            }
            if byte == b'/' {
                slot += 1;
            } else if is_space_or_new_line(byte) {
                slot = 0;
            } else {
                let value = atoi(&self.line[..end], self.it);
                let mut rest = value;
                if value < 0 {
                    step += 1;
                }
                loop {
                    rest /= 10;
                    if rest == 0 {
                        break;
                    }
                    step += 1;
                }
                if slot == 1 && !has_coordinates && has_normals {
                    slot = 2;
                }
                if value == 0 {
                    return fail("OBJ: Invalid face index.");
                }
                let index = if value > 0 { (value - 1) as u32 } else { vertex_count.wrapping_add(value) as u32 };
                match slot {
                    0 => face.vertices.push(if value > 0 { index } else { vertex_count.wrapping_add(value) as u32 }),
                    1 => face.texture_coords.push(if value > 0 { index } else { coordinate_count.wrapping_add(value) as u32 }),
                    2 => {
                        face.normals.push(if value > 0 { index } else { normal_count.wrapping_add(value) as u32 });
                        has_normal = true;
                    }
                    // `reportErrorTokenInFace` moves past the line end and
                    // the loop reads on from there.
                    _ => self.skip_line(),
                }
            }
            self.it += step;
        }

        if face.vertices.is_empty() {
            self.skip_line();
            return Ok(());
        }
        if self.model.current_object.is_none() {
            self.create_object(DEFAULT_OBJECT);
        }
        if self.model.current_mesh.is_none() {
            self.create_mesh(DEFAULT_OBJECT);
        }
        let mesh_index = self.model.current_mesh.unwrap();
        let mesh = &mut self.model.meshes[mesh_index];
        mesh.index_count = mesh.index_count.wrapping_add(face.vertices.len() as u32);
        mesh.uv_coordinates = mesh.uv_coordinates.wrapping_add(face.texture_coords.len() as u32);
        if has_normal {
            mesh.has_normals = true;
        }
        mesh.faces.push(face);
        self.skip_line();
        Ok(())
    }

    /// `getMaterialDesc` (`usemtl`).
    fn get_material_description(&mut self) {
        let end = self.end();
        self.it = next_token(&self.line, self.it, end);
        if self.it == end {
            return;
        }
        let start = self.it;
        while self.it != end && !is_line_end(self.byte()) {
            self.it += 1;
        }
        let name = trim(&self.line[start..self.it]);
        let mut skip = name.is_empty();
        if !skip && let Some(current) = self.model.current_material {
            let compared = &name[..name.len().min(MAX_AI_STRING)];
            skip = self.model.material_names[current] == compared;
        }
        if !skip {
            match self.model.material_map.get(&name) {
                Some(&material) => self.model.current_material = Some(material),
                None => {
                    let material = self.model.new_material(&name);
                    self.model.material_lib.push(name.clone());
                    self.model.material_map.insert(name.clone(), material);
                    self.model.current_material = Some(material);
                }
            }
            if self.needs_new_mesh(&name) {
                let mesh_name = if self.model.active_group.is_empty() { name.clone() } else { self.model.active_group.clone() };
                self.create_mesh(&mesh_name);
            }
            let index = self.model.material_index(&name);
            let mesh_index = self.model.current_mesh.unwrap();
            self.model.meshes[mesh_index].material_index = index as u32;
        }
        self.skip_line();
    }

    /// `getMaterialLib` (`mtllib`): the library next to the file, else the
    /// file's name with "mtl" for its last three bytes.
    fn get_material_library(&mut self) -> Result<(), ImportError> {
        let end = self.end();
        self.it = next_token(&self.line, self.it, end);
        if self.it == end {
            return Ok(());
        }
        let start = self.it;
        while self.it != end && !is_line_end(self.byte()) {
            self.it += 1;
        }
        let name = self.line[start..self.it].to_vec();
        if name.is_empty() {
            return Ok(());
        }
        let path = if self.files.system.directory_count() > 0 {
            let mut path = self.files.system.current_directory();
            if path.last() != Some(&b'/') {
                path.push(b'/');
            }
            path.extend_from_slice(&name);
            path
        } else {
            name.clone()
        };
        let mut contents = self.files.system.open_bytes(&path);
        if contents.is_none() {
            let original = &self.files.original_file;
            if original.len() < 3 {
                return fail("invalid string position");
            }
            let mut fallback = original[..original.len() - 3].to_vec();
            fallback.extend_from_slice(b"mtl");
            contents = self.files.system.open_bytes(&fallback);
        }
        let Some(contents) = contents else {
            self.skip_line();
            return Ok(());
        };
        let buffer = mtl::text_file_to_buffer(contents)?;
        MtlImporter::new(buffer, &mut self.model).load()
    }

    /// `getGroupName`: a new object for each change of group.
    fn get_group_name(&mut self) {
        let end = self.end();
        self.it = next_token(&self.line, self.it, end);
        let (it, name) = get_name(&self.line, self.it, end);
        self.it = it;
        if is_end_of_buffer(self.it, end) {
            return;
        }
        if self.model.active_group != name {
            self.create_object(&name);
            self.model.active_group = name;
        }
        self.skip_line();
    }

    /// `getObjectName`: an existing object of that name, or a new one.
    fn get_object_name(&mut self) {
        let end = self.end();
        self.it = next_token(&self.line, self.it, end);
        if self.it == end {
            return;
        }
        let start = self.it;
        while self.it != end && !is_space_or_new_line(self.byte()) {
            self.it += 1;
        }
        let name = self.line[start..self.it].to_vec();
        if !name.is_empty() {
            self.model.current_object = self.model.objects.iter().position(|object| object.name == name);
            if self.model.current_object.is_none() {
                self.create_object(&name);
            }
        }
        self.skip_line();
    }

    /// `createObject`: with a mesh, given the current material's index.
    fn create_object(&mut self, name: &[u8]) {
        self.model.objects.push(Object { name: name.to_vec(), meshes: Vec::new() });
        self.model.current_object = Some(self.model.objects.len() - 1);
        self.create_mesh(name);
        if let Some(material) = self.model.current_material {
            let material_name = self.model.material_names[material].clone();
            let index = self.model.material_index(&material_name);
            let mesh_index = self.model.current_mesh.unwrap();
            self.model.meshes[mesh_index].material_index = index as u32;
        }
    }

    /// `createMesh`: attached to the current object, if any.
    fn create_mesh(&mut self, name: &[u8]) {
        let mesh = ObjMesh {
            name: name.to_vec(),
            faces: Vec::new(),
            index_count: 0,
            uv_coordinates: 0,
            material_index: NO_MATERIAL,
            has_normals: false,
        };
        self.model.meshes.push(mesh);
        let mesh_index = self.model.meshes.len() - 1;
        self.model.current_mesh = Some(mesh_index);
        if let Some(object) = self.model.current_object {
            self.model.objects[object].meshes.push(mesh_index);
        }
    }

    /// `needsNewMesh`: a material change in a mesh that has faces.
    fn needs_new_mesh(&self, material_name: &[u8]) -> bool {
        let Some(mesh_index) = self.model.current_mesh else {
            return true;
        };
        let mesh = &self.model.meshes[mesh_index];
        let material_index = self.model.material_index(material_name);
        let current = mesh.material_index as i32;
        current != NO_MATERIAL as i32 && current != material_index && !mesh.faces.is_empty()
    }
}
