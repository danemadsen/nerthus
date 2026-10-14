//! Assimp's OFF importer (`OFFLoader.cpp`): a header of optional prefixes
//! (`ST` texture coordinates, `C` colours, `N` normals, `4` homogeneous
//! coordinates, `n` a dimension count) before `OFF`, the vertex, face and
//! edge counts, one vertex per line, then faces of one to nine indices.
//!
//! Its quirks are kept: the third normal component's parse doesn't move
//! the cursor, so the colour (or texture coordinate) after it reads the
//! normal's last value again; a colour with fewer than four values must
//! end in `#`, or the parse of the next one fails at the line end; faces
//! with no or more than nine indices are dropped; an index past the
//! vertices becomes the last vertex.

use crate::fast_atof::{fast_atoreal_move, strtoul10};
use crate::parsing::{BUFFER_SIZE, get_next_line, skip_line, skip_spaces, skip_spaces_and_line_end, starts_with_at, text_file_to_buffer};
use crate::scene::{ImportError, Mesh, Scene, check_allocation, fail};

/// `sizeof(aiFace)`.
const FACE_SIZE: u64 = 16;

/// `NextToken`: past blanks, line breaks and comment lines.
fn next_token(buffer: &[u8], mut car: usize, end: usize) -> usize {
    car = skip_spaces_and_line_end(buffer, car, end).0;
    while car < end && matches!(buffer[car], b'#' | b'\n' | b'\r') {
        car = skip_line(buffer, car, end).0;
        car = skip_spaces_and_line_end(buffer, car, end).0;
    }
    car
}

/// Blanks skipped, then a number read with `fast_atoreal_move`.
fn next_real(line: &[u8], position: usize) -> Result<(f32, usize), ImportError> {
    let start = skip_spaces(line, position, BUFFER_SIZE).0;
    fast_atoreal_move(line, start, true)
}

/// Whether the colour goes on at `position` (no `#`, `\n` or `\r` there).
fn colour_goes_on(line: &[u8], position: usize) -> bool {
    !matches!(line[position], b'#' | b'\n' | b'\r')
}

pub fn read(bytes: &[u8]) -> Result<Scene, ImportError> {
    let buffer = text_file_to_buffer(bytes, false)?;
    let end = buffer.len();

    let mut has_texture_coords = false;
    let mut has_normals = false;
    let mut has_colors = false;
    let mut has_homogeneous = false;
    let mut has_dimension = false;
    let mut dimensions = 3;
    let mut car = next_token(&buffer, 0, end);

    if car + 2 < end && buffer[car] == b'S' && buffer[car + 1] == b'T' {
        has_texture_coords = true;
        car += 2;
    }
    if car + 1 < end && buffer[car] == b'C' {
        has_colors = true;
        car += 1;
    }
    if car + 1 < end && buffer[car] == b'N' {
        has_normals = true;
        car += 1;
    }
    if car + 1 < end && buffer[car] == b'4' {
        has_homogeneous = true;
        car += 1;
    }
    if car + 1 < end && buffer[car] == b'n' {
        has_dimension = true;
        car += 1;
    }
    if car + 3 < end && starts_with_at(&buffer, car, b"OFF") {
        car = next_token(&buffer, car + 3, end);
    } else {
        // No `OFF` (the specification allows that): a count follows.
        dimensions = 3;
        has_homogeneous = false;
        car = next_token(&buffer, car, end);
        if car + 1 >= end || !buffer[car].is_ascii_digit() {
            return fail("OFF: Header is invalid");
        }
    }
    if has_dimension {
        let (value, next) = strtoul10(&buffer, car);
        dimensions = value;
        car = next_token(&buffer, next, end);
    }
    if dimensions > 3 {
        return fail("OFF: Number of vertex coordinates higher than 3 unsupported");
    }

    car = next_token(&buffer, car, end);
    let (vertex_count, next) = strtoul10(&buffer, car);
    car = next_token(&buffer, next, end);
    let (face_count, next) = strtoul10(&buffer, car);
    car = next_token(&buffer, next, end);
    let (_edge_count, next) = strtoul10(&buffer, car);
    car = next_token(&buffer, next, end);

    if vertex_count == 0 {
        return fail("OFF: There are no valid vertices");
    }
    if face_count == 0 {
        return fail("OFF: There are no valid faces");
    }

    let vertex_total = vertex_count as usize;
    check_allocation(face_count as u64, FACE_SIZE)?;
    check_allocation(vertex_count as u64, 12)?;
    let mut vertices = vec![[0.0f32; 3]; vertex_total];
    let mut normals = Vec::new();
    if has_normals {
        check_allocation(vertex_count as u64, 12)?;
        normals = vec![[0.0f32; 3]; vertex_total];
    }
    let mut colors = Vec::new();
    if has_colors {
        check_allocation(vertex_count as u64, 16)?;
        colors = vec![[0.0f32; 4]; vertex_total];
    }
    let mut texture_coords = Vec::new();
    if has_texture_coords {
        check_allocation(vertex_count as u64, 12)?;
        texture_coords = vec![[0.0f32; 3]; vertex_total];
    }

    let mut line = [0u8; BUFFER_SIZE];
    let mut position = car;
    for index in 0..vertex_total {
        if !get_next_line(&buffer, &mut position, &mut line)? {
            // "OFF: The number of verts in the header is incorrect"
            break;
        }
        let mut cursor = 0;
        let vertex = &mut vertices[index];
        for component in vertex.iter_mut().take(dimensions as usize) {
            let (value, next) = next_real(&line, cursor)?;
            *component = value;
            cursor = next;
        }
        if has_homogeneous {
            let (w, next) = next_real(&line, cursor)?;
            cursor = next;
            for component in vertex.iter_mut().take(dimensions as usize) {
                *component /= w;
            }
        }
        if has_normals {
            let normal = &mut normals[index];
            let (x, next) = next_real(&line, cursor)?;
            normal[0] = x;
            cursor = next;
            let (y, next) = next_real(&line, cursor)?;
            normal[1] = y;
            cursor = next;
            // The cursor stays before z.
            let (z, _) = next_real(&line, cursor)?;
            normal[2] = z;
            cursor = skip_spaces(&line, cursor, BUFFER_SIZE).0;
        }
        if has_colors {
            let colour = &mut colors[index];
            let (red, next) = next_real(&line, cursor)?;
            colour[0] = red;
            cursor = next;
            if colour_goes_on(&line, cursor) {
                let (green, next) = next_real(&line, cursor)?;
                colour[1] = green;
                cursor = next;
            } else {
                colour[1] = 0.0;
            }
            if colour_goes_on(&line, cursor) {
                let (blue, next) = next_real(&line, cursor)?;
                colour[2] = blue;
                cursor = next;
            } else {
                colour[2] = 0.0;
            }
            if colour_goes_on(&line, cursor) {
                let (alpha, next) = next_real(&line, cursor)?;
                colour[3] = alpha;
                cursor = next;
            } else {
                colour[3] = 1.0;
            }
        }
        if has_texture_coords {
            let coords = &mut texture_coords[index];
            let (u, next) = next_real(&line, cursor)?;
            coords[0] = u;
            cursor = next;
            let (v, _) = next_real(&line, cursor)?;
            coords[1] = v;
        }
    }

    let mut faces = Vec::new();
    for _ in 0..face_count {
        if !get_next_line(&buffer, &mut position, &mut line)? {
            return fail("OFF: The number of faces in the header is incorrect");
        }
        let start = skip_spaces(&line, 0, BUFFER_SIZE).0;
        let (index_count, mut cursor) = strtoul10(&line, start);
        if index_count == 0 || index_count > 9 {
            // "OFF: Faces with zero indices aren't allowed"
            continue;
        }
        let mut indices = Vec::with_capacity(index_count as usize);
        for _ in 0..index_count {
            let start = skip_spaces(&line, cursor, BUFFER_SIZE).0;
            let (mut vertex, next) = strtoul10(&line, start);
            cursor = next;
            if vertex >= vertex_count {
                // "OFF: Vertex index is out of range"
                vertex = vertex_count - 1;
            }
            indices.push(vertex);
        }
        faces.push(indices);
    }

    let mut mesh = Mesh { vertex_count: vertex_total, vertices: Some(vertices), faces: Some(faces), ..Mesh::default() };
    if has_normals {
        mesh.normals = Some(normals);
    }
    if has_colors {
        mesh.colors[0] = Some(colors);
    }
    if has_texture_coords {
        mesh.uv_components[0] = 2;
        mesh.texture_coords[0] = Some(texture_coords);
    }
    Ok(Scene { meshes: vec![mesh] })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cube_reads() {
        let text = b"OFF\n# cube\n8 6 12\n0 0 0\n1 0 0\n1 1 0\n0 1 0\n0 0 1\n1 0 1\n1 1 1\n0 1 1\n\
            4 0 3 2 1\n4 4 5 6 7\n4 0 1 5 4\n4 1 2 6 5\n4 2 3 7 6\n4 3 0 4 7\n";
        let scene = read(text).unwrap();
        let mesh = &scene.meshes[0];
        assert_eq!(mesh.vertex_count, 8);
        assert_eq!(mesh.face_count(), 6);
        assert_eq!(mesh.faces.as_ref().unwrap()[0], vec![0, 3, 2, 1]);
    }

    #[test]
    fn the_last_normal_value_is_read_again_as_the_colour() {
        let text = b"CNOFF\n1 1 0\n1 2 3 0 0 1 0.5 0.25 0.75\n1 0\n";
        let scene = read(text).unwrap();
        let mesh = &scene.meshes[0];
        assert_eq!(mesh.normals.as_ref().unwrap()[0], [0.0, 0.0, 1.0]);
        assert_eq!(mesh.colors[0].as_ref().unwrap()[0], [1.0, 0.5, 0.25, 0.75]);
    }

    #[test]
    fn faces_without_indices_are_dropped_and_indices_clamped() {
        let text = b"OFF\n3 3 0\n0 0 0\n1 0 0\n0 1 0\n0\n3 0 1 7\n10 0 1 2 0 1 2 0 1 2 0\n";
        let scene = read(text).unwrap();
        let faces = scene.meshes[0].faces.as_ref().unwrap();
        assert_eq!(faces, &vec![vec![0, 1, 2]]);
    }

    #[test]
    fn a_short_colour_fails_at_the_line_end() {
        assert!(read(b"COFF\n3 1 0\n0 0 0 1 1 1\n1 0 0 1 1 1\n0 1 0 1 1 1\n3 0 1 2\n").is_err());
        assert!(read(b"COFF\n3 1 0\n0 0 0 1 1 1#\n1 0 0 1 1 1#\n0 1 0 1 1 1#\n3 0 1 2\n").is_ok());
    }
}
