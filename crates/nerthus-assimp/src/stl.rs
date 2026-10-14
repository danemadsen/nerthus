//! Assimp's STL importer (`STLLoader.cpp`). The file goes through
//! `TextFileToBuffer` whatever its kind. A file whose size is exactly 84
//! bytes plus 50 per facet is binary: one mesh, a normal per facet, and
//! per-facet colours from the attribute word (VisCAM order, or Materialise
//! order with a `COLOR=` header). Otherwise it must read as ASCII: one
//! mesh per `solid`, a normal per `facet normal`, faces of three
//! consecutive vertices.

use crate::fast_atof::fast_atoreal_move;
use crate::parsing::{at, is_line_end, is_space_or_newline, skip_spaces, skip_spaces_and_line_end, starts_with_at, text_file_to_buffer};
use crate::scene::{ImportError, Mesh, Scene, fail};

/// `AI_MAXLEN`.
const MAX_NAME_LENGTH: usize = 1024;

/// `IsBinarySTL` on the buffer from `start`: at least the header, and the
/// size the facet count gives (in 32-bit arithmetic).
fn is_binary(buffer: &[u8], start: usize, size: usize) -> bool {
    if size < 84 {
        return false;
    }
    let count_bytes: [u8; 4] = std::array::from_fn(|offset| at(buffer, start + 80 + offset));
    let face_count = u32::from_le_bytes(count_bytes);
    let expected = face_count.wrapping_mul(50).wrapping_add(84);
    expected as usize == size
}

/// `IsAsciiSTL`: not binary, and "solid" after any leading blanks with
/// more after it. (Its check for bytes above 127 compares signed chars,
/// so it never fires.)
fn is_ascii(buffer: &[u8], start: usize, size: usize) -> bool {
    if is_binary(buffer, start, size) {
        return false;
    }
    let end = start + size;
    let (position, line_goes_on) = skip_spaces(buffer, start, end);
    if !line_goes_on {
        return false;
    }
    if position + 5 >= end {
        return false;
    }
    starts_with_at(buffer, position, b"solid")
}

/// Faces of three consecutive vertices (`addFacesToMesh`).
fn sequential_faces(face_count: usize) -> Vec<Vec<u32>> {
    (0..face_count).map(|face| (0..3).map(|corner| (face * 3 + corner) as u32).collect()).collect()
}

/// Three numbers parsed with `fast_atoreal_move`, blanks skipped between.
fn read_vector(buffer: &[u8], mut position: usize, end: usize) -> Result<([f32; 3], usize), ImportError> {
    let mut vector = [0.0f32; 3];
    for (index, component) in vector.iter_mut().enumerate() {
        if index > 0 {
            position = skip_spaces(buffer, position, end).0;
        }
        let (value, next) = fast_atoreal_move(buffer, position, true)?;
        *component = value;
        position = next;
    }
    Ok((vector, position))
}

/// `LoadASCIIFile`.
fn load_ascii(buffer: &[u8], file_size: usize) -> Result<Vec<Mesh>, ImportError> {
    let end = file_size;
    let mut meshes = Vec::new();
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut position = 0usize;
    while is_ascii(buffer, position, end.saturating_sub(position)) {
        let mut mesh = Mesh::default();
        position = skip_spaces(buffer, position, end).0;
        position += 5;
        position = skip_spaces(buffer, position, end).0;
        let name_start = position;
        while !is_space_or_newline(at(buffer, position)) {
            position += 1;
        }
        let name_length = position - name_start;
        if name_length > 0 {
            if name_length >= MAX_NAME_LENGTH {
                return fail("STL: Node name too long");
            }
            mesh.name = String::from_utf8_lossy(&buffer[name_start..position]).to_string();
        }

        let mut face_vertex_counter = 3u32;
        loop {
            let (next, goes_on) = skip_spaces_and_line_end(buffer, position, end);
            position = next;
            if !goes_on {
                break;
            }
            let after_facet = at(buffer, position + 5);
            let after_vertex = at(buffer, position + 6);
            if starts_with_at(buffer, position, b"facet") && is_space_or_newline(after_facet) && after_facet != 0 {
                face_vertex_counter = 0;
                position += 6;
                position = skip_spaces(buffer, position, end).0;
                if starts_with_at(buffer, position, b"normal") {
                    if at(buffer, position + 6) == 0 {
                        return fail("STL: unexpected EOF while parsing facet");
                    }
                    position += 7;
                    position = skip_spaces(buffer, position, end).0;
                    let (normal, next) = read_vector(buffer, position, end)?;
                    position = next;
                    normals.extend([normal; 3]);
                }
            } else if starts_with_at(buffer, position, b"vertex") && is_space_or_newline(after_vertex) {
                if face_vertex_counter >= 3 {
                    position += 1;
                } else {
                    if after_vertex == 0 {
                        return fail("STL: unexpected EOF while parsing facet");
                    }
                    position += 7;
                    position = skip_spaces(buffer, position, end).0;
                    let (vertex, next) = read_vector(buffer, position, end)?;
                    position = next;
                    positions.push(vertex);
                    face_vertex_counter += 1;
                }
            } else if starts_with_at(buffer, position, b"endsolid") {
                loop {
                    position += 1;
                    if is_line_end(at(buffer, position)) {
                        break;
                    }
                }
                position = skip_spaces_and_line_end(buffer, position, end).0;
                break;
            } else {
                loop {
                    position += 1;
                    if is_space_or_newline(at(buffer, position)) {
                        break;
                    }
                }
            }
        }

        if !positions.len().is_multiple_of(3) {
            return fail("STL: Invalid number of vertices");
        }
        if normals.len() != positions.len() {
            return fail("Normal buffer size does not match position buffer size");
        }
        let face_count = positions.len() / 3;
        mesh.vertex_count = positions.len();
        if !positions.is_empty() {
            mesh.vertices = Some(std::mem::take(&mut positions));
        }
        if !normals.is_empty() {
            mesh.normals = Some(std::mem::take(&mut normals));
        }
        mesh.faces = Some(sequential_faces(face_count));
        meshes.push(mesh);
    }
    Ok(meshes)
}

/// Three little-endian floats at `offset`.
fn vector_at(buffer: &[u8], offset: usize) -> [f32; 3] {
    std::array::from_fn(|component| {
        let bytes: [u8; 4] = std::array::from_fn(|byte| at(buffer, offset + component * 4 + byte));
        f32::from_le_bytes(bytes)
    })
}

/// `LoadBinaryFile`.
fn load_binary(buffer: &[u8], file_size: usize) -> Result<Mesh, ImportError> {
    if file_size < 84 {
        return fail("STL: file is too small for the header");
    }
    // "COLOR=" in the header, matched as the C `&&` chain does: each byte
    // compared moves the cursor, a mismatch included.
    let mut materialise = false;
    let mut default_colour = [0.6f32; 4];
    let mut cursor = 0usize;
    while cursor < 80 {
        let mut matched = true;
        for &expected in b"COLOR=" {
            let byte = at(buffer, cursor);
            cursor += 1;
            if byte != expected {
                matched = false;
                break;
            }
        }
        if matched {
            materialise = true;
            let inverse_byte = 1.0f32 / 255.0f32;
            for component in &mut default_colour {
                *component = at(buffer, cursor) as f32 * inverse_byte;
                cursor += 1;
            }
            break;
        }
    }

    let count_bytes: [u8; 4] = std::array::from_fn(|offset| at(buffer, 80 + offset));
    let face_count = u32::from_le_bytes(count_bytes);
    if (file_size as u64) < 84 + face_count as u64 * 50 {
        return fail("STL: file is too small to hold all facets");
    }
    if face_count == 0 {
        return fail("STL: file is empty. There are no facets defined");
    }
    let vertex_count = face_count as usize * 3;
    let mut vertices = Vec::with_capacity(vertex_count);
    let mut normals = Vec::with_capacity(vertex_count);
    let mut colours: Option<Vec<[f32; 4]>> = None;
    let inverse_five_bits = 1.0f32 / 31.0f32;
    for face in 0..face_count as usize {
        let offset = 84 + face * 50;
        let normal = vector_at(buffer, offset);
        normals.extend([normal; 3]);
        for corner in 0..3 {
            vertices.push(vector_at(buffer, offset + 12 + corner * 12));
        }
        let attribute = u16::from_le_bytes([at(buffer, offset + 48), at(buffer, offset + 49)]) as u32;
        if attribute & 0x8000 == 0 {
            continue;
        }
        let colours = colours.get_or_insert_with(|| vec![default_colour; vertex_count]);
        let low = (attribute & 0x1f) as f32 * inverse_five_bits;
        let middle = ((attribute & (0x1f << 5)) >> 5) as f32 * inverse_five_bits;
        let high = ((attribute & (0x1f << 10)) >> 10) as f32 * inverse_five_bits;
        let colour = if materialise { [low, middle, high, 1.0] } else { [high, middle, low, 1.0] };
        colours[face * 3..face * 3 + 3].fill(colour);
    }
    let mut mesh = Mesh { vertex_count, vertices: Some(vertices), normals: Some(normals), faces: Some(sequential_faces(face_count as usize)), ..Mesh::default() };
    mesh.colors[0] = colours;
    Ok(mesh)
}

/// `STLImporter::InternReadFile`.
pub fn read(bytes: &[u8]) -> Result<Scene, ImportError> {
    let file_size = bytes.len();
    let buffer = text_file_to_buffer(bytes, false)?;
    if is_binary(&buffer, 0, file_size) {
        let mesh = load_binary(&buffer, file_size)?;
        return Ok(Scene { meshes: vec![mesh] });
    }
    if is_ascii(&buffer, 0, file_size) {
        let meshes = load_ascii(&buffer, file_size)?;
        return Ok(Scene { meshes });
    }
    fail("Failed to determine STL storage representation")
}
