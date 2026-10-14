//! Assimp's Inter-Quake Model importer (`IQMImporter.cpp`): version 2
//! files read in place through the header's offsets, which it never
//! checks. A mesh per `iqmmesh`: its triangles (wound 0, 2, 1, indices
//! less the first vertex) and the float positions, texture coordinates,
//! normals and float or byte colours of the vertex arrays.
//!
//! Any read outside the file is undefined behaviour, an error here. The
//! vertex loops step from `first_vertex * size` to `num_vertexes * size`
//! elements in 32-bit arithmetic: a product that wraps to no multiple of
//! the size never ends (also an error).

use crate::scene::{ImportError, Mesh, PRIMITIVE_TRIANGLE, Scene, check_allocation, fail};

/// `sizeof(iqmheader)`.
const HEADER_SIZE: usize = 124;
const MESH_SIZE: u64 = 24;
const VERTEX_ARRAY_SIZE: u64 = 20;
const TRIANGLE_SIZE: u64 = 12;

const POSITION: u32 = 0;
const TEXCOORD: u32 = 1;
const NORMAL: u32 = 2;
const COLOR: u32 = 6;
const UBYTE: u32 = 1;
const FLOAT: u32 = 7;

fn read_past_the_file<T>() -> Result<T, ImportError> {
    fail("undefined behaviour: a read past the file")
}

/// The little-endian word at `offset`.
fn word(data: &[u8], offset: u64) -> Result<u32, ImportError> {
    if offset + 4 > data.len() as u64 {
        return read_past_the_file();
    }
    let start = offset as usize;
    Ok(u32::from_le_bytes([data[start], data[start + 1], data[start + 2], data[start + 3]]))
}

fn float(data: &[u8], offset: u64) -> Result<f32, ImportError> {
    Ok(f32::from_bits(word(data, offset)?))
}

fn byte(data: &[u8], offset: u64) -> Result<u8, ImportError> {
    if offset >= data.len() as u64 {
        return read_past_the_file();
    }
    Ok(data[offset as usize])
}

/// The header's words after the magic.
struct Header {
    words: [u32; 27],
}

impl Header {
    fn version(&self) -> u32 {
        self.words[0]
    }
    fn file_size(&self) -> u32 {
        self.words[1]
    }
    fn ofs_text(&self) -> u32 {
        self.words[4]
    }
    fn num_meshes(&self) -> u32 {
        self.words[5]
    }
    fn ofs_meshes(&self) -> u32 {
        self.words[6]
    }
    fn num_vertexarrays(&self) -> u32 {
        self.words[7]
    }
    fn ofs_vertexarrays(&self) -> u32 {
        self.words[9]
    }
    fn ofs_triangles(&self) -> u32 {
        self.words[11]
    }
}

/// The elements a vertex loop visits: `first * step` to the element
/// count `count * step`, in 32-bit arithmetic, stepping by `step`. The
/// first element (in the array's units) and the number of steps.
fn vertex_steps(first: u32, count: u32, step: u32) -> Result<(u64, u32), ImportError> {
    let start = first.wrapping_mul(step) as u64;
    let total = count.wrapping_mul(step);
    if !total.is_multiple_of(step) {
        return fail("undefined behaviour: a vertex loop that never ends");
    }
    Ok((start, total / step))
}

/// The float triples (or pairs, or quadruples) of a float array.
fn read_floats<const N: usize>(data: &[u8], offset: u32, first: u32, count: u32, step: u32) -> Result<Vec<[f32; N]>, ImportError> {
    let (start, steps) = vertex_steps(first, count, step)?;
    let mut values = vec![[0.0f32; N]; count as usize];
    for (index, value) in values.iter_mut().take(steps as usize).enumerate() {
        let element = start + index as u64 * step as u64;
        let address = offset as u64 + element * 4;
        for (component, slot) in value.iter_mut().enumerate() {
            *slot = float(data, address + component as u64 * 4)?;
        }
    }
    Ok(values)
}

fn read_mesh(data: &[u8], header: &Header, index: u32) -> Result<Mesh, ImportError> {
    let mesh_offset = header.ofs_meshes() as u64 + index as u64 * MESH_SIZE;
    let material = word(data, mesh_offset + 4)?;
    let first_vertex = word(data, mesh_offset + 8)?;
    let vertex_count = word(data, mesh_offset + 12)?;
    let first_triangle = word(data, mesh_offset + 16)?;
    let triangle_count = word(data, mesh_offset + 20)?;

    // The material's name, a C string in the text.
    let name_start = header.ofs_text() as u64 + material as u64;
    let name_ends = name_start < data.len() as u64 && data[name_start as usize..].contains(&0);
    if !name_ends {
        return read_past_the_file();
    }

    check_allocation(triangle_count as u64, 16)?;
    let mut faces = Vec::with_capacity(triangle_count as usize);
    for triangle in 0..triangle_count as u64 {
        let offset = header.ofs_triangles() as u64 + (first_triangle as u64 + triangle) * TRIANGLE_SIZE;
        let first = word(data, offset)?;
        let second = word(data, offset + 4)?;
        let third = word(data, offset + 8)?;
        faces.push(vec![first.wrapping_sub(first_vertex), third.wrapping_sub(first_vertex), second.wrapping_sub(first_vertex)]);
    }
    let mut mesh = Mesh { primitive_types: PRIMITIVE_TRIANGLE, faces: Some(faces), ..Mesh::default() };

    for array in 0..header.num_vertexarrays() as u64 {
        let array_offset = header.ofs_vertexarrays() as u64 + array * VERTEX_ARRAY_SIZE;
        let step = word(data, array_offset + 12)?;
        let kind = word(data, array_offset)?;
        match kind {
            POSITION => {
                if word(data, array_offset + 8)? == FLOAT && step >= 3 {
                    check_allocation(vertex_count as u64, 12)?;
                    let offset = word(data, array_offset + 16)?;
                    let positions = read_floats::<3>(data, offset, first_vertex, vertex_count, step)?;
                    mesh.vertex_count = vertex_count as usize;
                    mesh.vertices = Some(positions);
                }
            }
            TEXCOORD => {
                if word(data, array_offset + 8)? == FLOAT && step >= 2 {
                    check_allocation(vertex_count as u64, 12)?;
                    let offset = word(data, array_offset + 16)?;
                    let pairs = read_floats::<2>(data, offset, first_vertex, vertex_count, step)?;
                    let (_, steps) = vertex_steps(first_vertex, vertex_count, step)?;
                    let coords = pairs
                        .iter()
                        .enumerate()
                        .map(|(index, pair)| if (index as u32) < steps { [pair[0], 1.0 - pair[1], 0.0] } else { [0.0; 3] })
                        .collect();
                    mesh.uv_components[0] = 2;
                    mesh.texture_coords[0] = Some(coords);
                }
            }
            NORMAL => {
                if word(data, array_offset + 8)? == FLOAT && step >= 3 {
                    check_allocation(vertex_count as u64, 12)?;
                    let offset = word(data, array_offset + 16)?;
                    mesh.normals = Some(read_floats::<3>(data, offset, first_vertex, vertex_count, step)?);
                }
            }
            COLOR => {
                let format = word(data, array_offset + 8)?;
                if format == UBYTE && step >= 3 {
                    check_allocation(vertex_count as u64, 16)?;
                    let offset = word(data, array_offset + 16)?;
                    let (start, steps) = vertex_steps(first_vertex, vertex_count, step)?;
                    let mut colors = vec![[0.0f32; 4]; vertex_count as usize];
                    for (index, colour) in colors.iter_mut().take(steps as usize).enumerate() {
                        let address = offset as u64 + start + index as u64 * step as u64;
                        let red = byte(data, address)? as f32 / 255.0;
                        let green = byte(data, address + 1)? as f32 / 255.0;
                        let blue = byte(data, address + 2)? as f32 / 255.0;
                        let alpha = if step == 3 { 1.0 } else { byte(data, address + 3)? as f32 / 255.0 };
                        *colour = [red, green, blue, alpha];
                    }
                    mesh.colors[0] = Some(colors);
                } else if format == FLOAT && step >= 3 {
                    check_allocation(vertex_count as u64, 16)?;
                    let offset = word(data, array_offset + 16)?;
                    let (start, steps) = vertex_steps(first_vertex, vertex_count, step)?;
                    let mut colors = vec![[0.0f32; 4]; vertex_count as usize];
                    for (index, colour) in colors.iter_mut().take(steps as usize).enumerate() {
                        let address = offset as u64 + (start + index as u64 * step as u64) * 4;
                        let red = float(data, address)?;
                        let green = float(data, address + 4)?;
                        let blue = float(data, address + 8)?;
                        let alpha = if step == 3 { 1.0 } else { float(data, address + 12)? };
                        *colour = [red, green, blue, alpha];
                    }
                    mesh.colors[0] = Some(colors);
                }
            }
            // Tangents (read nowhere), blend indices and weights, custom
            // and unknown arrays.
            _ => {}
        }
    }
    Ok(mesh)
}

pub fn read(data: &[u8]) -> Result<Scene, ImportError> {
    if data.len() < HEADER_SIZE {
        return fail("IQM-file is too small.");
    }
    let words = std::array::from_fn(|index| {
        let start = 16 + index * 4;
        u32::from_le_bytes([data[start], data[start + 1], data[start + 2], data[start + 3]])
    });
    let header = Header { words };
    if &data[..16] != b"INTERQUAKEMODEL\0" || header.version() != 2 || header.file_size() as usize != data.len() {
        return fail("Bad binary header in file.");
    }

    let mesh_count = header.num_meshes();
    check_allocation(mesh_count as u64, 8)?;
    let mut meshes = Vec::new();
    for index in 0..mesh_count {
        meshes.push(read_mesh(data, &header, index)?);
    }
    Ok(Scene { meshes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrapping_vertex_loop_stops_or_never_ends() {
        assert_eq!(vertex_steps(1, 4, 3).unwrap(), (3, 4));
        assert_eq!(vertex_steps(0, 2, 1 << 31).unwrap(), (0, 0));
        assert!(vertex_steps(0, 3, 0x6000_0000).is_err());
    }
}
