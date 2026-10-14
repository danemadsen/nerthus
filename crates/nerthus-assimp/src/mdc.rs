//! Assimp's Return to Castle Wolfenstein importer (`MDCLoader.cpp`): frame
//! 0 of a file read in place. A mesh per surface with vertices and
//! triangles, a vertex per triangle corner (base vertices scaled by 1/64
//! and moved by the frame's origin, latitude/longitude normals, texture
//! coordinates), the winding flipped. Out-of-range indices become the
//! last vertex.
//!
//! The header checks don't cover the surface chain (each surface is
//! found from the previous one's end offset) or the base frame table's
//! entry, and the base vertices of frame `n` are looked up at
//! `n * vertices * 4`: reads outside the file are undefined behaviour,
//! errors here. A surface without vertices or triangles stalls the second
//! pass (its `continue` skips the step to the next surface), leaving the
//! meshes counted after it null, which crashes.

use crate::md3::lat_lng_normal;
use crate::scene::{ImportError, Mesh, PRIMITIVE_TRIANGLE, Scene, check_allocation, fail};

const HEADER_SIZE: usize = 112;
const SURFACE_SIZE: u64 = 124;
const FRAME_SIZE: u64 = 56;

fn read_past_the_file<T>() -> Result<T, ImportError> {
    fail("undefined behaviour: a read past the file")
}

fn bytes_at<const N: usize>(data: &[u8], offset: u64) -> Result<[u8; N], ImportError> {
    if offset + N as u64 > data.len() as u64 {
        return read_past_the_file();
    }
    let start = offset as usize;
    Ok(data[start..start + N].try_into().unwrap())
}

fn word(data: &[u8], offset: u64) -> Result<u32, ImportError> {
    Ok(u32::from_le_bytes(bytes_at(data, offset)?))
}

fn float(data: &[u8], offset: u64) -> Result<f32, ImportError> {
    Ok(f32::from_le_bytes(bytes_at(data, offset)?))
}

/// `MDC::Surface`'s counts and offsets (relative to the surface).
struct Surface {
    comp_frames: u32,
    base_frames: u32,
    shaders: u32,
    vertices: u32,
    triangles: u32,
    triangles_offset: u32,
    shaders_offset: u32,
    texcoords_offset: u32,
    base_vertices_offset: u32,
    comp_vertices_offset: u32,
    base_frames_offset: u32,
    comp_frames_offset: u32,
    end_offset: u32,
}

fn read_surface(data: &[u8], start: u64) -> Result<Surface, ImportError> {
    if start + SURFACE_SIZE > data.len() as u64 {
        return read_past_the_file();
    }
    let field = |index: u64| word(data, start + 72 + index * 4);
    Ok(Surface {
        comp_frames: field(0)?,
        base_frames: field(1)?,
        shaders: field(2)?,
        vertices: field(3)?,
        triangles: field(4)?,
        triangles_offset: field(5)?,
        shaders_offset: field(6)?,
        texcoords_offset: field(7)?,
        base_vertices_offset: field(8)?,
        comp_vertices_offset: field(9)?,
        base_frames_offset: field(10)?,
        comp_frames_offset: field(11)?,
        end_offset: field(12)?,
    })
}

/// `ValidateSurfaceHeader`: every table inside what follows the surface.
fn validate_surface(surface: &Surface, start: u64, file_size: u32) -> Result<(), ImportError> {
    let room = file_size.wrapping_sub(start as u32) as u64;
    let vertices = surface.vertices as u64;
    let outside = surface.base_vertices_offset as u64 + vertices * 8 > room
        || (surface.comp_frames != 0 && surface.comp_vertices_offset as u64 + vertices * 4 > room)
        || surface.triangles_offset as u64 + surface.triangles as u64 * 12 > room
        || surface.texcoords_offset as u64 + vertices * 8 > room
        || surface.shaders_offset as u64 + surface.shaders as u64 * 68 > room
        || surface.base_frames_offset as u64 + surface.base_frames as u64 * 2 > room
        || (surface.comp_frames != 0 && surface.comp_frames_offset as u64 + surface.comp_frames as u64 * 2 > room);
    if outside {
        return fail("Some of the offset values in the MDC surface header are invalid and point somewhere behind the file.");
    }
    Ok(())
}

fn build_mesh(data: &[u8], surface: &Surface, start: u64, origin: [f32; 3]) -> Result<Mesh, ImportError> {
    let base_frame = i16::from_le_bytes(bytes_at(data, start + surface.base_frames_offset as u64)?);
    let frame_offset = (base_frame as i32 as u32).wrapping_mul(surface.vertices).wrapping_mul(4) as u64;
    let base_vertices = start + surface.base_vertices_offset as u64 + frame_offset * 8;
    let texcoords = start + surface.texcoords_offset as u64;
    let triangles = start + surface.triangles_offset as u64;

    let corner_count = surface.triangles as usize * 3;
    let mut positions = Vec::with_capacity(corner_count);
    let mut normals = Vec::with_capacity(corner_count);
    let mut coords = Vec::with_capacity(corner_count);
    let mut faces = Vec::with_capacity(surface.triangles as usize);
    for triangle in 0..surface.triangles as u64 {
        let first = positions.len() as u32;
        for corner in 0..3u64 {
            let mut index = word(data, triangles + triangle * 12 + corner * 4)?;
            if index >= surface.vertices {
                // "MDC vertex index is out of range"
                index = surface.vertices - 1;
            }
            let vertex: [u8; 8] = bytes_at(data, base_vertices + index as u64 * 8)?;
            let x = i16::from_le_bytes([vertex[0], vertex[1]]);
            let y = i16::from_le_bytes([vertex[2], vertex[3]]);
            let z = i16::from_le_bytes([vertex[4], vertex[5]]);
            let packed_normal = u16::from_le_bytes([vertex[6], vertex[7]]);
            let scale = 1.0f32 / 64.0;
            let position = [x as f32 * scale + origin[0], y as f32 * scale + origin[1], z as f32 * scale + origin[2]];
            positions.push(position);
            normals.push(lat_lng_normal(packed_normal));
            let u = float(data, texcoords + index as u64 * 8)?;
            let v = float(data, texcoords + index as u64 * 8 + 4)?;
            coords.push([u, 1.0 - v, 0.0]);
        }
        // Wound the other way round.
        faces.push(vec![first + 2, first + 1, first]);
    }
    let mut mesh = Mesh {
        primitive_types: PRIMITIVE_TRIANGLE,
        vertex_count: corner_count,
        vertices: Some(positions),
        normals: Some(normals),
        faces: Some(faces),
        ..Mesh::default()
    };
    mesh.texture_coords[0] = Some(coords);
    Ok(mesh)
}

pub fn read(data: &[u8]) -> Result<Scene, ImportError> {
    let file_size = data.len() as u32;
    if data.len() < HEADER_SIZE {
        return fail("MDC File is too small.");
    }
    let header = |index: u64| word(data, index);
    let ident = header(0)?.to_le_bytes();
    if &ident != b"IDPC" && &ident != b"CPDI" {
        return fail("Invalid MDC magic word: expected IDPC");
    }
    let frame_count = header(76)?;
    let surface_count = header(84)?;
    let frames_offset = header(92)?;
    let surfaces_offset = header(104)?;
    let outside = frames_offset as u64 + frame_count as u64 * FRAME_SIZE > file_size as u64
        || surfaces_offset as u64 + surface_count as u64 * SURFACE_SIZE > file_size as u64;
    if outside {
        return fail("Some of the offset values in the MDC header are invalid and point to something behind the file.");
    }
    if frame_count == 0 {
        return fail("The requested frame is not available");
    }
    let frame = frames_offset as u64;
    let origin = [float(data, frame + 24)?, float(data, frame + 28)?, float(data, frame + 32)?];

    // First pass: validate the surfaces and count the meshes.
    let mut mesh_count = 0usize;
    let mut shader_count = 0u32;
    let mut start = surfaces_offset as u64;
    for _ in 0..surface_count {
        let surface = read_surface(data, start)?;
        validate_surface(&surface, start, file_size)?;
        if surface.vertices != 0 && surface.triangles != 0 {
            mesh_count += 1;
        }
        shader_count = shader_count.wrapping_add(surface.shaders);
        start += surface.end_offset as u64;
    }
    check_allocation(shader_count as u64, 32)?;

    // Second pass: an empty surface isn't stepped over.
    let mut meshes = Vec::with_capacity(mesh_count);
    let mut start = surfaces_offset as u64;
    for _ in 0..surface_count {
        let surface = read_surface(data, start)?;
        if surface.vertices == 0 || surface.triangles == 0 {
            continue;
        }
        if surface.shaders != 0 {
            // The shader's name.
            bytes_at::<64>(data, start + surface.shaders_offset as u64)?;
        }
        meshes.push(build_mesh(data, &surface, start, origin)?);
        start += surface.end_offset as u64;
    }
    if mesh_count == 0 {
        return fail("Invalid MDC file: File contains no valid mesh");
    }
    if meshes.len() < mesh_count {
        return fail("undefined behaviour: null meshes after an empty surface");
    }
    Ok(Scene { meshes })
}
