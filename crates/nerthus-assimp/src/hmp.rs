//! 3D GameStudio heightmaps (`HMPImporter`, an MDL importer): HMP5 and
//! HMP7 terrains, a grid of heights made into a quad per cell. The skins
//! are MDL7 ones (the first parsed, the others skipped).
//!
//! The file's copy has no NUL after it here; positions follow the MDL
//! importer's rules (see `mdl`).

use crate::io::FileSystem;
use crate::md2::lookup_normal;
use crate::mdl::Buffer;
use crate::mdl::skins::{self, Skins};
use crate::scene::{ImportError, Mesh, Scene, check_allocation, fail};
use crate::vector::{Vector3, normalize};

/// `sizeof(aiVector3D)` and `sizeof(aiFace)`.
const VECTOR_SIZE: u64 = 12;
const FACE_SIZE: u64 = 16;

/// Where the skins start (past `Header_HMP5`), and the bytes from their
/// end to the vertices.
const SKINS_START: i64 = 84;
const FRAME_HEADER_SIZE: i64 = 36;

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

/// `Header_HMP5`, as far as it is read, validated
/// (`ValidateHeader_HMP457`).
struct Header {
    triangle_size_x: f32,
    triangle_size_y: f32,
    skin_count: i32,
    vertex_count: i32,
    width: u32,
    height: u32,
}

impl Header {
    fn read(buffer: &Buffer) -> Result<Header, ImportError> {
        if buffer.size() < 120 {
            return fail("HMP file is too small (header size is 120 bytes, this file is smaller)");
        }
        let triangle_size_x = buffer.f32(36)?;
        let triangle_size_y = buffer.f32(40)?;
        if triangle_size_x == 0.0 || triangle_size_y == 0.0 {
            return fail("Size of triangles in either  x or y direction is zero");
        }
        let vertices_x = buffer.f32(44)?;
        let vertex_count = buffer.i32(60)?;
        let rows = vertex_count as f32 / vertices_x;
        if vertices_x < 1.0 || rows < 1.0 {
            return fail("Number of triangles in either x or y direction is zero");
        }
        if buffer.i32(68)? == 0 {
            return fail("There are no frames. At least one should be there");
        }
        Ok(Header {
            triangle_size_x,
            triangle_size_y,
            skin_count: buffer.i32(48)?,
            vertex_count,
            width: vertices_x as u32,
            height: rows as u32,
        })
    }

    /// The number of grid points the vertex arrays hold.
    fn capacity(&self) -> u64 {
        self.vertex_count as i64 as u64
    }
}

/// `ReadFirstSkin`: a type (after 12 more bytes when it is 0), width and
/// height, the skin, then the other skins skipped.
fn read_first_skin(buffer: &Buffer, skins: &mut Skins, count: u32, position: i64) -> Result<i64, ImportError> {
    let mut kind = buffer.u32(position)?;
    let mut position = position + 4;
    if kind == 0 {
        position += 8;
        kind = buffer.u32(position)?;
        position += 4;
        if kind == 0 {
            return fail("Unable to read HMP7 skin chunk");
        }
    }
    let width = buffer.u32(position)?;
    let height = buffer.u32(position + 4)?;
    position += 8;
    if let Some(next) = skins::parse_skin_lump(buffer, skins, position, kind, width, height)? {
        position = next;
    }
    for _ in 1..count {
        buffer.size_check(position.wrapping_add(12))?;
        let kind = buffer.u32(position)?;
        let width = buffer.u32(position + 4)?;
        let height = buffer.u32(position + 8)?;
        let next = skins::skip_skin_lump(buffer, skins, position.wrapping_add(12), kind, width, height)?;
        buffer.size_check(next)?;
        // A skin that ends where it began is skipped the same way again
        // and again.
        if next == position {
            break;
        }
        position = next;
    }
    Ok(position)
}

/// The grid's points (`InternReadFile_HMP5`/`_HMP7`): heights scaled from
/// 16 bits around the middle, HMP5 normals from the Quake II table, HMP7
/// ones from two signed bytes and an up component.
fn read_points(buffer: &Buffer, header: &Header, version: u32, start: i64) -> Result<(Vec<Vector3>, Vec<Vector3>), ImportError> {
    let count = header.width as u64 * header.height as u64;
    buffer.size_check(start.wrapping_add((4 * count) as i64))?;
    if count > header.capacity() {
        return undefined("grid points written past the vertex array");
    }
    let mut positions = Vec::with_capacity(count as usize);
    let mut normals = Vec::with_capacity(count as usize);
    let mut source = start;
    for y in 0..header.height {
        for x in 0..header.width {
            let height = buffer.u16(source)? as f32;
            let z = ((height / 65535.0) - 0.5) * header.triangle_size_x * 8.0;
            positions.push([x as f32 * header.triangle_size_x, y as f32 * header.triangle_size_y, z]);
            let normal = if version == 5 {
                lookup_normal(buffer.u8(source + 2)?)
            } else {
                let normal_x = buffer.i8(source + 2)? as f32 / 128.0;
                let normal_y = buffer.i8(source + 3)? as f32 / 128.0;
                normalize([normal_x, normal_y, 1.0])
            };
            normals.push(normal);
            source += 4;
        }
    }
    Ok((positions, normals))
}

/// `GenerateTextureCoords`: the grid's corners spread over a little more
/// than the unit square.
fn texture_coords(header: &Header) -> Vec<Vector3> {
    let width = header.width;
    let height = header.height;
    let mut coordinates = Vec::with_capacity(width as usize * height as usize);
    if height == 0 || width == 0 {
        return coordinates;
    }
    let step_y = (1.0 / height as f32) + (1.0 / height as f32) / height as f32;
    let step_x = (1.0 / width as f32) + (1.0 / width as f32) / width as f32;
    for y in 0..height {
        for x in 0..width {
            coordinates.push([step_x * x as f32, step_y * y as f32, 0.0]);
        }
    }
    coordinates
}

/// `CreateOutputFaceList`: a quad of four new vertices per grid cell.
fn quads(header: &Header, positions: &[Vector3], normals: &[Vector3], coordinates: Option<&[Vector3]>) -> Result<Mesh, ImportError> {
    let width = header.width;
    let height = header.height;
    let face_count = width.wrapping_sub(1).wrapping_mul(height.wrapping_sub(1));
    check_allocation(face_count as u64, FACE_SIZE)?;
    let vertex_count = face_count.wrapping_mul(4);
    check_allocation(vertex_count as u64, VECTOR_SIZE)?;

    let mut vertices = Vec::with_capacity(vertex_count as usize);
    let mut quad_normals = Vec::with_capacity(vertex_count as usize);
    let mut quad_coordinates = coordinates.map(|_| Vec::with_capacity(vertex_count as usize));
    let mut faces = Vec::with_capacity(face_count as usize);
    let mut corner = 0u32;
    for y in 0..height.wrapping_sub(1) {
        let row = y.wrapping_mul(width) as usize;
        let next_row = y.wrapping_add(1).wrapping_mul(width) as usize;
        for x in 0..width.wrapping_sub(1) {
            let x = x as usize;
            if row + x + 1 >= vertex_count as usize || next_row + x + 1 >= vertex_count as usize {
                return undefined("a quad left with uninitialised indices");
            }
            let points = [row + x, next_row + x, next_row + x + 1, row + x + 1];
            for point in points {
                let (Some(&position), Some(&normal)) = (positions.get(point), normals.get(point)) else {
                    return undefined("a grid point read past the vertex array");
                };
                vertices.push(position);
                quad_normals.push(normal);
                if let (Some(source), Some(target)) = (coordinates, quad_coordinates.as_mut()) {
                    let Some(&coordinate) = source.get(point) else {
                        return undefined("a grid point read past the texture coordinates");
                    };
                    target.push(coordinate);
                }
            }
            faces.push(vec![corner, corner + 1, corner + 2, corner + 3]);
            corner += 4;
        }
    }

    let mut mesh = Mesh {
        vertex_count: vertex_count as usize,
        vertices: Some(vertices),
        normals: Some(quad_normals),
        faces: Some(faces),
        ..Mesh::default()
    };
    if let Some(coordinates) = quad_coordinates {
        mesh.uv_components[0] = 2;
        mesh.texture_coords[0] = Some(coordinates);
    }
    Ok(mesh)
}

/// `InternReadFile_HMP5` and `InternReadFile_HMP7`.
fn read_terrain(buffer: &Buffer, skins: &mut Skins, version: u32) -> Result<Scene, ImportError> {
    let header = Header::read(buffer)?;
    check_allocation(header.capacity(), VECTOR_SIZE)?;

    // `CreateMaterial`: with skins, texture coordinates and the first skin.
    let mut position = SKINS_START;
    if header.skin_count != 0 {
        check_allocation(header.capacity(), VECTOR_SIZE)?;
        position = read_first_skin(buffer, skins, header.skin_count as u32, position)?;
    }
    position = position.wrapping_add(FRAME_HEADER_SIZE);

    let (positions, normals) = read_points(buffer, &header, version, position)?;
    let coordinates = (header.skin_count != 0).then(|| texture_coords(&header));
    let mesh = quads(&header, &positions, &normals, coordinates.as_deref())?;
    Ok(Scene { meshes: vec![mesh] })
}

/// `HMPImporter::InternReadFile`.
pub fn read(path: &str, bytes: &[u8], files: &FileSystem) -> Result<Scene, ImportError> {
    if bytes.len() < 50 {
        return fail("HMP File is too small.");
    }
    let buffer = Buffer::new(bytes, false);
    let mut skins = Skins::new(files);
    let magic = &bytes[..4];
    let is = |forward: &[u8; 4]| {
        let mut backward = *forward;
        backward.reverse();
        magic == forward || magic == backward
    };
    if is(b"HMP4") {
        return fail("HMP4 is currently not supported");
    }
    if is(b"HMP5") {
        return read_terrain(&buffer, &mut skins, 5);
    }
    if is(b"HMP7") {
        return read_terrain(&buffer, &mut skins, 7);
    }
    fail(format!("Unknown HMP subformat {path}."))
}
