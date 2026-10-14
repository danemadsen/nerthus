//! Assimp's Terragen terrain importer (`TerragenLoader.cpp`): after the
//! `TERRAGEN` and `TERRAIN ` magic, 4-byte chunks (`XPTS`, `YPTS`, `SIZE`,
//! `SCAL`, `CRAD`, `CRVM`, `ALTW`, `EOF `) each padded to a 4-byte
//! boundary. `ALTW` builds the mesh, a quad of four vertices per grid
//! cell, without moving the reader past the heights: the chunk loop goes
//! on through them, so a height pair spelling a chunk name is read as one.
//! The node's scaling doesn't reach the mesh.

use crate::scene::{ImportError, Mesh, Scene, check_allocation, fail};
use crate::stream_reader::StreamReader;

/// `sizeof(aiFace)`.
const FACE_SIZE: u64 = 16;

/// The quads of an `ALTW` chunk: heights `x` by `y` from `data_start`.
fn build_terrain(bytes: &[u8], data_start: usize, x: u32, y: u32, height_scale: f32, base_height: f32) -> Result<Mesh, ImportError> {
    let face_count = (x - 1) * (y - 1);
    check_allocation(face_count as u64, FACE_SIZE)?;
    let vertex_count = face_count.wrapping_mul(4);
    check_allocation(vertex_count as u64, 12)?;
    // The size check above is 32-bit, so a grid of 2^31 heights or more
    // passes it and reads past the file.
    let data_size = x as u64 * y as u64 * 2;
    if data_start as u64 + data_size > bytes.len() as u64 {
        return fail("undefined behaviour: heights past the file");
    }
    let height = |index: u32| {
        let offset = data_start + index as usize * 2;
        let value = i16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
        value as f32 * height_scale + base_height
    };

    let mut vertices = Vec::with_capacity(vertex_count as usize);
    let mut faces = Vec::with_capacity(face_count as usize);
    let mut next_index = 0u32;
    for row in 0..y - 1 {
        for column in 0..x - 1 {
            let fy = row as f32;
            let fx = column as f32;
            let this_row = x * row;
            let next_row = x * (row + 1);
            vertices.push([fx, fy, height(this_row + column)]);
            vertices.push([fx, fy + 1.0, height(next_row + column)]);
            vertices.push([fx + 1.0, fy + 1.0, height(next_row + column + 1)]);
            vertices.push([fx + 1.0, fy, height(this_row + column + 1)]);
            let quad = (next_index..next_index + 4).collect();
            faces.push(quad);
            next_index += 4;
        }
    }
    Ok(Mesh { vertex_count: vertices.len(), vertices: Some(vertices), faces: Some(faces), ..Mesh::default() })
}

pub fn read(bytes: &[u8]) -> Result<Scene, ImportError> {
    let mut reader = StreamReader::new(bytes, false)?;
    if reader.remaining_size() < 16 {
        return fail("TER: file is too small");
    }
    if &bytes[..8] != b"TERRAGEN" {
        return fail("TER: Magic string 'TERRAGEN' not found");
    }
    if &bytes[8..16] != b"TERRAIN " {
        return fail("TER: Magic string 'TERRAIN' not found");
    }

    let mut x = 0u32;
    let mut y = 0u32;
    let mut mesh = None;
    reader.inc_ptr(16)?;
    while reader.remaining_size() >= 4 {
        let head_start = reader.ptr() as usize;
        let head = &bytes[head_start..head_start + 4];
        reader.inc_ptr(4)?;
        if head == b"EOF " {
            break;
        }
        match head {
            b"XPTS" => x = reader.get_i2()? as u16 as u32,
            b"YPTS" => y = reader.get_i2()? as u16 as u32,
            b"SIZE" => {
                let size = reader.get_i2()? as u16 as u32 + 1;
                x = size;
                y = size;
            }
            b"SCAL" => {
                for _ in 0..3 {
                    reader.get_f4()?;
                }
            }
            b"CRAD" => {
                reader.get_f4()?;
            }
            b"CRVM" => {
                // A mode other than 0 is unsupported: the terrain stays flat.
                reader.get_i1()?;
            }
            b"ALTW" => {
                let mut height_scale = reader.get_i2()? as f32 / 65536.0;
                let base_height = reader.get_i2()? as f32;
                if height_scale == 0.0 {
                    height_scale = 1.0;
                }
                let needed = x.wrapping_mul(y).wrapping_mul(2);
                if reader.remaining_size() < needed as usize {
                    return fail("TER: ALTW chunk is too small");
                }
                if x <= 1 || y <= 1 {
                    return fail("TER: Invalid terrain size");
                }
                let data_start = reader.ptr() as usize;
                mesh = Some(build_terrain(bytes, data_start, x, y, height_scale, base_height)?);
            }
            _ => {}
        }
        // On to the next chunk, 4-byte aligned.
        let misalignment = reader.current_pos() & 3;
        if misalignment != 0 {
            reader.inc_ptr((4 - misalignment) as i64)?;
        }
    }

    let Some(mesh) = mesh else {
        return fail("TER: Unable to load terrain");
    };
    Ok(Scene { meshes: vec![mesh] })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terrain(chunks: &[u8]) -> Vec<u8> {
        let mut bytes = b"TERRAGENTERRAIN ".to_vec();
        bytes.extend_from_slice(chunks);
        bytes
    }

    #[test]
    fn a_two_by_two_grid_is_one_quad() {
        let mut chunks = b"SIZE\x01\x00\x00\x00ALTW\x00\x40\x0a\x00".to_vec();
        for height in [0i16, 4, 8, 12] {
            chunks.extend_from_slice(&height.to_le_bytes());
        }
        chunks.extend_from_slice(b"EOF ");
        let scene = read(&terrain(&chunks)).unwrap();
        let vertices = scene.meshes[0].vertices.as_ref().unwrap();
        assert_eq!(vertices[0], [0.0, 0.0, 10.0]);
        assert_eq!(vertices[1], [0.0, 1.0, 12.0]);
        assert_eq!(vertices[2], [1.0, 1.0, 13.0]);
        assert_eq!(vertices[3], [1.0, 0.0, 11.0]);
    }

    #[test]
    fn an_unpadded_end_fails() {
        assert!(read(&terrain(b"XPTS\x02\x00")).is_err());
        assert!(read(&terrain(b"XPTS\x02\x00\x00\x00")).is_err());
    }
}
