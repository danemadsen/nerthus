//! Assimp's Milkshape 3D importer (`MS3DLoader.cpp`): version 4 files of
//! vertices, triangles (three corners with normals and UVs), groups (a
//! mesh each, three vertices per triangle), materials, joints and the
//! optional comments and extra vertex weights.
//!
//! The joints only feed bones and the node graph, which don't reach the
//! mesh; what remains of them are the reads and the failures: bone
//! indices (signed bytes, -1 for none) past the joints, material indices
//! past the materials (by `>`, so equal passes), and the model comment,
//! which the stream isn't moved past (the extra vertex data is read from
//! it). Joint chains deeper than the C++ recursion survives fail, and so
//! do joint graphs that leave child nodes uninitialised.

use crate::scene::{ImportError, Mesh, PRIMITIVE_TRIANGLE, Scene, fail};
use crate::stream_reader::StreamReader;

/// Joint nesting the C++ recursion would only end with a stack overflow.
const MAX_DEPTH: usize = 2048;

struct TempVertex {
    position: [f32; 3],
    bone_ids: [u32; 4],
}

struct TempTriangle {
    indices: [u32; 3],
    normals: [[f32; 3]; 3],
    uv: [[f32; 2]; 3],
}

struct TempGroup {
    triangles: Vec<u32>,
    material: u32,
}

struct TempJoint {
    name: Vec<u8>,
    parent_name: Vec<u8>,
}

fn read_vector(stream: &mut StreamReader) -> Result<[f32; 3], ImportError> {
    Ok([stream.get_f4()?, stream.get_f4()?, stream.get_f4()?])
}

/// A 32-byte name, as `strcmp` sees it (to its first NUL).
fn read_name(stream: &mut StreamReader) -> Result<Vec<u8>, ImportError> {
    let bytes = stream.copy_and_advance(32)?;
    let length = bytes.iter().position(|&byte| byte == 0).unwrap_or(32);
    Ok(bytes[..length].to_vec())
}

/// `ReadComments` for a list of `count` items.
fn read_comments(stream: &mut StreamReader, count: usize) -> Result<(), ImportError> {
    let comment_count = stream.get_u2()?;
    for _ in 0..comment_count {
        let index = stream.get_u4()?;
        let length = stream.get_u4()?;
        if (index as usize) < count && length as usize > stream.remaining_size() {
            return fail("MS3D: Failure reading comment, length field is out of range");
        }
        stream.inc_ptr(length as i64)?;
    }
    Ok(())
}

/// `CollectChildJoints` for the node named `name`: the joints naming it
/// as parent are counted, then each still unclaimed one is claimed and
/// recursed into before the next. A claim deeper down (joint names can
/// repeat) leaves fewer children than counted, and the rest of the child
/// array uninitialised, freed when the scene is: undefined behaviour.
fn collect_child_joints(joints: &[TempJoint], claimed: &mut [bool], name: &[u8], depth: usize) -> Result<(), ImportError> {
    if depth > MAX_DEPTH {
        return fail("MS3D: joints nested too deep");
    }
    let matches = |index: usize, claimed: &[bool]| !claimed[index] && joints[index].parent_name == name;
    let counted = (0..joints.len()).filter(|&index| matches(index, claimed)).count();
    let mut children = 0;
    for index in 0..joints.len() {
        if matches(index, claimed) {
            children += 1;
            claimed[index] = true;
            collect_child_joints(joints, claimed, &joints[index].name, depth + 1)?;
        }
    }
    if children < counted {
        return fail("undefined behaviour: uninitialised child nodes");
    }
    Ok(())
}

pub fn read(bytes: &[u8]) -> Result<Scene, ImportError> {
    let mut stream = StreamReader::new(bytes, false)?;
    let head = stream.copy_and_advance(10)?;
    let version = stream.get_i4()?;
    if head != b"MS3D000000" {
        return fail("Not a MS3D file, magic string MS3D000000 not found");
    }
    if version != 4 {
        return fail("MS3D: Unsupported file format version, 4 was expected");
    }

    let vertex_count = stream.get_u2()?;
    let mut vertices = Vec::with_capacity(vertex_count as usize);
    for _ in 0..vertex_count {
        stream.inc_ptr(1)?;
        let position = read_vector(&mut stream)?;
        let bone = stream.get_i1()? as i32 as u32;
        // The reference count.
        stream.get_i1()?;
        vertices.push(TempVertex { position, bone_ids: [bone, u32::MAX, u32::MAX, u32::MAX] });
    }

    let triangle_count = stream.get_u2()?;
    let mut triangles = Vec::with_capacity(triangle_count as usize);
    for _ in 0..triangle_count {
        stream.inc_ptr(2)?;
        let mut indices = [0u32; 3];
        for index in &mut indices {
            *index = stream.get_i2()? as i32 as u32;
        }
        let mut normals = [[0.0f32; 3]; 3];
        for normal in &mut normals {
            *normal = read_vector(&mut stream)?;
        }
        let mut uv = [[0.0f32; 2]; 3];
        for corner in &mut uv {
            corner[0] = stream.get_f4()?;
        }
        for corner in &mut uv {
            corner[1] = stream.get_f4()?;
        }
        // Smoothing group and group.
        stream.get_i1()?;
        stream.get_i1()?;
        triangles.push(TempTriangle { indices, normals, uv });
    }

    let group_count = stream.get_u2()?;
    let mut need_default = false;
    let mut groups = Vec::with_capacity(group_count as usize);
    for _ in 0..group_count {
        stream.inc_ptr(1)?;
        stream.copy_and_advance(32)?;
        let count = stream.get_u2()?;
        let mut group_triangles = Vec::with_capacity(count as usize);
        for _ in 0..count {
            group_triangles.push(stream.get_i2()? as i32 as u32);
        }
        let material = stream.get_i1()? as i32 as u32;
        if material == u32::MAX {
            need_default = true;
        }
        groups.push(TempGroup { triangles: group_triangles, material });
    }

    let material_count = stream.get_u2()?;
    for _ in 0..material_count {
        stream.copy_and_advance(32)?;
        // Ambient, diffuse, specular and emissive colours, shininess and
        // transparency.
        for _ in 0..18 {
            stream.get_f4()?;
        }
        stream.inc_ptr(1)?;
        stream.copy_and_advance(128)?;
        stream.copy_and_advance(128)?;
    }

    // Frame rate, current time, frame count.
    stream.get_f4()?;
    stream.get_f4()?;
    stream.get_u4()?;
    let joint_count = stream.get_u2()?;
    let mut joints = Vec::with_capacity(joint_count as usize);
    for _ in 0..joint_count {
        stream.inc_ptr(1)?;
        let name = read_name(&mut stream)?;
        let parent_name = read_name(&mut stream)?;
        read_vector(&mut stream)?;
        read_vector(&mut stream)?;
        // A negative count is a vector length past the maximum.
        let rotation_frames = stream.get_i2()?;
        if rotation_frames < 0 {
            return fail("vector too long");
        }
        let position_frames = stream.get_i2()?;
        if position_frames < 0 {
            return fail("vector too long");
        }
        for _ in 0..rotation_frames as u32 + position_frames as u32 {
            // Time and value.
            for _ in 0..4 {
                stream.get_f4()?;
            }
        }
        joints.push(TempJoint { name, parent_name });
    }

    if stream.remaining_size() > 4 {
        let subversion = stream.get_u4()?;
        if subversion == 1 {
            read_comments(&mut stream, groups.len())?;
            read_comments(&mut stream, material_count as usize)?;
            read_comments(&mut stream, joints.len())?;
            if stream.get_i4()? != 0 {
                let length = stream.get_i4()? as i64 as usize;
                if length > stream.remaining_size() {
                    return fail("MS3D: Model comment is too long");
                }
                // The stream stays at the comment's start.
            }
            if stream.remaining_size() > 4 {
                let extra = stream.get_u4()?;
                if extra > 1 && extra <= 3 {
                    for vertex in &mut vertices {
                        for slot in 1..4 {
                            vertex.bone_ids[slot] = stream.get_i1()? as i32 as u32;
                            // The weight.
                            stream.get_i1()?;
                        }
                        stream.inc_ptr(((extra - 1) << 2) as i64)?;
                    }
                }
            }
        }
    }

    let mut material_total = material_count as u32;
    if need_default && material_total > 0 {
        // "MS3D: Found group with no material assigned, spawning default
        // material"
        material_total += 1;
        for group in &mut groups {
            if group.material == u32::MAX {
                group.material = material_total - 1;
            }
        }
    }
    if groups.is_empty() {
        return fail("MS3D: Didn't get any group records, file is malformed");
    }

    let mut meshes = Vec::with_capacity(groups.len());
    for group in &groups {
        if material_total > 0 && group.material > material_total {
            return fail("MS3D: Encountered invalid material index, file is malformed");
        }
        let vertex_total = group.triangles.len() * 3;
        let mut positions = Vec::with_capacity(vertex_total);
        let mut normals = Vec::with_capacity(vertex_total);
        let mut coords = Vec::with_capacity(vertex_total);
        let mut faces = Vec::with_capacity(group.triangles.len());
        for &triangle_index in &group.triangles {
            let Some(triangle) = triangles.get(triangle_index as usize) else {
                return fail("MS3D: Encountered invalid triangle index, file is malformed");
            };
            let first = positions.len() as u32;
            for corner in 0..3 {
                let Some(vertex) = vertices.get(triangle.indices[corner] as usize) else {
                    return fail("MS3D: Encountered invalid vertex index, file is malformed");
                };
                let bad_bone = vertex.bone_ids.iter().any(|&bone| bone != u32::MAX && bone as usize >= joints.len());
                if bad_bone {
                    return fail("MS3D: Encountered invalid bone index, file is malformed");
                }
                positions.push(vertex.position);
                normals.push(triangle.normals[corner]);
                let [u, v] = triangle.uv[corner];
                coords.push([u, 1.0 - v, 0.0]);
            }
            faces.push(vec![first, first + 1, first + 2]);
        }
        let mut mesh = Mesh {
            primitive_types: PRIMITIVE_TRIANGLE,
            vertex_count: vertex_total,
            vertices: Some(positions),
            normals: Some(normals),
            faces: Some(faces),
            ..Mesh::default()
        };
        mesh.uv_components[0] = 2;
        mesh.texture_coords[0] = Some(coords);
        meshes.push(mesh);
    }

    if !joints.is_empty() {
        // The joint root's name is still empty while its children are
        // collected.
        let mut claimed = vec![false; joints.len()];
        collect_child_joints(&joints, &mut claimed, b"", 0)?;
    }
    Ok(Scene { meshes })
}
