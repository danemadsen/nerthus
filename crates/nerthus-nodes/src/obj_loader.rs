//! The `.obj` loader: the native `MeshPort_Load` reads Wavefront OBJ
//! files with tinyobjloader (v2.0.0rc11–rc13, built-in ear clipping,
//! `triangulate` on, the material base directory empty) instead of Assimp,
//! then keeps every `v` position as (x, z, y) and every triangle.
//!
//! Only what decides the geometry is ported: the number parser, the face,
//! line and point index rules (a bad index fails the whole load), the
//! points where faces are exported to shapes (which vertices exist then),
//! the quad split and the polygon triangulation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::mesh_rasterizer::Mesh;

/// `MeshPort_Load` for an `.obj` file: the mesh, or an empty one when
/// tinyobjloader fails (nothing renders then). `None` when the file
/// cannot be read.
pub fn load_obj(path: &Path) -> Option<Mesh> {
    let bytes = std::fs::read(path).ok()?;
    let mesh = match parse_obj(&bytes) {
        Some(model) => mesh_from_model(&model),
        None => Mesh::default(),
    };
    Some(mesh)
}

/// What tinyobjloader returns that the loader reads.
pub struct ObjModel {
    /// `attrib.vertices`: x, y, z per `v` line.
    pub positions: Vec<f32>,
    /// Every face's `vertex_index` triples after triangulation, in shape
    /// order.
    pub triangles: Vec<[i32; 3]>,
}

/// Positions as (x, z, y), and the triangles.
pub fn mesh_from_model(model: &ObjModel) -> Mesh {
    let vertex_count = model.positions.len() / 3;
    let mut vertices = Vec::with_capacity(vertex_count);
    for index in 0..vertex_count {
        let x = model.positions[3 * index];
        let y = model.positions[3 * index + 1];
        let z = model.positions[3 * index + 2];
        vertices.push([x, z, y]);
    }
    let mut triangles = Vec::with_capacity(model.triangles.len());
    for triangle in &model.triangles {
        // Indices past the vertex array only come from malformed files,
        // which are skipped here.
        let in_range = triangle.iter().all(|&index| index >= 0 && (index as usize) < vertex_count);
        if in_range {
            triangles.push([triangle[0] as u32, triangle[1] as u32, triangle[2] as u32]);
        }
    }
    Mesh { vertices, triangles, normals: Vec::new() }
}

/// The lines of the file as `safeGetline` splits them (`\n`, `\r\n` or a
/// lone `\r`), each cut at its first NUL as `c_str()` would.
fn lines(bytes: &[u8]) -> Vec<&[u8]> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'\n' || byte == b'\r' {
            result.push(&bytes[start..index]);
            let crlf = byte == b'\r' && index + 1 < bytes.len() && bytes[index + 1] == b'\n';
            index += if crlf { 2 } else { 1 };
            start = index;
            continue;
        }
        index += 1;
    }
    if start < bytes.len() {
        result.push(&bytes[start..]);
    }
    result
        .into_iter()
        .map(|line| match line.iter().position(|&byte| byte == 0) {
            Some(end) => &line[..end],
            None => line,
        })
        .collect()
}

fn is_space(byte: u8) -> bool {
    byte == b' ' || byte == b'\t'
}

fn is_new_line(byte: u8) -> bool {
    byte == b'\r' || byte == b'\n' || byte == 0
}

/// A NUL-terminated view of one line, as tinyobjloader's `const char *`.
struct Cursor<'a> {
    text: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    fn at(&self, offset: usize) -> u8 {
        self.text.get(self.position + offset).copied().unwrap_or(0)
    }

    fn rest(&self) -> &'a [u8] {
        &self.text[self.position.min(self.text.len())..]
    }

    fn advance(&mut self, count: usize) {
        self.position += count;
    }

    /// `strspn(token, set)`.
    fn span(&self, set: &[u8]) -> usize {
        self.rest().iter().take_while(|byte| set.contains(byte)).count()
    }

    /// `strcspn(token, set)`.
    fn complement_span(&self, set: &[u8]) -> usize {
        self.rest().iter().take_while(|byte| !set.contains(byte)).count()
    }

    fn starts_with(&self, prefix: &[u8]) -> bool {
        self.rest().starts_with(prefix)
    }
}

/// tinyobjloader's `tryParseDouble` over `[start, end)`.
fn try_parse_double(text: &[u8]) -> Option<f64> {
    if text.is_empty() {
        return None;
    }
    let is_digit = |byte: u8| byte.is_ascii_digit();
    let mut mantissa = 0.0f64;
    let mut exponent = 0i32;
    let mut negative = false;
    let mut current = 0usize;
    let mut leading_decimal_dot = false;

    let first = text[0];
    if first == b'+' || first == b'-' {
        negative = first == b'-';
        current += 1;
        if current < text.len() && text[current] == b'.' {
            leading_decimal_dot = true;
        }
    } else if is_digit(first) {
    } else if first == b'.' {
        leading_decimal_dot = true;
    } else {
        return None;
    }

    let mut end_not_reached = current < text.len();
    if !leading_decimal_dot {
        let mut read = 0;
        while end_not_reached && is_digit(text[current]) {
            mantissa *= 10.0;
            mantissa += (text[current] - b'0') as i32 as f64;
            current += 1;
            read += 1;
            end_not_reached = current < text.len();
        }
        if read == 0 {
            return None;
        }
    }

    'assemble: {
        if !end_not_reached {
            break 'assemble;
        }
        if text[current] == b'.' {
            const POWERS: [f64; 8] = [1.0, 0.1, 0.01, 0.001, 0.0001, 0.00001, 0.000001, 0.0000001];
            current += 1;
            let mut read = 1i32;
            end_not_reached = current < text.len();
            while end_not_reached && is_digit(text[current]) {
                let digit = (text[current] - b'0') as i32 as f64;
                let scale = if read < 8 { POWERS[read as usize] } else { 10.0f64.powf(-read as f64) };
                mantissa += digit * scale;
                read += 1;
                current += 1;
                end_not_reached = current < text.len();
            }
        } else if text[current] == b'e' || text[current] == b'E' {
        } else {
            break 'assemble;
        }

        if !end_not_reached {
            break 'assemble;
        }
        if text[current] == b'e' || text[current] == b'E' {
            current += 1;
            end_not_reached = current < text.len();
            let mut exponent_negative = false;
            // `IS_DIGIT(*curr)` reads the terminator when the token ends
            // right after the `e`; it is never a digit.
            let next = text.get(current).copied().unwrap_or(0);
            if end_not_reached && (next == b'+' || next == b'-') {
                exponent_negative = next == b'-';
                current += 1;
            } else if is_digit(next) {
            } else {
                return None;
            }
            let mut read = 0;
            end_not_reached = current < text.len();
            while end_not_reached && is_digit(text[current]) {
                if exponent > 2147483647 / 10 {
                    return None;
                }
                exponent = exponent.wrapping_mul(10).wrapping_add((text[current] - b'0') as i32);
                current += 1;
                read += 1;
                end_not_reached = current < text.len();
            }
            if exponent_negative {
                exponent = exponent.wrapping_neg();
            }
            if read == 0 {
                return None;
            }
        }
    }

    let magnitude = if exponent != 0 { ldexp(mantissa * 5.0f64.powf(exponent as f64), exponent) } else { mantissa };
    let sign = if negative { -1.0 } else { 1.0 };
    Some(sign * magnitude)
}

fn ldexp(value: f64, exponent: i32) -> f64 {
    unsafe extern "C" {
        fn ldexp(value: f64, exponent: i32) -> f64;
    }
    unsafe { ldexp(value, exponent) }
}

/// `parseReal`: the next blank-separated token as a float (the default
/// when it does not parse).
fn parse_real(cursor: &mut Cursor, default: f64) -> f32 {
    let skip = cursor.span(b" \t");
    cursor.advance(skip);
    let length = cursor.complement_span(b" \t\r");
    let token = &cursor.rest()[..length];
    let value = try_parse_double(token).unwrap_or(default);
    cursor.advance(length);
    value as f32
}

/// UCRT's `atoi`: leading white space, a sign, digits; out-of-range values
/// clamp (`long` is 32-bit on Windows).
fn msvc_atoi(text: &[u8]) -> i32 {
    let mut index = 0;
    while index < text.len() && matches!(text[index], b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r') {
        index += 1;
    }
    let mut negative = false;
    if index < text.len() && (text[index] == b'+' || text[index] == b'-') {
        negative = text[index] == b'-';
        index += 1;
    }
    let mut value: i64 = 0;
    let mut overflow = false;
    while index < text.len() && text[index].is_ascii_digit() {
        value = value * 10 + (text[index] - b'0') as i64;
        if value > i32::MAX as i64 + 1 {
            overflow = true;
            value = i32::MAX as i64 + 1;
        }
        index += 1;
    }
    if negative {
        let negated = -value;
        if overflow || negated < i32::MIN as i64 { i32::MIN } else { negated as i32 }
    } else if overflow || value > i32::MAX as i64 {
        i32::MAX
    } else {
        value as i32
    }
}

/// `fixIndex`: one-based to zero-based, negative relative to `count`.
fn fix_index(index: i32, count: i32, allow_zero: bool) -> Option<i32> {
    if index > 0 {
        return Some(index - 1);
    }
    if index == 0 {
        return if allow_zero { Some(-1) } else { None };
    }
    let relative = count.wrapping_add(index);
    if relative < 0 { None } else { Some(relative) }
}

struct Counts {
    vertices: i32,
    normals: i32,
    texcoords: i32,
}

/// `parseTriple`: `v`, `v/vt`, `v//vn` or `v/vt/vn`; the vertex index.
fn parse_triple(cursor: &mut Cursor, counts: &Counts) -> Option<i32> {
    let vertex = fix_index(msvc_atoi(cursor.rest()), counts.vertices, false)?;
    let length = cursor.complement_span(b"/ \t\r");
    cursor.advance(length);
    if cursor.at(0) != b'/' {
        return Some(vertex);
    }
    cursor.advance(1);
    if cursor.at(0) == b'/' {
        cursor.advance(1);
        fix_index(msvc_atoi(cursor.rest()), counts.normals, true)?;
        let length = cursor.complement_span(b"/ \t\r");
        cursor.advance(length);
        return Some(vertex);
    }
    fix_index(msvc_atoi(cursor.rest()), counts.texcoords, true)?;
    let length = cursor.complement_span(b"/ \t\r");
    cursor.advance(length);
    if cursor.at(0) != b'/' {
        return Some(vertex);
    }
    cursor.advance(1);
    fix_index(msvc_atoi(cursor.rest()), counts.normals, true)?;
    let length = cursor.complement_span(b"/ \t\r");
    cursor.advance(length);
    Some(vertex)
}

/// The vertex indices of one primitive line (`f`, `l` or `p`); `None` when
/// an index is bad (tinyobjloader then fails the load).
fn parse_indices(cursor: &mut Cursor, counts: &Counts) -> Option<Vec<i32>> {
    let mut indices = Vec::with_capacity(3);
    while !is_new_line(cursor.at(0)) {
        indices.push(parse_triple(cursor, counts)?);
        let skip = cursor.span(b" \t\r");
        cursor.advance(skip);
    }
    Some(indices)
}

/// `parseString`.
fn parse_string(cursor: &mut Cursor) -> Vec<u8> {
    let skip = cursor.span(b" \t");
    cursor.advance(skip);
    let length = cursor.complement_span(b" \t\r");
    let word = cursor.rest()[..length].to_vec();
    cursor.advance(length);
    word
}

/// `SplitString` for `mtllib` (space separated, `\` escapes).
fn split_material_files(text: &[u8]) -> Vec<Vec<u8>> {
    let mut parts = Vec::new();
    let mut token = Vec::new();
    let mut escaping = false;
    for &byte in text {
        if escaping {
            escaping = false;
        } else if byte == b'\\' {
            escaping = true;
            continue;
        } else if byte == b' ' {
            if !token.is_empty() {
                parts.push(std::mem::take(&mut token));
            }
            continue;
        }
        token.push(byte);
    }
    parts.push(token);
    parts
}

/// `LoadMtl`, for the material names only: each name maps to its index
/// among all materials loaded so far (the first wins), and every file
/// flushes a last (possibly unnamed) material.
fn load_material_names(bytes: &[u8], material_map: &mut BTreeMap<Vec<u8>, i32>, material_count: &mut i32) {
    let mut name: Vec<u8> = Vec::new();
    for raw_line in lines(bytes) {
        let kept = raw_line.iter().rposition(|&byte| !is_space(byte)).map(|last| last + 1).unwrap_or(0);
        let line = &raw_line[..kept];
        if line.is_empty() {
            continue;
        }
        let mut cursor = Cursor { text: line, position: 0 };
        let skip = cursor.span(b" \t");
        cursor.advance(skip);
        if cursor.at(0) == 0 || cursor.at(0) == b'#' {
            continue;
        }
        if cursor.starts_with(b"newmtl") && is_space(cursor.at(6)) {
            if !name.is_empty() {
                material_map.entry(name.clone()).or_insert(*material_count);
                *material_count += 1;
            }
            cursor.advance(7);
            name = parse_string(&mut cursor);
        }
    }
    material_map.entry(name).or_insert(*material_count);
    *material_count += 1;
}

/// The face, line and point groups between two exports.
#[derive(Default)]
struct PrimitiveGroup {
    faces: Vec<Vec<i32>>,
    has_lines_or_points: bool,
}

/// `exportGroupsToShape` for the faces: triangles appended to `output`,
/// with the vertices known so far deciding quad splits and ear clipping.
fn export_faces(faces: &[Vec<i32>], positions: &[f32], output: &mut Vec<[i32; 3]>) {
    for face in faces {
        let polygon_count = face.len();
        if polygon_count < 3 {
            continue;
        }
        if polygon_count == 3 {
            output.push([face[0], face[1], face[2]]);
        } else if polygon_count == 4 {
            split_quad(face, positions, output);
        } else {
            ear_clip(face, positions, output);
        }
    }
}

fn split_quad(face: &[i32], positions: &[f32], output: &mut Vec<[i32; 3]>) {
    let size = positions.len();
    let valid = face.iter().all(|&index| 3 * (index as usize) + 2 < size);
    if !valid {
        return;
    }
    let corner = |index: i32| {
        let base = 3 * index as usize;
        [positions[base], positions[base + 1], positions[base + 2]]
    };
    let v0 = corner(face[0]);
    let v1 = corner(face[1]);
    let v2 = corner(face[2]);
    let v3 = corner(face[3]);
    let e02 = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];
    let e13 = [v3[0] - v1[0], v3[1] - v1[1], v3[2] - v1[2]];
    let squared_02 = e02[0] * e02[0] + e02[1] * e02[1] + e02[2] * e02[2];
    let squared_13 = e13[0] * e13[0] + e13[1] * e13[1] + e13[2] * e13[2];
    if squared_02 < squared_13 {
        output.push([face[0], face[1], face[2]]);
        output.push([face[0], face[2], face[3]]);
    } else {
        output.push([face[0], face[1], face[3]]);
        output.push([face[1], face[2], face[3]]);
    }
}

/// tinyobjloader's `pnpoly` (crossing number).
fn point_in_polygon(xs: &[f32; 3], ys: &[f32; 3], x: f32, y: f32) -> bool {
    let mut inside = false;
    let mut previous = 2;
    for current in 0..3 {
        let straddles = (ys[current] > y) != (ys[previous] > y);
        if straddles {
            let crossing = (xs[previous] - xs[current]) * (y - ys[current]) / (ys[previous] - ys[current]) + xs[current];
            if x < crossing {
                inside = !inside;
            }
        }
        previous = current;
    }
    inside
}

/// The built-in ear clipping: project onto the two axes the first proper
/// corner spans most, then cut ears whose corner turns with the first
/// triangle's area and that hold no other vertex.
fn ear_clip(face: &[i32], positions: &[f32], output: &mut Vec<[i32; 3]>) {
    let size = positions.len();
    let polygon_count = face.len();
    let mut axes = [1usize, 2usize];
    for k in 0..polygon_count {
        let i0 = face[k % polygon_count] as usize;
        let i1 = face[(k + 1) % polygon_count] as usize;
        let i2 = face[(k + 2) % polygon_count] as usize;
        if 3 * i0 + 2 >= size || 3 * i1 + 2 >= size || 3 * i2 + 2 >= size {
            continue;
        }
        let e0x = positions[3 * i1] - positions[3 * i0];
        let e0y = positions[3 * i1 + 1] - positions[3 * i0 + 1];
        let e0z = positions[3 * i1 + 2] - positions[3 * i0 + 2];
        let e1x = positions[3 * i2] - positions[3 * i1];
        let e1y = positions[3 * i2 + 1] - positions[3 * i1 + 1];
        let e1z = positions[3 * i2 + 2] - positions[3 * i1 + 2];
        let cx = (e0y * e1z - e0z * e1y).abs();
        let cy = (e0z * e1x - e0x * e1z).abs();
        let cz = (e0x * e1y - e0y * e1x).abs();
        let epsilon = f32::EPSILON;
        if cx > epsilon || cy > epsilon || cz > epsilon {
            let x_dominant = cx > cy && cx > cz;
            if !x_dominant {
                axes[0] = 0;
                if cz > cx && cz > cy {
                    axes[1] = 1;
                }
            }
            break;
        }
    }

    let projected = |index: i32, axis: usize| {
        let offset = 3 * index as usize + axes[axis];
        let outside = 3 * index as usize + axes[0] >= size || 3 * index as usize + axes[1] >= size;
        if outside { 0.0 } else { positions[offset] }
    };

    let mut remaining = face.to_vec();
    let mut guess = 0usize;
    let mut remaining_iterations = face.len();
    let mut previous_remaining = remaining.len();
    while remaining.len() > 3 && remaining_iterations > 0 {
        let count = remaining.len();
        if guess >= count {
            guess -= count;
        }
        if previous_remaining != count {
            previous_remaining = count;
            remaining_iterations = count;
        } else {
            remaining_iterations -= 1;
        }

        let mut corners = [0i32; 3];
        let mut xs = [0.0f32; 3];
        let mut ys = [0.0f32; 3];
        for k in 0..3 {
            corners[k] = remaining[(guess + k) % count];
            xs[k] = projected(corners[k], 0);
            ys[k] = projected(corners[k], 1);
        }
        let e0x = xs[1] - xs[0];
        let e0y = ys[1] - ys[0];
        let e1x = xs[2] - xs[1];
        let e1y = ys[2] - ys[1];
        let cross = e0x * e1y - e0y * e1x;
        let area = (xs[0] * ys[1] - ys[0] * xs[1]) * 0.5;
        if cross * area < 0.0 {
            guess += 1;
            continue;
        }

        let mut overlap = false;
        for other in 3..count {
            let index = (guess + other) % count;
            if index >= remaining.len() {
                continue;
            }
            let other_vertex = remaining[index] as usize;
            if 3 * other_vertex + axes[0] >= size || 3 * other_vertex + axes[1] >= size {
                continue;
            }
            let x = positions[3 * other_vertex + axes[0]];
            let y = positions[3 * other_vertex + axes[1]];
            if point_in_polygon(&xs, &ys, x, y) {
                overlap = true;
                break;
            }
        }
        if overlap {
            guess += 1;
            continue;
        }

        output.push(corners);
        let mut removed = (guess + 1) % count;
        while removed + 1 < count {
            remaining[removed] = remaining[removed + 1];
            removed += 1;
        }
        remaining.pop();
    }
    if remaining.len() == 3 {
        output.push([remaining[0], remaining[1], remaining[2]]);
    }
}

/// tinyobjloader's `LoadObj` (stream version, `triangulate` on), reduced to
/// positions and triangles; `None` when it fails.
pub fn parse_obj(bytes: &[u8]) -> Option<ObjModel> {
    let mut positions: Vec<f32> = Vec::new();
    let mut normal_count = 0i32;
    let mut texcoord_count = 0i32;
    let mut group = PrimitiveGroup::default();
    let mut triangles = Vec::new();
    let mut material_map: BTreeMap<Vec<u8>, i32> = BTreeMap::new();
    let mut material_count = 0i32;
    let mut material_files: BTreeSet<Vec<u8>> = BTreeSet::new();
    let mut material = -1i32;

    for line in lines(bytes) {
        if line.is_empty() {
            continue;
        }
        let mut cursor = Cursor { text: line, position: 0 };
        let skip = cursor.span(b" \t");
        cursor.advance(skip);
        let first = cursor.at(0);
        let second = cursor.at(1);
        if first == 0 || first == b'#' {
            continue;
        }

        if first == b'v' && is_space(second) {
            cursor.advance(2);
            let x = parse_real(&mut cursor, 0.0);
            let y = parse_real(&mut cursor, 0.0);
            let z = parse_real(&mut cursor, 0.0);
            positions.push(x);
            positions.push(y);
            positions.push(z);
            continue;
        }
        if first == b'v' && second == b'n' && is_space(cursor.at(2)) {
            normal_count += 1;
            continue;
        }
        if first == b'v' && second == b't' && is_space(cursor.at(2)) {
            texcoord_count += 1;
            continue;
        }
        if first == b'v' && second == b'w' && is_space(cursor.at(2)) {
            // Skin weights: a negative (or missing) joint fails the load.
            cursor.advance(3);
            msvc_atoi(cursor.rest());
            let skip = cursor.span(b" \t");
            cursor.advance(skip);
            let length = cursor.complement_span(b" \t\r");
            cursor.advance(length);
            while !is_new_line(cursor.at(0)) {
                let joint = parse_real(&mut cursor, -1.0);
                parse_real(&mut cursor, 0.0);
                if joint < 0.0 {
                    return None;
                }
                let skip = cursor.span(b" \t\r");
                cursor.advance(skip);
            }
            // tinyobjloader falls through to the other commands here; none
            // matches an exhausted line.
            continue;
        }

        let counts = Counts {
            vertices: (positions.len() / 3) as i32,
            normals: normal_count,
            texcoords: texcoord_count,
        };
        if (first == b'l' || first == b'p') && is_space(second) {
            cursor.advance(2);
            parse_indices(&mut cursor, &counts)?;
            group.has_lines_or_points = true;
            continue;
        }
        if first == b'f' && is_space(second) {
            cursor.advance(2);
            let skip = cursor.span(b" \t");
            cursor.advance(skip);
            let face = parse_indices(&mut cursor, &counts)?;
            group.faces.push(face);
            continue;
        }
        if cursor.starts_with(b"usemtl") {
            cursor.advance(6);
            let name = parse_string(&mut cursor);
            let new_material = material_map.get(&name).copied().unwrap_or(-1);
            if new_material != material {
                export_faces(&group.faces, &positions, &mut triangles);
                group.faces.clear();
                material = new_material;
            }
            continue;
        }
        if cursor.starts_with(b"mtllib") && is_space(cursor.at(6)) {
            cursor.advance(7);
            let names = split_material_files(cursor.rest());
            for name in names {
                if material_files.contains(&name) {
                    continue;
                }
                // The base directory is empty: names resolve against the
                // working directory.
                let path = String::from_utf8_lossy(&name).into_owned();
                if let Ok(material_bytes) = std::fs::read(&path) {
                    load_material_names(&material_bytes, &mut material_map, &mut material_count);
                    material_files.insert(name);
                    break;
                }
            }
            continue;
        }
        if (first == b'g' || first == b'o') && is_space(second) {
            export_faces(&group.faces, &positions, &mut triangles);
            group = PrimitiveGroup::default();
            continue;
        }
    }
    export_faces(&group.faces, &positions, &mut triangles);
    Some(ObjModel { positions, triangles })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_parse_like_tinyobjloader() {
        assert_eq!(try_parse_double(b"1.5"), Some(1.5));
        assert_eq!(try_parse_double(b"-.25"), Some(-0.25));
        assert_eq!(try_parse_double(b"2e2"), Some(200.0));
        assert_eq!(try_parse_double(b"1.5abc"), Some(1.5));
        assert_eq!(try_parse_double(b"e5"), None);
        assert_eq!(try_parse_double(b"1e"), None);
        assert_eq!(try_parse_double(b"."), Some(0.0));
    }

    #[test]
    fn quads_split_on_the_shorter_diagonal() {
        let text = b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 2 0\nf 1 2 3 4\n";
        let model = parse_obj(text).unwrap();
        assert_eq!(model.triangles, vec![[0, 1, 2], [0, 2, 3]]);
    }

    #[test]
    fn trailing_comments_fail_faces() {
        assert!(parse_obj(b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3 # note\n").is_none());
    }
}
