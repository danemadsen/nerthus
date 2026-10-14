//! TrueSpace COB/SCN files (`COBImporter`), little-endian ASCII or binary:
//! a 32-byte header ("Caligari ", 'A' for ASCII at 15, 'L' at 16), then
//! chunks. Polygon meshes ("PolH"), groups, lights, cameras and bones
//! become nodes, linked by parent ids: a node's children are the nodes from
//! it on whose parent id is its id, the top ones those whose parent id is
//! 0. Each time the graph reaches a mesh with texture coordinates, it
//! becomes a mesh per face material (in material order), each face corner
//! its own vertex; every face is then reversed
//! (`FlipWindingOrderProcess`). Transforms, units, materials, lights and
//! cameras make nothing here, but reading them can fail.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::fast_atof::{fast_atoreal_move, strtol10, strtoul10};
use crate::line_splitter::LineSplitter;
use crate::msvc::VectorCapacity;
use crate::parsing::{at, skip_spaces};
use crate::scene::{ImportError, Mesh, Scene, check_allocation, fail};
use crate::stream_reader::StreamReader;
use crate::vector::Vector3;

/// `COB::NO_SIZE`: a chunk whose size isn't given.
const NO_SIZE: u32 = u32::MAX;

const HEADER_SIZE: usize = 32;

/// Node graphs deeper than this overflow the stack of `BuildNodes`, whose
/// frames each hold a kilobyte-long material name.
const MAX_DEPTH: usize = 512;

/// More node visits than this would allocate gigabytes of nodes.
const MAX_VISITS: u64 = 1 << 22;

// Element sizes (MSVC, x64).
const POSITION_SIZE: u64 = 12;
const UV_SIZE: u64 = 8;
const FACE_SIZE: u64 = 32;
const CORNER_SIZE: u64 = 8;

fn throw<T>(message: &str) -> Result<T, ImportError> {
    fail(format!("COB: {message}"))
}

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

/// `COB::ChunkInfo`.
#[derive(Clone, Copy, Debug)]
struct ChunkInfo {
    id: u32,
    parent_id: u32,
    version: u32,
    size: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Mesh,
    Group,
    Light,
    Camera,
    Bone,
}

/// `COB::Face`: its material and (position, texture coordinate) corners.
#[derive(Clone, Debug, Default)]
struct Face {
    material: u32,
    corners: Vec<(u32, u32)>,
}

/// `COB::Node` (and `COB::Mesh`'s geometry).
struct Node {
    kind: Kind,
    id: u32,
    parent_id: u32,
    positions: Vec<Vector3>,
    position_capacity: VectorCapacity,
    uvs: Vec<[f32; 2]>,
    uv_capacity: VectorCapacity,
    faces: Vec<Face>,
    face_capacity: VectorCapacity,
}

impl Node {
    fn new(kind: Kind, info: &ChunkInfo) -> Self {
        Self {
            kind,
            id: info.id,
            parent_id: info.parent_id,
            positions: Vec::new(),
            position_capacity: VectorCapacity::new(POSITION_SIZE),
            uvs: Vec::new(),
            uv_capacity: VectorCapacity::new(UV_SIZE),
            faces: Vec::new(),
            face_capacity: VectorCapacity::new(FACE_SIZE),
        }
    }

    fn resize_positions(&mut self, size: usize) -> Result<(), ImportError> {
        self.position_capacity.grow(size)?;
        if self.positions.is_empty() {
            self.positions = vec![[0.0; 3]; size];
        } else {
            self.positions.resize(size, [0.0; 3]);
        }
        Ok(())
    }

    fn resize_uvs(&mut self, size: usize) -> Result<(), ImportError> {
        self.uv_capacity.grow(size)?;
        if self.uvs.is_empty() {
            self.uvs = vec![[0.0; 2]; size];
        } else {
            self.uvs.resize(size, [0.0; 2]);
        }
        Ok(())
    }

    fn push_face(&mut self, face: Face) -> Result<(), ImportError> {
        self.face_capacity.grow(self.faces.len() + 1)?;
        self.faces.push(face);
        Ok(())
    }
}

/// `COB::Scene` as far as reading on depends on it: the nodes, and the
/// ids among them (a "Unit" chunk looks for its parent).
#[derive(Default)]
struct Chunks {
    nodes: Vec<Node>,
    ids: HashSet<u32>,
}

impl Chunks {
    fn push(&mut self, node: Node) {
        self.ids.insert(node.id);
        self.nodes.push(node);
    }
}

/// `fast_atof(&s)` on a line.
fn read_real(line: &[u8], position: &mut usize) -> Result<f32, ImportError> {
    let (value, next) = fast_atoreal_move(line, *position, true)?;
    *position = next;
    Ok(value)
}

/// `ReadChunkInfo_Ascii`: "Kind Va.bc Id n Parent n Size n" (the version
/// from the bytes 1, 3 and 4 into its token, whatever they are).
fn read_chunk_info_ascii(splitter: &LineSplitter) -> Result<ChunkInfo, ImportError> {
    let tokens = splitter.tokens::<8>()?;
    let line = splitter.current();
    let version_start = tokens[1];
    let digit = |offset: usize| at(line, version_start + offset) as i8 as i32 - b'0' as i32;
    let version = digit(1) * 100 + digit(3) * 10 + digit(4);
    let (id, _) = strtoul10(line, tokens[3]);
    let (parent_id, _) = strtoul10(line, tokens[5]);
    let (size, _) = strtol10(line, tokens[7]);
    Ok(ChunkInfo { id, parent_id, version: version as u32, size: size as u32 })
}

/// `UnsupportedChunk_Ascii`: without a size an error; else the stream
/// skips `size` bytes and the caller's next increment is swallowed. Its
/// loop then meets this same header line again and skips again, until
/// the stream ends exactly (the loop stops) or would be passed (an
/// error); a size of 0 never moves on.
fn unsupported_ascii(splitter: &mut LineSplitter, info: &ChunkInfo, name: &str) -> Result<(), ImportError> {
    if info.size == NO_SIZE {
        let message = format!("Encountered unsupported chunk: {name} [version: {}, size: {}]", info.version, info.size);
        return throw(&message);
    }
    let size = info.size as u64;
    splitter.inc_ptr(size)?;
    let remaining = splitter.remaining() as u64;
    if remaining != 0 {
        if size == 0 {
            return fail("endless loop: an unsupported chunk of size 0 read again and again");
        }
        if !remaining.is_multiple_of(size) {
            return fail("End of file or read limit was reached");
        }
        splitter.inc_ptr(remaining)?;
    }
    splitter.swallow_next_increment();
    Ok(())
}

/// `ReadBasicNodeInfo_Ascii`: lines up to "Transform" (a "Name" line
/// needs a second token), then up to four matrix rows; a row that ends
/// the stream is left unread.
fn read_basic_node_info_ascii(splitter: &mut LineSplitter) -> Result<(), ImportError> {
    while splitter.is_valid() {
        if splitter.match_start(b"Name") {
            splitter.token(1)?;
        } else if splitter.match_start(b"Transform") {
            for _ in 0..4 {
                splitter.next_line()?;
                if !splitter.is_valid() {
                    break;
                }
                let line = splitter.current();
                let end = line.len();
                let mut position = 0;
                for _ in 0..4 {
                    position = skip_spaces(line, position, end).0;
                    read_real(line, &mut position)?;
                }
            }
            return Ok(());
        }
        splitter.next_line()?;
    }
    Ok(())
}

/// `ReadFloat3Tuple_Ascii`: three numbers, each maybe after a ','.
fn read_float3_ascii(line: &[u8], position: &mut usize, bound: usize) -> Result<(), ImportError> {
    for _ in 0..3 {
        *position = skip_spaces(line, *position, bound).0;
        if at(line, *position) == b',' {
            *position += 1;
        }
        *position = skip_spaces(line, *position, bound).0;
        read_real(line, position)?;
    }
    Ok(())
}

/// `ReadMat1_Ascii`: "mat#", "shader:", "rgb" and "alpha" lines; only the
/// first two must be there.
fn read_mat1_ascii(splitter: &mut LineSplitter, info: &ChunkInfo) -> Result<(), ImportError> {
    if info.version > 8 {
        return unsupported_ascii(splitter, info, "Mat1");
    }
    splitter.next_line()?;
    if !splitter.match_start(b"mat# ") {
        return Ok(());
    }
    splitter.token(1)?;
    splitter.next_line()?;
    if !splitter.match_start(b"shader: ") {
        return Ok(());
    }
    splitter.token(1)?;

    splitter.next_line()?;
    let mut position = splitter.token(1)?;
    read_float3_ascii(splitter.current(), &mut position, splitter.end_bound())?;

    splitter.next_line()?;
    let tokens = splitter.tokens::<10>()?;
    for index in [1, 3, 5, 7, 9] {
        fast_atoreal_move(splitter.current(), tokens[index], true)?;
    }
    Ok(())
}

/// `ReadUnit_Ascii`: a "Units" line, read when the parent node exists.
fn read_unit_ascii(chunks: &Chunks, splitter: &mut LineSplitter, info: &ChunkInfo) -> Result<(), ImportError> {
    if info.version > 1 {
        return unsupported_ascii(splitter, info, "Unit");
    }
    splitter.next_line()?;
    if !splitter.match_start(b"Units ") {
        return Ok(());
    }
    if chunks.ids.contains(&info.parent_id) {
        splitter.token(1)?;
    }
    Ok(())
}

/// Past the label `ReadLght_Ascii` expects, whatever is there: reading
/// past the line's end reads what longer lines left in its buffer.
fn skip_label(line: &[u8], position: usize, length: usize) -> Result<usize, ImportError> {
    let after = position + length;
    if after > line.len() {
        return undefined("a light's label skipped past its line");
    }
    Ok(after)
}

/// `ReadLght_Ascii`: the node's lines; then (its kind looked for on the
/// line that reading stopped at) a "color" line: r g b, "cone angle" a,
/// "inner angle" a.
fn read_light_ascii(chunks: &mut Chunks, splitter: &mut LineSplitter, info: &ChunkInfo) -> Result<(), ImportError> {
    if info.version > 8 {
        return unsupported_ascii(splitter, info, "Lght");
    }
    chunks.push(Node::new(Kind::Light, info));
    splitter.next_line()?;
    read_basic_node_info_ascii(splitter)?;

    splitter.next_line()?;
    let line = splitter.current();
    let bound = splitter.end_bound();
    let mut position = splitter.token(1)?;
    read_float3_ascii(line, &mut position, bound)?;
    position = skip_spaces(line, position, bound).0;
    position = skip_label(line, position, 10)?;
    position = skip_spaces(line, position, bound).0;
    read_real(line, &mut position)?;
    position = skip_spaces(line, position, bound).0;
    position = skip_label(line, position, 11)?;
    position = skip_spaces(line, position, bound).0;
    read_real(line, &mut position)?;
    Ok(())
}

/// `ReadCame_Ascii`, `ReadBone_Ascii` and `ReadGrou_Ascii`: the node's
/// lines (a camera's then one more).
fn read_node_ascii(chunks: &mut Chunks, splitter: &mut LineSplitter, info: &ChunkInfo, kind: Kind) -> Result<(), ImportError> {
    let (name, newest) = match kind {
        Kind::Camera => ("Came", 2),
        Kind::Bone => ("Bone", 5),
        _ => ("Grou", 1),
    };
    if info.version > newest {
        return unsupported_ascii(splitter, info, name);
    }
    chunks.push(Node::new(kind, info));
    splitter.next_line()?;
    read_basic_node_info_ascii(splitter)?;
    if kind == Kind::Camera {
        splitter.next_line()?;
    }
    Ok(())
}

/// A "World Vertices n" or "Texture Vertices n" block: the list resized to
/// n, then a line per entry while lines remain (a line that ends the
/// stream is left unread).
fn read_vertex_block_ascii(splitter: &mut LineSplitter, node: &mut Node, components: usize) -> Result<(), ImportError> {
    let count_start = splitter.token(2)?;
    let (count, _) = strtoul10(splitter.current(), count_start);
    if components == 3 {
        node.resize_positions(count as usize)?;
    } else {
        node.resize_uvs(count as usize)?;
    }
    for row in 0..count as usize {
        splitter.next_line()?;
        if !splitter.is_valid() {
            break;
        }
        let line = splitter.current();
        let bound = splitter.end_bound();
        let mut position = 0;
        let mut values = [0.0f32; 3];
        for value in values.iter_mut().take(components) {
            position = skip_spaces(line, position, bound).0;
            *value = read_real(line, &mut position)?;
        }
        if components == 3 {
            node.positions[row] = values;
        } else {
            node.uvs[row] = [values[0], values[1]];
        }
    }
    Ok(())
}

/// A face's corner line: "<position,uv>" per corner.
fn read_corners_ascii(line: &[u8], bound: usize, count: usize) -> Result<Vec<(u32, u32)>, ImportError> {
    let mut corners = Vec::new();
    let mut position = 0;
    for _ in 0..count {
        let (next, more) = skip_spaces(line, position, bound);
        position = next;
        if !more {
            return throw("Expected EOL token in Face entry");
        }
        if at(line, position) != b'<' {
            return throw("Expected < token in Face entry");
        }
        let (corner_position, next) = strtoul10(line, position + 1);
        position = next;
        if at(line, position) != b',' {
            return throw("Expected , token in Face entry");
        }
        let (corner_uv, next) = strtoul10(line, position + 1);
        position = next;
        if at(line, position) != b'>' {
            return throw("Expected < token in Face entry");
        }
        position += 1;
        corners.push((corner_position, corner_uv));
    }
    Ok(corners)
}

/// A "Faces n" block: n entries while lines remain, each a "Hole" line
/// (skipped alone) or "Face verts n flags n mat n" and its corner line.
fn read_faces_ascii(splitter: &mut LineSplitter, node: &mut Node) -> Result<(), ImportError> {
    let count_start = splitter.token(1)?;
    let (count, _) = strtoul10(splitter.current(), count_start);
    node.face_capacity.reserve(count as usize)?;
    for _ in 0..count {
        splitter.next_line()?;
        if !splitter.is_valid() {
            break;
        }
        if splitter.match_start(b"Hole") {
            continue;
        }
        if !splitter.match_start(b"Face") {
            return throw("Expected Face line");
        }
        let corner_count_start = splitter.token(2)?;
        let (corner_count, _) = strtoul10(splitter.current(), corner_count_start);
        check_allocation(corner_count as u64, CORNER_SIZE)?;
        splitter.token(4)?;
        let material_start = splitter.token(6)?;
        let (material, _) = strtoul10(splitter.current(), material_start);

        splitter.next_line()?;
        let corners = read_corners_ascii(splitter.current(), splitter.end_bound(), corner_count as usize)?;
        node.push_face(Face { material, corners })?;
    }
    Ok(())
}

/// `ReadPolH_Ascii`: the node's lines, then vertex, texture coordinate and
/// face blocks in any order, up to a "DrawFlags" line (or the faces, up to
/// version 4).
fn read_polh_ascii(chunks: &mut Chunks, splitter: &mut LineSplitter, info: &ChunkInfo) -> Result<(), ImportError> {
    if info.version > 8 {
        return unsupported_ascii(splitter, info, "PolH");
    }
    let mut node = Node::new(Kind::Mesh, info);
    splitter.next_line()?;
    read_basic_node_info_ascii(splitter)?;

    while splitter.is_valid() {
        if splitter.match_start(b"World Vertices") {
            read_vertex_block_ascii(splitter, &mut node, 3)?;
        } else if splitter.match_start(b"Texture Vertices") {
            read_vertex_block_ascii(splitter, &mut node, 2)?;
        } else if splitter.match_start(b"Faces") {
            read_faces_ascii(splitter, &mut node)?;
            if info.version <= 4 {
                break;
            }
        } else if splitter.match_start(b"DrawFlags") {
            splitter.token(1)?;
            break;
        }
        splitter.next_line()?;
    }
    chunks.push(node);
    Ok(())
}

/// `ReadAsciiFile`: chunk header lines, until "END " (or the stream's end).
/// A chunk's reading leaves the splitter wherever it stopped, and the next
/// kind is looked for on that line before moving on.
fn read_ascii(chunks: &mut Chunks, bytes: &[u8]) -> Result<(), ImportError> {
    let mut splitter = LineSplitter::begin(bytes, true, true)?;
    while splitter.is_valid() {
        if splitter.match_start(b"PolH ") {
            let info = read_chunk_info_ascii(&splitter)?;
            read_polh_ascii(chunks, &mut splitter, &info)?;
        }
        if splitter.match_start(b"BitM ") {
            let info = read_chunk_info_ascii(&splitter)?;
            if info.version > 1 {
                unsupported_ascii(&mut splitter, &info, "BitM")?;
            } else {
                splitter.next_line()?;
                splitter.token(1)?;
            }
        }
        if splitter.match_start(b"Mat1 ") {
            let info = read_chunk_info_ascii(&splitter)?;
            read_mat1_ascii(&mut splitter, &info)?;
        }
        if splitter.match_start(b"Grou ") {
            let info = read_chunk_info_ascii(&splitter)?;
            read_node_ascii(chunks, &mut splitter, &info, Kind::Group)?;
        }
        if splitter.match_start(b"Lght ") {
            let info = read_chunk_info_ascii(&splitter)?;
            read_light_ascii(chunks, &mut splitter, &info)?;
        }
        if splitter.match_start(b"Came ") {
            let info = read_chunk_info_ascii(&splitter)?;
            read_node_ascii(chunks, &mut splitter, &info, Kind::Camera)?;
        }
        if splitter.match_start(b"Bone ") {
            let info = read_chunk_info_ascii(&splitter)?;
            read_node_ascii(chunks, &mut splitter, &info, Kind::Bone)?;
        }
        if splitter.match_start(b"Chan ") {
            let info = read_chunk_info_ascii(&splitter)?;
            if info.version > 8 {
                unsupported_ascii(&mut splitter, &info, "Chan")?;
            }
        }
        if splitter.match_start(b"Unit ") {
            let info = read_chunk_info_ascii(&splitter)?;
            read_unit_ascii(chunks, &mut splitter, &info)?;
        }
        if splitter.match_start(b"END ") {
            return Ok(());
        }
        splitter.next_line()?;
    }
    Ok(())
}

/// `UnsupportedChunk_Binary`: without a size an error, else skipped.
fn unsupported_binary(reader: &mut StreamReader, info: &ChunkInfo, name: &str) -> Result<(), ImportError> {
    if info.size == NO_SIZE {
        let message = format!("Encountered unsupported chunk: {name} [version: {}, size: {}]", info.version, info.size);
        return throw(&message);
    }
    reader.inc_ptr(info.size as i64)
}

/// `chunk_guard`'s destructor, once a chunk was read: on to `(int)size`
/// bytes from where the chunk's data started (back, for a negative size).
/// Past the read limit its `IncPtr` throws, and so does the fallback, out
/// of a destructor: the program ends.
fn end_chunk(reader: &mut StreamReader, info: &ChunkInfo, start: i32) -> Result<(), ImportError> {
    if info.size == NO_SIZE {
        return Ok(());
    }
    let size = info.size as i32;
    let rest = size.checked_sub(reader.current_pos());
    let Some(offset) = rest.and_then(|rest| rest.checked_add(start)) else {
        return undefined("a chunk's end overflowing int");
    };
    if reader.inc_ptr(offset as i64).is_err() {
        return fail("COB: a chunk ends past the file (an exception out of a destructor)");
    }
    Ok(())
}

/// `ReadString_Binary`: a 16-bit length (negative: `std::length_error`),
/// then the bytes.
fn read_string_binary(reader: &mut StreamReader) -> Result<(), ImportError> {
    let length = reader.get_i2()?;
    if length < 0 {
        return fail("string too long");
    }
    for _ in 0..length {
        reader.get_i1()?;
    }
    Ok(())
}

/// `ReadBasicNodeInfo_Binary`: the duplicate count, the name, local axes
/// and a 3×4 transform.
fn read_basic_node_info_binary(reader: &mut StreamReader) -> Result<(), ImportError> {
    reader.get_i2()?;
    read_string_binary(reader)?;
    reader.inc_ptr(48)?;
    for _ in 0..12 {
        reader.get_f4()?;
    }
    Ok(())
}

/// A count read as `int` into a `size_t` (sign-extended): a negative one
/// is too long for any vector.
fn vector_count(count: i64) -> Result<usize, ImportError> {
    if count < 0 {
        return fail("vector too long");
    }
    Ok(count as usize)
}

/// A list of `count` entries of `entry_size` bytes the stream must hold:
/// its reads would run past the end (the list itself is not made).
fn check_room(reader: &StreamReader, count: usize, entry_size: u64) -> Result<(), ImportError> {
    let needed = (count as u64).saturating_mul(entry_size);
    if needed > reader.remaining_size_to_limit() as u64 {
        return fail("End of file or stream limit was reached");
    }
    Ok(())
}

/// `ReadPolH_Binary`.
fn read_polh_binary(chunks: &mut Chunks, reader: &mut StreamReader, info: &ChunkInfo) -> Result<(), ImportError> {
    if info.version > 8 {
        return unsupported_binary(reader, info, "PolH");
    }
    let start = reader.current_pos();
    let mut node = Node::new(Kind::Mesh, info);
    read_basic_node_info_binary(reader)?;

    let position_count = vector_count(reader.get_i4()? as i64)?;
    node.position_capacity.grow(position_count)?;
    check_room(reader, position_count, POSITION_SIZE)?;
    node.resize_positions(position_count)?;
    for position in &mut node.positions {
        *position = [reader.get_f4()?, reader.get_f4()?, reader.get_f4()?];
    }
    let uv_count = vector_count(reader.get_i4()? as i64)?;
    node.uv_capacity.grow(uv_count)?;
    check_room(reader, uv_count, UV_SIZE)?;
    node.resize_uvs(uv_count)?;
    for uv in &mut node.uvs {
        *uv = [reader.get_f4()?, reader.get_f4()?];
    }

    let face_count = vector_count(reader.get_i4()? as i64)?;
    node.face_capacity.reserve(face_count)?;
    for _ in 0..face_count {
        let hole = reader.get_i1()? & 0x08 != 0;
        if hole {
            if node.faces.is_empty() {
                let message = format!("A hole is the first entity in the `PolH` chunk with id {}", info.id);
                return throw(&message);
            }
        } else {
            node.push_face(Face::default())?;
        }
        let corner_count = vector_count(reader.get_i2()? as i64)?;
        let Some(face) = node.faces.last_mut() else {
            return undefined("a face read before any");
        };
        check_allocation((face.corners.len() + corner_count) as u64, CORNER_SIZE)?;
        if !hole {
            face.material = reader.get_i2()? as i32 as u32;
        }
        let first_new = face.corners.len();
        for _ in 0..corner_count {
            let position = reader.get_i4()? as u32;
            let uv = reader.get_i4()? as u32;
            face.corners.push((position, uv));
        }
        if hole {
            face.corners[first_new..].reverse();
        }
    }
    if info.version > 4 {
        reader.get_i4()?;
    }
    if info.version > 5 && info.version < 8 {
        reader.get_i4()?;
    }
    chunks.push(node);
    end_chunk(reader, info, start)
}

/// `ReadBitM_Binary`: a skipped block, a word, and a skip by a signed
/// length.
fn read_bitm_binary(reader: &mut StreamReader, info: &ChunkInfo) -> Result<(), ImportError> {
    if info.version > 1 {
        return unsupported_binary(reader, info, "BitM");
    }
    let start = reader.current_pos();
    let length = reader.get_u4()?;
    reader.inc_ptr(length as i64)?;
    reader.get_i4()?;
    let skip = reader.get_i4()?;
    reader.inc_ptr(skip as i64)?;
    end_chunk(reader, info, start)
}

/// `ReadMat1_Binary`: the material's fields, then textures tagged "e:",
/// "t:" and "b:" in that order, and two bytes back.
fn read_mat1_binary(reader: &mut StreamReader, info: &ChunkInfo) -> Result<(), ImportError> {
    if info.version > 8 {
        return unsupported_binary(reader, info, "Mat1");
    }
    let start = reader.current_pos();
    reader.get_i2()?;
    reader.get_i1()?;
    reader.get_i1()?;
    reader.get_i1()?;
    for _ in 0..8 {
        reader.get_f4()?;
    }
    let mut tag = [reader.get_u1()?, reader.get_u1()?];
    if &tag == b"e:" {
        reader.get_i1()?;
        read_string_binary(reader)?;
        tag = [reader.get_u1()?, reader.get_u1()?];
    }
    if &tag == b"t:" {
        reader.get_i1()?;
        read_string_binary(reader)?;
        for _ in 0..4 {
            reader.get_f4()?;
        }
        tag = [reader.get_u1()?, reader.get_u1()?];
    }
    if &tag == b"b:" {
        reader.get_i1()?;
        read_string_binary(reader)?;
        for _ in 0..5 {
            reader.get_f4()?;
        }
    }
    reader.inc_ptr(-2)?;
    end_chunk(reader, info, start)
}

/// `ReadCame_Binary`, `ReadLght_Binary` and `ReadGrou_Binary`: the node's
/// info (a camera from version 2 then a word, 512 skipping 42 bytes).
fn read_node_binary(chunks: &mut Chunks, reader: &mut StreamReader, info: &ChunkInfo, kind: Kind) -> Result<(), ImportError> {
    let name = match kind {
        Kind::Camera => "Came",
        Kind::Light => "Lght",
        _ => "Grou",
    };
    if info.version > 2 {
        return unsupported_binary(reader, info, name);
    }
    let start = reader.current_pos();
    chunks.push(Node::new(kind, info));
    read_basic_node_info_binary(reader)?;
    if kind == Kind::Camera && info.version > 1 && reader.get_i2()? == 512 {
        reader.inc_ptr(42)?;
    }
    end_chunk(reader, info, start)
}

/// `ReadUnit_Binary`: a word, read when the parent node exists.
fn read_unit_binary(chunks: &Chunks, reader: &mut StreamReader, info: &ChunkInfo) -> Result<(), ImportError> {
    if info.version > 1 {
        return unsupported_binary(reader, info, "Unit");
    }
    let start = reader.current_pos();
    if chunks.ids.contains(&info.parent_id) {
        reader.get_i2()?;
    }
    end_chunk(reader, info, start)
}

/// `ReadBinaryFile`: chunks ("PolH", "BitM", "Grou", "Lght", "Came",
/// "Mat1", "Unit", "OLay", others skipped) until "END ". A chunk's end
/// set by a negative size can lead back to a chunk read before; with no
/// new node ids since (which a "Unit" chunk depends on) the same chunks
/// follow forever.
fn read_binary(chunks: &mut Chunks, reader: &mut StreamReader) -> Result<(), ImportError> {
    let mut visited: HashMap<i64, usize> = HashMap::new();
    loop {
        let id_count = chunks.ids.len();
        if visited.insert(reader.ptr(), id_count) == Some(id_count) {
            return fail("endless loop: the same chunks read again and again");
        }
        let mut kind = [0u8; 4];
        for byte in &mut kind {
            *byte = reader.get_u1()?;
        }
        let major = reader.get_i2()? as i32;
        let minor = reader.get_i2()? as i32;
        let id = reader.get_i4()? as u32;
        let parent_id = reader.get_i4()? as u32;
        let size = reader.get_i4()? as u32;
        let info = ChunkInfo { id, parent_id, version: (major * 10 + minor) as u32, size };

        match &kind {
            b"PolH" => read_polh_binary(chunks, reader, &info)?,
            b"BitM" => read_bitm_binary(reader, &info)?,
            b"Grou" => read_node_binary(chunks, reader, &info, Kind::Group)?,
            b"Lght" => read_node_binary(chunks, reader, &info, Kind::Light)?,
            b"Came" => read_node_binary(chunks, reader, &info, Kind::Camera)?,
            b"Mat1" => read_mat1_binary(reader, &info)?,
            b"Unit" => read_unit_binary(chunks, reader, &info)?,
            b"OLay" => unsupported_binary(reader, &info, "OLay")?,
            b"END " => return Ok(()),
            _ => {
                let name = String::from_utf8_lossy(&kind).into_owned();
                unsupported_binary(reader, &info, &name)?;
            }
        }
    }
}

/// The meshes `BuildNodes` makes of one visit of a node with positions and
/// texture coordinates: one per face material holding corners, each
/// corner checked against the lists, faces reversed.
fn node_meshes(node: &Node, groups: &BTreeMap<u32, Vec<usize>>, meshes: &mut Vec<Mesh>, capacity: usize) -> Result<(), ImportError> {
    for face_indices in groups.values() {
        let corner_total: usize = face_indices.iter().map(|&face| node.faces[face].corners.len()).sum();
        if corner_total == 0 {
            continue;
        }
        if meshes.len() >= capacity {
            return undefined("a mesh past the scene's list (a node reached twice)");
        }
        check_allocation(corner_total as u64, POSITION_SIZE)?;

        let mut vertices = Vec::with_capacity(corner_total);
        let mut uvs = Vec::with_capacity(corner_total);
        let mut faces = Vec::new();
        for &face_index in face_indices {
            let face = &node.faces[face_index];
            if face.corners.is_empty() {
                continue;
            }
            let mut indices = Vec::with_capacity(face.corners.len());
            for &(position, uv) in &face.corners {
                let Some(&vertex) = node.positions.get(position as usize) else {
                    return throw("Position index out of range");
                };
                let Some(&coordinate) = node.uvs.get(uv as usize) else {
                    return throw("UV index out of range");
                };
                indices.push(vertices.len() as u32);
                vertices.push(vertex);
                uvs.push([coordinate[0], coordinate[1], 0.0]);
            }
            indices.reverse();
            faces.push(indices);
        }

        let mut mesh = Mesh {
            vertex_count: vertices.len(),
            vertices: Some(vertices),
            faces: Some(faces),
            material_index: meshes.len() as u32,
            ..Mesh::default()
        };
        mesh.texture_coords[0] = Some(uvs);
        meshes.push(mesh);
    }
    Ok(())
}

/// `InternReadFile` from the nodes on: the meshes of `BuildNodes`, depth
/// first from the top nodes. The scene's lists were sized for one visit of
/// each node (a mesh's materials, each light and camera), so a node
/// reached twice writes past them.
fn build_meshes(chunks: &Chunks) -> Result<Scene, ImportError> {
    let nodes = &chunks.nodes;
    let mut groups: Vec<BTreeMap<u32, Vec<usize>>> = Vec::with_capacity(nodes.len());
    for node in nodes {
        let mut by_material: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        for (face_index, face) in node.faces.iter().enumerate() {
            by_material.entry(face.material).or_default().push(face_index);
        }
        groups.push(by_material);
    }

    let has_geometry = |node: &Node| node.kind == Kind::Mesh && !node.positions.is_empty() && !node.uvs.is_empty();
    let mut mesh_capacity = 0usize;
    let mut light_capacity = 0usize;
    let mut camera_capacity = 0usize;
    // The nodes by parent id, in order.
    let mut by_parent: HashMap<u32, Vec<usize>> = HashMap::new();
    for (index, node) in nodes.iter().enumerate() {
        if has_geometry(node) {
            mesh_capacity += groups[index].len();
        }
        match node.kind {
            Kind::Light => light_capacity += 1,
            Kind::Camera => camera_capacity += 1,
            _ => {}
        }
        by_parent.entry(node.parent_id).or_default().push(index);
    }

    let mut meshes = Vec::new();
    let mut lights = 0usize;
    let mut cameras = 0usize;
    let mut visits = 0u64;
    // (node, how deep the recursion is there), the next last.
    let mut pending: Vec<(usize, usize)> = Vec::new();
    if let Some(top) = by_parent.get(&0) {
        for &index in top.iter().rev() {
            pending.push((index, 1));
        }
    }
    while let Some((index, depth)) = pending.pop() {
        visits += 1;
        if visits > MAX_VISITS {
            return fail("Internal error");
        }
        if depth > MAX_DEPTH {
            return undefined("a node graph too deep for the stack");
        }
        let node = &nodes[index];
        if has_geometry(node) {
            node_meshes(node, &groups[index], &mut meshes, mesh_capacity)?;
        }
        if node.kind == Kind::Light {
            if lights >= light_capacity {
                return undefined("a light past the scene's list (a node reached twice)");
            }
            lights += 1;
        }
        if node.kind == Kind::Camera {
            if cameras >= camera_capacity {
                return undefined("a camera past the scene's list (a node reached twice)");
            }
            cameras += 1;
        }
        // Its children: the nodes from it on whose parent id is its id.
        let Some(candidates) = by_parent.get(&node.id) else {
            continue;
        };
        let first = candidates.partition_point(|&candidate| candidate < index);
        for &child in candidates[first..].iter().rev() {
            pending.push((child, depth + 1));
        }
    }
    Ok(Scene { meshes })
}

/// `COBImporter::InternReadFile`.
pub fn read(bytes: &[u8]) -> Result<Scene, ImportError> {
    let mut reader = StreamReader::new(bytes, false)?;
    let head = reader.copy_and_advance(HEADER_SIZE)?;
    if !head.starts_with(b"Caligari ") {
        return throw("Could not found magic id: `Caligari`");
    }
    if head[16] != b'L' {
        return throw("File is big-endian, which is not supported");
    }

    let mut chunks = Chunks::default();
    if head[15] == b'A' {
        read_ascii(&mut chunks, &bytes[HEADER_SIZE..])?;
    } else {
        read_binary(&mut chunks, &mut reader)?;
    }
    if chunks.nodes.is_empty() {
        return throw("No nodes loaded");
    }
    build_meshes(&chunks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(kind: u8) -> Vec<u8> {
        let mut head = b"Caligari V00.01".to_vec();
        head.push(kind);
        head.extend_from_slice(b"LH");
        head.resize(31, b' ');
        head.push(b'\n');
        head
    }

    const TRIANGLE: &str = "PolH V0.06 Id 1 Parent 0 Size 100\nName tri\nTransform\n1 0 0 0\n0 1 0 0\n0 0 1 0\n0 0 0 1\nWorld Vertices 3\n0 0 0\n1 0 0\n0 1 0\nTexture Vertices 1\n0 0\nFaces 1\nFace verts 3 flags 0 mat 0\n<0,0> <1,0> <2,0>\nDrawFlags 0\nEND V1.00 Id 0 Parent 0 Size 0\n";

    #[test]
    fn ascii_faces_are_reversed_per_corner() {
        let mut file = header(b'A');
        file.extend_from_slice(TRIANGLE.as_bytes());
        let scene = read(&file).unwrap();
        assert_eq!(scene.meshes.len(), 1);
        let mesh = &scene.meshes[0];
        assert_eq!(mesh.vertices, Some(vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]));
        assert_eq!(mesh.faces, Some(vec![vec![2, 1, 0]]));
    }

    #[test]
    fn ascii_quirks() {
        // Without texture coordinates there is no mesh.
        let mut file = header(b'A');
        file.extend_from_slice(TRIANGLE.replace("Texture Vertices 1\n0 0\n", "").as_bytes());
        assert!(read(&file).unwrap().meshes.is_empty());
        // A corner past the positions.
        let mut file = header(b'A');
        file.extend_from_slice(TRIANGLE.replace("<2,0>", "<3,0>").as_bytes());
        assert_eq!(read(&file).unwrap_err().0, "COB: Position index out of range");
        // A node its own child recurses without end.
        let mut file = header(b'A');
        let looped = TRIANGLE.replace("Id 1 Parent 0", "Id 0 Parent 0");
        file.extend_from_slice(looped.as_bytes());
        assert!(read(&file).is_err());
        // An unsupported chunk of size 0 is met again and again.
        let mut file = header(b'A');
        file.extend_from_slice(b"Chan V0.12 Id 5 Parent 0 Size 0\nx\n");
        assert!(read(&file).is_err());
        // Ending exactly at the stream's end, it stops the reading.
        let mut file = header(b'A');
        let without_end = TRIANGLE.replace("END V1.00 Id 0 Parent 0 Size 0\n", "");
        file.extend_from_slice(without_end.as_bytes());
        file.extend_from_slice(b"Chan V0.12 Id 5 Parent 0 Size 3\nab\nab\n");
        assert_eq!(read(&file).unwrap().meshes.len(), 1);
    }

    fn binary_triangle(size_adjust: i32) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&0i16.to_le_bytes());
        body.extend_from_slice(&3i16.to_le_bytes());
        body.extend_from_slice(b"tri");
        body.extend_from_slice(&[0u8; 48]);
        for value in [1.0f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            body.extend_from_slice(&value.to_le_bytes());
        }
        body.extend_from_slice(&3i32.to_le_bytes());
        for value in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            body.extend_from_slice(&value.to_le_bytes());
        }
        body.extend_from_slice(&1i32.to_le_bytes());
        body.extend_from_slice(&[0u8; 8]);
        body.extend_from_slice(&1i32.to_le_bytes());
        body.push(0);
        body.extend_from_slice(&3i16.to_le_bytes());
        body.extend_from_slice(&0i16.to_le_bytes());
        for corner in 0..3i32 {
            body.extend_from_slice(&corner.to_le_bytes());
            body.extend_from_slice(&0i32.to_le_bytes());
        }
        body.extend_from_slice(&0i32.to_le_bytes());
        body.extend_from_slice(&0i32.to_le_bytes());

        let mut file = header(b'B');
        file.extend_from_slice(b"PolH");
        file.extend_from_slice(&0i16.to_le_bytes());
        file.extend_from_slice(&6i16.to_le_bytes());
        file.extend_from_slice(&1i32.to_le_bytes());
        file.extend_from_slice(&0i32.to_le_bytes());
        let size = body.len() as i32 + size_adjust;
        file.extend_from_slice(&size.to_le_bytes());
        file.extend_from_slice(&body);
        file.extend_from_slice(b"END ");
        file.extend_from_slice(&[0u8; 16]);
        file
    }

    #[test]
    fn binary_chunks_end_by_their_size() {
        let scene = read(&binary_triangle(0)).unwrap();
        assert_eq!(scene.meshes[0].faces, Some(vec![vec![2, 1, 0]]));
        // A size short of the data leads back into it.
        assert!(read(&binary_triangle(-4)).is_err());
        // A size back to the chunk's own start reads it forever.
        let looped = binary_triangle(-(binary_triangle(0).len() as i32 - 32 - 20 - 20) - 20);
        assert!(read(&looped).is_err());
    }
}
