//! 3D GameStudio MDL7 models (`InternReadFile_3DGS_MDL7`): groups of skins,
//! texture coordinates, triangles and vertices, the first frame's vertex
//! replacements, then a mesh per material of each group.
//!
//! Every skin is a material; faces are split by their material index (with
//! two texture coordinate sets, by combinations of two, and group 0 keeps
//! only the combinations). Bones only decide failures: the node graph of a
//! skeleton with a root reads past the bone list, and a mesh whose bones
//! are not the first ones writes past its bone array.

use super::skins::{self, Skins};
use super::{Buffer, advance};
use crate::md2::lookup_normal;
use crate::scene::{ImportError, Mesh, PRIMITIVE_TRIANGLE, Scene, check_allocation, fail};
use crate::vector::Vector3;

/// `sizeof(MDL::Header_MDL7)` and `sizeof(MDL::Group_MDL7)`.
const HEADER_SIZE: i64 = 48;
const GROUP_SIZE: i64 = 44;

/// The bone structure sizes bones are loaded with.
const BONE_SIZES: [u16; 3] = [36, 48, 16];

/// Triangle structure sizes: one texture coordinate set, with a material,
/// two sets.
const TRIANGLE_ONE_UV: u16 = 12;
const TRIANGLE_ONE_UV_MATERIAL: u16 = 16;
const TRIANGLE_TWO_UV: u16 = 26;

/// Vertex structure sizes: with a full normal, with a normal index.
const VERTEX_FULL_NORMAL: u16 = 26;
const VERTEX_NORMAL_INDEX: u16 = 16;

/// `sizeof(aiVector3D)` and `sizeof(MDL::IntFace_MDL7)`.
const VECTOR_SIZE: u64 = 12;
const FACE_SIZE: u64 = 20;

/// `IntBone_MDL7::iParent` of a bone no parent search reached.
const NO_PARENT: u16 = 0xffff;

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

/// `MDL::Header_MDL7`, as far as it is read.
struct Header {
    bones_num: u32,
    groups_num: u32,
    data_size: u32,
    bone_size: u16,
    triangle_size: u16,
    main_vertex_size: u16,
    frame_vertex_size: u16,
    bone_transform_size: u16,
    frame_size: u16,
}

impl Header {
    /// The header, validated (`ValidateHeader_3DGS_MDL7`).
    fn read(buffer: &Buffer) -> Result<Header, ImportError> {
        let color_value_size = buffer.u16(32)?;
        let skin_point_size = buffer.u16(36)?;
        let skin_size = buffer.u16(30)?;
        let header = Header {
            bones_num: buffer.u32(8)?,
            groups_num: buffer.u32(12)?,
            data_size: buffer.u32(16)?,
            bone_size: buffer.u16(28)?,
            triangle_size: buffer.u16(38)?,
            main_vertex_size: buffer.u16(40)?,
            frame_vertex_size: buffer.u16(42)?,
            bone_transform_size: buffer.u16(44)?,
            frame_size: buffer.u16(46)?,
        };
        if color_value_size != 16 {
            return fail("[3DGS MDL7] sizeof(MDL::ColorValue_MDL7) != pcHeader->colorvalue_stc_size");
        }
        if skin_point_size != 8 {
            return fail("[3DGS MDL7] sizeof(MDL::TexCoord_MDL7) != pcHeader->skinpoint_stc_size");
        }
        if skin_size != 28 {
            return fail("sizeof(MDL::Skin_MDL7) != pcHeader->skin_stc_size");
        }
        if header.groups_num == 0 {
            return fail("[3DGS MDL7] No frames found");
        }
        Ok(header)
    }

    /// A vertex's normal by the vertex structure's size.
    fn normal(&self, buffer: &Buffer, vertex: i64) -> Result<Vector3, ImportError> {
        if self.main_vertex_size >= VERTEX_FULL_NORMAL {
            return Ok([buffer.f32(vertex + 14)?, buffer.f32(vertex + 18)?, buffer.f32(vertex + 22)?]);
        }
        if self.main_vertex_size >= VERTEX_NORMAL_INDEX {
            return Ok(lookup_normal(buffer.u8(vertex + 14)?));
        }
        Ok([0.0; 3])
    }
}

/// `LoadBones_3DGS_MDL7` and `CalcAbsBoneMatrices_3DGS_MDL7`: each bone's
/// parent as the search set it. The search runs `bones_num` times over all
/// bones with the parent index 0xffff, 0, 1, …; a bone is reached when its
/// parent index comes up, and its position and name are read then.
fn load_bones(buffer: &Buffer, header: &Header) -> Result<Option<Vec<u16>>, ImportError> {
    let count = header.bones_num;
    if count == 0 || !BONE_SIZES.contains(&header.bone_size) {
        return Ok(None);
    }
    check_allocation(count as u64, 8)?;
    let mut parents = Vec::with_capacity(count as usize);
    for bone in 0..count {
        let start = HEADER_SIZE + bone.wrapping_mul(header.bone_size as u32) as i64;
        let parent = buffer.u16(start)?;
        // The search's parent indices: 0xffff, then 0 on, `count` of them.
        let reached = count > 0xffff || parent == NO_PARENT || (parent as u32) + 2 <= count;
        if !reached {
            parents.push(NO_PARENT);
            continue;
        }
        buffer.bytes(start + 4, 12)?;
        if header.bone_size != 16 {
            let name_size = header.bone_size as i64 - 16;
            for offset in 0..name_size {
                if buffer.u8(start + 16 + offset)? == 0 {
                    break;
                }
            }
        }
        parents.push(parent);
    }
    Ok(Some(parents))
}

/// `MDL::Group_MDL7`, as far as it is read.
struct Group {
    index: u32,
    skin_count: i32,
    texture_coord_count: i32,
    triangle_count: i32,
    vertex_count: i32,
    frame_count: i32,
    texture_coords: i64,
    triangles: i64,
    vertices: i64,
}

/// `IntFace_MDL7`: corners (as written: last first) and two material
/// indices.
#[derive(Clone, Copy, Default)]
struct Face {
    indices: [u32; 3],
    materials: [u32; 2],
}

/// `IntGroupData_MDL7`: a vertex per corner.
struct GroupData {
    faces: Vec<Face>,
    positions: Vec<Vector3>,
    normals: Vec<Vector3>,
    bones: Vec<u32>,
    texture_coords: Vec<Vector3>,
    second_texture_coords: Vec<Vector3>,
    two_sets: bool,
}

/// What groups share: the material count (`pcMats.size()`) and the size
/// of `abNeedMaterials`.
struct Shared {
    material_count: u64,
    needed_count: u64,
}

/// The largest `std::vector` size.
const VECTOR_MAX: u64 = (1 << 60) - 1;

/// `ReadFaces_3DGS_MDL7`: indices past the vertices (one past is let
/// through) are clamped and written back into the file's copy.
fn read_faces(buffer: &mut Buffer, header: &Header, group: &Group, data: &mut GroupData) -> Result<(), ImportError> {
    let mut triangle = group.triangles;
    let mut corner = 0usize;
    for face in 0..group.triangle_count as u32 as usize {
        for slot in 0..3 {
            let mut index = buffer.u16(triangle + 2 * slot as i64)? as u32;
            if index > group.vertex_count as u32 {
                index = group.vertex_count.wrapping_sub(1) as u32;
                buffer.set_u16(triangle + 2 * slot as i64, index as u16)?;
            }
            data.faces[face].indices[2 - slot] = corner as u32;

            let vertex = group.vertices + index.wrapping_mul(header.main_vertex_size as u32) as i64;
            data.positions[corner] = [buffer.f32(vertex)?, buffer.f32(vertex + 4)?, buffer.f32(vertex + 8)?];
            if !data.bones.is_empty() {
                data.bones[corner] = buffer.u16(vertex + 12)? as u32;
            }
            data.normals[corner] = header.normal(buffer, vertex)?;

            if header.triangle_size >= TRIANGLE_ONE_UV {
                if group.texture_coord_count != 0 {
                    let coordinate = texture_coordinate(buffer, group, triangle + 6 + 2 * slot as i64)?;
                    data.texture_coords[corner] = coordinate;
                }
                if header.triangle_size >= TRIANGLE_ONE_UV_MATERIAL {
                    data.faces[face].materials[0] = buffer.i32(triangle + 12)? as u32;
                }
            }
            if header.triangle_size >= TRIANGLE_TWO_UV {
                if group.texture_coord_count != 0 {
                    let coordinate = texture_coordinate(buffer, group, triangle + 16 + 2 * slot as i64)?;
                    data.second_texture_coords[corner] = coordinate;
                    // (Read only to decide that a second set is needed,
                    // which it already is.)
                    buffer.i32(triangle + 22)?;
                    buffer.i32(triangle + 12)?;
                }
                data.faces[face].materials[1] = buffer.i32(triangle + 22)? as u32;
            }
            corner += 1;
        }
        triangle = triangle.wrapping_add(header.triangle_size as i64);
    }
    Ok(())
}

/// A texture coordinate (index at `index_position`; one past the list is
/// let through), V flipped.
fn texture_coordinate(buffer: &Buffer, group: &Group, index_position: i64) -> Result<Vector3, ImportError> {
    let mut index = buffer.u16(index_position)? as u32;
    if index > group.texture_coord_count as u32 {
        index = group.texture_coord_count.wrapping_sub(1) as u32;
    }
    let coordinate = advance(group.texture_coords, 8, index as i64);
    let u = buffer.f32(coordinate)?;
    let v = 1.0 - buffer.f32(coordinate + 4)?;
    Ok([u, v, 0.0])
}

/// `SortByMaterials_3DGS_MDL7`: the faces of each material. With two
/// texture coordinate sets, faces whose second material differs go to a
/// combined material, numbered from 0 like the plain ones (whose lists
/// they share); group 0 then keeps only the combined materials.
fn sort_by_materials(group: &Group, data: &GroupData, shared: &mut Shared) -> Result<Vec<Vec<u32>>, ImportError> {
    let material_count = shared.material_count as u32;
    let face_count = group.triangle_count as u32;
    if !data.two_sets {
        let mut split = vec![Vec::new(); material_count as usize];
        for face in 0..face_count {
            let material = data.faces[face as usize].materials[0];
            let slot = if material >= material_count { material_count - 1 } else { material };
            split[slot as usize].push(face);
        }
        return Ok(split);
    }

    let mut combined: Vec<[u32; 2]> = Vec::new();
    // `aiTempSplit`: lists for the plain materials, then null pointers.
    let mut lists: Vec<Option<Vec<u32>>> = vec![None; material_count as usize * 2];
    for list in lists.iter_mut().take(material_count as usize) {
        *list = Some(Vec::new());
    }
    for face in 0..face_count {
        let materials = data.faces[face as usize].materials;
        let first = if materials[0] >= material_count { material_count - 1 } else { materials[0] };
        let mut second = materials[1];
        let mut slot = first as usize;
        if second != u32::MAX && first != second {
            if second >= material_count {
                second = material_count - 1;
            }
            slot = match combined.iter().position(|&pair| pair == [first, second]) {
                Some(found) => found,
                None => {
                    combined.push([first, second]);
                    combined.len() - 1
                }
            };
            if slot == lists.len() {
                lists.push(Some(Vec::new()));
            }
        }
        let Some(list) = lists[slot].as_mut() else {
            return undefined("a combined material's face list that was never made");
        };
        list.push(face);
    }

    if group.index == 0 {
        shared.material_count = combined.len() as u64;
    } else {
        if !combined.is_empty() {
            return undefined("face lists of combined materials left uninitialised");
        }
        shared.material_count = material_count as u64;
    }
    let split = lists.into_iter().take(material_count as usize).map(Option::unwrap_or_default).collect();
    Ok(split)
}

/// `ProcessFrames_3DGS_MDL7`: frames up to one that would pass the data
/// size; the first frame's vertices replace the corners of their index.
/// Returns the cursor after the frames read.
fn process_frames(buffer: &Buffer, header: &Header, group: &Group, data: &mut Option<GroupData>, bones: bool, position: i64) -> Result<i64, ImportError> {
    let mut position = position;
    for frame in 0..group.frame_count as u32 {
        let vertex_count = buffer.u32(position + 16)?;
        let transform_count = buffer.u32(position + 20)?;
        let vertices_size = vertex_count.wrapping_mul(header.frame_vertex_size as u32);
        let transforms_size = transform_count.wrapping_mul(header.bone_transform_size as u32);
        let frame_size = (header.frame_size as u32).wrapping_add(vertices_size).wrapping_add(transforms_size);
        if position + frame_size as i64 > header.data_size as i64 {
            return Ok(position);
        }

        if frame == 0 {
            let frame_vertices = position.wrapping_add(header.frame_size as i64);
            for vertex_index in 0..vertex_count {
                let vertex = frame_vertices + vertex_index.wrapping_mul(header.frame_vertex_size as u32) as i64;
                let index = buffer.u16(vertex + 12)?;
                if index as i32 >= group.vertex_count {
                    continue;
                }
                let replacement = [buffer.f32(vertex)?, buffer.f32(vertex + 4)?, buffer.f32(vertex + 8)?];
                let normal = header.normal(buffer, vertex)?;
                replace_vertex(buffer, header, group, data, index, replacement, normal)?;
            }
        }

        if bones && transform_count != 0 && group.index == 0 {
            let mut transform = position.wrapping_add(header.frame_size as i64).wrapping_add(vertices_size as i64);
            for _ in 0..transform_count {
                let bone = buffer.u16(transform + 64)?;
                if (bone as u32) < header.bones_num {
                    buffer.bytes(transform, 48)?;
                }
                transform = transform.wrapping_add(header.bone_transform_size as i64);
            }
        }
        position = position.wrapping_add(frame_size as i64);
    }
    Ok(position)
}

/// Every corner whose (clamped) vertex index is `index` gets the frame's
/// position and normal.
fn replace_vertex(buffer: &Buffer, header: &Header, group: &Group, data: &mut Option<GroupData>, index: u16, position: Vector3, normal: Vector3) -> Result<(), ImportError> {
    let mut triangle = group.triangles;
    let mut corner = 0usize;
    for _ in 0..group.triangle_count as u32 {
        for slot in 0..3 {
            if buffer.u16(triangle + 2 * slot)? == index {
                let Some(data) = data.as_mut() else {
                    return undefined("a frame vertex written to a group without vertices");
                };
                data.positions[corner] = position;
                data.normals[corner] = normal;
            }
            corner += 1;
        }
        triangle = triangle.wrapping_add(header.triangle_size as i64);
    }
    Ok(())
}

/// `GenerateOutputMeshes_3DGS_MDL7`: a mesh per material with faces, its
/// corners copied in face order.
fn generate_meshes(header: &Header, data: &GroupData, split: &[Vec<u32>], material_count: u64) -> Result<Vec<Mesh>, ImportError> {
    let mut meshes = Vec::new();
    for (material, faces) in split.iter().enumerate().take(material_count as usize) {
        if faces.is_empty() {
            continue;
        }
        let vertex_count = faces.len() * 3;
        check_allocation(vertex_count as u64, VECTOR_SIZE)?;
        let mut positions = Vec::with_capacity(vertex_count);
        let mut normals = Vec::with_capacity(vertex_count);
        let mut coordinates = Vec::with_capacity(vertex_count);
        let mut second_coordinates = Vec::with_capacity(vertex_count);
        let mut used_bones = Vec::new();
        let mut output_faces = Vec::with_capacity(faces.len());
        for &face in faces {
            let indices = data.faces[face as usize].indices;
            let start = positions.len() as u32;
            for index in indices {
                let index = index as usize;
                positions.push(data.positions[index]);
                normals.push(data.normals[index]);
                if !data.texture_coords.is_empty() {
                    coordinates.push(data.texture_coords[index]);
                    if !data.second_texture_coords.is_empty() {
                        second_coordinates.push(data.second_texture_coords[index]);
                    }
                }
                if !data.bones.is_empty() {
                    let bone = data.bones[index].min(header.bones_num.wrapping_sub(1));
                    if used_bones.len() <= bone as usize {
                        used_bones.resize(bone as usize + 1, false);
                    }
                    used_bones[bone as usize] = true;
                }
            }
            output_faces.push(vec![start, start + 1, start + 2]);
        }
        // `mBones[bone]` is written at each used bone's own index into an
        // array as long as the number of used bones.
        if used_bones.iter().any(|&used| !used) {
            return undefined("bone pointers written past a mesh's bone array");
        }

        let mut mesh = Mesh {
            primitive_types: PRIMITIVE_TRIANGLE,
            material_index: material as u32,
            vertex_count,
            vertices: Some(positions),
            normals: Some(normals),
            faces: Some(output_faces),
            ..Mesh::default()
        };
        if !data.texture_coords.is_empty() {
            mesh.uv_components[0] = 2;
            mesh.texture_coords[0] = Some(coordinates);
            if !data.second_texture_coords.is_empty() {
                mesh.uv_components[1] = 2;
                mesh.texture_coords[1] = Some(second_coordinates);
            }
        }
        meshes.push(mesh);
    }
    Ok(meshes)
}

/// The material list's growth past 2 GiB of pointers fails.
const MATERIAL_LIMIT: u64 = 1 << 28;

/// The group header at `start` (`Group_MDL7`): its counts. The cursor
/// after it must lie in the file.
fn read_group(buffer: &Buffer, index: u32, start: i64) -> Result<Group, ImportError> {
    buffer.size_check(start.wrapping_add(GROUP_SIZE))?;
    // (The type is only warned about; the name is copied.)
    buffer.u8(start)?;
    buffer.bytes(start + 8, 16)?;
    Ok(Group {
        index,
        skin_count: buffer.i32(start + 24)?,
        texture_coord_count: buffer.i32(start + 28)?,
        triangle_count: buffer.i32(start + 32)?,
        vertex_count: buffer.i32(start + 36)?,
        frame_count: buffer.i32(start + 40)?,
        texture_coords: 0,
        triangles: 0,
        vertices: 0,
    })
}

/// The group's skins (a material each), `reserve` and `resize` by the
/// signed count first. A skin that leaves the cursor where it was is parsed
/// again and again the same way.
fn read_skins(buffer: &Buffer, skins: &mut Skins, group: &Group, shared: &mut Shared, position: i64) -> Result<i64, ImportError> {
    let added = group.skin_count as i64 as u64;
    let reserved = shared.material_count.wrapping_add(added);
    if reserved > VECTOR_MAX {
        return fail("vector too long");
    }
    if reserved > shared.material_count {
        check_allocation(reserved, 8)?;
    }
    let needed = shared.needed_count.wrapping_add(added);
    if needed > VECTOR_MAX {
        return fail("vector too long");
    }
    check_allocation(needed / 8, 1)?;
    shared.needed_count = needed;

    let mut position = position;
    let count = group.skin_count as u32 as u64;
    for skin in 0..count {
        shared.material_count += 1;
        let next = skins::parse_skin(buffer, skins, position)?;
        if next == position {
            shared.material_count += count - skin - 1;
            if shared.material_count > MATERIAL_LIMIT {
                return fail("bad allocation");
            }
            break;
        }
        position = next;
    }
    if shared.material_count == 0 {
        shared.material_count = 1;
        shared.needed_count = 1;
    }
    Ok(position)
}

/// `InternReadFile_3DGS_MDL7`.
pub(super) fn read(mut buffer: Buffer, skins: &mut Skins) -> Result<Scene, ImportError> {
    let header = Header::read(&buffer)?;
    let bones_size = header.bones_num.wrapping_mul(header.bone_size as u32);
    let mut position = HEADER_SIZE + bones_size as i64;
    let bones = load_bones(&buffer, &header)?;
    // A mesh list (a `std::vector`) and a 16-byte name per group.
    check_allocation(header.groups_num as u64, 24)?;
    check_allocation(header.groups_num as u64, 16)?;

    let mut shared = Shared { material_count: 0, needed_count: 0 };
    let mut meshes = Vec::new();
    for group_index in 0..header.groups_num {
        let mut group = read_group(&buffer, group_index, position)?;
        position = position.wrapping_add(GROUP_SIZE);
        position = read_skins(&buffer, skins, &group, &mut shared, position)?;

        // (`int` sizes: the structure size times the signed count.)
        group.texture_coords = position;
        position = position.wrapping_add(8i32.wrapping_mul(group.texture_coord_count) as i64);
        group.triangles = position;
        position = position.wrapping_add((header.triangle_size as i32).wrapping_mul(group.triangle_count) as i64);
        group.vertices = position;
        position = position.wrapping_add((header.main_vertex_size as i32).wrapping_mul(group.vertex_count) as i64);
        buffer.size_check(position)?;

        let mut data = None;
        let mut split = None;
        if group.triangle_count != 0 && group.vertex_count != 0 {
            let corners = group.triangle_count.wrapping_mul(3) as u32 as u64;
            check_allocation(corners, VECTOR_SIZE)?;
            check_allocation(group.triangle_count as i64 as u64, FACE_SIZE)?;
            let corners = corners as usize;
            let mut group_data = GroupData {
                faces: vec![Face::default(); group.triangle_count as usize],
                positions: vec![[0.0; 3]; corners],
                normals: vec![[0.0; 3]; corners],
                bones: if bones.is_some() { vec![u32::MAX; corners] } else { Vec::new() },
                texture_coords: Vec::new(),
                second_texture_coords: Vec::new(),
                two_sets: false,
            };
            if group.texture_coord_count != 0 {
                group_data.texture_coords = vec![[0.0; 3]; corners];
                if header.triangle_size >= TRIANGLE_TWO_UV {
                    group_data.second_texture_coords = vec![[0.0; 3]; corners];
                    group_data.two_sets = true;
                }
            }
            read_faces(&mut buffer, &header, &group, &mut group_data)?;
            let lists = sort_by_materials(&group, &group_data, &mut shared)?;
            // `abNeedMaterials[material] = true` for each material in use.
            for (material, list) in lists.iter().enumerate().take(shared.material_count as usize) {
                if !list.is_empty() && material as u64 >= shared.needed_count {
                    return undefined("a material flag set past its list");
                }
            }
            data = Some(group_data);
            split = Some(lists);
        }

        position = process_frames(&buffer, &header, &group, &mut data, bones.is_some(), position)?;
        let (Some(data), Some(split)) = (data.as_ref(), split.as_ref()) else {
            // A group without faces has no face lists to look at.
            if shared.material_count != 0 {
                return undefined("the face lists of a group without faces read");
            }
            continue;
        };
        meshes.extend(generate_meshes(&header, data, split, shared.material_count)?);
    }

    // The skeleton's nodes: a bone left without a parent recurses into the
    // bone list from one past itself, reading past its end.
    if let Some(parents) = &bones
        && parents.contains(&NO_PARENT)
    {
        return undefined("the bone node graph read past the bone list");
    }
    Ok(Scene { meshes })
}
