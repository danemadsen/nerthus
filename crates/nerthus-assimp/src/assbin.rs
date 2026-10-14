//! Assimp's binary scene dumps (`AssbinImporter`): a 512-byte header
//! (format 1.0, not "shortened", maybe zlib-compressed after it), then the
//! scene's chunks, field by field: the node tree, meshes with their bones,
//! materials, animations, textures, lights and cameras. Only the meshes
//! are kept; the rest is read for how it can fail.

use zlib_rs::{InflateConfig, ReturnCode, decompress_slice};

use crate::scene::{ImportError, MAX_COLOR_SETS, MAX_TEXTURE_COORDS, Mesh, Scene, check_allocation, fail};

const CHUNK_CAMERA: u32 = 0x1234;
const CHUNK_LIGHT: u32 = 0x1235;
const CHUNK_TEXTURE: u32 = 0x1236;
const CHUNK_MESH: u32 = 0x1237;
const CHUNK_NODE_ANIMATION: u32 = 0x1238;
const CHUNK_SCENE: u32 = 0x1239;
const CHUNK_BONE: u32 = 0x123a;
const CHUNK_ANIMATION: u32 = 0x123b;
const CHUNK_NODE: u32 = 0x123c;
const CHUNK_MATERIAL: u32 = 0x123d;
const CHUNK_MATERIAL_PROPERTY: u32 = 0x123e;

const HAS_POSITIONS: u32 = 0x1;
const HAS_NORMALS: u32 = 0x2;
const HAS_TANGENTS_AND_BITANGENTS: u32 = 0x4;
const HAS_TEXCOORD_BASE: u32 = 0x100;
const HAS_COLOR_BASE: u32 = 0x10000;

/// `AI_MAXLEN`: an `aiString`'s bytes with its NUL.
const MAX_LENGTH: u32 = 1024;

/// `aiLightSource_DIRECTIONAL` and `aiLightSource_SPOT`.
const LIGHT_DIRECTIONAL: u32 = 1;
const LIGHT_SPOT: u32 = 3;

/// Node trees deeper than this overflow the stack of `ReadBinaryNode`,
/// whose frames hold kilobyte-long name strings.
const MAX_DEPTH: usize = 512;

// Element sizes (MSVC, x64).
const POINTER_SIZE: u64 = 8;
const INDEX_SIZE: u64 = 4;
const VECTOR_SIZE: u64 = 12;
const COLOR_SIZE: u64 = 16;
const FACE_SIZE: u64 = 16;
const WEIGHT_SIZE: u64 = 8;
const STRING_SIZE: u64 = 1028;
const METADATA_ENTRY_SIZE: u64 = 16;
const VECTOR_KEY_SIZE: u64 = 24;
const QUATERNION_KEY_SIZE: u64 = 32;
const TEXEL_SIZE: u64 = 4;

/// A reader of one of the scene's sections.
type SectionReader = fn(&mut Stream) -> Result<(), ImportError>;

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

/// The stream the scene is read from: the file (`fread`: a short read
/// copies what is left), or the inflated data (`MemoryIOStream`: only
/// whole elements are copied).
struct Stream<'a> {
    bytes: &'a [u8],
    /// May lie past the end, after a seek.
    position: usize,
    copies_partial: bool,
}

impl<'a> Stream<'a> {
    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
    }

    /// `Read(buffer, size, 1)`: the bytes copied, and whether the whole
    /// element came.
    fn read_raw(&mut self, size: usize) -> (&'a [u8], bool) {
        let start = self.position.min(self.bytes.len());
        if self.remaining() >= size {
            self.position += size;
            return (&self.bytes[start..start + size], true);
        }
        if !self.copies_partial {
            return (&[], false);
        }
        self.position = self.position.max(self.bytes.len());
        (&self.bytes[start..], false)
    }

    /// `Read<T>`: `size` bytes, else "Unexpected EOF".
    fn read(&mut self, size: usize) -> Result<&'a [u8], ImportError> {
        let (data, complete) = self.read_raw(size);
        if !complete {
            return fail("Unexpected EOF");
        }
        Ok(data)
    }

    fn read_u16(&mut self) -> Result<u16, ImportError> {
        let data = self.read(2)?;
        Ok(u16::from_le_bytes([data[0], data[1]]))
    }

    fn read_u32(&mut self) -> Result<u32, ImportError> {
        let data = self.read(4)?;
        Ok(u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
    }

    fn read_f32(&mut self) -> Result<f32, ImportError> {
        let bits = self.read_u32()?;
        Ok(f32::from_bits(bits))
    }

    /// `Read<aiVector3D>`.
    fn read_vector(&mut self) -> Result<[f32; 3], ImportError> {
        Ok([self.read_f32()?, self.read_f32()?, self.read_f32()?])
    }

    /// A list of `count` entries of `entry_size` bytes in the stream, which
    /// must hold them: reading past the end fails (the list is not made).
    fn check_room(&self, count: u64, entry_size: u64) -> Result<(), ImportError> {
        if count.saturating_mul(entry_size) > self.remaining() as u64 {
            return fail("Unexpected EOF");
        }
        Ok(())
    }
}

/// A chunk's identifier and (unused) size.
fn expect_chunk(stream: &mut Stream, identifier: u32) -> Result<(), ImportError> {
    if stream.read_u32()? != identifier {
        return fail("Magic chunk identifiers are wrong!");
    }
    stream.read_u32()?;
    Ok(())
}

/// `Read<aiString>`: a 32-bit length (a short read fills its low bytes),
/// then that many bytes, neither checked. A length of `MAX_LENGTH` or more
/// writes past the string.
fn read_string(stream: &mut Stream) -> Result<(), ImportError> {
    let (length_bytes, _) = stream.read_raw(4);
    let mut length_buffer = [0u8; 4];
    length_buffer[..length_bytes.len()].copy_from_slice(length_bytes);
    let length = u32::from_le_bytes(length_buffer);
    if length >= MAX_LENGTH {
        return undefined("a string longer than its buffer");
    }
    if length != 0 {
        stream.read_raw(length as usize);
    }
    Ok(())
}

/// `ReadBinaryNode`: its name, transform, mesh indices, children (depth
/// first) and metadata.
fn read_node(stream: &mut Stream, depth: usize) -> Result<(), ImportError> {
    if depth > MAX_DEPTH {
        return undefined("a node tree too deep for the stack");
    }
    expect_chunk(stream, CHUNK_NODE)?;
    read_string(stream)?;
    stream.read(64)?;
    let child_count = stream.read_u32()?;
    let mesh_count = stream.read_u32()?;
    let metadata_count = stream.read_u32()?;

    if mesh_count != 0 {
        check_allocation(mesh_count as u64, INDEX_SIZE)?;
        for _ in 0..mesh_count {
            stream.read_u32()?;
        }
    }
    if child_count != 0 {
        check_allocation(child_count as u64, POINTER_SIZE)?;
        for _ in 0..child_count {
            read_node(stream, depth + 1)?;
        }
    }
    if metadata_count != 0 {
        check_allocation(metadata_count as u64, STRING_SIZE)?;
        check_allocation(metadata_count as u64, METADATA_ENTRY_SIZE)?;
        for _ in 0..metadata_count {
            read_string(stream)?;
            let kind = stream.read_u16()?;
            match kind {
                0 => {
                    stream.read(1)?;
                }
                1 | 3 => {
                    stream.read(4)?;
                }
                2 | 4 => {
                    stream.read(8)?;
                }
                5 => read_string(stream)?,
                6 => {
                    stream.read_vector()?;
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// A vertex stream of `count` entries of `components` floats.
fn read_stream<const N: usize>(stream: &mut Stream, count: u32) -> Result<Vec<[f32; N]>, ImportError> {
    let entry_size = 4 * N as u64;
    let allocated_size = if N == 4 { COLOR_SIZE } else { VECTOR_SIZE };
    check_allocation(count as u64, allocated_size)?;
    stream.check_room(count as u64, entry_size)?;
    let mut values = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let mut value = [0.0f32; N];
        for component in &mut value {
            *component = stream.read_f32()?;
        }
        values.push(value);
    }
    Ok(values)
}

/// `ReadBinaryBone`: its name, weights and offset matrix; the weights'
/// vertices are kept.
fn read_bone(stream: &mut Stream) -> Result<Vec<u32>, ImportError> {
    expect_chunk(stream, CHUNK_BONE)?;
    read_string(stream)?;
    let weight_count = stream.read_u32()?;
    stream.read(64)?;
    check_allocation(weight_count as u64, WEIGHT_SIZE)?;
    stream.check_room(weight_count as u64, WEIGHT_SIZE)?;
    let mut vertices = Vec::with_capacity(weight_count as usize);
    for _ in 0..weight_count {
        vertices.push(stream.read_u32()?);
        stream.read_f32()?;
    }
    Ok(vertices)
}

/// `ReadBinaryMesh`: counts, then the streams its flags name (colour and
/// texture coordinate sets up to the first missing one), faces (16-bit
/// indices below 65536 vertices) and bones.
fn read_mesh(stream: &mut Stream) -> Result<Mesh, ImportError> {
    expect_chunk(stream, CHUNK_MESH)?;
    let primitive_types = stream.read_u32()?;
    let vertex_count = stream.read_u32()?;
    let face_count = stream.read_u32()?;
    let bone_count = stream.read_u32()?;
    let material_index = stream.read_u32()?;
    let flags = stream.read_u32()?;

    let mut mesh = Mesh { primitive_types, vertex_count: vertex_count as usize, material_index, ..Mesh::default() };
    if flags & HAS_POSITIONS != 0 {
        mesh.vertices = Some(read_stream::<3>(stream, vertex_count)?);
    }
    if flags & HAS_NORMALS != 0 {
        mesh.normals = Some(read_stream::<3>(stream, vertex_count)?);
    }
    if flags & HAS_TANGENTS_AND_BITANGENTS != 0 {
        mesh.tangents = Some(read_stream::<3>(stream, vertex_count)?);
        mesh.bitangents = Some(read_stream::<3>(stream, vertex_count)?);
    }
    for set in 0..MAX_COLOR_SETS {
        if flags & (HAS_COLOR_BASE << set) == 0 {
            break;
        }
        mesh.colors[set] = Some(read_stream::<4>(stream, vertex_count)?);
    }
    for set in 0..MAX_TEXTURE_COORDS {
        if flags & (HAS_TEXCOORD_BASE << set) == 0 {
            break;
        }
        mesh.uv_components[set] = stream.read_u32()?;
        mesh.texture_coords[set] = Some(read_stream::<3>(stream, vertex_count)?);
    }

    check_allocation(face_count as u64, FACE_SIZE)?;
    stream.check_room(face_count as u64, 2)?;
    let short_indices = vertex_count < 1 << 16;
    let mut faces = Vec::with_capacity(face_count as usize);
    for _ in 0..face_count {
        let index_count = stream.read_u16()?;
        let mut indices = Vec::with_capacity(index_count as usize);
        for _ in 0..index_count {
            let index = if short_indices { stream.read_u16()? as u32 } else { stream.read_u32()? };
            indices.push(index);
        }
        faces.push(indices);
    }
    mesh.faces = Some(faces);

    if bone_count != 0 {
        check_allocation(bone_count as u64, POINTER_SIZE)?;
        for _ in 0..bone_count {
            let bone = read_bone(stream)?;
            mesh.bones.push(bone);
        }
    }
    Ok(mesh)
}

/// `ReadBinaryMaterial`: its properties, each data read unchecked.
fn read_material(stream: &mut Stream) -> Result<(), ImportError> {
    expect_chunk(stream, CHUNK_MATERIAL)?;
    let property_count = stream.read_u32()?;
    if property_count == 0 {
        return Ok(());
    }
    check_allocation(property_count as u64, POINTER_SIZE)?;
    for _ in 0..property_count {
        expect_chunk(stream, CHUNK_MATERIAL_PROPERTY)?;
        read_string(stream)?;
        stream.read_u32()?;
        stream.read_u32()?;
        let length = stream.read_u32()?;
        stream.read_u32()?;
        check_allocation(length as u64, 1)?;
        stream.read_raw(length as usize);
    }
    Ok(())
}

/// `ReadBinaryAnim` and its channels' keys.
fn read_animation(stream: &mut Stream) -> Result<(), ImportError> {
    expect_chunk(stream, CHUNK_ANIMATION)?;
    read_string(stream)?;
    stream.read(16)?;
    let channel_count = stream.read_u32()?;
    if channel_count == 0 {
        return Ok(());
    }
    check_allocation(channel_count as u64, POINTER_SIZE)?;
    for _ in 0..channel_count {
        expect_chunk(stream, CHUNK_NODE_ANIMATION)?;
        read_string(stream)?;
        let position_count = stream.read_u32()?;
        let rotation_count = stream.read_u32()?;
        let scaling_count = stream.read_u32()?;
        stream.read_u32()?;
        stream.read_u32()?;
        // Keys: a double time, then 3 (vectors) or 4 (quaternions) floats.
        let lists = [(position_count, VECTOR_KEY_SIZE, 20), (rotation_count, QUATERNION_KEY_SIZE, 24), (scaling_count, VECTOR_KEY_SIZE, 20)];
        for (count, key_size, read_size) in lists {
            if count == 0 {
                continue;
            }
            check_allocation(count as u64, key_size)?;
            stream.check_room(count as u64, read_size)?;
            for _ in 0..count {
                stream.read(read_size as usize)?;
            }
        }
    }
    Ok(())
}

/// `ReadBinaryTexture`: a compressed texture's bytes (no height), or
/// width × height texels, both counts in 32 bits.
fn read_texture(stream: &mut Stream) -> Result<(), ImportError> {
    expect_chunk(stream, CHUNK_TEXTURE)?;
    let width = stream.read_u32()?;
    let height = stream.read_u32()?;
    stream.read_raw(8);
    if height == 0 {
        check_allocation(width as u64, TEXEL_SIZE)?;
        stream.read_raw(width as usize);
    } else {
        let texels = width.wrapping_mul(height);
        check_allocation(texels as u64, TEXEL_SIZE)?;
        let bytes = texels.wrapping_mul(4);
        stream.read_raw(bytes as usize);
    }
    Ok(())
}

/// `ReadBinaryLight`: attenuation unless directional, cone angles for a
/// spot.
fn read_light(stream: &mut Stream) -> Result<(), ImportError> {
    expect_chunk(stream, CHUNK_LIGHT)?;
    read_string(stream)?;
    let kind = stream.read_u32()?;
    stream.read(36)?;
    if kind != LIGHT_DIRECTIONAL {
        stream.read(12)?;
    }
    stream.read(36)?;
    if kind == LIGHT_SPOT {
        stream.read(8)?;
    }
    Ok(())
}

fn read_camera(stream: &mut Stream) -> Result<(), ImportError> {
    expect_chunk(stream, CHUNK_CAMERA)?;
    read_string(stream)?;
    stream.read(36)?;
    stream.read(16)?;
    Ok(())
}

/// `ReadBinaryScene`.
fn read_scene(stream: &mut Stream) -> Result<Vec<Mesh>, ImportError> {
    expect_chunk(stream, CHUNK_SCENE)?;
    stream.read_u32()?;
    let mesh_count = stream.read_u32()?;
    let material_count = stream.read_u32()?;
    let animation_count = stream.read_u32()?;
    let texture_count = stream.read_u32()?;
    let light_count = stream.read_u32()?;
    let camera_count = stream.read_u32()?;

    read_node(stream, 1)?;
    let mut meshes = Vec::new();
    let sections: [(u32, SectionReader); 5] =
        [(material_count, read_material), (animation_count, read_animation), (texture_count, read_texture), (light_count, read_light), (camera_count, read_camera)];
    if mesh_count != 0 {
        check_allocation(mesh_count as u64, POINTER_SIZE)?;
        for _ in 0..mesh_count {
            let mesh = read_mesh(stream)?;
            meshes.push(mesh);
        }
    }
    for (count, read_one) in sections {
        if count == 0 {
            continue;
        }
        check_allocation(count as u64, POINTER_SIZE)?;
        for _ in 0..count {
            read_one(stream)?;
        }
    }
    Ok(meshes)
}

/// `AssbinImporter::InternReadFile`: past the 44-byte signature, the
/// version (1.0), revision, compile flags, the shortened and compressed
/// flags, then (past the file name, options and padding) the scene, maybe
/// after its inflated size and as zlib data (`uncompress`) to the end.
pub fn read(bytes: &[u8]) -> Result<Scene, ImportError> {
    let mut file = Stream { bytes, position: 44, copies_partial: true };
    let major = file.read_u32()?;
    let minor = file.read_u32()?;
    if minor != 0 || major != 1 {
        return fail("Invalid version, data format not compatible!");
    }
    file.read_u32()?;
    file.read_u32()?;
    let shortened = file.read_u16()? > 0;
    let compressed = file.read_u16()? > 0;
    if shortened {
        return fail("Shortened binaries are not supported!");
    }
    file.position += 256 + 128 + 64;

    let meshes = if compressed {
        let declared_size = file.read_u32()?;
        let start = file.position.min(bytes.len());
        let compressed_data = &bytes[start..];
        check_allocation(declared_size as u64, 1)?;
        let mut output = vec![0u8; declared_size as usize];
        let (inflated, code) = decompress_slice(&mut output, compressed_data, InflateConfig::default());
        if code != ReturnCode::Ok {
            return fail("Zlib decompression failed.");
        }
        let inflated_length = inflated.len();
        let mut memory = Stream { bytes: &output[..inflated_length], position: 0, copies_partial: false };
        read_scene(&mut memory)?
    } else {
        read_scene(&mut file)?
    };
    Ok(Scene { meshes })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_u32(data: &mut Vec<u8>, value: u32) {
        data.extend_from_slice(&value.to_le_bytes());
    }

    fn triangle_scene() -> Vec<u8> {
        let mut scene = Vec::new();
        push_u32(&mut scene, CHUNK_SCENE);
        push_u32(&mut scene, 0);
        for value in [0, 1, 0, 0, 0, 0, 0] {
            push_u32(&mut scene, value);
        }
        // The root node: no name, identity, no children or meshes.
        push_u32(&mut scene, CHUNK_NODE);
        push_u32(&mut scene, 0);
        push_u32(&mut scene, 0);
        for index in 0..16 {
            let value = if index % 5 == 0 { 1.0f32 } else { 0.0 };
            scene.extend_from_slice(&value.to_le_bytes());
        }
        for _ in 0..3 {
            push_u32(&mut scene, 0);
        }
        // A triangle with a bone.
        push_u32(&mut scene, CHUNK_MESH);
        push_u32(&mut scene, 0);
        for value in [4, 3, 1, 1, 0, HAS_POSITIONS] {
            push_u32(&mut scene, value);
        }
        for value in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            scene.extend_from_slice(&value.to_le_bytes());
        }
        scene.extend_from_slice(&3u16.to_le_bytes());
        for index in 0..3u16 {
            scene.extend_from_slice(&index.to_le_bytes());
        }
        push_u32(&mut scene, CHUNK_BONE);
        push_u32(&mut scene, 0);
        push_u32(&mut scene, 0);
        push_u32(&mut scene, 1);
        scene.extend_from_slice(&[0u8; 64]);
        push_u32(&mut scene, 2);
        scene.extend_from_slice(&1.0f32.to_le_bytes());
        scene
    }

    fn file(compressed: bool, scene: &[u8]) -> Vec<u8> {
        let mut data = b"ASSIMP.binary-dump.".to_vec();
        data.resize(44, b' ');
        for value in [1, 0, 0, 0] {
            push_u32(&mut data, value);
        }
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&(compressed as u16).to_le_bytes());
        data.resize(512, 0);
        if compressed {
            push_u32(&mut data, scene.len() as u32);
            let mut packed = vec![0u8; scene.len() + 64];
            let config = zlib_rs::DeflateConfig::default();
            let (written, _) = zlib_rs::compress_slice(&mut packed, scene, config);
            let length = written.len();
            data.extend_from_slice(&packed[..length]);
        } else {
            data.extend_from_slice(scene);
        }
        data
    }

    #[test]
    fn plain_and_compressed_dumps_read_alike() {
        let scene = triangle_scene();
        for compressed in [false, true] {
            let read_scene = read(&file(compressed, &scene)).unwrap();
            assert_eq!(read_scene.meshes.len(), 1);
            assert_eq!(read_scene.meshes[0].faces, Some(vec![vec![0, 1, 2]]));
            assert_eq!(read_scene.meshes[0].bones, vec![vec![2]]);
        }
    }

    #[test]
    fn short_and_long_strings() {
        let mut scene = triangle_scene();
        // The root's name: 2000 bytes overrun the string.
        let name_at = 8 + 28 + 8;
        scene[name_at..name_at + 4].copy_from_slice(&2000u32.to_le_bytes());
        assert!(read(&file(false, &scene)).is_err());
        // A file cut inside a string's length leaves the length's low bytes.
        let mut stream = Stream { bytes: &[5, 0], position: 0, copies_partial: true };
        read_string(&mut stream).unwrap();
        assert_eq!(stream.position, 2);
    }
}
