//! MikuMikuDance PMX models (`MMDImporter`, `pmx::PmxModel::Read`): a
//! mesh per material, each corner of its index range a vertex of its own,
//! then turned left-handed, UVs flipped and faces reversed.
//!
//! The parser reads through a `std::istream` and checks nothing: once a
//! read falls short the stream fails and later reads leave their
//! destinations as they were. Fields the constructors set stay
//! well-defined; a local or field left uninitialised (an index, a string
//! length, a skinning, morph or joint type) is then read uninitialised,
//! undefined behaviour here an error.

use crate::scene::{ImportError, MAX_TEXTURE_COORDS, Mesh, Scene, check_allocation, fail};
use crate::vector::Vector3;

/// `sizeof(pmx::PmxModel)` with MSVC's 32-byte `std::string`.
const MODEL_SIZE: usize = 304;

/// Bone trees deeper than this overflow the recursion of the node
/// processing.
const MAX_DEPTH: usize = 2048;

// Element sizes (MSVC, x64; arrays of classes with destructors carry an
// 8-byte count).
const VERTEX_SIZE: u64 = 120;
const STRING_SIZE: u64 = 32;
const MATERIAL_SIZE: u64 = 192;
const BONE_SIZE: u64 = 176;
const IK_LINK_SIZE: u64 = 32;
const MORPH_SIZE: u64 = 128;
const FRAME_SIZE: u64 = 80;
const FRAME_ELEMENT_SIZE: u64 = 8;
const RIGID_BODY_SIZE: u64 = 136;
const JOINT_SIZE: u64 = 176;
const VECTOR_SIZE: u64 = 12;
const FACE_SIZE: u64 = 16;

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

/// `new T[count]` (`make_unique<T[]>`): a negative count throws
/// `bad_array_new_length`, a large one `bad_alloc`.
fn check_array(count: i32, element_size: u64) -> Result<usize, ImportError> {
    if count < 0 {
        return fail("bad array new length");
    }
    check_allocation(count as u64, element_size)?;
    Ok(count as usize)
}

/// A `std::istringstream` over the file: a short read copies what is left
/// and fails the stream, after which nothing is read.
struct Stream<'a> {
    bytes: &'a [u8],
    position: usize,
    failed: bool,
}

impl Stream<'_> {
    /// `read(destination, n)`: the bytes copied over the start of
    /// `destination`; false when it fell short.
    fn read_into(&mut self, destination: &mut [u8]) -> bool {
        if self.failed {
            return false;
        }
        let available = self.bytes.len() - self.position;
        let count = destination.len().min(available);
        destination[..count].copy_from_slice(&self.bytes[self.position..self.position + count]);
        self.position += count;
        if count < destination.len() {
            self.failed = true;
            return false;
        }
        true
    }

    /// A read into a destination that was never initialised: falling
    /// short leaves it (partly) uninitialised.
    fn read_fresh<const N: usize>(&mut self, what: &str) -> Result<[u8; N], ImportError> {
        let mut bytes = [0u8; N];
        if !self.read_into(&mut bytes) {
            return undefined(what);
        }
        Ok(bytes)
    }

    fn i32_into(&mut self, value: &mut i32) {
        let mut bytes = value.to_le_bytes();
        self.read_into(&mut bytes);
        *value = i32::from_le_bytes(bytes);
    }

    fn u8_into(&mut self, value: &mut u8) {
        let mut bytes = [*value];
        self.read_into(&mut bytes);
        *value = bytes[0];
    }

    fn u16_into(&mut self, value: &mut u16) {
        let mut bytes = value.to_le_bytes();
        self.read_into(&mut bytes);
        *value = u16::from_le_bytes(bytes);
    }

    fn floats_into(&mut self, values: &mut [f32]) {
        let mut bytes: Vec<u8> = values.iter().flat_map(|value| value.to_le_bytes()).collect();
        self.read_into(&mut bytes);
        for (value, chunk) in values.iter_mut().zip(bytes.as_chunks::<4>().0) {
            *value = f32::from_le_bytes(*chunk);
        }
    }

    /// Bytes read only to be skipped (their value is never used).
    fn skip(&mut self, count: usize) {
        let mut bytes = vec![0u8; count];
        self.read_into(&mut bytes);
    }

    /// `ReadIndex`: 1, 2 or 4 bytes (all ones in 1 or 2 bytes: -1); any
    /// other size reads nothing and gives -1.
    fn index(&mut self, size: u8) -> Result<i32, ImportError> {
        match size {
            1 => {
                let [value] = self.read_fresh::<1>("an index read past the file")?;
                Ok(if value == 255 { -1 } else { value as i32 })
            }
            2 => {
                let bytes = self.read_fresh::<2>("an index read past the file")?;
                let value = u16::from_le_bytes(bytes);
                Ok(if value == 65535 { -1 } else { value as i32 })
            }
            4 => {
                let bytes = self.read_fresh::<4>("an index read past the file")?;
                Ok(i32::from_le_bytes(bytes))
            }
            _ => Ok(-1),
        }
    }

    /// `ReadString`: a length, then that many bytes (zeros where the file
    /// ends), UTF-16 (`utf16to8`, which throws on a broken surrogate) or
    /// UTF-8 by the encoding. Only whether it throws matters here.
    fn string(&mut self, encoding: u8) -> Result<(), ImportError> {
        let size = i32::from_le_bytes(self.read_fresh::<4>("a string length read past the file")?);
        if size == 0 {
            return Ok(());
        }
        if size < 0 {
            return fail("vector too long");
        }
        check_allocation(size as u64, 1)?;
        let mut buffer = vec![0u8; size as usize];
        self.read_into(&mut buffer);
        if encoding == 0 {
            check_allocation(size as u64, 3)?;
            check_utf16(&buffer)?;
        }
        Ok(())
    }
}

/// `utf8::utf16to8` over the whole code units: a lead surrogate must be
/// followed by a trail one, and a trail one must follow a lead.
fn check_utf16(bytes: &[u8]) -> Result<(), ImportError> {
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
    let mut index = 0;
    while index < units.len() {
        let unit = units[index];
        index += 1;
        if (0xd800..=0xdbff).contains(&unit) {
            let Some(&trail) = units.get(index) else {
                return fail("Invalid UTF-16");
            };
            index += 1;
            if !(0xdc00..=0xdfff).contains(&trail) {
                return fail("Invalid UTF-16");
            }
        } else if (0xdc00..=0xdfff).contains(&unit) {
            return fail("Invalid UTF-16");
        }
    }
    Ok(())
}

/// `PmxSetting`.
#[derive(Default)]
struct Setting {
    encoding: u8,
    uv: u8,
    vertex_index_size: u8,
    texture_index_size: u8,
    material_index_size: u8,
    bone_index_size: u8,
    morph_index_size: u8,
    rigidbody_index_size: u8,
}

/// `PmxVertex` as far as meshes need it.
#[derive(Clone, Default)]
struct Vertex {
    position: Vector3,
    normal: Vector3,
    uv: [f32; 2],
    uva: [[f32; 4]; 4],
}

/// `PmxMaterial` as far as meshes need it.
struct Material {
    diffuse_texture_index: i32,
    index_count: i32,
}

/// `PmxModel`: what the meshes are made of.
struct Model {
    setting: Setting,
    vertices: Vec<Vertex>,
    indices: Vec<i32>,
    texture_count: usize,
    materials: Vec<Material>,
    bone_parents: Vec<i32>,
}

/// `PmxSetting::Read`: at least 8 settings, the rest skipped.
fn read_setting(stream: &mut Stream) -> Result<Setting, ImportError> {
    let [count] = stream.read_fresh::<1>("the settings count read past the file")?;
    if count < 8 {
        return fail("MMD: invalid size");
    }
    let mut setting = Setting::default();
    let fields = [
        &mut setting.encoding,
        &mut setting.uv,
        &mut setting.vertex_index_size,
        &mut setting.texture_index_size,
        &mut setting.material_index_size,
        &mut setting.bone_index_size,
        &mut setting.morph_index_size,
        &mut setting.rigidbody_index_size,
    ];
    for field in fields {
        stream.u8_into(field);
    }
    stream.skip((count - 8) as usize);
    Ok(setting)
}

/// `PmxVertex::Read` (more than four extra UV sets write past `uva`).
fn read_vertex(stream: &mut Stream, setting: &Setting) -> Result<Vertex, ImportError> {
    let mut vertex = Vertex::default();
    stream.floats_into(&mut vertex.position);
    stream.floats_into(&mut vertex.normal);
    stream.floats_into(&mut vertex.uv);
    if setting.uv > 4 {
        return undefined("extra UV sets read past the vertex's four");
    }
    for set in vertex.uva.iter_mut().take(setting.uv as usize) {
        stream.floats_into(set);
    }
    let [skinning] = stream.read_fresh::<1>("a skinning type read past the file")?;
    let bone_size = setting.bone_index_size;
    let mut weights = [0.0f32; 4];
    match skinning {
        // BDEF1
        0 => {
            stream.index(bone_size)?;
        }
        // BDEF2
        1 => {
            stream.index(bone_size)?;
            stream.index(bone_size)?;
            stream.floats_into(&mut weights[..1]);
        }
        // BDEF4, QDEF
        2 | 4 => {
            for _ in 0..4 {
                stream.index(bone_size)?;
            }
            stream.floats_into(&mut weights);
        }
        // SDEF
        3 => {
            stream.index(bone_size)?;
            stream.index(bone_size)?;
            let mut values = [0.0f32; 10];
            stream.floats_into(&mut values);
        }
        // `throw "invalid skinning type"`: no exception class, so nothing
        // catches it.
        _ => return fail("invalid skinning type"),
    }
    let mut edge = [0.0f32];
    stream.floats_into(&mut edge);
    Ok(vertex)
}

/// `PmxMaterial::Read`.
fn read_material(stream: &mut Stream, setting: &Setting) -> Result<Material, ImportError> {
    stream.string(setting.encoding)?;
    stream.string(setting.encoding)?;
    // Diffuse, specular, specularity, ambient.
    let mut colors = [0.0f32; 11];
    stream.floats_into(&mut colors);
    let mut flag = 0u8;
    stream.u8_into(&mut flag);
    let mut edge = [0.0f32; 5];
    stream.floats_into(&mut edge);
    let diffuse_texture_index = stream.index(setting.texture_index_size)?;
    stream.index(setting.texture_index_size)?;
    let mut sphere_op_mode = 0u8;
    stream.u8_into(&mut sphere_op_mode);
    let mut common_toon_flag = 0u8;
    stream.u8_into(&mut common_toon_flag);
    if common_toon_flag != 0 {
        let mut toon = 0u8;
        stream.u8_into(&mut toon);
    } else {
        stream.index(setting.texture_index_size)?;
    }
    stream.string(setting.encoding)?;
    let mut index_count = 0i32;
    stream.i32_into(&mut index_count);
    Ok(Material { diffuse_texture_index, index_count })
}

/// `PmxBone::Read` (with its IK links): its parent.
fn read_bone(stream: &mut Stream, setting: &Setting) -> Result<i32, ImportError> {
    let bone_size = setting.bone_index_size;
    stream.string(setting.encoding)?;
    stream.string(setting.encoding)?;
    let mut position = [0.0f32; 3];
    stream.floats_into(&mut position);
    let parent = stream.index(bone_size)?;
    let mut level = 0i32;
    stream.i32_into(&mut level);
    let mut flags = 0u16;
    stream.u16_into(&mut flags);
    if flags & 0x0001 != 0 {
        stream.index(bone_size)?;
    } else {
        stream.floats_into(&mut [0.0; 3]);
    }
    if flags & (0x0100 | 0x0200) != 0 {
        stream.index(bone_size)?;
        stream.floats_into(&mut [0.0]);
    }
    if flags & 0x0400 != 0 {
        stream.floats_into(&mut [0.0; 3]);
    }
    if flags & 0x0800 != 0 {
        stream.floats_into(&mut [0.0; 6]);
    }
    if flags & 0x2000 != 0 {
        let mut key = 0i32;
        stream.i32_into(&mut key);
    }
    if flags & 0x0020 != 0 {
        stream.index(bone_size)?;
        let mut loop_count = 0i32;
        stream.i32_into(&mut loop_count);
        stream.floats_into(&mut [0.0]);
        let mut link_count = 0i32;
        stream.i32_into(&mut link_count);
        let links = check_array(link_count, IK_LINK_SIZE)?;
        for _ in 0..links {
            stream.index(bone_size)?;
            let mut angle_lock = 0u8;
            stream.u8_into(&mut angle_lock);
            if angle_lock == 1 {
                stream.floats_into(&mut [0.0; 6]);
            }
        }
    }
    Ok(parent)
}

/// `PmxMorph::Read`: offsets of its type (flip and impulse morphs, and
/// unknown types, throw).
fn read_morph(stream: &mut Stream, setting: &Setting) -> Result<(), ImportError> {
    stream.string(setting.encoding)?;
    stream.string(setting.encoding)?;
    // (The category is never used.)
    stream.skip(1);
    let [kind] = stream.read_fresh::<1>("a morph type read past the file")?;
    let mut offset_count = 0i32;
    stream.i32_into(&mut offset_count);
    // (element size with its vtable pointer, index size, then floats and
    // bytes after the index)
    let (element_size, index_size, after) = match kind {
        0 => (16, setting.morph_index_size, 4),
        1 => (24, setting.vertex_index_size, 12),
        2 => (40, setting.bone_index_size, 28),
        3..=7 => (32, setting.vertex_index_size, 16),
        8 => (128, setting.material_index_size, 113),
        _ => return fail("MMD: unknown morth type"),
    };
    let count = check_array(offset_count, element_size)?;
    for _ in 0..count {
        stream.index(index_size)?;
        stream.skip(after);
    }
    Ok(())
}

/// `PmxFrame::Read`.
fn read_frame(stream: &mut Stream, setting: &Setting) -> Result<(), ImportError> {
    stream.string(setting.encoding)?;
    stream.string(setting.encoding)?;
    let mut flag = 0u8;
    stream.u8_into(&mut flag);
    let mut element_count = 0i32;
    stream.i32_into(&mut element_count);
    let count = check_array(element_count, FRAME_ELEMENT_SIZE)?;
    for _ in 0..count {
        let mut target = 0u8;
        stream.u8_into(&mut target);
        let size = if target == 0 { setting.bone_index_size } else { setting.morph_index_size };
        stream.index(size)?;
    }
    Ok(())
}

/// `PmxRigidBody::Read`.
fn read_rigid_body(stream: &mut Stream, setting: &Setting) -> Result<(), ImportError> {
    stream.string(setting.encoding)?;
    stream.string(setting.encoding)?;
    stream.index(setting.bone_index_size)?;
    stream.skip(1 + 2 + 1);
    stream.skip(4 * 14);
    stream.skip(1);
    Ok(())
}

/// `PmxJoint::Read`.
fn read_joint(stream: &mut Stream, setting: &Setting) -> Result<(), ImportError> {
    stream.string(setting.encoding)?;
    stream.string(setting.encoding)?;
    // (The joint type is never used.)
    stream.skip(1);
    stream.index(setting.rigidbody_index_size)?;
    stream.index(setting.rigidbody_index_size)?;
    stream.skip(4 * 24);
    Ok(())
}

/// A section's count (left 0 when the file ends before it).
fn count(stream: &mut Stream) -> i32 {
    let mut value = 0i32;
    stream.i32_into(&mut value);
    value
}

/// `PmxModel::Read`.
fn read_model(bytes: &[u8]) -> Result<Model, ImportError> {
    let mut stream = Stream { bytes, position: 0, failed: false };
    let magic = stream.read_fresh::<4>("the magic read past the file")?;
    if &magic != b"PMX " {
        return fail("MMD: Invalid magic number.");
    }
    let mut version = [0.0f32];
    stream.floats_into(&mut version);
    if version[0] != 2.0 && version[0] != 2.1 {
        return fail(format!("MMD: Unsupported version (must be 2.0 or 2.1): {}", version[0]));
    }
    let setting = read_setting(&mut stream)?;
    for _ in 0..4 {
        stream.string(setting.encoding)?;
    }

    let vertex_count = check_array(count(&mut stream), VERTEX_SIZE)?;
    let mut vertices = Vec::with_capacity(vertex_count.min(bytes.len()));
    for _ in 0..vertex_count {
        vertices.push(read_vertex(&mut stream, &setting)?);
    }
    let index_count = check_array(count(&mut stream), 4)?;
    let mut indices = Vec::with_capacity(index_count.min(bytes.len()));
    for _ in 0..index_count {
        indices.push(stream.index(setting.vertex_index_size)?);
    }
    let texture_count = check_array(count(&mut stream), STRING_SIZE)?;
    for _ in 0..texture_count {
        stream.string(setting.encoding)?;
    }
    let material_count = check_array(count(&mut stream), MATERIAL_SIZE)?;
    let mut materials = Vec::with_capacity(material_count.min(bytes.len()));
    for _ in 0..material_count {
        materials.push(read_material(&mut stream, &setting)?);
    }
    let bone_count = check_array(count(&mut stream), BONE_SIZE)?;
    let mut bone_parents = Vec::with_capacity(bone_count.min(bytes.len()));
    for _ in 0..bone_count {
        bone_parents.push(read_bone(&mut stream, &setting)?);
    }
    let morph_count = check_array(count(&mut stream), MORPH_SIZE)?;
    for _ in 0..morph_count {
        read_morph(&mut stream, &setting)?;
    }
    let frame_count = check_array(count(&mut stream), FRAME_SIZE)?;
    for _ in 0..frame_count {
        read_frame(&mut stream, &setting)?;
    }
    let rigid_body_count = check_array(count(&mut stream), RIGID_BODY_SIZE)?;
    for _ in 0..rigid_body_count {
        read_rigid_body(&mut stream, &setting)?;
    }
    let joint_count = check_array(count(&mut stream), JOINT_SIZE)?;
    for _ in 0..joint_count {
        read_joint(&mut stream, &setting)?;
    }
    Ok(Model { setting, vertices, indices, texture_count, materials, bone_parents })
}

/// `CreateMesh`, then the left-handed, UV and winding flips: a vertex per
/// index of the material's range, faces of three in order.
fn create_mesh(model: &Model, index_start: u32, index_count: i32) -> Result<Mesh, ImportError> {
    let vertex_count = index_count as u32 as usize;
    let face_count = (index_count / 3) as u32 as usize;
    check_allocation(face_count as u64, FACE_SIZE)?;
    check_allocation(vertex_count as u64, VECTOR_SIZE)?;
    let extra_sets = model.setting.uv as usize;
    if 1 + extra_sets > MAX_TEXTURE_COORDS {
        return undefined("UV sets past the mesh's eight");
    }

    let mut positions = vec![[0.0f32; 3]; vertex_count];
    let mut normals = vec![[0.0f32; 3]; vertex_count];
    let mut uv_sets: Vec<Vec<Vector3>> = vec![vec![[0.0f32; 3]; vertex_count]; 1 + extra_sets];
    for corner in 0..index_count.max(0) as usize {
        let slot = (index_start as i32).wrapping_add(corner as i32);
        let Some(&vertex_index) = usize::try_from(slot).ok().and_then(|slot| model.indices.get(slot)) else {
            return undefined("an index past the index list");
        };
        let Some(vertex) = usize::try_from(vertex_index).ok().and_then(|index| model.vertices.get(index)) else {
            return undefined("an index past the vertices");
        };
        let [x, y, z] = vertex.position;
        positions[corner] = [x, y, -z];
        let [nx, ny, nz] = vertex.normal;
        normals[corner] = [nx, ny, -nz];
        uv_sets[0][corner] = [vertex.uv[0], 1.0 - vertex.uv[1], 0.0];
        for (set, coordinates) in uv_sets.iter_mut().enumerate().skip(1) {
            let Some(extra) = vertex.uva.get(set) else {
                return undefined("an extra UV set past the vertex's four");
            };
            coordinates[corner] = [extra[0], 1.0 - extra[1], 0.0];
        }
    }

    let mut faces = Vec::with_capacity(face_count);
    for face in 0..face_count as u32 {
        faces.push(vec![3 * face + 2, 3 * face + 1, 3 * face]);
    }
    // (No primitive types: the preprocessing finds them from the faces, and
    // a range of fewer than three indices leaves a mesh without any.)
    let mut mesh = Mesh {
        vertex_count,
        vertices: Some(positions),
        normals: Some(normals),
        faces: Some(faces),
        ..Mesh::default()
    };
    for (set, coordinates) in uv_sets.into_iter().enumerate() {
        mesh.uv_components[set] = if set == 0 { 2 } else { 4 };
        mesh.texture_coords[set] = Some(coordinates);
    }
    Ok(mesh)
}

/// `CreateDataFromImport`: the meshes, then the bone nodes (a parent past
/// the bones is read past their list; the tree's depth is the recursion's)
/// and the materials (a texture index past the textures).
fn create_scene(model: &Model) -> Result<Scene, ImportError> {
    let mut meshes = Vec::with_capacity(model.materials.len());
    let mut index_start = 0u32;
    for (index, material) in model.materials.iter().enumerate() {
        let mut mesh = create_mesh(model, index_start, material.index_count)?;
        mesh.material_index = index as u32;
        meshes.push(mesh);
        index_start = index_start.wrapping_add(material.index_count as u32);
    }

    // Each bone's node goes under its parent's (the root's for a negative
    // parent); the tree below the root is walked recursively.
    let bone_count = model.bone_parents.len();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); bone_count];
    let mut pending = Vec::new();
    for (bone, &parent) in model.bone_parents.iter().enumerate() {
        if parent < 0 {
            pending.push((bone, 1usize));
            continue;
        }
        let Some(list) = children.get_mut(parent as usize) else {
            return undefined("a bone's parent past the bones");
        };
        list.push(bone);
    }
    while let Some((bone, depth)) = pending.pop() {
        if depth > MAX_DEPTH {
            return undefined("a bone tree too deep for the stack");
        }
        for &child in &children[bone] {
            pending.push((child, depth + 1));
        }
    }

    for material in &model.materials {
        let index = material.diffuse_texture_index;
        if index >= 0 && index as usize >= model.texture_count {
            return undefined("a texture index past the textures");
        }
    }
    Ok(Scene { meshes })
}

/// `MMDImporter::InternReadFile`.
pub fn read(path: &str, bytes: &[u8]) -> Result<Scene, ImportError> {
    if bytes.len() < MODEL_SIZE {
        return fail(format!("{path} is too small."));
    }
    let model = read_model(bytes)?;
    create_scene(&model)
}
