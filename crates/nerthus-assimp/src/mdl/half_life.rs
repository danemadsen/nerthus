//! Half-Life 1 models (`HL1MDLLoader`): bones in bind pose, then
//! bodyparts of models of meshes made of triangle strips and fans, with a
//! vertex per distinct (position, normal, s, t) use of a model vertex.
//!
//! Everything is reached through offsets in the headers, unchecked: a read
//! outside a file's copy is undefined behaviour, an error here. Textures,
//! skins, animations, sequences, attachments, hitboxes and bone controllers
//! make nothing a mesh shows; only their reads and failures are kept.
//! Textures may come from "<name>T.<ext>" and animations from sequence
//! group files "<name>01.<ext>" on.

use std::collections::{BTreeMap, BTreeSet};

use super::Buffer;
use crate::io::FileSystem;
use crate::matrix::{Matrix4, euler_quaternion};
use crate::scene::{ImportError, Mesh, PRIMITIVE_TRIANGLE, Scene, check_allocation, fail};
use crate::vector::Vector3;

/// `AI_MDL_HL1_VERSION`.
pub(super) const VERSION: i32 = 10;

/// `sizeof(Header_HL1)` and `sizeof(SequenceHeader_HL1)`: the smallest
/// texture and sequence group files.
const HEADER_SIZE: usize = 244;
const SEQUENCE_HEADER_SIZE: usize = 76;

/// Bone chains deeper than this overflow the recursive hierarchy build.
const MAX_DEPTH: usize = 2048;

/// Structure sizes.
const BONE_SIZE: i64 = 112;
const BONE_CONTROLLER_SIZE: i64 = 24;
const HITBOX_SIZE: i64 = 32;
const SEQUENCE_GROUP_SIZE: i64 = 104;
const SEQUENCE_SIZE: i64 = 176;
const EVENT_SIZE: i64 = 76;
const ATTACHMENT_SIZE: i64 = 88;
const ANIMATION_OFFSETS_SIZE: i64 = 12;
const BODYPART_SIZE: i64 = 76;
const TEXTURE_SIZE: i64 = 80;
const MODEL_SIZE: i64 = 112;
const MESH_SIZE: i64 = 20;
const TRIVERT_SIZE: i64 = 8;

/// Element sizes of the allocations made (MSVC, x64).
const STRING_SIZE: u64 = 32;
const TEMP_BONE_SIZE: u64 = 160;
const AI_STRING_SIZE: u64 = 1028;
const KEY_SIZE: u64 = 24;
const MESH_TRIVERT_SIZE: u64 = 10;

/// Header offsets.
const NUM_BONES: i64 = 140;
const BONE_INDEX: i64 = 144;
const NUM_BONE_CONTROLLERS: i64 = 148;
const BONE_CONTROLLER_INDEX: i64 = 152;
const NUM_HITBOXES: i64 = 156;
const HITBOX_INDEX: i64 = 160;
const NUM_SEQUENCES: i64 = 164;
const SEQUENCE_INDEX: i64 = 168;
const NUM_SEQUENCE_GROUPS: i64 = 172;
const SEQUENCE_GROUP_INDEX: i64 = 176;
const NUM_TEXTURES: i64 = 180;
const TEXTURE_INDEX: i64 = 184;
const NUM_SKIN_REFS: i64 = 192;
const NUM_SKIN_FAMILIES: i64 = 196;
const SKIN_INDEX: i64 = 200;
const NUM_BODYPARTS: i64 = 204;
const BODYPART_INDEX: i64 = 208;
const NUM_ATTACHMENTS: i64 = 212;
const ATTACHMENT_INDEX: i64 = 216;
const NUM_TRANSITIONS: i64 = 236;
const TRANSITION_INDEX: i64 = 240;

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

/// A `std::vector` sized by a signed count: a negative one is too long.
fn vector_size(count: i32, element_size: u64) -> Result<usize, ImportError> {
    if count < 0 {
        return fail("vector too long");
    }
    check_allocation(count as u64, element_size)?;
    Ok(count as usize)
}

/// `new T[count]` with the count as `unsigned int`.
fn array_size(count: i32, element_size: u64) -> Result<(), ImportError> {
    check_allocation(count as u32 as u64, element_size)
}

/// `DefaultIOSystem::absolutePath`: up to the last separator.
fn folder(path: &str) -> &str {
    match path.rfind(['\\', '/']) {
        Some(last) => &path[..last],
        None => path,
    }
}

/// `DefaultIOSystem::completeBaseName`: the file name before its last dot.
fn base_name(path: &str) -> &str {
    let name = match path.rfind(['\\', '/']) {
        Some(last) => &path[last + 1..],
        None => path,
    };
    match name.rfind('.') {
        Some(dot) => &name[..dot],
        None => name,
    }
}

/// `BaseImporter::GetExtension`: after the path's last dot, lower case.
fn extension(path: &str) -> String {
    match path.rfind('.') {
        Some(dot) => path[dot + 1..].to_ascii_lowercase(),
        None => String::new(),
    }
}

/// A trivert (`HL1MeshTrivert`): a model vertex's use.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Trivert {
    vertex: i16,
    normal: i16,
    s: i16,
    t: i16,
}

/// `HL1MeshTrivert` with its index in the mesh.
#[derive(Clone, Copy)]
struct MeshTrivert {
    trivert: Trivert,
    local: i16,
}

struct Loader<'a> {
    path: &'a str,
    files: &'a FileSystem,
    model: &'a Buffer,
    /// The external texture file, when the model has no textures.
    textures: Option<Buffer>,
    /// Sequence group files by group (none for group 0).
    sequences: Vec<Option<Buffer>>,
    /// `temp_bones_`: each bone's transform in bind pose.
    bones: Vec<Matrix4>,
    /// The size of `unique_sequence_groups_names_`.
    sequence_group_count: usize,
    meshes: Vec<Mesh>,
}

impl Loader<'_> {
    fn texture_file(&self) -> &Buffer {
        self.textures.as_ref().unwrap_or(self.model)
    }

    fn bone(&self, index: i64) -> Result<&Matrix4, ImportError> {
        let Some(bone) = usize::try_from(index).ok().and_then(|index| self.bones.get(index)) else {
            return undefined("a bone index past the bones");
        };
        Ok(bone)
    }

    /// `load_file_into_buffer`.
    fn load_file(&self, path: &str, smallest: usize) -> Result<Buffer, ImportError> {
        if !self.files.exists(path) {
            return fail(format!("Missing file {path}."));
        }
        let Some(bytes) = self.files.open(path) else {
            return fail(format!("Failed to open MDL file {path}."));
        };
        if bytes.len() < smallest {
            return fail("MDL file is too small.");
        }
        Ok(Buffer::new(&bytes, true))
    }

    /// `load_texture_file`, then the texture header's checks.
    fn load_texture_file(&mut self) -> Result<(), ImportError> {
        if self.model.i32(NUM_TEXTURES)? == 0 {
            let separator = self.files.separator();
            let path = format!("{}{separator}{}T.{}", folder(self.path), base_name(self.path), extension(self.path));
            self.textures = Some(self.load_file(&path, HEADER_SIZE)?);
        }
        let textures = self.texture_file();
        if textures.i32(NUM_TEXTURES)? == 0 {
            return fail("[Half-Life 1 MDL] There are no textures in the file");
        }
        textures.i32(NUM_SKIN_FAMILIES)?;
        Ok(())
    }

    /// `load_sequence_groups_files`.
    fn load_sequence_group_files(&mut self) -> Result<(), ImportError> {
        let count = self.model.i32(NUM_SEQUENCE_GROUPS)?;
        if count <= 1 {
            return Ok(());
        }
        array_size(count, 8)?;
        let separator = self.files.separator();
        let mut sequences = vec![None];
        for group in 1..count {
            let path = format!("{}{separator}{}{group:02}.{}", folder(self.path), base_name(self.path), extension(self.path));
            sequences.push(Some(self.load_file(&path, SEQUENCE_HEADER_SIZE)?));
        }
        self.sequences = sequences;
        Ok(())
    }

    /// `read_textures`: names, sizes, indexed texels and the palette
    /// entries they use (and its last one).
    fn read_textures(&self) -> Result<(), ImportError> {
        let file = self.texture_file();
        let count = file.i32(NUM_TEXTURES)?;
        let start = file.i32(TEXTURE_INDEX)? as i64;
        array_size(count, 8)?;
        for index in 0..count.max(0) as i64 {
            let texture = start.wrapping_add(TEXTURE_SIZE * index);
            file.c_string_length(texture)?;
            let width = file.i32(texture + 68)?;
            let height = file.i32(texture + 72)?;
            let pixels = (width as u32).wrapping_mul(height as u32) as usize;
            check_allocation(pixels as u64, 4)?;
            let data = file.i32(texture + 76)?;
            let palette = (width.wrapping_mul(height) as i64).wrapping_add(data as i64);
            let indices = file.bytes(data as i64, pixels)?;
            let mut used = [false; 256];
            for &value in indices {
                used[value as usize] = true;
            }
            for (value, _) in used.iter().enumerate().filter(|(_, used)| **used) {
                file.bytes(palette + 3 * value as i64, 3)?;
            }
            file.bytes(palette + 765, 3)?;
            file.c_string_length(texture)?;
            file.i32(texture + 64)?;
        }
        Ok(())
    }

    /// `read_skins`: replacement skins name textures and materials by
    /// index.
    fn read_skins(&self) -> Result<(), ImportError> {
        let file = self.texture_file();
        let families = file.i32(NUM_SKIN_FAMILIES)?;
        if families <= 1 {
            return Ok(());
        }
        let references = file.i32(NUM_SKIN_REFS)?;
        let texture_count = file.i32(NUM_TEXTURES)? as u32;
        let defaults = file.i32(SKIN_INDEX)? as i64;
        let mut replacements = defaults.wrapping_add(2 * references as i64);
        for _ in 1..families {
            for reference in 0..references.max(0) as i64 {
                let default = file.i16(defaults + 2 * reference)?;
                let replacement = file.i16(replacements + 2 * reference)?;
                if default == replacement {
                    continue;
                }
                if replacement < 0 || replacement as u32 >= texture_count || default < 0 || default as u32 >= texture_count {
                    return undefined("a skin's texture index past the textures");
                }
            }
            replacements = replacements.wrapping_add(2 * references as i64);
        }
        Ok(())
    }

    /// `read_bones`: each bone's transform from its rotation (Euler angles
    /// as pitch Y, yaw Z, roll X) and position, times its parent's (a
    /// later parent's is still the identity).
    fn read_bones(&mut self) -> Result<(), ImportError> {
        let model = self.model;
        let count = model.i32(NUM_BONES)?;
        if count == 0 {
            return Ok(());
        }
        let start = model.i32(BONE_INDEX)? as i64;
        let count = vector_size(count, STRING_SIZE)?;
        for index in 0..count as i64 {
            model.c_string_length(start.wrapping_add(BONE_SIZE * index))?;
        }
        check_allocation(count as u64, TEMP_BONE_SIZE)?;

        self.bones = vec![Matrix4::IDENTITY; count];
        let mut roots = Vec::new();
        let mut children = vec![Vec::new(); count];
        for index in 0..count {
            let bone = start.wrapping_add(BONE_SIZE * index as i64);
            let mut values = [0.0f32; 6];
            for (slot, value) in values.iter_mut().enumerate() {
                *value = model.f32(bone + 64 + 4 * slot as i64)?;
            }
            let rotation = euler_quaternion(values[4], values[5], values[3]);
            let local = Matrix4::compose([1.0; 3], rotation, [values[0], values[1], values[2]]);
            self.bones[index] = local;
            let parent = model.i32(bone + 32)?;
            if parent == -1 {
                roots.push(index);
                continue;
            }
            let parent_transform = *self.bone(parent as i64)?;
            children[parent as usize].push(index);
            self.bones[index] = parent_transform.product(&local);
        }

        // `build_bone_children_hierarchy`, recursive from each root.
        let mut pending: Vec<(usize, usize)> = roots.iter().map(|&root| (root, 1)).collect();
        while let Some((bone, depth)) = pending.pop() {
            if depth > MAX_DEPTH {
                return undefined("a bone chain too deep for the stack");
            }
            for &child in &children[bone] {
                pending.push((child, depth + 1));
            }
        }
        Ok(())
    }

    /// `read_meshes`: names counted and made unique first, then each
    /// model's vertices and normals in bind pose and a mesh per model mesh.
    fn read_meshes(&mut self) -> Result<(), ImportError> {
        let model = self.model;
        let count = model.i32(NUM_BODYPARTS)?;
        if count == 0 {
            return Ok(());
        }
        let bodyparts = model.i32(BODYPART_INDEX)? as i64;
        let textures = self.texture_file();
        let texture_start = textures.i32(TEXTURE_INDEX)? as i64;
        let skin_start = textures.i32(SKIN_INDEX)? as i64;
        let count = vector_size(count, STRING_SIZE)?;

        let mut mesh_count = 0u32;
        let mut model_total = 0i32;
        for index in 0..count as i64 {
            let bodypart = bodyparts.wrapping_add(BODYPART_SIZE * index);
            model.c_string_length(bodypart)?;
            let model_count = model.i32(bodypart + 64)?;
            let models = model.i32(bodypart + 72)? as i64;
            for slot in 0..model_count.max(0) as i64 {
                let entry = models.wrapping_add(MODEL_SIZE * slot);
                mesh_count = mesh_count.wrapping_add(model.i32(entry + 72)? as u32);
                model.i32(entry + 80)?;
            }
            model_total = model_total.wrapping_add(model_count);
        }
        vector_size(model_total, STRING_SIZE)?;
        for index in 0..count as i64 {
            let bodypart = bodyparts.wrapping_add(BODYPART_SIZE * index);
            let model_count = model.i32(bodypart + 64)?;
            let models = model.i32(bodypart + 72)? as i64;
            for slot in 0..model_count.max(0) as i64 {
                model.c_string_length(models.wrapping_add(MODEL_SIZE * slot))?;
            }
        }
        check_allocation(mesh_count as u64, 8)?;
        check_allocation(count as u64, 8)?;

        let mut mesh_index = 0u32;
        for index in 0..count as i64 {
            let bodypart = bodyparts.wrapping_add(BODYPART_SIZE * index);
            let model_count = model.i32(bodypart + 64)?;
            let models = model.i32(bodypart + 72)? as i64;
            model.i32(bodypart + 68)?;
            array_size(model_count, 8)?;
            for slot in 0..model_count.max(0) as i64 {
                let entry = models.wrapping_add(MODEL_SIZE * slot);
                let mesh_list = model.i32(entry + 76)? as i64;
                let vertex_bones = model.i32(entry + 84)? as i64;
                let normal_bones = model.i32(entry + 96)? as i64;
                let vertex_start = model.i32(entry + 88)? as i64;
                let normal_start = model.i32(entry + 100)? as i64;
                let vertex_count = vector_size(model.i32(entry + 80)?, 12)?;
                let normal_count = vector_size(model.i32(entry + 92)?, 12)?;

                let mut vertices = Vec::with_capacity(vertex_count);
                for vertex in 0..vertex_count as i64 {
                    let position = read_vector(model, vertex_start.wrapping_add(12 * vertex))?;
                    let bone = model.u8(vertex_bones.wrapping_add(vertex))?;
                    vertices.push(self.bone(bone as i64)?.transform(position));
                }
                let mut normals = Vec::with_capacity(normal_count);
                for normal in 0..normal_count as i64 {
                    let direction = read_vector(model, normal_start.wrapping_add(12 * normal))?;
                    let bone = model.u8(normal_bones.wrapping_add(normal))?;
                    let normal_matrix = self.bone(bone as i64)?.inverse().transpose();
                    normals.push(normal_matrix.transform(direction));
                }

                let model_mesh_count = model.i32(entry + 72)?;
                array_size(model_mesh_count, 4)?;
                for mesh_slot in 0..model_mesh_count.max(0) as i64 {
                    if mesh_index >= mesh_count {
                        return undefined("meshes written past the scene's mesh array");
                    }
                    let source = MeshSource {
                        mesh: mesh_list.wrapping_add(MESH_SIZE * mesh_slot),
                        vertex_bones,
                        texture_start,
                        skin_start,
                        vertex_count,
                        vertices: &vertices,
                        normals: &normals,
                    };
                    let mesh = self.read_mesh(&source)?;
                    self.meshes.push(mesh);
                    mesh_index += 1;
                }
            }
        }
        Ok(())
    }

    /// One mesh: its strips and fans (each fan or strip a run of triverts
    /// after a signed count, a zero count ending them).
    fn read_mesh(&self, source: &MeshSource) -> Result<Mesh, ImportError> {
        let model = self.model;
        let textures = self.texture_file();
        let mut position = model.i32(source.mesh + 4)? as i64;
        let skin = model.i32(source.mesh + 8)? as i64;
        let texture_number = textures.i16(source.skin_start.wrapping_add(2 * skin))?;
        let texture = source.texture_start.wrapping_add(TEXTURE_SIZE * texture_number as i64);
        let s_scale = 1.0 / textures.i32(texture + 68)? as f32;
        let t_scale = 1.0 / textures.i32(texture + 72)? as f32;

        check_allocation(source.vertex_count as u64, MESH_TRIVERT_SIZE)?;
        let unused = MeshTrivert { trivert: Trivert { vertex: -1, normal: -1, s: 0, t: 0 }, local: -1 };
        let mut triverts = vec![unused; source.vertex_count];
        let mut mesh_triverts: Vec<i16> = Vec::new();
        let mut faces: Vec<[i16; 3]> = Vec::new();
        let mut similars: BTreeMap<i16, BTreeSet<i16>> = BTreeMap::new();
        let mut bones: BTreeSet<i32> = BTreeSet::new();

        loop {
            let mut count = model.i16(position)? as i32;
            position = position.wrapping_add(2);
            if count == 0 {
                break;
            }
            let fan = count < 0;
            if fan {
                count = -count;
            }
            let mut commands: Vec<i16> = Vec::new();
            while count > 0 {
                let input = Trivert {
                    vertex: model.i16(position)?,
                    normal: model.i16(position + 2)?,
                    s: model.i16(position + 4)?,
                    t: model.i16(position + 6)?,
                };
                let bone = model.u8(source.vertex_bones.wrapping_add(input.vertex as i64))? as i32;
                let Some(private) = usize::try_from(input.vertex).ok().filter(|&slot| slot < triverts.len()) else {
                    return undefined("a trivert's vertex index past the triverts");
                };
                if triverts[private].local == -1 {
                    let local = mesh_triverts.len() as i16;
                    triverts[private] = MeshTrivert { trivert: input, local };
                    mesh_triverts.push(input.vertex);
                    commands.push(local);
                    bones.insert(bone);
                } else if triverts[private].trivert == input {
                    commands.push(triverts[private].local);
                } else {
                    // The first registered similar trivert that is the same.
                    let mut similar = None;
                    if let Some(set) = similars.get(&input.vertex) {
                        for &index in set {
                            let Some(other) = usize::try_from(index).ok().and_then(|index| triverts.get(index)) else {
                                return undefined("a similar trivert index past the triverts");
                            };
                            if other.trivert == input {
                                similar = Some(index);
                                break;
                            }
                        }
                    }
                    match similar {
                        Some(index) => commands.push(triverts[index as usize].local),
                        None => {
                            let local = mesh_triverts.len() as i16;
                            let new_index = triverts.len() as i16;
                            similars.entry(input.vertex).or_default().insert(new_index);
                            triverts.push(MeshTrivert { trivert: input, local });
                            mesh_triverts.push(new_index);
                            commands.push(local);
                            bones.insert(bone);
                        }
                    }
                }
                count -= 1;
                position = position.wrapping_add(TRIVERT_SIZE);
            }

            // (`reserve` of `int(size − 2)`: fewer than two triverts ask for
            // a vector too long.)
            if commands.len() < 2 {
                return fail("vector too long");
            }
            for face in 0..commands.len() - 2 {
                let corners = if fan {
                    [commands[0], commands[face + 1], commands[face + 2]]
                } else if face & 1 == 1 {
                    [commands[face + 1], commands[face], commands[face + 2]]
                } else {
                    [commands[face], commands[face + 1], commands[face + 2]]
                };
                faces.push(corners);
            }
        }

        textures.i16(source.skin_start.wrapping_add(2 * skin))?;
        let mut mesh = Mesh { primitive_types: PRIMITIVE_TRIANGLE, ..Mesh::default() };
        if mesh_triverts.is_empty() {
            return Ok(mesh);
        }
        let count = mesh_triverts.len();
        let mut positions = Vec::with_capacity(count);
        let mut normals = Vec::with_capacity(count);
        let mut coordinates = Vec::with_capacity(count);
        for &index in &mesh_triverts {
            let Some(entry) = usize::try_from(index).ok().and_then(|index| triverts.get(index)) else {
                return undefined("a mesh trivert index past the triverts");
            };
            let trivert = entry.trivert;
            let Some(&position) = usize::try_from(trivert.vertex).ok().and_then(|index| source.vertices.get(index)) else {
                return undefined("a vertex index past the model's vertices");
            };
            let Some(&normal) = usize::try_from(trivert.normal).ok().and_then(|index| source.normals.get(index)) else {
                return undefined("a normal index past the model's normals");
            };
            positions.push(position);
            normals.push(normal);
            coordinates.push([trivert.s as f32 * s_scale, trivert.t as f32 * -t_scale, 0.0]);
        }
        for &bone in &bones {
            self.bone(bone as i64)?;
        }
        let output_faces = faces.iter().map(|face| vec![face[2] as u32, face[1] as u32, face[0] as u32]).collect();
        mesh.vertex_count = count;
        mesh.vertices = Some(positions);
        mesh.normals = Some(normals);
        mesh.uv_components[0] = 2;
        mesh.texture_coords[0] = Some(coordinates);
        mesh.faces = Some(output_faces);
        Ok(mesh)
    }

    /// `read_sequence_groups_info`: labels, and file names past group 0.
    fn read_sequence_groups_info(&mut self) -> Result<(), ImportError> {
        let model = self.model;
        let count = model.i32(NUM_SEQUENCE_GROUPS)?;
        if count == 0 {
            return Ok(());
        }
        array_size(count, 8)?;
        let start = model.i32(SEQUENCE_GROUP_INDEX)? as i64;
        let count = vector_size(count, STRING_SIZE)?;
        for index in 0..count as i64 {
            model.c_string_length(start.wrapping_add(SEQUENCE_GROUP_SIZE * index))?;
        }
        for index in 1..count as i64 {
            model.c_string_length(start.wrapping_add(SEQUENCE_GROUP_SIZE * index) + 32)?;
        }
        self.sequence_group_count = count;
        Ok(())
    }

    /// `read_animations`: each sequence's blends, a channel per bone, a key
    /// per frame from compressed values.
    fn read_animations(&self) -> Result<(), ImportError> {
        let model = self.model;
        let count = model.i32(NUM_SEQUENCES)?;
        if count == 0 {
            return Ok(());
        }
        let start = model.i32(SEQUENCE_INDEX)? as i64;
        let count = vector_size(count, STRING_SIZE)?;
        for index in 0..count as i64 {
            model.c_string_length(start.wrapping_add(SEQUENCE_SIZE * index))?;
        }
        let mut animation_count = 0u32;
        for index in 0..count as i64 {
            animation_count = animation_count.wrapping_add(model.i32(start.wrapping_add(SEQUENCE_SIZE * index) + 120)? as u32);
        }
        check_allocation(animation_count as u64, 8)?;

        let bone_count = model.i32(NUM_BONES)?;
        for index in 0..count as i64 {
            let sequence = start.wrapping_add(SEQUENCE_SIZE * index);
            let group = model.i32(sequence + 156)?;
            let groups = model.i32(SEQUENCE_GROUP_INDEX)? as i64;
            let (file, mut animation) = if group == 0 {
                let group_entry = groups;
                let base = model.i32(group_entry + 100)? as i64;
                let offset = model.i32(sequence + 124)? as i64;
                (model, base.wrapping_add(offset))
            } else {
                let Some(Some(file)) = usize::try_from(group).ok().and_then(|group| self.sequences.get(group)) else {
                    return undefined("a sequence group file that was never loaded");
                };
                (file, model.i32(sequence + 124)? as i64)
            };
            let blends = model.i32(sequence + 120)?;
            let frames = model.i32(sequence + 56)?;
            model.f32(sequence + 32)?;
            for _ in 0..blends.max(0) {
                let bones = model.i32(BONE_INDEX)? as i64;
                array_size(bone_count, 8)?;
                for bone in 0..bone_count.max(0) as i64 {
                    self.bone(bone)?;
                    array_size(frames, KEY_SIZE)?;
                    let bone_entry = bones.wrapping_add(BONE_SIZE * bone);
                    for frame in 0..frames.max(0) {
                        for axis in 0..3i64 {
                            let rotation = file.u16(animation + 2 * (axis + 3))?;
                            if rotation != 0 {
                                model.f32(bone_entry + 88 + 4 * (axis + 3))?;
                                extract_value(file, animation.wrapping_add(rotation as i64), frame)?;
                            }
                            model.f32(bone_entry + 64 + 4 * (axis + 3))?;
                            let translation = file.u16(animation + 2 * axis)?;
                            if translation != 0 {
                                model.f32(bone_entry + 88 + 4 * axis)?;
                                extract_value(file, animation.wrapping_add(translation as i64), frame)?;
                            }
                            model.f32(bone_entry + 64 + 4 * axis)?;
                        }
                    }
                    animation = animation.wrapping_add(ANIMATION_OFFSETS_SIZE);
                }
            }
        }
        Ok(())
    }

    /// `read_sequence_infos`: each sequence's group (by name), motion bone,
    /// blend controllers and events.
    fn read_sequence_infos(&self) -> Result<(), ImportError> {
        let model = self.model;
        let count = model.i32(NUM_SEQUENCES)?;
        if count == 0 {
            return Ok(());
        }
        let start = model.i32(SEQUENCE_INDEX)? as i64;
        array_size(count, 8)?;
        for index in 0..count.max(0) as i64 {
            let sequence = start.wrapping_add(SEQUENCE_SIZE * index);
            // (The fields read lie from the frame rate to the node flags.)
            model.bytes(sequence + 32, 140)?;
            let group = model.i32(sequence + 156)?;
            if group < 0 || group as usize >= self.sequence_group_count {
                return undefined("a sequence group index past the groups");
            }
            self.bone(model.i32(sequence + 72)? as i64)?;

            let events = model.i32(sequence + 48)?;
            if events == 0 {
                continue;
            }
            let event_start = model.i32(sequence + 52)? as i64;
            array_size(events, 8)?;
            for event in 0..events as u32 as i64 {
                let entry = event_start.wrapping_add(EVENT_SIZE * event);
                model.bytes(entry, 8)?;
                model.c_string_length(entry + 12)?;
            }
        }
        Ok(())
    }

    /// `read_sequence_transitions`: a metadata entry per pair of nodes.
    fn read_sequence_transitions(&self) -> Result<(), ImportError> {
        let model = self.model;
        let count = model.i32(NUM_TRANSITIONS)?;
        if count == 0 {
            return Ok(());
        }
        let start = model.i32(TRANSITION_INDEX)? as i64;
        let entries = count.wrapping_mul(count) as u32;
        if entries == 0 {
            return undefined("metadata of no entries written");
        }
        check_allocation(entries as u64, AI_STRING_SIZE)?;
        model.bytes(start, entries as usize)?;
        Ok(())
    }

    /// `read_attachments`, `read_hitboxes` and `read_bone_controllers`:
    /// each names its bone. (Offsets of the bone index and the bytes read
    /// in each entry.)
    fn read_bone_users(&self, count_offset: i64, index_offset: i64, size: i64, bone: i64, read: (i64, usize)) -> Result<(), ImportError> {
        let model = self.model;
        let count = model.i32(count_offset)?;
        if count == 0 {
            return Ok(());
        }
        let start = model.i32(index_offset)? as i64;
        array_size(count, 8)?;
        for index in 0..count.max(0) as i64 {
            let entry = start.wrapping_add(size * index);
            model.bytes(entry + read.0, read.1)?;
            self.bone(model.i32(entry + bone)? as i64)?;
        }
        Ok(())
    }

    /// `load_file`.
    fn load(&mut self) -> Result<(), ImportError> {
        let model = self.model;
        for offset in [NUM_BODYPARTS, NUM_BONES, NUM_BONE_CONTROLLERS, NUM_SEQUENCES, NUM_SEQUENCE_GROUPS, NUM_ATTACHMENTS] {
            model.i32(offset)?;
        }
        self.load_texture_file()?;
        self.load_sequence_group_files()?;
        self.read_textures()?;
        self.read_skins()?;
        self.read_bones()?;
        self.read_meshes()?;
        self.read_sequence_groups_info()?;
        self.read_animations()?;
        self.read_sequence_infos()?;
        self.read_sequence_transitions()?;
        self.read_bone_users(NUM_ATTACHMENTS, ATTACHMENT_INDEX, ATTACHMENT_SIZE, 36, (36, 16))?;
        self.read_bone_users(NUM_HITBOXES, HITBOX_INDEX, HITBOX_SIZE, 0, (0, 32))?;
        self.read_bone_users(NUM_BONE_CONTROLLERS, BONE_CONTROLLER_INDEX, BONE_CONTROLLER_SIZE, 0, (0, 24))?;
        // `read_global_info`.
        model.bytes(76, 60)?;
        for offset in [NUM_TRANSITIONS, NUM_BODYPARTS] {
            model.i32(offset)?;
        }
        self.texture_file().i32(NUM_SKIN_FAMILIES)?;
        Ok(())
    }
}

/// What a mesh is built from: its entry, its model's vertex bones, the
/// texture and skin tables, and the model's vertices and normals.
struct MeshSource<'a> {
    mesh: i64,
    vertex_bones: i64,
    texture_start: i64,
    skin_start: i64,
    vertex_count: usize,
    vertices: &'a [Vector3],
    normals: &'a [Vector3],
}

fn read_vector(buffer: &Buffer, position: i64) -> Result<Vector3, ImportError> {
    Ok([buffer.f32(position)?, buffer.f32(position + 4)?, buffer.f32(position + 8)?])
}

/// `extract_anim_value`: past whole spans to the frame's, then its value
/// (or the span's last).
fn extract_value(file: &Buffer, position: i64, frame: i32) -> Result<(), ImportError> {
    let mut position = position;
    let mut k = frame;
    loop {
        let total = file.u8(position + 1)? as i32;
        if total > k {
            break;
        }
        k -= total;
        let valid = file.u8(position)? as i64;
        position = position.wrapping_add(2 * (valid + 1));
    }
    let valid = file.u8(position)? as i32;
    let index = if valid > k { k as i64 + 1 } else { valid as i64 };
    file.i16(position + 2 * index)?;
    Ok(())
}

/// `InternReadFile_HL1`.
pub(super) fn read(path: &str, model: &Buffer, files: &FileSystem) -> Result<Scene, ImportError> {
    let mut loader = Loader {
        path,
        files,
        model,
        textures: None,
        sequences: Vec::new(),
        bones: Vec::new(),
        sequence_group_count: 0,
        meshes: Vec::new(),
    };
    loader.load()?;
    Ok(Scene { meshes: loader.meshes })
}
