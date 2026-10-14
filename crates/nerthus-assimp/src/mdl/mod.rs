//! Quake 1 and 3D GameStudio models (`MDLImporter`): Quake 1 (and MDL2)
//! models, MDL3 to MDL5 ones, MDL7 ones (`mdl7`) and Half-Life 1 ones
//! (`half_life`).
//!
//! The importer reads a copy of the file with a NUL after it through
//! pointers it checks against the file's end only here and there
//! (`SizeCheck`, which lets a position reach the end). Cursor arithmetic
//! is kept as the C++ does it (`unsigned int` sizes, `int` counts added to
//! pointers); a read outside the copy is undefined behaviour, an error
//! here.

mod half_life;
mod mdl7;
pub(crate) mod skins;

use crate::io::FileSystem;
use crate::md2::lookup_normal;
use crate::scene::{ImportError, Mesh, PRIMITIVE_TRIANGLE, Scene, check_allocation, fail};
use crate::vector::Vector3;
use skins::{Skins, Texels, Texture};

/// `sizeof(MDL::HalfLife::SequenceHeader_HL1)`: the smallest file read.
const SMALLEST_FILE: usize = 76;

/// `sizeof(MDL::Header)`.
const HEADER_SIZE: i64 = 84;

/// `sizeof(aiVector3D)` and `sizeof(aiFace)`.
const VECTOR_SIZE: u64 = 12;
const FACE_SIZE: u64 = 16;

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

/// The importer's copy of a file (`mBuffer`): the file's bytes, then for
/// MDL files a NUL. Positions are offsets from its start; they may lie
/// anywhere, as the pointers may.
pub(crate) struct Buffer {
    bytes: Vec<u8>,
    size: u64,
}

impl Buffer {
    /// The copy of `file`, with a NUL after it when `terminated`.
    pub(crate) fn new(file: &[u8], terminated: bool) -> Buffer {
        let mut bytes = file.to_vec();
        if terminated {
            bytes.push(0);
        }
        Buffer { bytes, size: file.len() as u64 }
    }

    /// `iFileSize`.
    pub(crate) fn size(&self) -> u64 {
        self.size
    }

    /// `SizeCheck` (`VALIDATE_FILE_SIZE`): a position inside the file or at
    /// its end.
    pub(crate) fn size_check(&self, position: i64) -> Result<(), ImportError> {
        if position < 0 || position as u64 > self.size {
            return fail("Invalid MDL file. The file is too small or contains invalid data.");
        }
        Ok(())
    }

    /// `count` bytes at `position`, which must lie inside the copy.
    pub(crate) fn bytes(&self, position: i64, count: usize) -> Result<&[u8], ImportError> {
        let inside = position >= 0 && (position as u64).checked_add(count as u64).is_some_and(|end| end <= self.bytes.len() as u64);
        if !inside {
            return undefined("a read past the file");
        }
        let start = position as usize;
        Ok(&self.bytes[start..start + count])
    }

    fn array<const N: usize>(&self, position: i64) -> Result<[u8; N], ImportError> {
        let bytes = self.bytes(position, N)?;
        let mut array = [0u8; N];
        array.copy_from_slice(bytes);
        Ok(array)
    }

    pub(crate) fn u8(&self, position: i64) -> Result<u8, ImportError> {
        Ok(self.array::<1>(position)?[0])
    }

    pub(crate) fn i8(&self, position: i64) -> Result<i8, ImportError> {
        Ok(self.u8(position)? as i8)
    }

    pub(crate) fn u16(&self, position: i64) -> Result<u16, ImportError> {
        Ok(u16::from_le_bytes(self.array(position)?))
    }

    pub(crate) fn i16(&self, position: i64) -> Result<i16, ImportError> {
        Ok(i16::from_le_bytes(self.array(position)?))
    }

    pub(crate) fn u32(&self, position: i64) -> Result<u32, ImportError> {
        Ok(u32::from_le_bytes(self.array(position)?))
    }

    pub(crate) fn i32(&self, position: i64) -> Result<i32, ImportError> {
        Ok(i32::from_le_bytes(self.array(position)?))
    }

    pub(crate) fn f32(&self, position: i64) -> Result<f32, ImportError> {
        Ok(f32::from_le_bytes(self.array(position)?))
    }

    /// `strlen` at `position`: a NUL must follow inside the copy.
    pub(crate) fn c_string_length(&self, position: i64) -> Result<usize, ImportError> {
        if position < 0 || position as u64 >= self.bytes.len() as u64 {
            return undefined("a string read past the file");
        }
        let tail = &self.bytes[position as usize..];
        match tail.iter().position(|&byte| byte == 0) {
            Some(length) => Ok(length),
            None => undefined("a string read past the file"),
        }
    }

    /// A write into the copy (MDL7 clamps indices in place).
    pub(crate) fn set_u16(&mut self, position: i64, value: u16) -> Result<(), ImportError> {
        self.bytes(position, 2)?;
        let start = position as usize;
        self.bytes[start..start + 2].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }
}

/// `MDL::Header` (Quake 1, MDL2 to MDL5), as far as it is read.
struct Header {
    scale: Vector3,
    translate: Vector3,
    num_skins: i32,
    skin_width: i32,
    skin_height: i32,
    num_verts: i32,
    num_tris: i32,
    num_frames: i32,
}

impl Header {
    fn read(buffer: &Buffer) -> Result<Header, ImportError> {
        let vector = |offset: i64| -> Result<Vector3, ImportError> {
            Ok([buffer.f32(offset)?, buffer.f32(offset + 4)?, buffer.f32(offset + 8)?])
        };
        Ok(Header {
            scale: vector(8)?,
            translate: vector(20)?,
            num_skins: buffer.i32(48)?,
            skin_width: buffer.i32(52)?,
            skin_height: buffer.i32(56)?,
            num_verts: buffer.i32(60)?,
            num_tris: buffer.i32(64)?,
            num_frames: buffer.i32(68)?,
        })
    }

    /// `ValidateHeader_Quake1` (the rest only warns).
    fn validate(&self) -> Result<(), ImportError> {
        if self.num_frames == 0 {
            return fail("[Quake 1 MDL] There are no frames in the file");
        }
        if self.num_verts == 0 {
            return fail("[Quake 1 MDL] There are no vertices in the file");
        }
        if self.num_tris == 0 {
            return fail("[Quake 1 MDL] There are no triangles in the file");
        }
        Ok(())
    }

    /// A vertex index past the list becomes its last one.
    fn clamp_vertex(&self, index: u32) -> u32 {
        if index >= self.num_verts as u32 { self.num_verts.wrapping_sub(1) as u32 } else { index }
    }

    /// The scaled and moved position of a packed vertex.
    fn position(&self, packed: [f32; 3]) -> Vector3 {
        let mut position = [0.0f32; 3];
        for axis in 0..3 {
            position[axis] = packed[axis] * self.scale[axis];
            position[axis] += self.translate[axis];
        }
        position
    }
}

/// A position plus `count` items of `size` bytes, as `size_t` (or `int`)
/// arithmetic on a pointer does it.
fn advance(position: i64, size: i64, count: i64) -> i64 {
    position.wrapping_add(size.wrapping_mul(count))
}

/// The triangle mesh both Quake 1 readers build: `num_tris × 3` corners,
/// faces reversed.
struct TriangleMesh {
    vertices: Vec<Vector3>,
    normals: Vec<Vector3>,
    texture_coords: Option<Vec<Vector3>>,
    faces: Vec<Vec<u32>>,
}

impl TriangleMesh {
    fn new(num_tris: i32, texture_coords: bool) -> Result<TriangleMesh, ImportError> {
        let face_count = num_tris as u32;
        let vertex_count = num_tris.wrapping_mul(3) as u32;
        check_allocation(face_count as u64, FACE_SIZE)?;
        check_allocation(vertex_count as u64, VECTOR_SIZE)?;
        Ok(TriangleMesh {
            vertices: Vec::with_capacity(vertex_count as usize),
            normals: Vec::with_capacity(vertex_count as usize),
            texture_coords: texture_coords.then(|| Vec::with_capacity(vertex_count as usize)),
            faces: Vec::with_capacity(face_count as usize),
        })
    }

    fn into_mesh(self, num_tris: i32) -> Mesh {
        let vertex_count = num_tris.wrapping_mul(3) as u32 as usize;
        let mut mesh = Mesh {
            primitive_types: PRIMITIVE_TRIANGLE,
            vertex_count,
            vertices: Some(self.vertices),
            normals: Some(self.normals),
            faces: Some(self.faces),
            ..Mesh::default()
        };
        if let Some(coordinates) = self.texture_coords {
            mesh.uv_components[0] = 2;
            mesh.texture_coords[0] = Some(coordinates);
        }
        mesh
    }
}

/// `SetupMaterialProperties_3DGS_MDL5_Quake1`: the first texture is
/// dropped when all its texels are equal (it becomes the colour).
fn setup_material(header: &Header, textures: &mut Vec<Texture>) -> Result<(), ImportError> {
    if header.num_skins == 0 || textures.is_empty() {
        return Ok(());
    }
    if skins::is_single_color(&textures[0])? {
        textures.clear();
    }
    Ok(())
}

/// `InternReadFile_Quake1` (also MDL2): skins (group skins and single
/// ones), texture coordinates, triangles, then the first frame's vertices.
fn read_quake1(buffer: &Buffer, skins: &mut Skins) -> Result<Scene, ImportError> {
    let header = Header::read(buffer)?;
    header.validate()?;

    let mut position = HEADER_SIZE;
    let mut textures = Vec::new();
    let skin_count = header.num_skins as u32;
    for skin in 0..skin_count {
        // (`sizeof(MDL::Skin)`: an `int32_t` and a pointer.)
        if position.wrapping_add(12) > buffer.size() as i64 {
            return fail("[Quake 1 MDL] Unexpected EOF");
        }
        let group = buffer.i32(position)?;
        if group == 1 {
            let images = buffer.i32(position + 4)? as u32;
            position = position.wrapping_add(8);
            if images == 0 {
                continue;
            }
            if skin == 0 {
                let data = advance(position, 4, images as i64);
                let texture = skins::create_texture_mdl3(buffer, skins, data, header.skin_width, header.skin_height)?;
                textures.push(texture);
            }
            let (width, height) = (header.skin_width, header.skin_height);
            if width == 0 && height == 0 {
                continue;
            }
            // `INT_MAX / width`, then `INT_MAX / height` (a zero traps).
            if width == 0 {
                return undefined("a division by a zero skin width");
            }
            let mut overflow = height > i32::MAX / width;
            if !overflow {
                if height == 0 {
                    return undefined("a division by a zero skin height");
                }
                overflow = width > i32::MAX / height;
            }
            if !overflow {
                let pixels = height.wrapping_mul(width) as i64;
                position = position.wrapping_add(pixels).wrapping_add(4 * images as i64);
            }
        } else {
            position = position.wrapping_add(4);
            let mut skip = if skin != 0 { u32::MAX } else { 0 };
            let texture = skins::create_texture_mdl4(buffer, skins, position, group as u32, &mut skip, header.skin_width, header.skin_height)?;
            textures.extend(texture);
            position = position.wrapping_add(skip as i64);
        }
    }

    let texture_coords = position;
    position = advance(position, 12, header.num_verts as i64);
    let triangles = position;
    position = advance(position, 16, header.num_tris as i64);
    buffer.size_check(position)?;

    // A group frame starts with its frame count and times.
    let frame_type = buffer.i32(position)?;
    let first_frame = if frame_type == 0 {
        position.wrapping_add(4)
    } else {
        let frame_count = buffer.i32(position + 4)?;
        advance(position.wrapping_add(16), 4, frame_count as i64)
    };
    let vertices = first_frame.wrapping_add(24);
    buffer.size_check(advance(vertices, 4, header.num_verts as i64))?;

    setup_material(&header, &mut textures)?;

    let mut mesh = TriangleMesh::new(header.num_tris, true)?;
    let mut corner = 0u32;
    for triangle in 0..header.num_tris as u32 {
        let start = advance(triangles, 16, triangle as i64);
        for slot in 0..3 {
            let raw = buffer.i32(start + 4 + 4 * slot)? as u32;
            let index = header.clamp_vertex(raw) as i64;

            let vertex = advance(vertices, 4, index);
            let packed = buffer.bytes(vertex, 4)?;
            let coordinates = [packed[0] as f32, packed[1] as f32, packed[2] as f32];
            mesh.vertices.push(header.position(coordinates));
            mesh.normals.push(lookup_normal(packed[3]));

            // (The seam flag is read only for a back face.)
            let coordinate = advance(texture_coords, 12, index);
            let mut s = buffer.i32(coordinate + 4)? as f32;
            let t = buffer.i32(coordinate + 8)? as f32;
            if buffer.i32(start)? == 0 && buffer.i32(coordinate)? != 0 {
                s += header.skin_width as f32 * 0.5;
            }
            let u = (s + 0.5) / header.skin_width as f32;
            let v = 1.0 - (t + 0.5) / header.skin_height as f32;
            if let Some(coordinates) = mesh.texture_coords.as_mut() {
                coordinates.push([u, v, 0.0]);
            }
        }
        mesh.faces.push(vec![corner + 2, corner + 1, corner]);
        corner += 3;
    }
    Ok(Scene { meshes: vec![mesh.into_mesh(header.num_tris)] })
}

/// `CalculateUVCoordinates_MDL5`: texture coordinates divided by the first
/// texture's size (a DDS file's from its header), then flipped.
fn scale_mdl5_coordinates(header: &Header, textures: &[Texture], mesh: &mut Mesh) -> Result<(), ImportError> {
    if header.num_skins == 0 || textures.is_empty() {
        return Ok(());
    }
    // A texture of height 0 is taken for a DDS file: its size is read
    // from the DDS header (a texture of texels has none of them then).
    let texture = &textures[0];
    let (width, height) = if texture.height == 0 {
        let data: &[u8] = match &texture.texels {
            Texels::Dds(bytes) => bytes,
            _ => &[],
        };
        let Some(size) = data.get(12..20) else {
            return undefined("a DDS header read past the texture");
        };
        let height = u32::from_le_bytes([size[0], size[1], size[2], size[3]]);
        let width = u32::from_le_bytes([size[4], size[5], size[6], size[7]]);
        if height == 0 || width == 0 { (1, 1) } else { (width, height) }
    } else {
        (texture.width, texture.height)
    };
    if width == 1 && height == 1 {
        return Ok(());
    }
    let width = width as f32;
    let height = height as f32;
    let count = mesh.vertex_count;
    if let Some(coordinates) = mesh.texture_coords[0].as_mut() {
        for coordinate in coordinates.iter_mut().take(count) {
            coordinate[0] /= width;
            coordinate[1] /= height;
            coordinate[1] = 1.0 - coordinate[1];
        }
    }
    Ok(())
}

/// `InternReadFile_3DGS_MDL345`: skins (MDL5 ones with their own size),
/// texture coordinates (as many as `synctype`), triangles, then the first
/// frame's byte or short packed vertices.
fn read_mdl345(buffer: &Buffer, skins: &mut Skins, version: u32) -> Result<Scene, ImportError> {
    let header = Header::read(buffer)?;
    header.validate()?;
    let sync_type = buffer.i32(72)?;

    let mut position = HEADER_SIZE;
    let mut textures = Vec::new();
    for skin in 0..header.num_skins as u32 {
        if position.wrapping_add(4) > buffer.size() as i64 {
            return fail("Texture data past end of file.");
        }
        let group = buffer.i32(position)? as u32;
        let mut skip = if skin != 0 { u32::MAX } else { 0 };
        let data = position.wrapping_add(4);
        let texture = if version >= 5 {
            skins::create_texture_mdl5(buffer, skins, data, group, &mut skip)?
        } else {
            skins::create_texture_mdl4(buffer, skins, data, group, &mut skip, header.skin_width, header.skin_height)?
        };
        textures.extend(texture);
        position = position.wrapping_add(skip as i64 + 4);
    }

    let texture_coords = position;
    position = advance(position, 4, sync_type as i64);
    let triangles = position;
    position = advance(position, 12, header.num_tris as i64);
    buffer.size_check(position)?;

    setup_material(&header, &mut textures)?;

    let mut mesh = TriangleMesh::new(header.num_tris, sync_type != 0)?;
    let frame_type = buffer.i32(position)?;
    let byte_packed = frame_type == 0 || version <= 3;
    // (Past the frame type, the bounding box and the 16-byte name.)
    let (vertices, vertex_size) = if byte_packed { (position.wrapping_add(28), 4) } else { (position.wrapping_add(36), 8) };
    buffer.size_check(advance(vertices, vertex_size, header.num_verts as i64))?;

    let mut corner = 0u32;
    for triangle in 0..header.num_tris as u32 {
        let start = advance(triangles, 12, triangle as i64);
        for slot in 0..3 {
            let raw = buffer.u16(start + 2 * slot)? as u32;
            let index = header.clamp_vertex(raw) as i64;
            let vertex = advance(vertices, vertex_size, index);
            let (coordinates, normal_index) = if byte_packed {
                let packed = buffer.bytes(vertex, 4)?;
                ([packed[0] as f32, packed[1] as f32, packed[2] as f32], packed[3])
            } else {
                let x = buffer.u16(vertex)? as f32;
                let y = buffer.u16(vertex + 2)? as f32;
                let z = buffer.u16(vertex + 4)? as f32;
                ([x, y, z], buffer.u8(vertex + 6)?)
            };
            mesh.vertices.push(header.position(coordinates));
            mesh.normals.push(lookup_normal(normal_index));

            if sync_type != 0 {
                let mut uv_index = buffer.u16(start + 6 + 2 * slot)? as u32;
                if uv_index >= sync_type as u32 {
                    uv_index = sync_type.wrapping_sub(1) as u32;
                }
                let coordinate = advance(texture_coords, 4, uv_index as i64);
                let mut s = buffer.i16(coordinate)? as f32;
                let mut t = buffer.i16(coordinate + 2)? as f32;
                if version != 5 {
                    s = (s + 0.5) / header.skin_width as f32;
                    t = 1.0 - (t + 0.5) / header.skin_height as f32;
                }
                if let Some(coordinates) = mesh.texture_coords.as_mut() {
                    coordinates.push([s, t, 0.0]);
                }
            }
        }
        mesh.faces.push(vec![corner + 2, corner + 1, corner]);
        corner += 3;
    }

    let mut mesh = mesh.into_mesh(header.num_tris);
    if version == 5 {
        scale_mdl5_coordinates(&header, &textures, &mut mesh)?;
    }
    Ok(Scene { meshes: vec![mesh] })
}

/// `MDLImporter::InternReadFile`: the subformat by the magic word.
pub fn read(path: &str, bytes: &[u8], files: &FileSystem) -> Result<Scene, ImportError> {
    if bytes.len() < SMALLEST_FILE {
        return fail("MDL File is too small.");
    }
    let buffer = Buffer::new(bytes, true);
    let mut skins = Skins::new(files);
    let magic = &bytes[..4];
    let is = |forward: &[u8; 4]| {
        let mut backward = *forward;
        backward.reverse();
        magic == forward || magic == backward
    };
    if is(b"IDPO") || is(b"MDL2") {
        return read_quake1(&buffer, &mut skins);
    }
    if is(b"MDL3") {
        return read_mdl345(&buffer, &mut skins, 3);
    }
    if is(b"MDL4") {
        return read_mdl345(&buffer, &mut skins, 4);
    }
    if is(b"MDL5") {
        return read_mdl345(&buffer, &mut skins, 5);
    }
    if is(b"MDL7") {
        return mdl7::read(buffer, &mut skins);
    }
    if is(b"IDST") || is(b"IDSQ") {
        if buffer.i32(4)? != half_life::VERSION {
            return fail("HL2 MDLs are not implemented");
        }
        if is(b"IDSQ") {
            return fail("Impossible to properly load a model from an MDL sequence file.");
        }
        return half_life::read(path, &buffer, files);
    }
    fail(format!("Unknown MDL subformat {path}."))
}
