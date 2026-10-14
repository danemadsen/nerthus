//! Assimp's Raw importer (`RawLoader.cpp`): lines of 9 floats (a triangle)
//! or 12 (a colour, then a triangle), each optionally followed by a
//! texture name; a line starting with anything but a digit, `-` or `+`
//! opens a group. A mesh per group and texture, in the order they appear.
//!
//! Names are matched with `strcmp` against the rest of the line, so a name
//! with anything after it never matches and starts another group or
//! mesh. With 12 floats the cursor stays on the blank after the last one,
//! so a texture after a blank reads as no name.

use crate::fast_atof::fast_atoreal_move;
use crate::parsing::{BUFFER_SIZE, at, get_next_line, is_line_end, is_space_or_newline, skip_spaces, text_file_to_buffer};
use crate::scene::{ImportError, Mesh, PRIMITIVE_TRIANGLE, Scene, fail};

/// `IsNumeric`.
fn is_numeric(byte: u8) -> bool {
    byte.is_ascii_digit() || byte == b'-' || byte == b'+'
}

/// `MeshInformation`.
struct MeshInformation {
    name: Vec<u8>,
    vertices: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
}

/// `GroupInformation`.
struct GroupInformation {
    name: Vec<u8>,
    meshes: Vec<MeshInformation>,
}

/// The line from `start` to its NUL (what `strcmp` compares).
fn rest_of_line(line: &[u8], start: usize) -> &[u8] {
    let length = line[start..].iter().position(|&byte| byte == 0).unwrap_or(line.len() - start);
    &line[start..start + length]
}

/// The end of the token at `start`.
fn token_end(line: &[u8], start: usize) -> usize {
    let mut end = start;
    while !is_space_or_newline(at(line, end)) {
        end += 1;
    }
    end
}

pub fn read(bytes: &[u8]) -> Result<Scene, ImportError> {
    let buffer = text_file_to_buffer(bytes, false)?;
    let mut groups = vec![GroupInformation { name: b"<default>".to_vec(), meshes: Vec::new() }];
    let mut current_group = 0;

    let mut line = [0u8; BUFFER_SIZE];
    let mut position = 0;
    while get_next_line(&buffer, &mut position, &mut line)? {
        let mut cursor = skip_spaces(&line, 0, BUFFER_SIZE).0;
        if is_line_end(line[cursor]) {
            continue;
        }
        if !is_numeric(line[cursor]) {
            let name_end = token_end(&line, cursor);
            let name = &line[cursor..name_end];
            let rest = rest_of_line(&line, cursor);
            let existing = groups.iter().position(|group| group.name.len() == name.len() && group.name == rest);
            match existing {
                Some(index) => current_group = index,
                None => {
                    groups.push(GroupInformation { name: name.to_vec(), meshes: Vec::new() });
                    current_group = groups.len() - 1;
                }
            }
            continue;
        }

        // Up to 12 floats, then an optional texture name.
        let mut data = [0.0f32; 12];
        let mut count = 0;
        while count < 12 {
            let (next, goes_on) = skip_spaces(&line, cursor, BUFFER_SIZE);
            cursor = next;
            if !goes_on || !is_numeric(line[cursor]) {
                break;
            }
            let (value, next) = fast_atoreal_move(&line, cursor, true)?;
            data[count] = value;
            cursor = next;
            count += 1;
        }
        if count != 12 && count != 9 {
            // "A line may have either 9 or 12 floats and an optional texture"
            continue;
        }

        let name: &[u8];
        let compared: &[u8];
        if !is_line_end(line[cursor]) {
            let name_end = token_end(&line, cursor);
            name = &line[cursor..name_end];
            compared = rest_of_line(&line, cursor);
        } else if count == 9 {
            name = b"%default%";
            compared = name;
        } else {
            name = b"";
            compared = name;
        }
        let group = &mut groups[current_group];
        let found = group.meshes.iter().position(|mesh| mesh.name.len() == name.len() && (name.is_empty() || mesh.name == compared));
        let index = match found {
            Some(index) => index,
            None => {
                group.meshes.push(MeshInformation { name: name.to_vec(), vertices: Vec::new(), colors: Vec::new() });
                group.meshes.len() - 1
            }
        };
        let output = &mut group.meshes[index];
        if count == 12 {
            let colour = [data[0], data[1], data[2], 1.0];
            output.colors.extend_from_slice(&[colour; 3]);
            output.vertices.push([data[3], data[4], data[5]]);
            output.vertices.push([data[6], data[7], data[8]]);
            output.vertices.push([data[9], data[10], data[11]]);
        } else {
            output.vertices.push([data[0], data[1], data[2]]);
            output.vertices.push([data[3], data[4], data[5]]);
            output.vertices.push([data[6], data[7], data[8]]);
        }
    }

    let mut meshes = Vec::new();
    for group in groups {
        for information in group.meshes {
            let vertex_count = information.vertices.len();
            let mut mesh = Mesh { primitive_types: PRIMITIVE_TRIANGLE, vertex_count, ..Mesh::default() };
            if !information.colors.is_empty() {
                // The colours are copied for every vertex.
                if information.colors.len() < vertex_count {
                    return fail("undefined behaviour: colours copied past their array");
                }
                mesh.colors[0] = Some(information.colors);
            }
            let faces = (0..vertex_count as u32 / 3).map(|face| vec![face * 3, face * 3 + 1, face * 3 + 2]).collect();
            mesh.vertices = Some(information.vertices);
            mesh.faces = Some(faces);
            meshes.push(mesh);
        }
    }
    if meshes.is_empty() {
        return fail("RAW: No meshes loaded. The file seems to be corrupt or empty.");
    }
    Ok(Scene { meshes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_and_textures_split_meshes() {
        let text = b"0 0 0 1 0 0 0 1 0\nhouse\n0 0 0 1 0 0 0 1 0 wall.png\n0.5 0.5 0.5 0 0 0 1 0 0 0 1 0\n";
        let scene = read(text).unwrap();
        assert_eq!(scene.meshes.len(), 3);
        assert!(scene.meshes[2].colors[0].is_some());
    }

    #[test]
    fn a_name_with_more_after_it_never_matches() {
        let text = b"a x\n0 0 0 1 0 0 0 1 0\na x\n0 0 0 1 0 0 0 1 0\n";
        let scene = read(text).unwrap();
        assert_eq!(scene.meshes.len(), 2);
        let text = b"a\n0 0 0 1 0 0 0 1 0\na\n0 0 0 1 0 0 0 1 0\n";
        assert_eq!(read(text).unwrap().meshes.len(), 1);
    }
}
