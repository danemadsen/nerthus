//! Assimp's Unreal mesh importer (`UnrealLoader.cpp`): `name_d.3d` (the
//! triangles), `name_a.3d` (the first frame's packed vertices) and the
//! optional `name.uc` script, whichever of them is opened. A mesh per
//! texture number, a vertex per corner, mirrored along z and with the
//! winding flipped (`MakeLeftHandedProcess`, `FlipWindingOrderProcess`).
//!
//! Vertex indices are checked against the triangle count, not the vertex
//! count, so one between the two reads past the vertices: undefined
//! behaviour, an error here, as is the script scan stepping past its NUL.

use crate::fast_atof::{fast_atoreal_move, strtoul10};
use crate::io::FileSystem;
use crate::parsing::{checked_at, is_line_end, is_space_or_newline, skip_spaces_and_line_end, starts_with_ignore_case_at, text_file_to_buffer, token_match_i};
use crate::registry::get_extension;
use crate::scene::{ImportError, Mesh, PRIMITIVE_TRIANGLE, Scene, fail};
use crate::stream_reader::StreamReader;

/// `Unreal::Triangle`, as far as the mesh needs it.
struct Triangle {
    vertex: [u16; 3],
    tex: [[u8; 2]; 3],
    texture_number: u8,
}

/// `Unreal::DecompressVertex`: x, y and z in 11, 11 and 10 signed bits.
fn decompress_vertex(packed: i32) -> [f32; 3] {
    let x = (packed << 21) >> 21;
    let y = (packed << 10) >> 21;
    let z = packed >> 22;
    [x as f32, y as f32, z as f32]
}

/// Steps over a token (to a blank or line end).
fn skip_token(data: &[u8], mut position: usize) -> Result<usize, ImportError> {
    while !is_space_or_newline(checked_at(data, position)?) {
        position += 1;
    }
    Ok(position)
}

/// The rest of an `#exec TEXTURE IMPORT` line: `NAME=` and `FILE=` words.
fn scan_texture_import(data: &[u8], mut position: usize) -> Result<usize, ImportError> {
    while !is_line_end(checked_at(data, position)?) {
        if starts_with_ignore_case_at(data, position, b"NAME=") || starts_with_ignore_case_at(data, position, b"FILE=") {
            position = skip_token(data, position + 5)?;
        }
        position += 1;
    }
    Ok(position)
}

/// The rest of an `#exec MESHMAP SETTEXTURE` line: `NUM=` and `TEXTURE=`.
fn scan_set_texture(data: &[u8], mut position: usize) -> Result<usize, ImportError> {
    while !is_line_end(checked_at(data, position)?) {
        if starts_with_ignore_case_at(data, position, b"NUM=") {
            position = strtoul10(data, position + 4).1;
        } else if starts_with_ignore_case_at(data, position, b"TEXTURE=") {
            position = skip_token(data, position + 8)?;
        }
        position += 1;
    }
    Ok(position)
}

/// The rest of an `#exec MESHMAP SCALE` line: `X=`, `Y=` and `Z=` numbers.
fn scan_scale(data: &[u8], mut position: usize) -> Result<usize, ImportError> {
    while !is_line_end(checked_at(data, position)?) {
        let axis = data[position];
        if matches!(axis, b'X' | b'Y' | b'Z') && checked_at(data, position + 1)? == b'=' {
            position = fast_atoreal_move(data, position + 2, true)?.1;
        }
        position += 1;
    }
    Ok(position)
}

/// The `.uc` scan for texture names and the scale, kept for its failures:
/// a bad number, or a read past the end.
fn scan_script(bytes: &[u8]) -> Result<(), ImportError> {
    let data = text_file_to_buffer(bytes, false)?;
    let end = data.len();
    let mut position = 0;
    while checked_at(&data, position)? != 0 {
        if token_match_i(&data, &mut position, b"#exec")? {
            position = skip_spaces_and_line_end(&data, position, end).0;
            if token_match_i(&data, &mut position, b"TEXTURE")? {
                position = skip_spaces_and_line_end(&data, position, end).0;
                if token_match_i(&data, &mut position, b"IMPORT")? {
                    position = scan_texture_import(&data, position)?;
                }
            } else if token_match_i(&data, &mut position, b"MESHMAP")? {
                position = skip_spaces_and_line_end(&data, position, end).0;
                if token_match_i(&data, &mut position, b"SETTEXTURE")? {
                    position = scan_set_texture(&data, position)?;
                } else if token_match_i(&data, &mut position, b"SCALE")? {
                    position = scan_scale(&data, position)?;
                }
            }
        }
        position += 1;
    }
    Ok(())
}

pub fn read(path: &str, files: &FileSystem) -> Result<Scene, ImportError> {
    let extension = get_extension(path);
    let base = if extension == "3d" {
        let Some(underscore) = path.rfind('_') else {
            return fail("UNREAL: Unexpected naming scheme");
        };
        &path[..underscore]
    } else {
        match path.rfind('.') {
            Some(dot) => &path[..dot],
            None => path,
        }
    };
    let data_path = format!("{base}_d.3d");
    let animation_path = format!("{base}_a.3d");
    let script_path = format!("{base}.uc");

    let Some(data_file) = files.open(&data_path) else {
        return fail("UNREAL: Unable to open _d file");
    };
    let mut data_reader = StreamReader::new(&data_file, false)?;
    let triangle_count = data_reader.get_i2()? as u16;
    let vertex_count = data_reader.get_i2()? as u16;
    data_reader.inc_ptr(44)?;
    if triangle_count == 0 || vertex_count < 3 {
        return fail("UNREAL: Invalid number of vertices/triangles");
    }

    let mut triangles = Vec::with_capacity(triangle_count as usize);
    for _ in 0..triangle_count {
        let mut vertex = [0u16; 3];
        for corner in &mut vertex {
            *corner = data_reader.get_i2()? as u16;
            if *corner >= triangle_count {
                // "UNREAL: vertex index out of range"
                *corner = 0;
            }
        }
        // The mesh type (always read as one-sided), then the colour.
        data_reader.get_i1()?;
        data_reader.inc_ptr(1)?;
        let mut tex = [[0u8; 2]; 3];
        for corner in &mut tex {
            for value in corner.iter_mut() {
                *value = data_reader.get_i1()? as u8;
            }
        }
        let texture_number = data_reader.get_i1()? as u8;
        data_reader.inc_ptr(1)?;
        triangles.push(Triangle { vertex, tex, texture_number });
    }

    let Some(animation_file) = files.open(&animation_path) else {
        return fail("UNREAL: Unable to open _a file");
    };
    let mut animation_reader = StreamReader::new(&animation_file, false)?;
    let frame_count = animation_reader.get_i2()? as i32 as u32;
    if frame_count == 0 {
        return fail("UNREAL: The requested frame does not exist");
    }
    let frame_size = animation_reader.get_i2()? as i32 as u32;
    if frame_size != vertex_count as u32 * 4 {
        return fail("UNREAL: Unexpected aniv file length");
    }
    let mut vertices = Vec::with_capacity(vertex_count as usize);
    for _ in 0..vertex_count {
        vertices.push(decompress_vertex(animation_reader.get_i4()?));
    }

    if let Some(script) = files.open(&script_path) {
        scan_script(&script)?;
    }

    // A mesh per texture number, in order of first use.
    let mut materials: Vec<u8> = Vec::new();
    let mut face_counts: Vec<usize> = Vec::new();
    for triangle in &triangles {
        match materials.iter().position(|&number| number == triangle.texture_number) {
            Some(index) => face_counts[index] += 1,
            None => {
                materials.push(triangle.texture_number);
                face_counts.push(1);
            }
        }
    }
    let mut meshes: Vec<Mesh> = face_counts
        .iter()
        .map(|&count| Mesh {
            primitive_types: PRIMITIVE_TRIANGLE,
            vertices: Some(Vec::with_capacity(count * 3)),
            texture_coords: [Some(Vec::with_capacity(count * 3)), None, None, None, None, None, None, None],
            faces: Some(Vec::with_capacity(count)),
            ..Mesh::default()
        })
        .collect();
    for triangle in &triangles {
        let index = materials.iter().position(|&number| number == triangle.texture_number).unwrap_or_default();
        let mesh = &mut meshes[index];
        let first = mesh.vertex_count as u32;
        for corner in 0..3 {
            let Some(&position) = vertices.get(triangle.vertex[corner] as usize) else {
                return fail("undefined behaviour: a vertex index past the vertices");
            };
            let [u, v] = triangle.tex[corner];
            let coords = [u as f32 / 255.0, 1.0 - v as f32 / 255.0, 0.0];
            mesh.vertices.as_mut().unwrap().push(position);
            mesh.texture_coords[0].as_mut().unwrap().push(coords);
            mesh.vertex_count += 1;
        }
        // Flipped from first, first + 1, first + 2.
        mesh.faces.as_mut().unwrap().push(vec![first + 2, first + 1, first]);
    }
    for mesh in &mut meshes {
        for position in mesh.vertices.as_mut().unwrap() {
            position[2] *= -1.0;
        }
    }
    Ok(Scene { meshes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertices_unpack_signed_bit_fields() {
        let packed = (5i32 & 0x7ff) | ((-3i32 & 0x7ff) << 11) | (-1i32 << 22);
        assert_eq!(decompress_vertex(packed), [5.0, -3.0, -1.0]);
    }

    #[test]
    fn a_script_ending_in_a_token_reads_past_its_end() {
        assert!(scan_script(b"#exec MESHMAP SCALE X=0.1 Y=2 \n").is_ok());
        // The scan steps over the line end after a number, then the main
        // loop over the NUL.
        assert!(scan_script(b"#exec MESHMAP SCALE X=0.1 Y=2\n").is_err());
        assert!(scan_script(b"// comment\n#exec").is_err());
        assert!(scan_script(b"#exec MESHMAP SCALE X=a\n").is_err());
    }
}
