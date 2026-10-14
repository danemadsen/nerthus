//! Assimp's Quake II importer (`MD2Loader.cpp`): frame 0 of a file read
//! in place through its header's offsets. Three vertices per triangle
//! (byte coordinates scaled and moved by the frame, normals from the
//! 162-entry table), texture coordinates divided by the skin size, the
//! winding flipped. Out-of-range vertex and texture indices become the
//! last one.
//!
//! The header checks reject sections ending at the file's end (`>=`),
//! and add the frames' size in 32 bits, which can wrap: reads outside the
//! file are undefined behaviour, errors here, as is the vertex index a
//! file without vertices clamps to (`0xffffffff`) and a skin name that
//! runs past the file or 1023 bytes.

use crate::scene::{ImportError, Mesh, PRIMITIVE_TRIANGLE, Scene, fail};

/// `sizeof(MD2::Header)`.
const HEADER_SIZE: usize = 68;

/// `AI_MAX_ALLOC(type)`: 256 MiB worth of the type.
const fn max_alloc(size: u64) -> u64 {
    256 * 1024 * 1024 / size
}

/// `g_avNormals`, the Quake II vertex normals.
const NORMALS: [[f32; 3]; 162] = [
    [-0.525731, 0.000000, 0.850651],
    [-0.442863, 0.238856, 0.864188],
    [-0.295242, 0.000000, 0.955423],
    [-0.309017, 0.500000, 0.809017],
    [-0.162460, 0.262866, 0.951056],
    [0.000000, 0.000000, 1.000000],
    [0.000000, 0.850651, 0.525731],
    [-0.147621, 0.716567, 0.681718],
    [0.147621, 0.716567, 0.681718],
    [0.000000, 0.525731, 0.850651],
    [0.309017, 0.500000, 0.809017],
    [0.525731, 0.000000, 0.850651],
    [0.295242, 0.000000, 0.955423],
    [0.442863, 0.238856, 0.864188],
    [0.162460, 0.262866, 0.951056],
    [-0.681718, 0.147621, 0.716567],
    [-0.809017, 0.309017, 0.500000],
    [-0.587785, 0.425325, 0.688191],
    [-0.850651, 0.525731, 0.000000],
    [-0.864188, 0.442863, 0.238856],
    [-0.716567, 0.681718, 0.147621],
    [-0.688191, 0.587785, 0.425325],
    [-0.500000, 0.809017, 0.309017],
    [-0.238856, 0.864188, 0.442863],
    [-0.425325, 0.688191, 0.587785],
    [-0.716567, 0.681718, -0.147621],
    [-0.500000, 0.809017, -0.309017],
    [-0.525731, 0.850651, 0.000000],
    [0.000000, 0.850651, -0.525731],
    [-0.238856, 0.864188, -0.442863],
    [0.000000, 0.955423, -0.295242],
    [-0.262866, 0.951056, -0.162460],
    [0.000000, 1.000000, 0.000000],
    [0.000000, 0.955423, 0.295242],
    [-0.262866, 0.951056, 0.162460],
    [0.238856, 0.864188, 0.442863],
    [0.262866, 0.951056, 0.162460],
    [0.500000, 0.809017, 0.309017],
    [0.238856, 0.864188, -0.442863],
    [0.262866, 0.951056, -0.162460],
    [0.500000, 0.809017, -0.309017],
    [0.850651, 0.525731, 0.000000],
    [0.716567, 0.681718, 0.147621],
    [0.716567, 0.681718, -0.147621],
    [0.525731, 0.850651, 0.000000],
    [0.425325, 0.688191, 0.587785],
    [0.864188, 0.442863, 0.238856],
    [0.688191, 0.587785, 0.425325],
    [0.809017, 0.309017, 0.500000],
    [0.681718, 0.147621, 0.716567],
    [0.587785, 0.425325, 0.688191],
    [0.955423, 0.295242, 0.000000],
    [1.000000, 0.000000, 0.000000],
    [0.951056, 0.162460, 0.262866],
    [0.850651, -0.525731, 0.000000],
    [0.955423, -0.295242, 0.000000],
    [0.864188, -0.442863, 0.238856],
    [0.951056, -0.162460, 0.262866],
    [0.809017, -0.309017, 0.500000],
    [0.681718, -0.147621, 0.716567],
    [0.850651, 0.000000, 0.525731],
    [0.864188, 0.442863, -0.238856],
    [0.809017, 0.309017, -0.500000],
    [0.951056, 0.162460, -0.262866],
    [0.525731, 0.000000, -0.850651],
    [0.681718, 0.147621, -0.716567],
    [0.681718, -0.147621, -0.716567],
    [0.850651, 0.000000, -0.525731],
    [0.809017, -0.309017, -0.500000],
    [0.864188, -0.442863, -0.238856],
    [0.951056, -0.162460, -0.262866],
    [0.147621, 0.716567, -0.681718],
    [0.309017, 0.500000, -0.809017],
    [0.425325, 0.688191, -0.587785],
    [0.442863, 0.238856, -0.864188],
    [0.587785, 0.425325, -0.688191],
    [0.688191, 0.587785, -0.425325],
    [-0.147621, 0.716567, -0.681718],
    [-0.309017, 0.500000, -0.809017],
    [0.000000, 0.525731, -0.850651],
    [-0.525731, 0.000000, -0.850651],
    [-0.442863, 0.238856, -0.864188],
    [-0.295242, 0.000000, -0.955423],
    [-0.162460, 0.262866, -0.951056],
    [0.000000, 0.000000, -1.000000],
    [0.295242, 0.000000, -0.955423],
    [0.162460, 0.262866, -0.951056],
    [-0.442863, -0.238856, -0.864188],
    [-0.309017, -0.500000, -0.809017],
    [-0.162460, -0.262866, -0.951056],
    [0.000000, -0.850651, -0.525731],
    [-0.147621, -0.716567, -0.681718],
    [0.147621, -0.716567, -0.681718],
    [0.000000, -0.525731, -0.850651],
    [0.309017, -0.500000, -0.809017],
    [0.442863, -0.238856, -0.864188],
    [0.162460, -0.262866, -0.951056],
    [0.238856, -0.864188, -0.442863],
    [0.500000, -0.809017, -0.309017],
    [0.425325, -0.688191, -0.587785],
    [0.716567, -0.681718, -0.147621],
    [0.688191, -0.587785, -0.425325],
    [0.587785, -0.425325, -0.688191],
    [0.000000, -0.955423, -0.295242],
    [0.000000, -1.000000, 0.000000],
    [0.262866, -0.951056, -0.162460],
    [0.000000, -0.850651, 0.525731],
    [0.000000, -0.955423, 0.295242],
    [0.238856, -0.864188, 0.442863],
    [0.262866, -0.951056, 0.162460],
    [0.500000, -0.809017, 0.309017],
    [0.716567, -0.681718, 0.147621],
    [0.525731, -0.850651, 0.000000],
    [-0.238856, -0.864188, -0.442863],
    [-0.500000, -0.809017, -0.309017],
    [-0.262866, -0.951056, -0.162460],
    [-0.850651, -0.525731, 0.000000],
    [-0.716567, -0.681718, -0.147621],
    [-0.716567, -0.681718, 0.147621],
    [-0.525731, -0.850651, 0.000000],
    [-0.500000, -0.809017, 0.309017],
    [-0.238856, -0.864188, 0.442863],
    [-0.262866, -0.951056, 0.162460],
    [-0.864188, -0.442863, 0.238856],
    [-0.809017, -0.309017, 0.500000],
    [-0.688191, -0.587785, 0.425325],
    [-0.681718, -0.147621, 0.716567],
    [-0.442863, -0.238856, 0.864188],
    [-0.587785, -0.425325, 0.688191],
    [-0.309017, -0.500000, 0.809017],
    [-0.147621, -0.716567, 0.681718],
    [-0.425325, -0.688191, 0.587785],
    [-0.162460, -0.262866, 0.951056],
    [0.442863, -0.238856, 0.864188],
    [0.162460, -0.262866, 0.951056],
    [0.309017, -0.500000, 0.809017],
    [0.147621, -0.716567, 0.681718],
    [0.000000, -0.525731, 0.850651],
    [0.425325, -0.688191, 0.587785],
    [0.587785, -0.425325, 0.688191],
    [0.688191, -0.587785, 0.425325],
    [-0.955423, 0.295242, 0.000000],
    [-0.951056, 0.162460, 0.262866],
    [-1.000000, 0.000000, 0.000000],
    [-0.850651, 0.000000, 0.525731],
    [-0.955423, -0.295242, 0.000000],
    [-0.951056, -0.162460, 0.262866],
    [-0.864188, 0.442863, -0.238856],
    [-0.951056, 0.162460, -0.262866],
    [-0.809017, 0.309017, -0.500000],
    [-0.864188, -0.442863, -0.238856],
    [-0.951056, -0.162460, -0.262866],
    [-0.809017, -0.309017, -0.500000],
    [-0.681718, 0.147621, -0.716567],
    [-0.681718, -0.147621, -0.716567],
    [-0.850651, 0.000000, -0.525731],
    [-0.688191, 0.587785, -0.425325],
    [-0.587785, 0.425325, -0.688191],
    [-0.425325, 0.688191, -0.587785],
    [-0.425325, -0.688191, -0.587785],
    [-0.587785, -0.425325, -0.688191],
    [-0.688191, -0.587785, -0.425325],
];

/// `MD2::LookupNormalIndex`: past the table, its last entry.
pub(crate) fn lookup_normal(index: u8) -> [f32; 3] {
    NORMALS[(index as usize).min(NORMALS.len() - 1)]
}

fn word(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]])
}

fn read_past_the_file<T>() -> Result<T, ImportError> {
    fail("undefined behaviour: a read past the file")
}

/// The little-endian `u16` at `offset`, which must lie in the file.
fn half(data: &[u8], offset: u64) -> Result<u16, ImportError> {
    if offset + 2 > data.len() as u64 {
        return read_past_the_file();
    }
    let start = offset as usize;
    Ok(u16::from_le_bytes([data[start], data[start + 1]]))
}

fn float(data: &[u8], offset: u64) -> Result<f32, ImportError> {
    if offset + 4 > data.len() as u64 {
        return read_past_the_file();
    }
    Ok(f32::from_bits(word(data, offset as usize)))
}

fn byte(data: &[u8], offset: u64) -> Result<u8, ImportError> {
    if offset >= data.len() as u64 {
        return read_past_the_file();
    }
    Ok(data[offset as usize])
}

pub fn read(data: &[u8]) -> Result<Scene, ImportError> {
    let file_size = data.len() as u32;
    if data.len() < HEADER_SIZE {
        return fail("MD2 File is too small");
    }
    let header: [u32; 17] = std::array::from_fn(|index| word(data, index * 4));
    let [magic, _version, skin_width, skin_height, _frame_size, skin_count, vertex_count, texcoord_count, triangle_count, _gl_command_count, frame_count, skins_offset, texcoords_offset, triangles_offset, frames_offset, _gl_commands_offset, end_offset] = header;

    if &magic.to_le_bytes() != b"IDP2" && &magic.to_le_bytes() != b"2PDI" {
        return fail("Invalid MD2 magic word: expected IDP2");
    }
    if frame_count == 0 {
        return fail("Invalid MD2 file: NUM_FRAMES is 0");
    }
    if end_offset > file_size {
        return fail("Invalid MD2 file: File is too small");
    }
    let too_many = skin_count as u64 > max_alloc(64)
        || vertex_count as u64 > max_alloc(4)
        || texcoord_count as u64 > max_alloc(4)
        || triangle_count as u64 > max_alloc(12)
        || frame_count as u64 > max_alloc(44);
    if too_many {
        return fail("Invalid MD2 header: too many elements, would overflow");
    }
    let frame_size = 44u32.wrapping_add(vertex_count.wrapping_sub(1).wrapping_mul(4));
    let frames_end = frames_offset.wrapping_add(frame_count.wrapping_mul(frame_size));
    let outside = skins_offset as u64 + skin_count as u64 * 64 >= file_size as u64
        || texcoords_offset as u64 + texcoord_count as u64 * 4 >= file_size as u64
        || triangles_offset as u64 + triangle_count as u64 * 12 >= file_size as u64
        || frames_end >= file_size
        || end_offset > file_size;
    if outside {
        return fail("Invalid MD2 header: Some offsets are outside the file");
    }

    if texcoord_count != 0 && skin_count != 0 && byte(data, skins_offset as u64)? != 0 {
        // The skin name, copied with `strlen` into an `aiString`.
        let start = skins_offset as usize;
        let Some(length) = data[start..].iter().position(|&value| value == 0) else {
            return read_past_the_file();
        };
        if length >= 1024 {
            return fail("undefined behaviour: a skin name overflowing its aiString");
        }
    }

    let frame = frames_offset as u64;
    let scale = [float(data, frame)?, float(data, frame + 4)?, float(data, frame + 8)?];
    let translate = [float(data, frame + 12)?, float(data, frame + 16)?, float(data, frame + 20)?];
    let first_vertex = frame + 40;

    let mut divisor_u = 1.0f32;
    let mut divisor_v = 1.0f32;
    if texcoord_count != 0 {
        // A zero skin size leaves the divisor at 1.
        if skin_width != 0 {
            divisor_u = skin_width as f32;
        }
        if skin_height != 0 {
            divisor_v = skin_height as f32;
        }
    }

    let corner_count = triangle_count as usize * 3;
    let mut vertices = Vec::with_capacity(corner_count);
    let mut normals = Vec::with_capacity(corner_count);
    let mut coords = Vec::with_capacity(if texcoord_count != 0 { corner_count } else { 0 });
    let mut faces = Vec::with_capacity(triangle_count as usize);
    for triangle in 0..triangle_count as u64 {
        let triangle_offset = triangles_offset as u64 + triangle * 12;
        let first = vertices.len() as u32;
        for corner in 0..3u64 {
            let mut index = half(data, triangle_offset + corner * 2)? as u32;
            if index >= vertex_count {
                // "MD2: Vertex index is outside the allowed range"
                index = vertex_count.wrapping_sub(1);
            }
            let vertex = first_vertex + index as u64 * 4;
            let mut position = [0.0f32; 3];
            for axis in 0..3 {
                let value = byte(data, vertex + axis as u64)? as f32 * scale[axis];
                position[axis] = value + translate[axis];
            }
            vertices.push(position);
            normals.push(lookup_normal(byte(data, vertex + 3)?));
            if texcoord_count != 0 {
                let mut texcoord = half(data, triangle_offset + 6 + corner * 2)? as u32;
                if texcoord >= texcoord_count {
                    // "MD2: UV index is outside the allowed range"
                    texcoord = texcoord_count - 1;
                }
                let texcoord_offset = texcoords_offset as u64 + texcoord as u64 * 4;
                let s = half(data, texcoord_offset)? as f32;
                let t = half(data, texcoord_offset + 2)? as f32;
                coords.push([s / divisor_u, 1.0 - t / divisor_v, 0.0]);
            }
        }
        // Wound the other way round.
        faces.push(vec![first + 2, first + 1, first]);
    }

    let mut mesh = Mesh {
        primitive_types: PRIMITIVE_TRIANGLE,
        vertex_count: corner_count,
        vertices: Some(vertices),
        normals: Some(normals),
        faces: Some(faces),
        ..Mesh::default()
    };
    if texcoord_count != 0 {
        mesh.uv_components[0] = 2;
        mesh.texture_coords[0] = Some(coords);
    }
    Ok(Scene { meshes: vec![mesh] })
}
