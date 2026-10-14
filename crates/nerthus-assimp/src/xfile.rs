//! Assimp's DirectX importer (`XFileParser.cpp`, `XFileImporter.cpp`):
//! text and binary files, plain or MSZIP-compressed (each chunk inflated
//! on its own, the previous chunk's output its dictionary). Frames form
//! the node tree (an unnamed only child holding meshes merges into its
//! parent, its own children dropped); each mesh is split by its face
//! materials, each face's vertices written out in turn (an index past the
//! vertices takes the next vertex's place, which stays zero), texture
//! coordinates flipped in v; meshes come in frame order, then the global
//! ones. Everything is mirrored in z and each face reversed (left-handed
//! to right-handed).
//!
//! Failures here where Assimp does undefined things: reads past the
//! buffer (binary tokens jump unchecked; a string longer than 2 GiB),
//! face materials without materials, bone weights for vertices past the
//! vertices, frame names of 1024 bytes or more.

use zlib_rs::{Inflate, InflateFlush, Status};

use crate::fast_atof::fast_atoreal_move;
use crate::parsing::convert_to_utf8;
use crate::scene::{self, ImportError, Mesh, Scene, fail};
use crate::vector::Vector3;

/// `MSZIP_MAGIC` ("CK") and `MSZIP_BLOCK`.
const MSZIP_MAGIC: u16 = 0x4B43;
const MSZIP_BLOCK: usize = 32786;
/// `AI_MAX_NUMBER_OF_TEXTURECOORDS` and `AI_MAX_NUMBER_OF_COLOR_SETS`.
const MAX_TEXTURE_COORDS: usize = 8;
const MAX_COLOR_SETS: usize = 8;
/// `AI_MAXLEN`.
const MAX_LENGTH: usize = 1024;
/// How deep frames can nest before Assimp's recursion through them would
/// overflow the stack.
const MAX_DEPTH: usize = 4096;
/// The Windows build's element sizes, for the allocations counts ask for.
const VECTOR_SIZE: u64 = 12;
const VECTOR2_SIZE: u64 = 8;
const COLOR_SIZE: u64 = 16;
const FACE_SIZE: u64 = 24;
const WEIGHT_SIZE: u64 = 8;

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

/// `XFile::Mesh`, as far as the output depends on it.
#[derive(Default)]
struct XMesh {
    positions: Vec<Vector3>,
    position_faces: Vec<Vec<u32>>,
    normals: Vec<Vector3>,
    normal_faces: Vec<Vec<u32>>,
    texture_coords: Vec<Vec<[f32; 2]>>,
    colors: Vec<Vec<[f32; 4]>>,
    face_materials: Vec<u32>,
    material_count: usize,
    /// Each bone's weights' vertices.
    bones: Vec<Vec<u32>>,
}

/// `XFile::Node`.
struct XNode {
    name: Vec<u8>,
    children: Vec<usize>,
    meshes: Vec<XMesh>,
}

/// `XFileParser`.
struct Parser {
    buffer: Vec<u8>,
    position: i64,
    end: i64,
    binary: bool,
    float_size: i64,
    number_count: u32,
    nodes: Vec<XNode>,
    root: Option<usize>,
    global_meshes: Vec<XMesh>,
    depth: usize,
}

/// C's `isspace` in the C locale.
fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

impl Parser {
    /// The byte at a position, which must lie in the buffer.
    fn byte_at(&self, position: i64) -> Result<u8, ImportError> {
        if position < 0 || position >= self.buffer.len() as i64 {
            return undefined("a read past the buffer");
        }
        Ok(self.buffer[position as usize])
    }

    fn current(&self) -> u8 {
        if self.position < 0 {
            return 0;
        }
        self.buffer.get(self.position as usize).copied().unwrap_or(0)
    }

    fn remaining(&self) -> i64 {
        self.end - self.position
    }

    /// A binary file read to its end: every further number reads as 0
    /// without moving, so long counts are filled at once.
    fn at_binary_end(&self) -> bool {
        self.binary && self.position == self.end
    }

    fn throw<T>(&self, what: &str) -> Result<T, ImportError> {
        fail(what.to_string())
    }

    /// `ReadBinWord`.
    fn read_word(&mut self) -> Result<u32, ImportError> {
        let low = self.byte_at(self.position)? as u32;
        let high = self.byte_at(self.position + 1)? as u32;
        self.position += 2;
        Ok(low | (high << 8))
    }

    /// `ReadBinDWord`.
    fn read_dword(&mut self) -> Result<u32, ImportError> {
        let mut value = 0u32;
        for offset in 0..4 {
            value |= (self.byte_at(self.position + offset)? as u32) << (offset * 8);
        }
        self.position += 4;
        Ok(value)
    }

    /// `ReadUntilEndOfLine` (text): past the next line break.
    fn read_until_end_of_line(&mut self) {
        if self.binary {
            return;
        }
        while self.position < self.end {
            let byte = self.current();
            self.position += 1;
            if byte == b'\n' || byte == b'\r' {
                return;
            }
        }
    }

    /// `FindNextNoneWhiteSpace` (text): past blanks and comments.
    fn find_next_non_white_space(&mut self) {
        if self.binary {
            return;
        }
        loop {
            while self.position < self.end && is_space(self.current()) {
                self.position += 1;
            }
            if self.position >= self.end {
                return;
            }
            let first = self.current();
            let second = self.buffer.get(self.position as usize + 1).copied().unwrap_or(0);
            if (first == b'/' && second == b'/') || first == b'#' {
                self.read_until_end_of_line();
            } else {
                return;
            }
        }
    }

    /// `GetNextToken`.
    fn next_token(&mut self) -> Result<Vec<u8>, ImportError> {
        if self.binary {
            return self.next_binary_token();
        }
        self.find_next_non_white_space();
        let mut token = Vec::new();
        if self.position >= self.end {
            return Ok(token);
        }
        while self.position < self.end && !is_space(self.current()) {
            let byte = self.current();
            if matches!(byte, b';' | b'}' | b'{' | b',') {
                if token.is_empty() {
                    token.push(byte);
                    self.position += 1;
                }
                break;
            }
            token.push(byte);
            self.position += 1;
        }
        Ok(token)
    }

    /// `GetNextToken` for binary files: names and strings, the other
    /// tokens as their spelling, values passed over.
    fn next_binary_token(&mut self) -> Result<Vec<u8>, ImportError> {
        if self.remaining() < 2 {
            return Ok(Vec::new());
        }
        let token = self.read_word()?;
        let text: &[u8] = match token {
            1 => {
                if self.remaining() < 4 {
                    return Ok(Vec::new());
                }
                let length = self.read_dword()?;
                let bounds = self.remaining() as i32;
                if (length as i32) < 0 || bounds < length as i32 {
                    return Ok(Vec::new());
                }
                return self.take_bytes(length as i64, length as i64);
            }
            2 => {
                if self.remaining() < 4 {
                    return Ok(Vec::new());
                }
                let length = self.read_dword()?;
                if self.remaining() < length as i32 as i64 {
                    return Ok(Vec::new());
                }
                return self.take_bytes(length as i64, length as i64 + 2);
            }
            3 => {
                self.position += 4;
                b"<integer>"
            }
            5 => {
                self.position += 16;
                b"<guid>"
            }
            6 => {
                if self.remaining() < 4 {
                    return Ok(Vec::new());
                }
                let length = self.read_dword()?;
                self.position += length.wrapping_mul(4) as i64;
                b"<int_list>"
            }
            7 => {
                if self.remaining() < 4 {
                    return Ok(Vec::new());
                }
                let length = self.read_dword()?;
                self.position += length.wrapping_mul(self.float_size as u32) as i64;
                b"<flt_list>"
            }
            0x0a => b"{",
            0x0b => b"}",
            0x0c => b"(",
            0x0d => b")",
            0x0e => b"[",
            0x0f => b"]",
            0x10 => b"<",
            0x11 => b">",
            0x12 => b".",
            0x13 => b",",
            0x14 => b";",
            0x1f => b"template",
            0x28 => b"WORD",
            0x29 => b"DWORD",
            0x2a => b"FLOAT",
            0x2b => b"DOUBLE",
            0x2c => b"CHAR",
            0x2d => b"UCHAR",
            0x2e => b"SWORD",
            0x2f => b"SDWORD",
            0x30 => b"void",
            0x31 => b"string",
            0x32 => b"unicode",
            0x33 => b"cstring",
            0x34 => b"array",
            _ => b"",
        };
        Ok(text.to_vec())
    }

    /// `std::string(mP, length)`, then the cursor moved on `advance`.
    fn take_bytes(&mut self, length: i64, advance: i64) -> Result<Vec<u8>, ImportError> {
        let start = self.position;
        if start < 0 || start + length > self.buffer.len() as i64 {
            return undefined("a string read past the buffer");
        }
        let bytes = self.buffer[start as usize..(start + length) as usize].to_vec();
        self.position += advance;
        Ok(bytes)
    }

    /// `GetNextTokenAsString`: a quoted string followed by a semicolon (any
    /// name token in binary files).
    fn next_token_as_string(&mut self) -> Result<Vec<u8>, ImportError> {
        if self.binary {
            return self.next_token();
        }
        self.find_next_non_white_space();
        if self.position >= self.end {
            return self.throw("Unexpected end of file while parsing string");
        }
        if self.current() != b'"' {
            return self.throw("Expected quotation mark.");
        }
        self.position += 1;
        let mut text = Vec::new();
        while self.position < self.end && self.current() != b'"' {
            text.push(self.current());
            self.position += 1;
        }
        if self.position >= self.end - 1 {
            return self.throw("Unexpected end of file while parsing string");
        }
        let next = self.byte_at(self.position + 1)?;
        if next != b';' || self.current() != b'"' {
            return self.throw("Expected quotation mark and semicolon at the end of a string.");
        }
        self.position += 2;
        Ok(text)
    }

    /// `ReadInt`.
    fn read_int(&mut self) -> Result<u32, ImportError> {
        if self.binary {
            if self.number_count == 0 && self.remaining() >= 2 {
                let token = self.read_word()?;
                if token == 0x06 && self.remaining() >= 4 {
                    self.number_count = self.read_dword()?;
                } else {
                    self.number_count = 1;
                }
            }
            self.number_count = self.number_count.wrapping_sub(1);
            // `size_t len(mEnd - mP)`: past the end it is huge.
            let length = self.remaining() as u64;
            if length >= 4 {
                return self.read_dword();
            }
            self.position = self.end;
            return Ok(0);
        }

        self.find_next_non_white_space();
        let negative = self.current() == b'-';
        if negative {
            self.position += 1;
        }
        if !self.current().is_ascii_digit() {
            return self.throw("Number expected.");
        }
        let mut number = 0u32;
        while self.position < self.end && self.current().is_ascii_digit() {
            number = number.wrapping_mul(10).wrapping_add((self.current() - b'0') as u32);
            self.position += 1;
        }
        self.check_for_separator()?;
        Ok(if negative { (number as i32).wrapping_neg() as u32 } else { number })
    }

    /// `ReadFloat`.
    fn read_float(&mut self) -> Result<f32, ImportError> {
        if self.binary {
            if self.number_count == 0 && self.remaining() >= 2 {
                let token = self.read_word()?;
                if token == 0x07 && self.remaining() >= 4 {
                    self.number_count = self.read_dword()?;
                } else {
                    self.number_count = 1;
                }
            }
            self.number_count = self.number_count.wrapping_sub(1);
            if self.remaining() >= self.float_size {
                let start = self.position as usize;
                let size = self.float_size as usize;
                let bytes = &self.buffer[start..start + size];
                self.position += self.float_size;
                if size == 8 {
                    let value = f64::from_le_bytes(bytes.try_into().unwrap_or([0; 8]));
                    return Ok(value as f32);
                }
                let value = f32::from_le_bytes(bytes.try_into().unwrap_or([0; 4]));
                return Ok(value);
            }
            self.position = self.end;
            return Ok(0.0);
        }

        self.find_next_non_white_space();
        let start = self.position as usize;
        let rest = &self.buffer[start.min(self.buffer.len())..];
        if rest.starts_with(b"-1.#IND00") || rest.starts_with(b"1.#IND00") {
            self.position += 9;
            self.check_for_separator()?;
            return Ok(0.0);
        }
        if rest.starts_with(b"1.#QNAN0") {
            self.position += 8;
            self.check_for_separator()?;
            return Ok(0.0);
        }
        let (value, after) = fast_atoreal_move(&self.buffer, start, true)?;
        self.position = after as i64;
        self.check_for_separator()?;
        Ok(value)
    }

    fn read_vector3(&mut self) -> Result<Vector3, ImportError> {
        let x = self.read_float()?;
        let y = self.read_float()?;
        let z = self.read_float()?;
        self.test_for_separator();
        Ok([x, y, z])
    }

    fn read_vector2(&mut self) -> Result<[f32; 2], ImportError> {
        let x = self.read_float()?;
        let y = self.read_float()?;
        self.test_for_separator();
        Ok([x, y])
    }

    fn read_rgba(&mut self) -> Result<[f32; 4], ImportError> {
        let r = self.read_float()?;
        let g = self.read_float()?;
        let b = self.read_float()?;
        let a = self.read_float()?;
        self.test_for_separator();
        Ok([r, g, b, a])
    }

    fn read_rgb(&mut self) -> Result<(), ImportError> {
        for _ in 0..3 {
            self.read_float()?;
        }
        self.test_for_separator();
        Ok(())
    }

    fn read_matrix(&mut self) -> Result<(), ImportError> {
        for _ in 0..16 {
            self.read_float()?;
        }
        Ok(())
    }

    /// `CheckForClosingBrace`.
    fn check_for_closing_brace(&mut self) -> Result<(), ImportError> {
        if self.next_token()? != b"}" {
            return self.throw("Closing brace expected.");
        }
        Ok(())
    }

    /// `CheckForSemicolon` (text only).
    fn check_for_semicolon(&mut self) -> Result<(), ImportError> {
        if self.binary {
            return Ok(());
        }
        if self.next_token()? != b";" {
            return self.throw("Semicolon expected.");
        }
        Ok(())
    }

    /// `CheckForSeparator` (text only): a ',' or ';'.
    fn check_for_separator(&mut self) -> Result<(), ImportError> {
        if self.binary {
            return Ok(());
        }
        let token = self.next_token()?;
        if token != b"," && token != b";" {
            return self.throw("Separator character (';' or ',') expected.");
        }
        Ok(())
    }

    /// `TestForSeparator` (text only): an optional ',' or ';'.
    fn test_for_separator(&mut self) {
        if self.binary {
            return;
        }
        self.find_next_non_white_space();
        if self.position >= self.end {
            return;
        }
        if matches!(self.current(), b';' | b',') {
            self.position += 1;
        }
    }

    /// `readHeadOfDataObject`: an optional name, then '{'.
    fn read_head(&mut self) -> Result<Vec<u8>, ImportError> {
        let name_or_brace = self.next_token()?;
        if name_or_brace == b"{" {
            return Ok(Vec::new());
        }
        if self.next_token()? != b"{" {
            return self.throw("Opening brace expected.");
        }
        Ok(name_or_brace)
    }

    /// `ParseFile`.
    fn parse_file(&mut self) -> Result<(), ImportError> {
        loop {
            let object = self.next_token()?;
            if object.is_empty() {
                return Ok(());
            }
            match object.as_slice() {
                b"template" => self.parse_template()?,
                b"Frame" => self.parse_frame(None)?,
                b"Mesh" => {
                    let mesh = self.parse_mesh()?;
                    self.global_meshes.push(mesh);
                }
                b"AnimTicksPerSecond" => {
                    self.read_head()?;
                    self.read_int()?;
                    self.check_for_closing_brace()?;
                }
                b"AnimationSet" => self.parse_animation_set()?,
                b"Material" => {
                    self.parse_material()?;
                }
                b"}" => {}
                _ => self.parse_unknown()?,
            }
        }
    }

    /// `ParseDataObjectTemplate`: read past.
    fn parse_template(&mut self) -> Result<(), ImportError> {
        self.read_head()?;
        self.next_token()?;
        loop {
            let token = self.next_token()?;
            if token == b"}" {
                return Ok(());
            }
            if token.is_empty() {
                return self.throw("Unexpected end of file reached while parsing template definition");
            }
        }
    }

    /// `ParseDataObjectFrame`: a node under its parent (a second top-level
    /// frame puts both under a dummy root), its matrix, meshes and child
    /// frames.
    fn parse_frame(&mut self, parent: Option<usize>) -> Result<(), ImportError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return undefined("frames nested too deep for the stack");
        }
        let name = self.read_head()?;
        let node = self.nodes.len();
        self.nodes.push(XNode { name, children: Vec::new(), meshes: Vec::new() });
        match parent {
            Some(parent) => self.nodes[parent].children.push(node),
            None => match self.root {
                Some(root) => {
                    let root = if self.nodes[root].name != b"$dummy_root" {
                        let dummy = self.nodes.len();
                        self.nodes.push(XNode { name: b"$dummy_root".to_vec(), children: vec![root], meshes: Vec::new() });
                        self.root = Some(dummy);
                        dummy
                    } else {
                        root
                    };
                    self.nodes[root].children.push(node);
                }
                None => self.root = Some(node),
            },
        }

        loop {
            let object = self.next_token()?;
            if object.is_empty() {
                return self.throw("Unexpected end of file reached while parsing frame");
            }
            match object.as_slice() {
                b"}" => break,
                b"Frame" => self.parse_frame(Some(node))?,
                b"FrameTransformMatrix" => {
                    self.read_head()?;
                    self.read_matrix()?;
                    self.check_for_semicolon()?;
                    self.check_for_closing_brace()?;
                }
                b"Mesh" => {
                    // The node holds the mesh while it is parsed.
                    let mesh = self.parse_mesh()?;
                    self.nodes[node].meshes.push(mesh);
                }
                _ => self.parse_unknown()?,
            }
        }
        self.depth -= 1;
        Ok(())
    }

    /// `ParseDataObjectMesh`.
    fn parse_mesh(&mut self) -> Result<XMesh, ImportError> {
        let mut mesh = XMesh::default();
        self.read_head()?;
        let vertex_count = self.read_int()?;
        scene::check_allocation(vertex_count as u64, VECTOR_SIZE)?;
        while mesh.positions.len() < vertex_count as usize {
            if self.at_binary_end() {
                mesh.positions.resize(vertex_count as usize, [0.0; 3]);
                break;
            }
            let position = self.read_vector3()?;
            mesh.positions.push(position);
        }
        let face_count = self.read_int()?;
        scene::check_allocation(face_count as u64, FACE_SIZE)?;
        while mesh.position_faces.len() < face_count as usize {
            if self.at_binary_end() {
                mesh.position_faces.resize(face_count as usize, Vec::new());
                break;
            }
            let face = self.read_index_list(vertex_count, Some(vertex_count))?;
            mesh.position_faces.push(face);
        }

        loop {
            let object = self.next_token()?;
            match object.as_slice() {
                b"" => return self.throw("Unexpected end of file while parsing mesh structure"),
                b"}" => return Ok(mesh),
                b"MeshNormals" => self.parse_normals(&mut mesh)?,
                b"MeshTextureCoords" => self.parse_texture_coords(&mut mesh)?,
                b"MeshVertexColors" => self.parse_vertex_colors(&mut mesh)?,
                b"MeshMaterialList" => self.parse_material_list(&mut mesh)?,
                b"VertexDuplicationIndices" => self.parse_unknown()?,
                b"XSkinMeshHeader" => {
                    self.read_head()?;
                    for _ in 0..3 {
                        self.read_int()?;
                    }
                    self.check_for_closing_brace()?;
                }
                b"SkinWeights" => self.parse_skin_weights(&mut mesh)?,
                _ => self.parse_unknown()?,
            }
        }
    }

    /// A face's index count and indices (those past `limit` dropped, as
    /// position faces do), then an optional separator.
    fn read_index_list(&mut self, limit: u32, check: Option<u32>) -> Result<Vec<u32>, ImportError> {
        let index_count = self.read_int()?;
        let mut face = Vec::new();
        let mut read = 0u32;
        while read < index_count {
            if self.at_binary_end() {
                // Zeros from here on, each kept.
                let left = (index_count - read) as u64;
                let kept = face.len() as u64 + left;
                scene::check_allocation(kept, 4)?;
                face.resize(kept as usize, 0);
                break;
            }
            let index = self.read_int()?;
            read += 1;
            if check.is_none() || index <= limit {
                face.push(index);
            }
        }
        self.test_for_separator();
        Ok(face)
    }

    /// `ParseDataObjectSkinWeights`.
    fn parse_skin_weights(&mut self, mesh: &mut XMesh) -> Result<(), ImportError> {
        self.read_head()?;
        self.next_token_as_string()?;
        let weight_count = self.read_int()?;
        scene::check_allocation(weight_count as u64, WEIGHT_SIZE)?;
        let mut vertices = Vec::new();
        while vertices.len() < weight_count as usize {
            if self.at_binary_end() {
                vertices.resize(weight_count as usize, 0);
                break;
            }
            vertices.push(self.read_int()?);
        }
        mesh.bones.push(vertices);
        for _ in 0..weight_count {
            if self.at_binary_end() {
                break;
            }
            self.read_float()?;
        }
        self.read_matrix()?;
        self.check_for_semicolon()?;
        self.check_for_closing_brace()
    }

    /// `ParseDataObjectMeshNormals`.
    fn parse_normals(&mut self, mesh: &mut XMesh) -> Result<(), ImportError> {
        self.read_head()?;
        let normal_count = self.read_int()?;
        scene::check_allocation(normal_count as u64, VECTOR_SIZE)?;
        let mut normals = Vec::new();
        while normals.len() < normal_count as usize {
            if self.at_binary_end() {
                normals.resize(normal_count as usize, [0.0; 3]);
                break;
            }
            normals.push(self.read_vector3()?);
        }
        mesh.normals = normals;
        let face_count = self.read_int()?;
        if face_count as usize != mesh.position_faces.len() {
            return self.throw("Normal face count does not match vertex face count.");
        }
        if face_count > 0 {
            let mut faces = Vec::with_capacity(face_count as usize);
            while faces.len() < face_count as usize {
                if self.at_binary_end() {
                    faces.resize(face_count as usize, Vec::new());
                    break;
                }
                let face = self.read_index_list(u32::MAX, None)?;
                faces.push(face);
            }
            mesh.normal_faces = faces;
        }
        self.check_for_closing_brace()
    }

    /// `ParseDataObjectMeshTextureCoords`.
    fn parse_texture_coords(&mut self, mesh: &mut XMesh) -> Result<(), ImportError> {
        self.read_head()?;
        if mesh.texture_coords.len() + 1 > MAX_TEXTURE_COORDS {
            return self.throw("Too many sets of texture coordinates");
        }
        mesh.texture_coords.push(Vec::new());
        let channel = mesh.texture_coords.len() - 1;
        let count = self.read_int()?;
        if count as usize != mesh.positions.len() {
            return self.throw("Texture coord count does not match vertex count");
        }
        scene::check_allocation(count as u64, VECTOR2_SIZE)?;
        while mesh.texture_coords[channel].len() < count as usize {
            if self.at_binary_end() {
                mesh.texture_coords[channel].resize(count as usize, [0.0; 2]);
                break;
            }
            let coord = self.read_vector2()?;
            mesh.texture_coords[channel].push(coord);
        }
        self.check_for_closing_brace()
    }

    /// `ParseDataObjectMeshVertexColors`.
    fn parse_vertex_colors(&mut self, mesh: &mut XMesh) -> Result<(), ImportError> {
        self.read_head()?;
        if mesh.colors.len() + 1 > MAX_COLOR_SETS {
            return self.throw("Too many colorsets");
        }
        let count = self.read_int()?;
        if count as usize != mesh.positions.len() {
            return self.throw("Vertex color count does not match vertex count");
        }
        scene::check_allocation(count as u64, COLOR_SIZE)?;
        let mut colors = vec![[0.0, 0.0, 0.0, 1.0]; count as usize];
        let mut failure = None;
        for _ in 0..count {
            if self.at_binary_end() {
                // Index 0 read over and over, then four zeros.
                colors[0] = [0.0; 4];
                break;
            }
            let index = self.read_int()?;
            if index as usize >= mesh.positions.len() {
                failure = Some("Vertex color index out of bounds");
                break;
            }
            colors[index as usize] = self.read_rgba()?;
            if !self.binary {
                self.find_next_non_white_space();
                if matches!(self.current(), b';' | b',') {
                    self.position += 1;
                }
            }
        }
        // The set exists (counted) even when its reading throws.
        mesh.colors.push(colors);
        if let Some(what) = failure {
            return self.throw(what);
        }
        self.check_for_closing_brace()
    }

    /// `ParseDataObjectMeshMaterialList`: per-face material indices (one
    /// repeated over all faces), then the materials (or references to
    /// them).
    fn parse_material_list(&mut self, mesh: &mut XMesh) -> Result<(), ImportError> {
        self.read_head()?;
        self.read_int()?;
        let index_count = self.read_int()?;
        if index_count as usize != mesh.position_faces.len() && index_count != 1 {
            return self.throw("Per-Face material index count does not match face count.");
        }
        while mesh.face_materials.len() < index_count as usize {
            if self.at_binary_end() {
                mesh.face_materials.resize(index_count as usize, 0);
                break;
            }
            let material = self.read_int()?;
            mesh.face_materials.push(material);
        }
        if !self.binary && self.position < self.end && self.current() == b';' {
            self.position += 1;
        }
        while mesh.face_materials.len() < mesh.position_faces.len() {
            let first = mesh.face_materials[0];
            mesh.face_materials.push(first);
        }
        loop {
            let object = self.next_token()?;
            match object.as_slice() {
                b"" => return self.throw("Unexpected end of file while parsing mesh material list."),
                b"}" => return Ok(()),
                b"{" => {
                    self.next_token()?;
                    mesh.material_count += 1;
                    self.check_for_closing_brace()?;
                }
                b"Material" => {
                    mesh.material_count += 1;
                    self.parse_material()?;
                }
                b";" => {}
                _ => self.parse_unknown()?,
            }
        }
    }

    /// `ParseDataObjectMaterial`: read past.
    fn parse_material(&mut self) -> Result<(), ImportError> {
        self.read_head()?;
        self.read_rgba()?;
        self.read_float()?;
        self.read_rgb()?;
        self.read_rgb()?;
        loop {
            let object = self.next_token()?;
            match object.as_slice() {
                b"" => return self.throw("Unexpected end of file while parsing mesh material"),
                b"}" => return Ok(()),
                b"TextureFilename" | b"TextureFileName" | b"NormalmapFilename" | b"NormalmapFileName" => {
                    self.read_head()?;
                    self.next_token_as_string()?;
                    self.check_for_closing_brace()?;
                }
                _ => self.parse_unknown()?,
            }
        }
    }

    /// `ParseDataObjectAnimationSet` and `ParseDataObjectAnimation`: read
    /// past.
    fn parse_animation_set(&mut self) -> Result<(), ImportError> {
        self.read_head()?;
        loop {
            let object = self.next_token()?;
            match object.as_slice() {
                b"" => return self.throw("Unexpected end of file while parsing animation set."),
                b"}" => return Ok(()),
                b"Animation" => self.parse_animation()?,
                _ => self.parse_unknown()?,
            }
        }
    }

    fn parse_animation(&mut self) -> Result<(), ImportError> {
        self.read_head()?;
        loop {
            let object = self.next_token()?;
            match object.as_slice() {
                b"" => return self.throw("Unexpected end of file while parsing animation."),
                b"}" => return Ok(()),
                b"AnimationKey" => self.parse_animation_key()?,
                b"AnimationOptions" => self.parse_unknown()?,
                b"{" => {
                    self.next_token()?;
                    self.check_for_closing_brace()?;
                }
                _ => self.parse_unknown()?,
            }
        }
    }

    /// `ParseDataObjectAnimationKey`.
    fn parse_animation_key(&mut self) -> Result<(), ImportError> {
        self.read_head()?;
        let key_type = self.read_int()?;
        let key_count = self.read_int()?;
        for _ in 0..key_count {
            self.read_int()?;
            match key_type {
                0 => {
                    if self.read_int()? != 4 {
                        return self.throw("Invalid number of arguments for quaternion key in animation");
                    }
                    for _ in 0..4 {
                        self.read_float()?;
                    }
                    self.check_for_semicolon()?;
                }
                1 | 2 => {
                    if self.read_int()? != 3 {
                        return self.throw("Invalid number of arguments for vector key in animation");
                    }
                    self.read_vector3()?;
                }
                3 | 4 => {
                    if self.read_int()? != 16 {
                        return self.throw("Invalid number of arguments for matrix key in animation");
                    }
                    self.read_matrix()?;
                    self.check_for_semicolon()?;
                }
                _ => return self.throw("Unknown key type in animation."),
            }
            self.check_for_separator()?;
        }
        self.check_for_closing_brace()
    }

    /// `ParseUnknownDataObject`: past its braces.
    fn parse_unknown(&mut self) -> Result<(), ImportError> {
        loop {
            let token = self.next_token()?;
            if token.is_empty() {
                return self.throw("Unexpected end of file while parsing unknown segment.");
            }
            if token == b"{" {
                break;
            }
        }
        let mut counter = 1u32;
        while counter > 0 {
            let token = self.next_token()?;
            if token.is_empty() {
                return self.throw("Unexpected end of file while parsing unknown segment.");
            }
            if token == b"{" {
                counter += 1;
            } else if token == b"}" {
                counter -= 1;
            }
        }
        Ok(())
    }

    /// `FilterHierarchy`: an only, unnamed child holding meshes merges into
    /// a node without meshes; the child's own children go with it.
    fn filter_hierarchy(&mut self, node: usize) {
        let mut pending = vec![node];
        while let Some(node) = pending.pop() {
            if self.nodes[node].children.len() == 1 && self.nodes[node].meshes.is_empty() {
                let child = self.nodes[node].children[0];
                if self.nodes[child].name.is_empty() && !self.nodes[child].meshes.is_empty() {
                    let meshes = std::mem::take(&mut self.nodes[child].meshes);
                    self.nodes[node].meshes.extend(meshes);
                    self.nodes[node].children.clear();
                }
            }
            for &child in self.nodes[node].children.iter().rev() {
                pending.push(child);
            }
        }
    }
}

/// `CreateMeshes` for one source mesh: a mesh per material with faces.
fn create_meshes(source: &XMesh, output: &mut Vec<Mesh>) -> Result<(), ImportError> {
    let material_count = source.material_count.max(1);
    for material in 0..material_count {
        let faces: Vec<usize> = if source.face_materials.is_empty() {
            (0..source.position_faces.len()).collect()
        } else {
            (0..source.face_materials.len()).filter(|&face| source.face_materials[face] as usize == material).collect()
        };
        if faces.iter().any(|&face| face >= source.position_faces.len()) {
            return undefined("a face material for a face past the faces");
        }
        let vertex_count: usize = faces.iter().map(|&face| source.position_faces[face].len()).sum();
        if vertex_count == 0 {
            continue;
        }
        if !source.face_materials.is_empty() && source.material_count == 0 {
            return undefined("face materials without materials");
        }

        let has_normals = !source.normals.is_empty();
        let mut vertices = vec![[0.0f32; 3]; vertex_count];
        let mut normals = vec![[0.0f32; 3]; if has_normals { vertex_count } else { 0 }];
        let mut coords: Vec<Vec<Vector3>> = source.texture_coords.iter().map(|set| if set.is_empty() { Vec::new() } else { vec![[0.0; 3]; vertex_count] }).collect();
        let mut colors: Vec<Vec<[f32; 4]>> = source.colors.iter().map(|set| if set.is_empty() { Vec::new() } else { vec![[0.0; 4]; vertex_count] }).collect();
        let mut face_lists = Vec::with_capacity(faces.len());
        let mut next = 0usize;
        for &face in &faces {
            let indices = &source.position_faces[face];
            let mut out_face = Vec::with_capacity(indices.len());
            for (corner, &index) in indices.iter().enumerate() {
                out_face.push(next as u32);
                if index as usize >= source.positions.len() {
                    continue;
                }
                vertices[next] = source.positions[index as usize];
                if has_normals
                    && let Some(normal_face) = source.normal_faces.get(face)
                    && let Some(&normal_index) = normal_face.get(corner)
                    && let Some(&normal) = source.normals.get(normal_index as usize)
                {
                    normals[next] = normal;
                }
                for (set, list) in coords.iter_mut().enumerate() {
                    if !list.is_empty() {
                        let [u, v] = source.texture_coords[set][index as usize];
                        list[next] = [u, 1.0 - v, 0.0];
                    }
                }
                for (set, list) in colors.iter_mut().enumerate() {
                    if !list.is_empty() {
                        list[next] = source.colors[set][index as usize];
                    }
                }
                next += 1;
            }
            face_lists.push(out_face);
        }
        for bone in &source.bones {
            if bone.iter().any(|&vertex| vertex as usize >= source.positions.len()) {
                return undefined("a bone weight for a vertex past the vertices");
            }
        }

        let mut mesh = Mesh { vertex_count, vertices: Some(vertices), faces: Some(face_lists), ..Mesh::default() };
        if has_normals {
            mesh.normals = Some(normals);
        }
        for (set, list) in coords.into_iter().enumerate() {
            if !list.is_empty() {
                mesh.texture_coords[set] = Some(list);
            }
        }
        for (set, list) in colors.into_iter().enumerate() {
            if !list.is_empty() {
                mesh.colors[set] = Some(list);
            }
        }
        output.push(mesh);
    }
    Ok(())
}

/// The MSZIP chunks inflated (`Compression::decompressBlock` each): the
/// whole output buffer (zeros after the data) and the data's length.
fn decompress(buffer: &[u8], start: usize, end: usize) -> Result<(Vec<u8>, usize), ImportError> {
    let read_u16 = |position: usize| -> u16 { u16::from_le_bytes([buffer[position], buffer[position + 1]]) };
    // The estimate (and the checks it makes).
    let mut cursor = start + 6;
    let mut block_count = 0usize;
    while cursor + 3 < end {
        let offset = read_u16(cursor) as usize;
        cursor += 2;
        if offset >= MSZIP_BLOCK {
            return fail("X: Invalid offset to next MSZIP compressed block");
        }
        let magic = read_u16(cursor);
        cursor += 2;
        if magic != MSZIP_MAGIC {
            return fail("X: Unsupported compressed format, expected MSZIP header");
        }
        cursor += offset;
        block_count += 1;
    }

    let mut output = vec![0u8; block_count * MSZIP_BLOCK + 1];
    let mut written = 0usize;
    let mut inflate = Inflate::new(false, 15);
    let mut cursor = start + 6;
    while cursor + 3 < end {
        let offset = read_u16(cursor) as usize;
        cursor += 4;
        if cursor + offset > end + 2 {
            return fail("X: Unexpected EOF in compressed chunk");
        }
        if offset != 0 {
            let available = buffer.len().min(cursor + offset);
            let input = &buffer[cursor..available];
            let target = &mut output[written..written + MSZIP_BLOCK];
            let before_in = inflate.total_in();
            let before_out = inflate.total_out();
            let result = inflate.decompress(input, target, InflateFlush::SyncFlush);
            let consumed = (inflate.total_in() - before_in) as usize;
            let produced = (inflate.total_out() - before_out) as usize;
            let finished = matches!(result, Ok(Status::StreamEnd));
            if !matches!(result, Ok(Status::Ok) | Ok(Status::StreamEnd)) {
                return fail("X: Failed to decompress MSZIP-compressed data");
            }
            if available < cursor + offset && consumed == input.len() && !finished {
                return undefined("compressed data read past the buffer");
            }
            inflate.reset(false);
            let dictionary = &output[written..written + produced];
            let _ = inflate.set_dictionary(dictionary);
            written += produced;
        }
        cursor += offset;
    }
    Ok((output, written))
}

pub fn read(bytes: &[u8]) -> Result<Scene, ImportError> {
    if bytes.len() < 16 {
        return fail("XFile is too small.");
    }
    let mut buffer = bytes.to_vec();
    buffer.push(0);
    convert_to_utf8(&mut buffer)?;
    let end = buffer.len() - 1;
    if &buffer[..4] != b"xof " {
        return fail("Header mismatch, file is not an XFile.");
    }
    let format = &buffer[8..12];
    let (binary, compressed) = match format {
        b"txt " => (false, false),
        b"bin " => (true, false),
        b"tzip" => (false, true),
        b"bzip" => (true, true),
        _ => return fail("Unsupported x-file format"),
    };
    // `(unsigned int)(mP[i] - 48)` with a signed `char`.
    let digit = |index: usize| (buffer[index] as i8 as i32 - 48) as u32;
    let float_bits = digit(12).wrapping_mul(1000).wrapping_add(digit(13).wrapping_mul(100)).wrapping_add(digit(14).wrapping_mul(10)).wrapping_add(digit(15));
    if float_bits != 32 && float_bits != 64 {
        return fail("Unknown float size specified in x-file header.");
    }

    let (data, start, data_end) = if compressed {
        let (output, written) = decompress(&buffer, 16, end)?;
        (output, 0, written)
    } else {
        (buffer, 16, end)
    };
    let mut parser = Parser {
        buffer: data,
        position: start as i64,
        end: data_end as i64,
        binary,
        float_size: (float_bits / 8) as i64,
        number_count: 0,
        nodes: Vec::new(),
        root: None,
        global_meshes: Vec::new(),
        depth: 0,
    };
    if !compressed {
        parser.read_until_end_of_line();
    }
    parser.parse_file()?;
    if let Some(root) = parser.root {
        parser.filter_hierarchy(root);
    }

    // `CreateNodes`, depth first, then the global meshes.
    let mut meshes = Vec::new();
    if let Some(root) = parser.root {
        let mut pending = vec![root];
        while let Some(node) = pending.pop() {
            if parser.nodes[node].name.len() > MAX_LENGTH - 1 {
                return undefined("a frame name copied past its string");
            }
            for mesh in &parser.nodes[node].meshes {
                create_meshes(mesh, &mut meshes)?;
            }
            for &child in parser.nodes[node].children.iter().rev() {
                pending.push(child);
            }
        }
    }
    for mesh in &parser.global_meshes {
        create_meshes(mesh, &mut meshes)?;
    }
    if parser.root.is_none() && parser.global_meshes.is_empty() {
        return fail("No root node");
    }

    // `MakeLeftHandedProcess` and `FlipWindingOrderProcess`.
    for mesh in &mut meshes {
        if let Some(vertices) = mesh.vertices.as_mut() {
            for vertex in vertices.iter_mut() {
                vertex[2] *= -1.0;
            }
        }
        if mesh.vertex_count > 0
            && let Some(normals) = mesh.normals.as_mut()
        {
            for normal in normals.iter_mut() {
                normal[2] *= -1.0;
            }
        }
        if let Some(faces) = mesh.faces.as_mut() {
            for face in faces.iter_mut() {
                face.reverse();
            }
        }
    }
    Ok(Scene { meshes })
}
