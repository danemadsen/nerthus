//! Doom 3's MD5 files (`MD5Importer`): an .md5mesh's meshes in their bind
//! pose, then the .md5anim of the same name; an .md5anim alone becomes a
//! skeleton mesh, and an .md5camera has no meshes.
//!
//! Files are first split into sections (`MD5Parser`). A section's name
//! runs to its first blank. Each word after it is its value, the last one
//! standing, taken with the blank or line end after it; a line end there
//! carries the loop on into the next line, whose name becomes a value
//! too. So "numJoints 3\nnumMeshes 1\n\njoints {" is one section,
//! numJoints, valued "1\n", then "joints". A `{` opens a block whose
//! lines, up to one opening with `}`, are the section's elements, each
//! line end overwritten by a NUL. Elements are parsed with the file's end
//! as their bound, so a parse that misses its NUL reads on into the next
//! line.

use std::collections::HashMap;

use crate::fast_atof::{fast_atoreal_move, strtol10, strtoul10};
use crate::io::FileSystem;
use crate::matrix::Matrix4;
use crate::msvc::VectorCapacity;
use crate::node::Node;
use crate::parsing::{checked_at, is_line_end, is_space, is_space_or_newline, remove_line_comments, skip_line, skip_spaces, skip_spaces_and_line_end, token_match};
use crate::registry::get_extension;
use crate::scene::{ImportError, Mesh, PRIMITIVE_TRIANGLE, Scene, check_allocation, fail};
use crate::skeleton;
use crate::vector::{Vector3, scale};

/// `AI_MAXLEN`: an `aiString`'s bytes with its NUL.
const MAX_LENGTH: usize = 1024;

/// `AI_MD5_WEIGHT_EPSILON` (float's epsilon): weights this close to zero
/// are skipped.
const WEIGHT_EPSILON: f32 = f32::EPSILON;

/// Joint hierarchies deeper than this overflow the stack of the
/// recursion that builds them.
const MAX_DEPTH: usize = 2048;

// Element sizes (MSVC, x64) of the vectors and arrays whose allocations
// a file's counts decide.
const SECTION_SIZE: u64 = 96;
const ELEMENT_SIZE: u64 = 24;
const MESH_SIZE: u64 = 1104;
const JOINT_SIZE: u64 = 1204;
const VERTEX_SIZE: u64 = 16;
const FACE_SIZE: u64 = 16;
const WEIGHT_SIZE: u64 = 20;
const VERTEX_WEIGHT_SIZE: u64 = 8;
const ANIMATED_BONE_SIZE: u64 = 1040;
const BASE_FRAME_SIZE: u64 = 24;
const FRAME_SIZE: u64 = 32;
const VALUE_SIZE: u64 = 4;
const CAMERA_FRAME_SIZE: u64 = 28;
const CUT_SIZE: u64 = 4;
const POINTER_SIZE: u64 = 8;
const VECTOR_KEY_SIZE: u64 = 24;
const QUATERNION_KEY_SIZE: u64 = 32;

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

/// `aiQuaternion` (w, x, y, z), as far as MD5 uses it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Quaternion {
    w: f32,
    x: f32,
    y: f32,
    z: f32,
}

impl Quaternion {
    const IDENTITY: Quaternion = Quaternion { w: 1.0, x: 0.0, y: 0.0, z: 0.0 };

    /// `MD5::ConvertQuaternion`: x, y and z as given, w from the unit
    /// length (0 when they are longer), negated.
    fn from_md5(vector: Vector3) -> Quaternion {
        let [x, y, z] = vector;
        let rest = 1.0f32 - x * x - y * y - z * z;
        let length = if rest < 0.0 { 0.0 } else { rest.sqrt() };
        Quaternion { w: -length, x, y, z }
    }

    /// `operator*`.
    fn product(&self, other: &Quaternion) -> Quaternion {
        let Quaternion { w, x, y, z } = *self;
        Quaternion {
            w: w * other.w - x * other.x - y * other.y - z * other.z,
            x: w * other.x + x * other.w + y * other.z - z * other.y,
            y: w * other.y + y * other.w + z * other.x - x * other.z,
            z: w * other.z + z * other.w + x * other.y - y * other.x,
        }
    }

    /// `Rotate`: q × (0, v) × q's conjugate.
    fn rotate(&self, vector: Vector3) -> Vector3 {
        let pure = Quaternion { w: 0.0, x: vector[0], y: vector[1], z: vector[2] };
        let conjugate = Quaternion { w: self.w, x: -self.x, y: -self.y, z: -self.z };
        let half = self.product(&pure);
        let rotated = half.product(&conjugate);
        [rotated.x, rotated.y, rotated.z]
    }

    /// `GetMatrix`, as the rotation part of a 4×4 matrix.
    fn matrix(&self) -> Matrix4 {
        let Quaternion { w, x, y, z } = *self;
        Matrix4([
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - z * w),
            2.0 * (x * z + y * w),
            0.0,
            2.0 * (x * y + z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - x * w),
            0.0,
            2.0 * (x * z - y * w),
            2.0 * (y * z + x * w),
            1.0 - 2.0 * (x * x + y * y),
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
        ])
    }
}

/// A section: its name, its last value (with the byte after it), and
/// where each line of its block starts.
struct Section {
    name: Vec<u8>,
    value: Vec<u8>,
    elements: Vec<usize>,
    element_capacity: VectorCapacity,
}

impl Section {
    fn new() -> Self {
        Self { name: Vec::new(), value: Vec::new(), elements: Vec::new(), element_capacity: VectorCapacity::new(ELEMENT_SIZE) }
    }
}

/// `MD5Parser`: the file with a NUL after it (its "//" comments blanked)
/// split into sections.
struct Parser {
    buffer: Vec<u8>,
    /// `bufferEnd`: the file's size, where the NUL is.
    end: usize,
    position: usize,
    sections: Vec<Section>,
}

impl Parser {
    /// The header, then sections until one meets the file's end (that one
    /// is kept too).
    fn parse(bytes: &[u8]) -> Result<Parser, ImportError> {
        let mut buffer = bytes.to_vec();
        buffer.push(0);
        remove_line_comments(b"//", &mut buffer, b' ');
        let mut parser = Parser { buffer, end: bytes.len(), position: 0, sections: Vec::new() };
        parser.parse_header()?;

        let mut section_capacity = VectorCapacity::new(SECTION_SIZE);
        loop {
            section_capacity.grow(parser.sections.len() + 1)?;
            let mut section = Section::new();
            let more = parser.parse_section(&mut section)?;
            parser.sections.push(section);
            if !more {
                return Ok(parser);
            }
        }
    }

    /// `ParseHeader`: "MD5Version", 10, then the command line, read up to
    /// and past its line end (past the final NUL when that ends it).
    fn parse_header(&mut self) -> Result<(), ImportError> {
        let (position, _) = skip_spaces(&self.buffer, self.position, self.end);
        self.position = position;
        if !token_match(&self.buffer, &mut self.position, b"MD5Version") {
            return fail("[MD5] Line 0: Invalid MD5 file: MD5Version tag has not been found");
        }
        let (position, _) = skip_spaces(&self.buffer, self.position, self.end);
        let (version, position) = strtoul10(&self.buffer, position);
        self.position = position;
        if version != 10 {
            return fail("[MD5] Line 0: MD5 version tag is unknown (10 is expected)");
        }
        let (position, _) = skip_line(&self.buffer, self.position, self.end);
        self.position = position;
        if self.position == self.end {
            return Ok(());
        }

        loop {
            let byte = self.buffer[self.position];
            self.position += 1;
            if is_line_end(byte) {
                break;
            }
        }
        self.skip_spaces_and_line_end()
    }

    /// `MD5Parser::SkipSpacesAndLineEnd`: nothing at the file's end, else
    /// past blanks and line breaks. It reads where it starts, which may be
    /// past the final NUL.
    fn skip_spaces_and_line_end(&mut self) -> Result<(), ImportError> {
        if self.position == self.end {
            return Ok(());
        }
        loop {
            let byte = checked_at(&self.buffer, self.position)?;
            if !matches!(byte, b'\r' | b'\n' | b'\t' | b' ') {
                return Ok(());
            }
            self.position += 1;
            if self.position == self.end {
                return Ok(());
            }
        }
    }

    /// `ParseSection`: false once the file ends.
    fn parse_section(&mut self, section: &mut Section) -> Result<bool, ImportError> {
        let name_start = self.position;
        while !is_space_or_newline(self.buffer[self.position]) {
            self.position += 1;
            if self.position == self.end {
                return Ok(false);
            }
        }
        section.name = self.buffer[name_start..self.position].to_vec();
        while is_space(self.buffer[self.position]) {
            self.position += 1;
            if self.position == self.end {
                return Ok(false);
            }
        }

        loop {
            let byte = self.buffer[self.position];
            if byte == b'{' {
                if !self.parse_block(section)? {
                    return Ok(false);
                }
                break;
            }
            if is_space_or_newline(byte) {
                break;
            }
            // A value at global scope: the word and the byte after it.
            let value_start = self.position;
            loop {
                let byte = self.buffer[self.position];
                self.position += 1;
                if is_space_or_newline(byte) {
                    break;
                }
                if self.position == self.end {
                    return Ok(false);
                }
            }
            section.value = self.buffer[value_start..self.position].to_vec();
        }

        if self.position == self.end {
            return Ok(false);
        }
        while is_space_or_newline(self.buffer[self.position]) {
            if self.position == self.end {
                break;
            }
            self.position += 1;
        }
        Ok(self.buffer[self.position] != 0)
    }

    /// A `{ … }` block: each line (from its first non-blank byte) is an
    /// element, its line end overwritten by a NUL, until a line opens with
    /// `}`. False when the file ends first.
    fn parse_block(&mut self, section: &mut Section) -> Result<bool, ImportError> {
        self.position += 1;
        if self.position == self.end {
            return Ok(false);
        }
        loop {
            // (A NUL counts as a line end here, so none is ever met after.)
            while is_space_or_newline(self.buffer[self.position]) {
                self.position += 1;
                if self.position == self.end {
                    return Ok(false);
                }
            }
            if self.buffer[self.position] == b'}' {
                self.position += 1;
                return Ok(true);
            }

            section.element_capacity.grow(section.elements.len() + 1)?;
            section.elements.push(self.position);
            while !is_line_end(self.buffer[self.position]) {
                self.position += 1;
                if self.position == self.end {
                    return Ok(false);
                }
            }
            if self.buffer[self.position] != 0 {
                self.buffer[self.position] = 0;
                self.position += 1;
                if self.position == self.end {
                    return Ok(false);
                }
            }
        }
    }

    fn cursor(&self, start: usize) -> Cursor<'_> {
        Cursor { buffer: &self.buffer, end: self.end, position: start }
    }
}

/// A parse of an element, bounded by the file's end rather than the
/// element's.
struct Cursor<'a> {
    buffer: &'a [u8],
    end: usize,
    position: usize,
}

impl Cursor<'_> {
    /// `AI_MD5_SKIP_SPACES`. It reads where it starts, which may be past
    /// the final NUL.
    fn skip_spaces(&mut self) -> Result<(), ImportError> {
        checked_at(self.buffer, self.position)?;
        let (position, _) = skip_spaces(self.buffer, self.position, self.end);
        self.position = position;
        Ok(())
    }

    /// `*sz++` over a '(' or ')' that is only checked for a warning.
    fn step(&mut self) {
        self.position += 1;
    }

    fn unsigned(&mut self) -> u32 {
        let (value, position) = strtoul10(self.buffer, self.position);
        self.position = position;
        value
    }

    fn signed(&mut self) -> i32 {
        let (value, position) = strtol10(self.buffer, self.position);
        self.position = position;
        value
    }

    /// `fast_atoreal_move<float>`, which fails on anything but a number.
    fn real(&mut self) -> Result<f32, ImportError> {
        let (value, position) = fast_atoreal_move(self.buffer, self.position, true)?;
        self.position = position;
        Ok(value)
    }

    fn token(&mut self, token: &[u8]) -> bool {
        token_match(self.buffer, &mut self.position, token)
    }

    /// `AI_MD5_READ_TRIPLE`: "( x y z )". A missing '(' passes a byte
    /// more; the file's end there stops it before the numbers (`None`, the
    /// vector unchanged).
    fn triple(&mut self) -> Result<Option<Vector3>, ImportError> {
        self.skip_spaces()?;
        if self.buffer[self.position] != b'(' {
            if self.position == self.end {
                return Ok(None);
            }
            self.position += 1;
        }
        if self.position == self.end {
            return Ok(None);
        }
        self.position += 1;

        let mut triple = [0.0f32; 3];
        for component in &mut triple {
            self.skip_spaces()?;
            *component = self.real()?;
        }
        self.skip_spaces()?;
        if self.position != self.end {
            self.position += 1;
        }
        Ok(Some(triple))
    }

    /// `AI_MD5_PARSE_STRING_IN_QUOTATION`: the text between the next two
    /// quotes before a NUL, else empty. Text of `MAX_LENGTH` bytes or more
    /// is copied past the end of the `aiString`.
    fn quoted(&mut self) -> Result<Vec<u8>, ImportError> {
        self.skip_to_quote()?;
        if self.buffer[self.position] == 0 {
            return Ok(Vec::new());
        }
        self.position += 1;
        let start = self.position;
        self.skip_to_quote()?;
        if self.buffer[self.position] == 0 {
            return Ok(Vec::new());
        }
        let length = self.position - start;
        self.position += 1;
        if length >= MAX_LENGTH {
            return undefined("a quoted name longer than its string");
        }
        Ok(self.buffer[start..start + length].to_vec())
    }

    /// To a quote, a NUL or the file's end.
    fn skip_to_quote(&mut self) -> Result<(), ImportError> {
        loop {
            let byte = checked_at(self.buffer, self.position)?;
            if byte == b'"' || byte == 0 || self.position == self.end {
                return Ok(());
            }
            self.position += 1;
        }
    }
}

/// `MD5::BoneDesc` as read: "name" parent ( x y z ) ( qx qy qz ). (The
/// name only names nodes and bones, which no mesh here keeps.)
#[derive(Clone, Debug, Default)]
struct Joint {
    parent: i32,
    position: Vector3,
    rotation: Vector3,
}

/// `MD5::VertexDesc`.
#[derive(Clone, Copy, Debug, Default)]
struct Vertex {
    uv: [f32; 2],
    first_weight: u32,
    weight_count: u32,
}

/// `MD5::WeightDesc`.
#[derive(Clone, Copy, Debug, Default)]
struct Weight {
    joint: u32,
    weight: f32,
    offset: Vector3,
}

/// `MD5::MeshDesc`. A face is `None` until a "tri" line gives its
/// corners (its index array is null).
struct MeshSource {
    shader: Vec<u8>,
    vertices: Vec<Vertex>,
    vertex_capacity: VectorCapacity,
    faces: Vec<Option<[u32; 3]>>,
    face_capacity: VectorCapacity,
    weights: Vec<Weight>,
    weight_capacity: VectorCapacity,
}

impl MeshSource {
    fn new() -> Self {
        Self {
            shader: Vec::new(),
            vertices: Vec::new(),
            vertex_capacity: VectorCapacity::new(VERTEX_SIZE),
            faces: Vec::new(),
            face_capacity: VectorCapacity::new(FACE_SIZE),
            weights: Vec::new(),
            weight_capacity: VectorCapacity::new(WEIGHT_SIZE),
        }
    }

    fn resize_vertices(&mut self, size: usize) -> Result<(), ImportError> {
        self.vertex_capacity.grow(size)?;
        self.vertices.resize(size, Vertex::default());
        Ok(())
    }

    fn resize_faces(&mut self, size: usize) -> Result<(), ImportError> {
        self.face_capacity.grow(size)?;
        self.faces.resize(size, None);
        Ok(())
    }

    fn resize_weights(&mut self, size: usize) -> Result<(), ImportError> {
        self.weight_capacity.grow(size)?;
        self.weights.resize(size, Weight::default());
        Ok(())
    }

    /// A "vert" line's index: the list grows to hold it, but `index + 1`
    /// wraps to 0 for the largest.
    fn vertex_slot(&mut self, index: u32) -> Result<usize, ImportError> {
        if index as usize >= self.vertices.len() {
            let size = index.wrapping_add(1);
            self.resize_vertices(size as usize)?;
        }
        if index as usize >= self.vertices.len() {
            return undefined("a vertex index past the list");
        }
        Ok(index as usize)
    }

    fn face_slot(&mut self, index: u32) -> Result<usize, ImportError> {
        if index as usize >= self.faces.len() {
            let size = index.wrapping_add(1);
            self.resize_faces(size as usize)?;
        }
        if index as usize >= self.faces.len() {
            return undefined("a triangle index past the list");
        }
        Ok(index as usize)
    }

    fn weight_slot(&mut self, index: u32) -> Result<usize, ImportError> {
        if index as usize >= self.weights.len() {
            let size = index.wrapping_add(1);
            self.resize_weights(size as usize)?;
        }
        if index as usize >= self.weights.len() {
            return undefined("a weight index past the list");
        }
        Ok(index as usize)
    }
}

fn read_joint(cursor: &mut Cursor) -> Result<Joint, ImportError> {
    cursor.quoted()?;
    cursor.skip_spaces()?;
    let parent = cursor.signed();
    let position = cursor.triple()?.unwrap_or_default();
    let rotation = cursor.triple()?.unwrap_or_default();
    Ok(Joint { parent, position, rotation })
}

/// "vert index ( u v ) firstWeight weightCount".
fn read_vertex(mesh: &mut MeshSource, cursor: &mut Cursor) -> Result<(), ImportError> {
    cursor.skip_spaces()?;
    let index = cursor.unsigned();
    cursor.skip_spaces()?;
    let slot = mesh.vertex_slot(index)?;
    cursor.step();
    cursor.skip_spaces()?;
    let u = cursor.real()?;
    cursor.skip_spaces()?;
    let v = cursor.real()?;
    cursor.skip_spaces()?;
    cursor.step();
    cursor.skip_spaces()?;
    let first_weight = cursor.unsigned();
    cursor.skip_spaces()?;
    let weight_count = cursor.unsigned();
    mesh.vertices[slot] = Vertex { uv: [u, v], first_weight, weight_count };
    Ok(())
}

/// "tri index a b c".
fn read_triangle(mesh: &mut MeshSource, cursor: &mut Cursor) -> Result<(), ImportError> {
    cursor.skip_spaces()?;
    let index = cursor.unsigned();
    let slot = mesh.face_slot(index)?;
    let mut corners = [0u32; 3];
    for corner in &mut corners {
        cursor.skip_spaces()?;
        *corner = cursor.unsigned();
    }
    mesh.faces[slot] = Some(corners);
    Ok(())
}

/// "weight index joint weight ( x y z )".
fn read_weight(mesh: &mut MeshSource, cursor: &mut Cursor) -> Result<(), ImportError> {
    cursor.skip_spaces()?;
    let index = cursor.unsigned();
    cursor.skip_spaces()?;
    let slot = mesh.weight_slot(index)?;
    let joint = cursor.unsigned();
    cursor.skip_spaces()?;
    let value = cursor.real()?;
    let offset = cursor.triple()?;

    let weight = &mut mesh.weights[slot];
    weight.joint = joint;
    weight.weight = value;
    if let Some(offset) = offset {
        weight.offset = offset;
    }
    Ok(())
}

/// A line of a "mesh" block.
fn read_mesh_element(mesh: &mut MeshSource, cursor: &mut Cursor) -> Result<(), ImportError> {
    if cursor.token(b"shader") {
        cursor.skip_spaces()?;
        mesh.shader = cursor.quoted()?;
    } else if cursor.token(b"numverts") {
        cursor.skip_spaces()?;
        let count = cursor.unsigned();
        mesh.resize_vertices(count as usize)?;
    } else if cursor.token(b"numtris") {
        cursor.skip_spaces()?;
        let count = cursor.unsigned();
        mesh.resize_faces(count as usize)?;
    } else if cursor.token(b"numweights") {
        cursor.skip_spaces()?;
        let count = cursor.unsigned();
        mesh.resize_weights(count as usize)?;
    } else if cursor.token(b"vert") {
        read_vertex(mesh, cursor)?;
    } else if cursor.token(b"tri") {
        read_triangle(mesh, cursor)?;
    } else if cursor.token(b"weight") {
        read_weight(mesh, cursor)?;
    }
    Ok(())
}

/// `MD5MeshParser`.
struct MeshFile {
    joints: Vec<Joint>,
    meshes: Vec<MeshSource>,
}

fn parse_mesh_file(parser: &Parser) -> Result<MeshFile, ImportError> {
    let mut joints = Vec::new();
    let mut joint_capacity = VectorCapacity::new(JOINT_SIZE);
    let mut meshes = Vec::new();
    let mut mesh_capacity = VectorCapacity::new(MESH_SIZE);
    for section in &parser.sections {
        match section.name.as_slice() {
            b"numMeshes" => {
                let (count, _) = strtoul10(&section.value, 0);
                mesh_capacity.reserve(count as usize)?;
            }
            b"numJoints" => {
                let (count, _) = strtoul10(&section.value, 0);
                joint_capacity.reserve(count as usize)?;
            }
            b"joints" => {
                for &start in &section.elements {
                    joint_capacity.grow(joints.len() + 1)?;
                    let mut cursor = parser.cursor(start);
                    let joint = read_joint(&mut cursor)?;
                    joints.push(joint);
                }
            }
            b"mesh" => {
                mesh_capacity.grow(meshes.len() + 1)?;
                let mut mesh = MeshSource::new();
                for &start in &section.elements {
                    let mut cursor = parser.cursor(start);
                    read_mesh_element(&mut mesh, &mut cursor)?;
                }
                meshes.push(mesh);
            }
            _ => {}
        }
    }
    Ok(MeshFile { joints, meshes })
}

/// The order `AttachChilds_Mesh` and `AttachChilds_Anim` visit joints in,
/// each with its parent joint: depth first from the joints whose parent
/// is -1, a joint's children (the joints whose parent index is its index,
/// itself never among them) in index order.
fn hierarchy_order(parents: &[i32]) -> Result<Vec<(usize, Option<usize>)>, ImportError> {
    let mut top = Vec::new();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); parents.len()];
    for (index, &parent) in parents.iter().enumerate() {
        if parent == -1 {
            top.push(index);
            continue;
        }
        let parent_index = parent as usize;
        if parent >= 0 && parent_index < parents.len() && parent_index != index {
            children[parent_index].push(index);
        }
    }

    let mut order = Vec::new();
    // (joint, its parent, how deep the recursion is there), next last.
    let mut pending: Vec<(usize, Option<usize>, usize)> = Vec::new();
    for &joint in top.iter().rev() {
        pending.push((joint, None, 1));
    }
    while let Some((joint, parent, depth)) = pending.pop() {
        if depth > MAX_DEPTH {
            return undefined("a joint hierarchy too deep for the stack");
        }
        order.push((joint, parent));
        for &child in children[joint].iter().rev() {
            pending.push((child, Some(joint), depth + 1));
        }
    }
    Ok(order)
}

/// `MakeDataUnique`: each face corner its own vertex, a vertex used again
/// copied to the next free place after the original list (which is
/// resized, even shrunk, to three per face), then each face's first and
/// last corners swapped. Only a face's first corner is checked against
/// the list, and copies made while vertices go unused run past its end.
fn make_data_unique(mesh: &mut MeshSource) -> Result<Vec<[u32; 3]>, ImportError> {
    let original_count = mesh.vertices.len();
    let mut used = vec![false; original_count];
    let new_count = (mesh.faces.len() * 3) as u32;
    let mut next_index = original_count as u32;
    mesh.resize_vertices(new_count as usize)?;

    // Room for as many weights per vertex as there are now.
    let weights_per_vertex = mesh.weights.len() as f32 / next_index as f32;
    let guess = weights_per_vertex * new_count as f32;
    if guess >= 4_294_967_296.0 {
        return undefined("a weight count guess past unsigned int");
    }
    let guess = guess as u32;
    let reserved = guess.wrapping_add(guess >> 3);
    mesh.weight_capacity.reserve(reserved as usize)?;

    let mut faces = Vec::with_capacity(mesh.faces.len());
    for face in &mesh.faces {
        let Some(mut corners) = *face else {
            return undefined("a triangle without corners");
        };
        for corner in 0..3 {
            if corners[0] as usize >= mesh.vertices.len() {
                return fail("MD5MESH: Invalid vertex index");
            }
            let index = corners[corner] as usize;
            let Some(&seen) = used.get(index) else {
                return undefined("a corner past the vertices");
            };
            if !seen {
                used[index] = true;
                continue;
            }
            let target = next_index as usize;
            if index >= mesh.vertices.len() || target >= mesh.vertices.len() {
                return undefined("a vertex copied past the list");
            }
            mesh.vertices[target] = mesh.vertices[index];
            corners[corner] = next_index;
            next_index += 1;
        }
        corners.swap(0, 2);
        faces.push(corners);
    }
    Ok(faces)
}

/// Whether a weight is skipped (within `WEIGHT_EPSILON` of zero).
fn is_negligible(weight: f32) -> bool {
    (-WEIGHT_EPSILON..WEIGHT_EPSILON).contains(&weight)
}

/// The shader's name with ".msh" (an `aiString` assigned a longer
/// `std::string` stays empty).
fn mesh_name(shader: &[u8]) -> String {
    let mut name = shader.to_vec();
    name.extend_from_slice(b".msh");
    if name.len() > MAX_LENGTH - 1 {
        return String::new();
    }
    String::from_utf8_lossy(&name).into_owned()
}

/// A mesh of `LoadMD5MeshFile`: its unique vertices with the texture's v
/// flipped, placed (when any weight counts) by the sum of their weights'
/// offsets turned and moved by their joints, each scaled by its weight.
/// A vertex whose weights sum to zero stays at the origin, as do all of
/// them when no weight counts.
fn build_mesh(source: &mut MeshSource, joints: &[Joint], material_index: u32) -> Result<Mesh, ImportError> {
    let faces = make_data_unique(source)?;
    let vertex_count = source.vertices.len();

    let mut uvs = Vec::with_capacity(vertex_count);
    for vertex in &source.vertices {
        let flipped = 1.0f32 - vertex.uv[1];
        uvs.push([vertex.uv[0], flipped, 0.0]);
    }

    // How many weights that count each joint has (its bone's weights).
    let mut counts = vec![0u32; joints.len()];
    for vertex in &source.vertices {
        let end = vertex.first_weight.wrapping_add(vertex.weight_count);
        for index in vertex.first_weight..end {
            let Some(weight) = source.weights.get(index as usize) else {
                return undefined("a vertex weight past the list");
            };
            if is_negligible(weight.weight) {
                continue;
            }
            let Some(count) = counts.get_mut(weight.joint as usize) else {
                return undefined("a weight's joint past the joints");
            };
            *count = count.wrapping_add(1);
        }
    }

    let mut positions = vec![[0.0f32; 3]; vertex_count];
    let has_bones = counts.iter().any(|&count| count != 0);
    if has_bones {
        let mut rotations = vec![Quaternion::IDENTITY; joints.len()];
        for (joint_index, joint) in joints.iter().enumerate() {
            let count = counts[joint_index];
            if count == 0 {
                continue;
            }
            check_allocation(count as u64, VERTEX_WEIGHT_SIZE)?;
            rotations[joint_index] = Quaternion::from_md5(joint.rotation);
        }

        // The ranges were all checked above.
        for (vertex, position) in source.vertices.iter().zip(&mut positions) {
            let end = vertex.first_weight.wrapping_add(vertex.weight_count);
            let mut sum = 0.0f32;
            for index in vertex.first_weight..end {
                sum += source.weights[index as usize].weight;
            }
            if sum == 0.0 {
                // "The sum of all vertex bone weights is 0".
                continue;
            }
            for index in vertex.first_weight..end {
                let weight = &source.weights[index as usize];
                if is_negligible(weight.weight) {
                    continue;
                }
                let joint_index = weight.joint as usize;
                let joint = &joints[joint_index];
                let turned = rotations[joint_index].rotate(weight.offset);
                let placed = [joint.position[0] + turned[0], joint.position[1] + turned[1], joint.position[2] + turned[2]];
                let weighted = scale(placed, weight.weight);
                position[0] += weighted[0];
                position[1] += weighted[1];
                position[2] += weighted[2];
            }
        }
    }

    let face_lists = faces.iter().map(|corners| corners.to_vec()).collect();
    let mut mesh = Mesh {
        name: mesh_name(&source.shader),
        primitive_types: PRIMITIVE_TRIANGLE,
        vertex_count,
        vertices: Some(positions),
        faces: Some(face_lists),
        material_index,
        ..Mesh::default()
    };
    mesh.texture_coords[0] = Some(uvs);
    mesh.uv_components[0] = 2;
    Ok(mesh)
}

/// `LoadMD5MeshFile` once the file is open: a mesh for each "mesh"
/// section with both faces and vertices.
fn read_mesh_file(bytes: &[u8]) -> Result<Vec<Mesh>, ImportError> {
    let parser = Parser::parse(bytes)?;
    let mut file = parse_mesh_file(&parser)?;

    // The joint hierarchy (which no mesh here uses) is built by recursion.
    let parents: Vec<i32> = file.joints.iter().map(|joint| joint.parent).collect();
    hierarchy_order(&parents)?;

    let mut meshes = Vec::new();
    for source in &mut file.meshes {
        if source.faces.is_empty() || source.vertices.is_empty() {
            continue;
        }
        let material_index = meshes.len() as u32;
        let mesh = build_mesh(source, &file.joints, material_index)?;
        meshes.push(mesh);
    }
    Ok(meshes)
}

/// `MD5::AnimBoneDesc`: "name" parent flags firstKey.
struct AnimatedBone {
    name: Vec<u8>,
    parent: i32,
    flags: u32,
    first_key: u32,
}

/// `MD5::BaseFrameDesc`.
#[derive(Clone, Copy, Default)]
struct BaseFrame {
    position: Vector3,
    rotation: Vector3,
}

/// `MD5AnimParser`, without what only the animation itself (its times
/// and rate) needs.
struct AnimationFile {
    bones: Vec<AnimatedBone>,
    base_frames: Vec<BaseFrame>,
    frames: Vec<Vec<f32>>,
}

fn read_animated_bone(cursor: &mut Cursor) -> Result<AnimatedBone, ImportError> {
    let name = cursor.quoted()?;
    cursor.skip_spaces()?;
    let parent = cursor.signed();
    cursor.skip_spaces()?;
    // (Flags above 63 only draw a warning.)
    let flags = cursor.unsigned();
    cursor.skip_spaces()?;
    let first_key = cursor.unsigned();
    Ok(AnimatedBone { name, parent, flags, first_key })
}

/// A "frame" block's numbers, line by line.
fn read_frame(parser: &Parser, section: &Section, component_count: u32) -> Result<Vec<f32>, ImportError> {
    let mut values = Vec::new();
    let mut value_capacity = VectorCapacity::new(VALUE_SIZE);
    if component_count != u32::MAX {
        value_capacity.reserve(component_count as usize)?;
    }
    for &start in &section.elements {
        let mut position = start;
        loop {
            let (next, more) = skip_spaces_and_line_end(&parser.buffer, position, parser.end);
            position = next;
            if !more {
                break;
            }
            let (value, next) = fast_atoreal_move(&parser.buffer, position, true)?;
            position = next;
            value_capacity.grow(values.len() + 1)?;
            values.push(value);
        }
    }
    Ok(values)
}

fn parse_animation_file(parser: &Parser) -> Result<AnimationFile, ImportError> {
    let mut bones = Vec::new();
    let mut bone_capacity = VectorCapacity::new(ANIMATED_BONE_SIZE);
    let mut base_frames = Vec::new();
    let mut base_frame_capacity = VectorCapacity::new(BASE_FRAME_SIZE);
    let mut frames = Vec::new();
    let mut frame_capacity = VectorCapacity::new(FRAME_SIZE);
    // Each frame's values are reserved for six per joint once a
    // "numJoints" is read.
    let mut component_count = u32::MAX;
    for section in &parser.sections {
        match section.name.as_slice() {
            b"hierarchy" => {
                for &start in &section.elements {
                    bone_capacity.grow(bones.len() + 1)?;
                    let mut cursor = parser.cursor(start);
                    let bone = read_animated_bone(&mut cursor)?;
                    bones.push(bone);
                }
            }
            b"baseframe" => {
                for &start in &section.elements {
                    base_frame_capacity.grow(base_frames.len() + 1)?;
                    let mut cursor = parser.cursor(start);
                    let position = cursor.triple()?.unwrap_or_default();
                    let rotation = cursor.triple()?.unwrap_or_default();
                    base_frames.push(BaseFrame { position, rotation });
                }
            }
            b"frame" => {
                // "A frame section must have a frame index".
                if section.value.is_empty() {
                    continue;
                }
                frame_capacity.grow(frames.len() + 1)?;
                let values = read_frame(parser, section, component_count)?;
                frames.push(values);
            }
            b"numFrames" => {
                let (count, _) = strtoul10(&section.value, 0);
                frame_capacity.reserve(count as usize)?;
            }
            b"numJoints" => {
                let (count, _) = strtoul10(&section.value, 0);
                bone_capacity.reserve(count as usize)?;
                if component_count == u32::MAX {
                    component_count = count.wrapping_mul(6);
                }
            }
            b"numAnimatedComponents" => {
                // It reserves bones, and leaves the component count.
                let (count, _) = strtoul10(&section.value, 0);
                bone_capacity.reserve(count as usize)?;
            }
            b"frameRate" => {
                fast_atoreal_move(&section.value, 0, true)?;
            }
            _ => {}
        }
    }
    Ok(AnimationFile { bones, base_frames, frames })
}

/// What an .md5anim gives here: its bones, and for each the first key
/// (position, rotation) a frame set.
struct Animation {
    bones: Vec<AnimatedBone>,
    first_keys: Vec<Option<(Vector3, Quaternion)>>,
}

/// `LoadMD5AnimFile` once the file is open: `None` (an error logged) when
/// it has no bones or frames, or base frames not matching the bones. A
/// bone's flags pick which of its position's, then its rotation's,
/// components a frame gives, from its first key on; the first frame is
/// read even when empty, later empty ones are skipped.
fn read_animation_file(bytes: &[u8]) -> Result<Option<Animation>, ImportError> {
    let parser = Parser::parse(bytes)?;
    let file = parse_animation_file(&parser)?;
    if file.bones.is_empty() || file.frames.is_empty() || file.base_frames.len() != file.bones.len() {
        return Ok(None);
    }
    check_allocation(file.frames.len() as u64, QUATERNION_KEY_SIZE)?;

    let mut first_keys: Vec<Option<(Vector3, Quaternion)>> = vec![None; file.bones.len()];
    for (frame_number, values) in file.frames.iter().enumerate() {
        if values.is_empty() && frame_number != 0 {
            continue;
        }
        for (bone_index, bone) in file.bones.iter().enumerate() {
            let first = bone.first_key as usize;
            if first >= values.len() {
                // Empty frames are allowed for bones not animated.
                if bone.flags != 0 {
                    return fail("MD5: Keyframe index is out of range");
                }
                continue;
            }
            let base = file.base_frames[bone_index];
            let mut components = values[first..].iter();
            let mut position = base.position;
            for (axis, component) in position.iter_mut().enumerate() {
                if bone.flags & (1 << axis) == 0 {
                    continue;
                }
                let Some(&value) = components.next() else {
                    return undefined("a keyframe read past its frame");
                };
                *component = value;
            }
            let mut rotation = base.rotation;
            for (axis, component) in rotation.iter_mut().enumerate() {
                if bone.flags & (8 << axis) == 0 {
                    continue;
                }
                let Some(&value) = components.next() else {
                    return undefined("a keyframe read past its frame");
                };
                *component = value;
            }
            if first_keys[bone_index].is_none() {
                let quaternion = Quaternion::from_md5(rotation);
                first_keys[bone_index] = Some((position, quaternion));
            }
        }
    }
    Ok(Some(Animation { bones: file.bones, first_keys }))
}

/// Without an .md5mesh: `AttachChilds_Anim` builds the hierarchy under a
/// root, each node placed by the first keys of the first channel of its
/// name (keys never set are zero and the identity), then
/// `SkeletonMeshBuilder` makes a mesh of the first top bone's tree.
fn skeleton_mesh(animation: &Animation) -> Result<Option<Mesh>, ImportError> {
    let parents: Vec<i32> = animation.bones.iter().map(|bone| bone.parent).collect();
    let order = hierarchy_order(&parents)?;

    let mut channel_of_name: HashMap<&[u8], usize> = HashMap::new();
    for (index, bone) in animation.bones.iter().enumerate() {
        channel_of_name.entry(bone.name.as_slice()).or_insert(index);
    }

    let root = Node::new(b"<MD5_Hierarchy>".to_vec(), Matrix4::IDENTITY, None);
    let mut nodes = vec![root];
    let mut node_of_bone = vec![0usize; animation.bones.len()];
    for (bone_index, parent) in order {
        let parent_node = match parent {
            Some(parent_bone) => node_of_bone[parent_bone],
            None => 0,
        };
        let name = &animation.bones[bone_index].name;
        let channel = channel_of_name[name.as_slice()];
        let default_key = ([0.0f32; 3], Quaternion::IDENTITY);
        let (position, rotation) = animation.first_keys[channel].unwrap_or(default_key);
        let translation = Matrix4::translation(position);
        let rotation_matrix = rotation.matrix();
        let transform = translation.product(&rotation_matrix);

        let node = nodes.len();
        nodes.push(Node::new(name.clone(), transform, Some(parent_node)));
        nodes[parent_node].children.push(node);
        node_of_bone[bone_index] = node;
    }

    let Some(&first_top) = nodes[0].children.first() else {
        return Ok(None);
    };
    let mesh = skeleton::build_mesh(&nodes, first_top, false);
    Ok(Some(mesh))
}

/// `LoadMD5CameraFile` once the file is open: the frames and cuts are
/// checked as the animations are made of them, and no mesh comes of it.
/// Cuts (each one past its number) split the frames into animations,
/// from 0 to the last frame; a cut past the frames reads past them.
fn read_camera_file(bytes: &[u8]) -> Result<(), ImportError> {
    let parser = Parser::parse(bytes)?;
    let mut frame_count = 0usize;
    let mut frame_capacity = VectorCapacity::new(CAMERA_FRAME_SIZE);
    let mut cuts: Vec<u32> = Vec::new();
    let mut cut_capacity = VectorCapacity::new(CUT_SIZE);
    for section in &parser.sections {
        match section.name.as_slice() {
            b"numFrames" => {
                let (count, _) = strtoul10(&section.value, 0);
                frame_capacity.reserve(count as usize)?;
            }
            b"frameRate" => {
                fast_atoreal_move(&section.value, 0, true)?;
            }
            b"numCuts" => {
                let (count, _) = strtoul10(&section.value, 0);
                cut_capacity.reserve(count as usize)?;
            }
            b"cuts" => {
                for &start in &section.elements {
                    let (cut, _) = strtoul10(&parser.buffer, start);
                    cut_capacity.grow(cuts.len() + 1)?;
                    cuts.push(cut.wrapping_add(1));
                }
            }
            b"camera" => {
                for &start in &section.elements {
                    frame_capacity.grow(frame_count + 1)?;
                    frame_count += 1;
                    let mut cursor = parser.cursor(start);
                    cursor.triple()?;
                    cursor.triple()?;
                    cursor.skip_spaces()?;
                    // The field of view.
                    cursor.real()?;
                }
            }
            _ => {}
        }
    }
    if frame_count == 0 {
        return fail("MD5CAMERA: No frames parsed");
    }

    let last_frame = frame_count - 1;
    if cuts.is_empty() {
        cut_capacity.grow(2)?;
        cuts.push(0);
        cuts.push(last_frame as u32);
    } else {
        cut_capacity.grow(cuts.len() + 1)?;
        cuts.insert(0, 0);
        let last_cut = cuts[cuts.len() - 1];
        if (last_cut as usize) < last_frame {
            cut_capacity.grow(cuts.len() + 1)?;
            cuts.push(last_frame as u32);
        }
    }
    check_allocation((cuts.len() - 1) as u64, POINTER_SIZE)?;
    for pair in cuts.windows(2) {
        let key_count = pair[1].wrapping_sub(pair[0]);
        check_allocation(key_count as u64, VECTOR_KEY_SIZE)?;
        check_allocation(key_count as u64, QUATERNION_KEY_SIZE)?;
        let past_last = pair[0] as u64 + key_count as u64;
        if key_count != 0 && past_last > frame_count as u64 {
            return undefined("a camera cut past the frames");
        }
    }
    Ok(())
}

/// `mIOHandler->Open(mFile + extension)`: the imported file itself when
/// that is its name; `None` when it can't be opened or is empty.
fn open_part(file_system: &FileSystem, input_path: &str, input_bytes: &[u8], name: &str) -> Option<Vec<u8>> {
    let bytes = if name == input_path { input_bytes.to_vec() } else { file_system.open(name)? };
    if bytes.is_empty() {
        return None;
    }
    Some(bytes)
}

/// `MD5Importer::InternReadFile`: by the extension, the camera file, the
/// animation file alone, or the mesh file and then the animation file,
/// each named as the imported file up to its last '.'.
pub fn read(path: &str, bytes: &[u8], files: &FileSystem) -> Result<Scene, ImportError> {
    let stem = match path.rfind('.') {
        Some(dot) => &path[..dot + 1],
        None => path,
    };
    let extension = get_extension(path);

    if extension == "md5camera" {
        let camera_path = format!("{stem}md5camera");
        let Some(camera) = open_part(files, path, bytes, &camera_path) else {
            return fail(format!("Failed to read MD5CAMERA file: {camera_path}"));
        };
        read_camera_file(&camera)?;
        return Ok(Scene::default());
    }

    let mut had_mesh = false;
    let mut meshes = Vec::new();
    if extension != "md5anim" {
        let mesh_path = format!("{stem}md5mesh");
        if let Some(mesh_file) = open_part(files, path, bytes, &mesh_path) {
            had_mesh = true;
            meshes = read_mesh_file(&mesh_file)?;
        }
    }

    let animation_path = format!("{stem}md5anim");
    let mut animation = None;
    if let Some(animation_file) = open_part(files, path, bytes, &animation_path) {
        animation = read_animation_file(&animation_file)?;
    }
    if !had_mesh
        && let Some(animation) = &animation
        && let Some(mesh) = skeleton_mesh(animation)?
    {
        meshes.push(mesh);
    }

    if !had_mesh && animation.is_none() {
        return fail("Failed to read valid contents out of this MD5* file");
    }
    Ok(Scene { meshes })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sections(text: &str) -> Vec<(String, String, usize)> {
        let parser = Parser::parse(text.as_bytes()).unwrap();
        let mut found = Vec::new();
        for section in &parser.sections {
            let name = String::from_utf8_lossy(&section.name).into_owned();
            let value = String::from_utf8_lossy(&section.value).into_owned();
            found.push((name, value, section.elements.len()));
        }
        found
    }

    #[test]
    fn values_swallow_the_names_after_them() {
        let text = "MD5Version 10\ncommandline \"x\"\n\nnumJoints 3\nnumMeshes 1\n\njoints {\n\"a\" -1 ( 0 0 0 ) ( 0 0 0 ) // c\n}\n\nframe 2 {\n 1 2\n}\n";
        let found = sections(text);
        assert_eq!(found[0], ("numJoints".to_string(), "1\n".to_string(), 0));
        assert_eq!(found[1], ("joints".to_string(), String::new(), 1));
        assert_eq!(found[2], ("frame".to_string(), "2 ".to_string(), 1));
    }

    #[test]
    fn a_command_line_ending_the_file_reads_past_it() {
        assert!(Parser::parse(b"MD5Version 10\nx").is_err());
        assert!(Parser::parse(b"MD5Version 10\nx\n").is_ok());
        assert!(Parser::parse(b"MD5Version 11\nx\n").is_err());
    }

    #[test]
    fn quotes_and_triples_read_on_past_their_line() {
        let text = b"MD5Version 10\ncommandline \"\"\n\njoints {\n\"a b\" 2 x( 1 2 3 ) ( 4 5 6 )\n\"unclosed -1\n( 7 8 9 ) ( 1 1 1 )\n}\n";
        let parser = Parser::parse(text).unwrap();
        let joints = &parser.sections[0];
        assert_eq!(joints.elements.len(), 3);

        // A missing '(' passes the byte after it too.
        let mut cursor = parser.cursor(joints.elements[0]);
        assert_eq!(cursor.quoted().unwrap(), b"a b");
        let mut cursor = parser.cursor(joints.elements[0]);
        let joint = read_joint(&mut cursor).unwrap();
        assert_eq!(joint.parent, 2);
        assert_eq!(joint.position, [1.0, 2.0, 3.0]);
        assert_eq!(joint.rotation, [4.0, 5.0, 6.0]);

        // Without its closing quote the name is empty, and the triples
        // come from the next line.
        let mut cursor = parser.cursor(joints.elements[1]);
        assert!(cursor.quoted().unwrap().is_empty());
        let mut cursor = parser.cursor(joints.elements[1]);
        let joint = read_joint(&mut cursor).unwrap();
        assert_eq!(joint.parent, 0);
        assert_eq!(joint.position, [7.0, 8.0, 9.0]);
        assert_eq!(joint.rotation, [1.0, 1.0, 1.0]);
    }

    const CUBE_CORNER: &str = "MD5Version 10\ncommandline \"\"\n\nnumJoints 1\nnumMeshes 1\n\njoints {\n\t\"root\" -1 ( 1 2 3 ) ( 0 0 0 )\n}\n\nmesh {\n\tshader \"skin\"\n\tnumverts 3\n\tvert 0 ( 0 0 ) 0 1\n\tvert 1 ( 1 0 ) 1 1\n\tvert 2 ( 0 1 ) 2 1\n\tnumtris 2\n\ttri 0 0 1 2\n\ttri 1 0 2 1\n\tnumweights 3\n\tweight 0 0 1 ( 1 0 0 )\n\tweight 1 0 1 ( 0 1 0 )\n\tweight 2 0 1 ( 0 0 1 )\n}\n";

    #[test]
    fn vertices_are_made_unique_and_placed_by_their_weights() {
        let meshes = read_mesh_file(CUBE_CORNER.as_bytes()).unwrap();
        assert_eq!(meshes.len(), 1);
        let mesh = &meshes[0];
        assert_eq!(mesh.name, "skin.msh");
        assert_eq!(mesh.faces, Some(vec![vec![2, 1, 0], vec![5, 4, 3]]));
        let corners = [[2.0, 2.0, 3.0], [1.0, 3.0, 3.0], [1.0, 2.0, 4.0]];
        let expected = vec![corners[0], corners[1], corners[2], corners[0], corners[2], corners[1]];
        assert_eq!(mesh.vertices, Some(expected));
        let uvs = mesh.texture_coords[0].as_ref().unwrap();
        assert_eq!(uvs[0], [0.0, 1.0, 0.0]);
        assert_eq!(uvs[2], [0.0, 0.0, 0.0]);
    }

    #[test]
    fn copies_past_the_list_fail() {
        // A fourth, unused vertex leaves no room for the second face's copies.
        let text = CUBE_CORNER.replace("numverts 3", "numverts 4");
        assert!(read_mesh_file(text.as_bytes()).is_err());
        // A face never given its corners has none to read.
        let text = CUBE_CORNER.replace("numtris 2", "numtris 3");
        assert!(read_mesh_file(text.as_bytes()).is_err());
        // Only the first corner is checked against the list.
        let text = CUBE_CORNER.replace("tri 0 0 1 2", "tri 0 7 1 2");
        assert_eq!(read_mesh_file(text.as_bytes()).unwrap_err().0, "MD5MESH: Invalid vertex index");
    }

    #[test]
    fn quaternions_turn_as_assimp_turns_them() {
        let identity = Quaternion::from_md5([0.0, 0.0, 0.0]);
        assert_eq!(identity.rotate([1.0, 2.0, 3.0]), [1.0, 2.0, 3.0]);
        let quarter = Quaternion::from_md5([0.0, 0.0, std::f32::consts::FRAC_1_SQRT_2]);
        let turned = quarter.rotate([1.0, 0.0, 0.0]);
        assert!((turned[0]).abs() < 1e-6 && (turned[1] + 1.0).abs() < 1e-6);
        // Too long a vector part leaves w at (negative) zero.
        assert_eq!(Quaternion::from_md5([1.0, 1.0, 0.0]).w.to_bits(), (-0.0f32).to_bits());
    }

    #[test]
    fn anim_files_alone_make_a_skeleton() {
        let text = "MD5Version 10\ncommandline \"\"\n\nnumFrames 1\n\nhierarchy {\n\t\"root\" -1 0 0\n\t\"arm\" 0 1 0\n}\n\nbaseframe {\n\t( 0 0 0 ) ( 0 0 0 )\n\t( 0 0 0 ) ( 0 0 0 )\n}\n\nframe 0 {\n\t2\n}\n";
        let animation = read_animation_file(text.as_bytes()).unwrap().unwrap();
        assert_eq!(animation.first_keys[1].unwrap().0, [2.0, 0.0, 0.0]);
        let mesh = skeleton_mesh(&animation).unwrap().unwrap();
        assert!(mesh.vertex_count > 0);

        // An animated bone needs its values in every frame read.
        let empty_frame = text.replace("frame 0 {\n\t2\n}", "frame 0 {\n}");
        assert!(read_animation_file(empty_frame.as_bytes()).is_err());
    }

    #[test]
    fn camera_cuts_past_the_frames_fail() {
        let text = "MD5Version 10\ncommandline \"\"\n\nnumFrames 2\n\ncuts {\n\t1\n}\n\ncamera {\n\t( 0 0 0 ) ( 0 0 0 ) 90\n\t( 0 0 0 ) ( 0 0 0 ) 90\n}\n";
        assert!(read_camera_file(text.as_bytes()).is_ok());
        let past = text.replace("\t1\n}", "\t5\n}");
        assert!(read_camera_file(past.as_bytes()).is_err());
    }
}
